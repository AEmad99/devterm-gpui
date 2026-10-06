//! Live terminal surface.
//!
//! `alacritty_terminal` owns the grid, truecolor, scrollback, selection, and
//! mouse/focus modes. The PTY underneath is still `portable-pty` (in-box
//! ConPTY on Windows). OSC 7 and OSC 133 are read off the same byte stream.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::{fill, point, px, size, Bounds, Hsla, Pixels, SharedString, TextRun, UnderlineStyle};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::logic::osc::OscParser;
use crate::logic::output_ring::{OutputRingBuffer, OutputRingOptions};
use crate::logic::pty::{
    build_pty_base_env, clamp_scrollback, has_real_output, resolve_shell, shell_args,
    should_use_bundled_conpty, startup_failure_message, ShellPref, TERM,
};
use crate::logic::scrollback::trim_session_scrollback;

#[derive(Clone, Copy, Debug)]
pub struct TermSize {
    pub columns: usize,
    pub screen_lines: usize,
    pub history: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines + self.history
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines.max(1)
    }
    fn columns(&self) -> usize {
        self.columns.max(1)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            foreground: 0xc0caf5,
            background: 0x1a1b26,
            cursor: 0xc0caf5,
            selection: 0x33467c,
            ansi: [
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CellSnap {
    pub ch: char,
    pub fg: u32,
    pub bg: u32,
    pub cursor: bool,
    pub selected: bool,
    pub underline: bool,
    pub hidden: bool,
    pub wide_spacer: bool,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<CellSnap>,
}

impl Frame {
    pub fn row_text(&self, row: usize) -> String {
        let start = row * self.cols;
        self.cells[start..start + self.cols]
            .iter()
            .map(|cell| {
                if cell.hidden || cell.wide_spacer {
                    ' '
                } else {
                    cell.ch
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }
}

struct LivePty {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Drop for LivePty {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(pid) = self.child.process_id() {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        let _ = self.child.kill();
    }
}

pub struct TermView {
    term: Term<VoidListener>,
    parser: Processor,
    osc: OscParser,
    ring: OutputRingBuffer,
    raw_tail: String,
    pty: Option<LivePty>,
    shell: String,
    saw_output: bool,
    exited: bool,
    exit_code: Option<i32>,
    cwd: Option<String>,
    marker: Option<&'static str>,
    palette: Palette,
    find_open: bool,
    find_query: String,
    find_hit: Option<(Point, Point)>,
    selecting: bool,
    focused: bool,
    scrollback: usize,
}

impl TermView {
    pub fn open(cols: usize, rows: usize, scrollback: usize) -> Self {
        let scrollback = clamp_scrollback(scrollback as i64) as usize;
        let size = TermSize {
            columns: cols.max(1),
            screen_lines: rows.max(1),
            history: scrollback,
        };
        let mut config = Config::default();
        config.scrolling_history = scrollback;
        Self {
            term: Term::new(config, &size, VoidListener),
            parser: Processor::new(),
            osc: OscParser::new(),
            ring: OutputRingBuffer::new(OutputRingOptions {
                max_lines: Some(scrollback as i64),
                max_bytes: None,
            }),
            raw_tail: String::new(),
            pty: None,
            shell: String::new(),
            saw_output: false,
            exited: false,
            exit_code: None,
            cwd: None,
            marker: None,
            palette: Palette::default(),
            find_open: false,
            find_query: String::new(),
            find_hit: None,
            selecting: false,
            focused: true,
            scrollback,
        }
    }

    pub fn spawn(opts: &SpawnOpts) -> anyhow::Result<(Self, std::sync::mpsc::Receiver<PtyMsg>)> {
        prepare_conpty_search_path();
        let cols = opts.cols.max(1);
        let rows = opts.rows.max(1);
        let mut view = Self::open(cols, rows, opts.scrollback);
        let source: HashMap<String, String> = std::env::vars().collect();
        let platform = host_platform();
        let shell = resolve_shell(
            platform,
            &source,
            Some(&opts.pref),
            opts.explicit_shell.as_deref(),
            |path| Path::new(path).exists(),
        );
        let args: Vec<String> = if let Some(args) = &opts.explicit_args {
            args.clone()
        } else {
            shell_args(&shell).into_iter().map(str::to_string).collect()
        };
        let mut env = build_pty_base_env(&source);
        for (key, value) in &opts.extra_env {
            env.insert(key.clone(), value.clone());
        }
        env.insert("TERM".into(), TERM.into());
        env.insert("COLORTERM".into(), "truecolor".into());

        let system = native_pty_system();
        let pair = system.openpty(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: opts.pixel_width,
            pixel_height: opts.pixel_height,
        })?;
        let mut command = CommandBuilder::new(&shell);
        command.args(&args);
        command.env_clear();
        for (key, value) in &env {
            command.env(key, value);
        }
        if let Some(cwd) = &opts.cwd {
            command.cwd(cwd);
        }
        let child = pair.slave.spawn_command(command)?;
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(PtyMsg::Exit(None));
                        break;
                    }
                    Ok(n) => {
                        if tx.send(PtyMsg::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(PtyMsg::Exit(None));
                        break;
                    }
                }
            }
        });
        view.shell = shell;
        view.pty = Some(LivePty {
            writer: Arc::new(Mutex::new(writer)),
            master: pair.master,
            child,
        });
        Ok((view, rx))
    }

    pub fn shell(&self) -> &str {
        &self.shell
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub fn marker(&self) -> Option<&'static str> {
        self.marker
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        if !self.saw_output && has_real_output(&String::from_utf8_lossy(bytes)) {
            self.saw_output = true;
        }
        if let Ok(text) = std::str::from_utf8(bytes) {
            self.osc.push(text);
            if let Some(cwd) = self.osc.cwd() {
                self.cwd = Some(cwd.to_string());
            }
            self.marker = match self.osc.marker() {
                Some("B") => Some("B"),
                Some("A") => Some("A"),
                _ => self.marker,
            };
            self.ring.append(text);
            self.raw_tail.push_str(text);
            if self.raw_tail.len() > 3 * 1024 * 1024 {
                self.raw_tail =
                    trim_session_scrollback(&self.raw_tail, Some(10_000), Some(2 * 1024 * 1024));
            }
        }
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn note_exit(&mut self, code: Option<i32>) {
        if self.exited {
            return;
        }
        self.exited = true;
        self.exit_code = code;
        self.pty = None;
        let notice = if self.saw_output {
            process_exited_banner(code)
        } else {
            startup_failure_message(&self.shell, code)
        };
        self.saw_output = true;
        self.push_bytes(notice.as_bytes());
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if let Some(pty) = &self.pty {
            if let Ok(mut writer) = pty.writer.lock() {
                let _ = writer.write_all(bytes);
                let _ = writer.flush();
            }
        }
    }

    pub fn paste(&mut self, text: &str) {
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.write(&bytes);
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn focus_changed(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if self.term.mode().contains(TermMode::FOCUS_IN_OUT) {
            self.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    pub fn resize(
        &mut self,
        cols: usize,
        rows: usize,
        pixel_width: u16,
        pixel_height: u16,
    ) -> bool {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if self.term.columns() == cols && self.term.screen_lines() == rows {
            return false;
        }
        let size = TermSize {
            columns: cols,
            screen_lines: rows,
            history: self.scrollback,
        };
        self.term.resize(size);
        if let Some(pty) = self.pty.as_mut() {
            let _ = pty.master.resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width,
                pixel_height,
            });
        }
        true
    }

    pub fn scroll_lines(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    pub fn find_open(&self) -> bool {
        self.find_open
    }

    pub fn find_query(&self) -> &str {
        &self.find_query
    }

    pub fn open_find(&mut self) {
        self.find_open = true;
    }

    pub fn close_find(&mut self) {
        self.find_open = false;
        self.find_hit = None;
    }

    pub fn set_find_query(&mut self, query: String) {
        self.find_query = query;
        self.find_hit = None;
        self.find_next(true);
    }

    pub fn find_next(&mut self, forward: bool) -> bool {
        let query = self.find_query.clone();
        if query.is_empty() {
            self.find_hit = None;
            return false;
        }
        let pattern = if RegexSearch::new(&query).is_ok() {
            query
        } else {
            escape_regex(&query)
        };
        let Ok(mut regex) = RegexSearch::new(&pattern) else {
            return false;
        };
        let direction = if forward {
            Direction::Right
        } else {
            Direction::Left
        };
        let origin = self.find_hit.map(|(_, end)| end).unwrap_or_else(|| {
            Point::new(Line(-(self.term.grid().history_size() as i32)), Column(0))
        });
        let Some(hit) =
            self.term
                .search_next(&mut regex, origin, direction, Direction::Right, None)
        else {
            return false;
        };
        let start = *hit.start();
        let end = *hit.end();
        self.find_hit = Some((start, end));
        let screen = self.term.screen_lines() as i32;
        let target = start.line.0;
        let top = -(self.term.grid().display_offset() as i32);
        let bottom = top + screen - 1;
        if target < top || target > bottom {
            let desired = (-target).max(0) as usize;
            let current = self.term.grid().display_offset();
            let delta = desired as i32 - current as i32;
            if delta != 0 {
                self.term.scroll_display(Scroll::Delta(delta));
            }
        }
        true
    }

    pub fn mouse(
        &mut self,
        button: MouseButton,
        col: usize,
        row: usize,
        pressed: bool,
        motion: bool,
    ) -> bool {
        let mode = *self.term.mode();
        if mode.intersects(TermMode::MOUSE_MODE) {
            let report = mouse_report(mode, button, col, row, pressed, motion);
            if !report.is_empty() {
                self.write(&report);
                return true;
            }
            // Click tracking owns the button. A move with the button up must
            // not fall through into text selection.
            if !motion && mode.contains(TermMode::MOUSE_REPORT_CLICK) {
                return true;
            }
        }
        // Motion only extends a drag that is already in progress. Passing
        // `pressed = false` for a hover used to take the button-up path and
        // keep moving the selection end, so sweeping the pointer highlighted
        // the pane.
        if motion {
            if self.selecting {
                self.update_selection(col, row);
            }
            return self.selecting;
        }
        if pressed && button == MouseButton::Left {
            self.selecting = true;
            let point = self.viewport_point(col, row);
            self.term.selection = Some(Selection::new(
                SelectionType::Simple,
                point,
                Direction::Left,
            ));
            return true;
        }
        if !pressed && button == MouseButton::Left && self.selecting {
            self.update_selection(col, row);
            self.selecting = false;
            if self
                .selection_text()
                .map(|text| text.trim().is_empty())
                .unwrap_or(true)
            {
                self.term.selection = None;
            }
            return true;
        }
        false
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
        self.selecting = false;
    }

    pub fn restore_tail(&self) -> String {
        trim_session_scrollback(&self.raw_tail, Some(2_000), Some(2 * 1024 * 1024))
    }

    pub fn frame(&self) -> Frame {
        let content = self.term.renderable_content();
        let cols = self.term.columns();
        let mut rows: Vec<Vec<CellSnap>> = Vec::new();
        let mut current_line: Option<i32> = None;
        let cursor = content.cursor.point;
        let selection = content.selection;
        let find = self.find_hit;
        for indexed in content.display_iter {
            let line = indexed.point.line.0;
            if current_line != Some(line) {
                rows.push(Vec::with_capacity(cols));
                current_line = Some(line);
            }
            let cell = indexed.cell;
            let selected = selection_contains(selection.as_ref(), indexed.point)
                || find_contains(find, indexed.point);
            let (mut fg, mut bg) =
                resolve_pair(cell.fg, cell.bg, cell.flags, &self.palette, &content.colors);
            if selected {
                bg = self.palette.selection;
            }
            let cursor_here = indexed.point == cursor
                && content.cursor.shape != alacritty_terminal::vte::ansi::CursorShape::Hidden
                && content.display_offset == 0
                || (indexed.point == cursor && content.display_offset > 0 && cursor.line.0 < 0);
            if indexed.point == cursor
                && cursor_is_visible(cursor, content.display_offset, self.term.screen_lines())
            {
                fg = self.palette.background;
                bg = self.palette.cursor;
            }
            let _ = cursor_here;
            rows.last_mut().unwrap().push(CellSnap {
                ch: cell.c,
                fg,
                bg,
                cursor: indexed.point == cursor
                    && cursor_is_visible(cursor, content.display_offset, self.term.screen_lines()),
                selected,
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                hidden: cell.flags.contains(Flags::HIDDEN),
                wide_spacer: cell.flags.contains(Flags::WIDE_CHAR_SPACER),
            });
        }
        let row_count = rows.len();
        let mut cells = Vec::with_capacity(row_count * cols);
        for row in rows {
            let mut row = row;
            row.truncate(cols);
            while row.len() < cols {
                row.push(CellSnap {
                    ch: ' ',
                    fg: self.palette.foreground,
                    bg: self.palette.background,
                    cursor: false,
                    selected: false,
                    underline: false,
                    hidden: false,
                    wide_spacer: false,
                });
            }
            cells.extend(row);
        }
        Frame {
            cols,
            rows: row_count,
            cells,
        }
    }

    fn viewport_point(&self, col: usize, row: usize) -> Point {
        let top = -(self.term.grid().display_offset() as i32);
        let line = top + row as i32;
        let col = col.min(self.term.columns().saturating_sub(1));
        Point::new(Line(line), Column(col))
    }

    fn update_selection(&mut self, col: usize, row: usize) {
        let point = self.viewport_point(col, row);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Direction::Right);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

pub enum PtyMsg {
    Data(Vec<u8>),
    Exit(Option<i32>),
}

pub struct SpawnOpts {
    pub pref: ShellPref,
    pub explicit_shell: Option<String>,
    pub explicit_args: Option<Vec<String>>,
    pub cwd: Option<PathBuf>,
    pub cols: usize,
    pub rows: usize,
    pub pixel_width: u16,
    pub pixel_height: u16,
    pub scrollback: usize,
    pub extra_env: HashMap<String, String>,
}

impl Default for SpawnOpts {
    fn default() -> Self {
        Self {
            pref: ShellPref::Auto,
            explicit_shell: None,
            explicit_args: None,
            cwd: std::env::current_dir().ok(),
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
            scrollback: 10_000,
            extra_env: HashMap::new(),
        }
    }
}

pub fn host_platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// In-box ConPTY is the default. The bundled `conpty.dll` / `OpenConsole.exe`
/// pair is loaded only when `DEVTERM_USE_BUNDLED_CONPTY=1` and the DLL sits
/// beside the executable. `portable-pty` prefers a sideloaded `conpty.dll`
/// when the loader can see it, so the opt-in adds that directory and the
/// default path leaves the loader on `kernel32`.
pub fn prepare_conpty_search_path() {
    let opt_in = std::env::var("DEVTERM_USE_BUNDLED_CONPTY").ok();
    #[cfg(windows)]
    let bundled = bundled_conpty_dll();
    #[cfg(not(windows))]
    let bundled: Option<PathBuf> = None;
    if should_use_bundled_conpty(host_platform(), bundled.is_some(), opt_in.as_deref()) {
        #[cfg(windows)]
        if let Some(dll) = bundled {
            if let Some(dir) = dll.parent() {
                set_dll_directory(dir);
            }
        }
    }
}

#[cfg(windows)]
fn bundled_conpty_dll() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("conpty").join("conpty.dll");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(windows)]
fn set_dll_directory(dir: &Path) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    extern "system" {
        fn SetDllDirectoryW(path: *const u16) -> i32;
    }
    unsafe {
        SetDllDirectoryW(wide.as_ptr());
    }
}

fn process_exited_banner(code: Option<i32>) -> String {
    let code = match code {
        Some(code) => format!(" with code {code}"),
        None => String::new(),
    };
    format!("\u{1b}[?1049l\u{1b}[?1000l\u{1b}[?1002l\u{1b}[?1003l\u{1b}[?1006l\u{1b}[?2004l\u{1b}[?1004l\u{1b}[0m\u{1b}[?25h\r\n\u{1b}[90m[process exited{code}]\u{1b}[0m\r\n")
}

fn cursor_is_visible(cursor: Point, display_offset: usize, screen_lines: usize) -> bool {
    let top = -(display_offset as i32);
    let bottom = top + screen_lines as i32 - 1;
    cursor.line.0 >= top && cursor.line.0 <= bottom
}

fn selection_contains(
    selection: Option<&alacritty_terminal::selection::SelectionRange>,
    point: Point,
) -> bool {
    let Some(range) = selection else {
        return false;
    };
    if point.line < range.start.line || point.line > range.end.line {
        return false;
    }
    if range.start.line == range.end.line {
        return point.column >= range.start.column && point.column < range.end.column;
    }
    if point.line == range.start.line {
        return point.column >= range.start.column;
    }
    if point.line == range.end.line {
        return point.column < range.end.column;
    }
    true
}

fn find_contains(hit: Option<(Point, Point)>, point: Point) -> bool {
    let Some((start, end)) = hit else {
        return false;
    };
    if point.line < start.line || point.line > end.line {
        return false;
    }
    if start.line == end.line {
        return point.column >= start.column && point.column <= end.column;
    }
    if point.line == start.line {
        return point.column >= start.column;
    }
    if point.line == end.line {
        return point.column <= end.column;
    }
    true
}

fn resolve_pair(
    fg: Color,
    bg: Color,
    flags: Flags,
    palette: &Palette,
    colors: &alacritty_terminal::term::color::Colors,
) -> (u32, u32) {
    let mut fg = resolve_color(fg, palette, colors, true);
    let mut bg = resolve_color(bg, palette, colors, false);
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if flags.contains(Flags::DIM) {
        fg = dim(fg);
    }
    (fg, bg)
}

fn resolve_color(
    color: Color,
    palette: &Palette,
    colors: &alacritty_terminal::term::color::Colors,
    foreground: bool,
) -> u32 {
    match color {
        Color::Spec(Rgb { r, g, b }) => ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
        Color::Indexed(index) => {
            if let Some(rgb) = colors[index as usize].as_ref() {
                return rgb_u32(*rgb);
            }
            indexed_color(index, palette)
        }
        Color::Named(named) => {
            if let Some(rgb) = colors[named].as_ref() {
                return rgb_u32(*rgb);
            }
            match named {
                NamedColor::Foreground | NamedColor::BrightForeground => palette.foreground,
                NamedColor::Background => palette.background,
                NamedColor::Cursor => palette.cursor,
                NamedColor::Black => palette.ansi[0],
                NamedColor::Red => palette.ansi[1],
                NamedColor::Green => palette.ansi[2],
                NamedColor::Yellow => palette.ansi[3],
                NamedColor::Blue => palette.ansi[4],
                NamedColor::Magenta => palette.ansi[5],
                NamedColor::Cyan => palette.ansi[6],
                NamedColor::White => palette.ansi[7],
                NamedColor::BrightBlack | NamedColor::DimBlack => palette.ansi[8],
                NamedColor::BrightRed | NamedColor::DimRed => palette.ansi[9],
                NamedColor::BrightGreen | NamedColor::DimGreen => palette.ansi[10],
                NamedColor::BrightYellow | NamedColor::DimYellow => palette.ansi[11],
                NamedColor::BrightBlue | NamedColor::DimBlue => palette.ansi[12],
                NamedColor::BrightMagenta | NamedColor::DimMagenta => palette.ansi[13],
                NamedColor::BrightCyan | NamedColor::DimCyan => palette.ansi[14],
                NamedColor::BrightWhite | NamedColor::DimWhite => palette.ansi[15],
                _ => {
                    if foreground {
                        palette.foreground
                    } else {
                        palette.background
                    }
                }
            }
        }
    }
}

fn rgb_u32(rgb: Rgb) -> u32 {
    ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32
}

fn indexed_color(index: u8, palette: &Palette) -> u32 {
    match index {
        0..=15 => palette.ansi[index as usize],
        16..=231 => {
            let i = index - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            let level = |n: u8| if n == 0 { 0 } else { 55 + 40 * n as u32 };
            (level(r) << 16) | (level(g) << 8) | level(b)
        }
        232..=255 => {
            let v = 8 + (index - 232) as u32 * 10;
            (v << 16) | (v << 8) | v
        }
    }
}

fn dim(color: u32) -> u32 {
    let r = ((color >> 16) & 0xff) / 2;
    let g = ((color >> 8) & 0xff) / 2;
    let b = (color & 0xff) / 2;
    (r << 16) | (g << 8) | b
}

fn mouse_report(
    mode: TermMode,
    button: MouseButton,
    col: usize,
    row: usize,
    pressed: bool,
    motion: bool,
) -> Vec<u8> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return Vec::new();
    }
    if motion && !mode.contains(TermMode::MOUSE_MOTION) && !mode.contains(TermMode::MOUSE_DRAG) {
        return Vec::new();
    }
    let mut cb = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
    };
    if motion {
        cb += 32;
    }
    let x = col as u32 + 1;
    let y = row as u32 + 1;
    if mode.contains(TermMode::SGR_MOUSE) {
        let tail = if pressed && button != MouseButton::None {
            'M'
        } else {
            'm'
        };
        format!("\u{1b}[<{cb};{x};{y}{tail}").into_bytes()
    } else if x < 223 && y < 223 {
        let mut code = cb + 32;
        if !pressed {
            code = 3 + 32;
        }
        vec![
            0x1b,
            b'[',
            b'M',
            code as u8,
            (x as u8).saturating_add(32),
            (y as u8).saturating_add(32),
        ]
    } else {
        Vec::new()
    }
}

pub fn terminal_font() -> gpui::Font {
    gpui::Font {
        family: "DejaVu Sans Mono".into(),
        features: gpui::FontFeatures::default(),
        fallbacks: Some(gpui::FontFallbacks(Arc::new(vec![
            "Cascadia Mono".into(),
            "Cascadia Code".into(),
            "Consolas".into(),
            "Menlo".into(),
            "monospace".into(),
        ]))),
        weight: gpui::FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
    }
}

/// Cell size from the real font metrics, not a fixed 7.8×18 guess.
///
/// Prefer the width of a shaped `"0"`. `ch_advance` on this GPUI build can
/// come back many times larger than the glyphs that actually paint, which
/// stretches one prompt across the pane. Reject a cell wider than twice the
/// font size.
pub fn measure_cell(window: &gpui::Window, font_px: f32) -> (Pixels, Pixels) {
    let font = terminal_font();
    let size = px(font_px);
    let id = window.text_system().resolve_font(&font);
    let shaped = window.text_system().shape_line(
        SharedString::from("0"),
        size,
        &[TextRun {
            len: 1,
            font: font.clone(),
            color: gpui::rgb(0xffffff).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let mut width = shaped.width;
    if !(1.0..font_px * 2.0).contains(&f32::from(width)) {
        width = window
            .text_system()
            .ch_advance(id, size)
            .unwrap_or(px(font_px * 0.6));
    }
    if !(1.0..font_px * 2.0).contains(&f32::from(width)) {
        width = px(font_px * 0.6);
    }
    let mut height = shaped.ascent + shaped.descent;
    if !(font_px * 0.8..font_px * 2.5).contains(&f32::from(height)) {
        height = window.text_system().ascent(id, size) + window.text_system().descent(id, size);
    }
    if !(font_px * 0.8..font_px * 2.5).contains(&f32::from(height)) {
        height = px(font_px * 1.35);
    }
    (width, height)
}

pub fn paint_frame(
    frame: &Frame,
    origin: gpui::Point<Pixels>,
    cell_w: Pixels,
    cell_h: Pixels,
    font_px: f32,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) {
    if frame.cols == 0 || frame.rows == 0 {
        return;
    }
    let font = terminal_font();
    let font_size = px(font_px);
    for row in 0..frame.rows {
        let y = origin.y + cell_h * row as f32;
        let mut col = 0;
        while col < frame.cols {
            let cell = frame.cells[row * frame.cols + col];
            let mut end = col + 1;
            while end < frame.cols && frame.cells[row * frame.cols + end].bg == cell.bg {
                end += 1;
            }
            window.paint_quad(fill(
                Bounds {
                    origin: point(origin.x + cell_w * col as f32, y),
                    size: size(cell_w * (end - col) as f32, cell_h),
                },
                gpui::rgb(cell.bg),
            ));
            let mut cursor = col;
            while cursor < end {
                let here = frame.cells[row * frame.cols + cursor];
                if here.wide_spacer || here.hidden {
                    cursor += 1;
                    continue;
                }
                let start = cursor;
                let mut text = String::new();
                while cursor < end {
                    let next = frame.cells[row * frame.cols + cursor];
                    if next.fg != here.fg
                        || next.underline != here.underline
                        || next.wide_spacer
                        || next.hidden
                    {
                        break;
                    }
                    text.push(next.ch);
                    cursor += 1;
                }
                if text.trim().is_empty() && !here.underline {
                    continue;
                }
                let color: Hsla = gpui::rgb(here.fg).into();
                let underline = here.underline.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: Some(color),
                    wavy: false,
                });
                // Paint each glyph in its own cell. Forcing the whole run to
                // `cols * cell_w` letter-spaces the prompt across the pane when
                // the shaper's advance does not match the cell.
                for (offset, ch) in text.chars().enumerate() {
                    if ch == ' ' {
                        continue;
                    }
                    let column = start + offset;
                    let glyph = ch.to_string();
                    let run = TextRun {
                        len: glyph.len(),
                        font: font.clone(),
                        color,
                        background_color: None,
                        underline: underline.clone(),
                        strikethrough: None,
                    };
                    let shaped = window.text_system().shape_line(
                        SharedString::from(glyph),
                        font_size,
                        &[run],
                        None,
                    );
                    let _ = shaped.paint(
                        point(origin.x + cell_w * column as f32, y),
                        cell_h,
                        window,
                        cx,
                    );
                }
            }
            col = end;
        }
    }
}

fn escape_regex(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(text: &str) -> TermView {
        let mut view = TermView::open(8, 4, 100);
        view.push_bytes(text.as_bytes());
        view
    }

    #[test]
    fn writes_text_and_moves_to_the_next_line() {
        let view = feed("hi\r\nthere");
        let frame = view.frame();
        assert_eq!(frame.row_text(0), "hi");
        assert_eq!(frame.row_text(1), "there");
    }

    #[test]
    fn truecolor_is_stored_on_the_cell() {
        let view = feed("\u{1b}[38;2;1;2;3mX");
        let frame = view.frame();
        assert_eq!(frame.cells[0].ch, 'X');
        assert_eq!(frame.cells[0].fg, 0x010203);
    }

    #[test]
    fn osc_7_and_133_are_recorded() {
        let mut view = TermView::open(40, 6, 100);
        view.push_bytes(b"\x1b]7;file:///home/ada\x07\x1b]133;A\x07");
        assert_eq!(view.cwd(), Some("/home/ada"));
        assert_eq!(view.marker(), Some("A"));
        view.push_bytes(b"\x1b]133;B\x07");
        assert_eq!(view.marker(), Some("B"));
    }

    #[test]
    fn find_jumps_to_the_match() {
        let mut view = feed("alpha\r\nbeta\r\ngamma");
        view.set_find_query("beta".into());
        assert!(view.find_hit.is_some());
        let frame = view.frame();
        let hit = frame
            .cells
            .iter()
            .any(|cell| cell.selected && cell.ch == 'b');
        assert!(hit);
    }

    #[test]
    fn bracketed_paste_wraps_when_the_mode_is_on() {
        let mut view = TermView::open(20, 4, 100);
        view.push_bytes(b"\x1b[?2004h");
        assert!(view.term.mode().contains(TermMode::BRACKETED_PASTE));
    }

    #[test]
    fn mouse_tracking_and_focus_reports() {
        let mut view = TermView::open(20, 4, 100);
        view.push_bytes(b"\x1b[?1000h\x1b[?1006h");
        let report = mouse_report(*view.term.mode(), MouseButton::Left, 0, 0, true, false);
        assert_eq!(report, b"\x1b[<0;1;1M");
        view.push_bytes(b"\x1b[?1004h");
        assert!(view.term.mode().contains(TermMode::FOCUS_IN_OUT));
    }

    #[test]
    fn moving_the_pointer_does_not_highlight_the_pane() {
        let mut view = TermView::open(20, 6, 100);
        view.push_bytes(b"hello world\r\nsecond line\r\n");
        assert!(view.mouse(MouseButton::Left, 1, 0, false, true) == false);
        assert!(view.mouse(MouseButton::Left, 10, 1, false, true) == false);
        let highlighted = view
            .frame()
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .count();
        assert_eq!(highlighted, 0);

        view.mouse(MouseButton::Left, 0, 0, true, false);
        view.mouse(MouseButton::Left, 4, 0, true, true);
        let dragged = view
            .frame()
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .count();
        assert!(dragged > 0, "a held drag should select cells");
        view.mouse(MouseButton::Left, 4, 0, false, false);
        view.mouse(MouseButton::Left, 12, 1, false, true);
        let after = view
            .frame()
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .count();
        assert_eq!(after, dragged);
    }

    #[test]
    fn startup_failure_uses_the_original_diagnostic() {
        let mut view = TermView::open(80, 8, 100);
        view.shell = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into();
        view.note_exit(Some(1));
        let tail = view.restore_tail();
        assert!(tail.contains("Windows PowerShell failed to start"));
        assert!(tail.contains("Exit code: 1"));
    }
}
