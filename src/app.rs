//! DevTerm's window, drawn with GPUI.
//!
//! The chrome follows the Electron app: a 40px icon rail, an optional library
//! column, the group bar, pane tabs over the terminal, and a 22px status bar.
//! Local shells are real PTYs. SSH, the editor, settings, and agent launch
//! call the ported DevTerm logic directly.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rgba, size, AnyElement, App, Bounds, Context, FocusHandle, Focusable,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use crate::editor::{self, EditorDoc};
use crate::icons::{file_kind, glyph, Icon};
use crate::logic::hotkeys::{match_hotkey, KeyEvent};
use crate::logic::settings::AppSettings;
use crate::persist;
use crate::ssh_config::{parse_ssh_config, SshHost};
use crate::term_view::{self, PtyMsg, SpawnOpts, TermView};
use crate::theme;

const SNIPPETS: &[(&str, &str, &str)] = &[
    ("List files", "ls -la", "ls -la\n"),
    ("Git status", "git status", "git status\n"),
    ("Working directory", "pwd", "pwd\n"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Library {
    Files,
    Connections,
    Workspaces,
    Snippets,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modal {
    Settings,
    Shortcuts,
    Palette,
}

struct Session {
    title: String,
    view: TermView,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    remote: Option<std::sync::mpsc::Sender<Vec<u8>>>,
}

#[derive(Clone)]
struct Pane {
    tabs: Vec<u64>,
    active: u64,
}

#[derive(Clone)]
struct Group {
    id: u64,
    name: String,
    panes: Vec<Pane>,
    active_pane: usize,
}

struct FileRow {
    name: String,
    dir: bool,
}

struct GitSnapshot {
    branch: String,
    changes: Vec<String>,
}

pub struct Shell {
    focus: FocusHandle,
    library: Option<Library>,
    git_open: bool,
    agent_open: bool,
    activity_open: bool,
    transfers_open: bool,
    dragging: bool,
    focus_mode: bool,
    zen_mode: bool,
    settings: AppSettings,
    file_filter: String,
    filter_focus: bool,
    editor: Option<EditorDoc>,
    editor_active: bool,
    agent_child: Option<std::process::Child>,
    welcome_dismissed: bool,
    ssh_done: bool,
    agent_done: bool,
    modal: Option<Modal>,
    status: String,
    next_id: u64,
    home_group: u64,
    active_group: u64,
    groups: Vec<Group>,
    sessions: std::collections::HashMap<u64, Session>,
    files_cwd: PathBuf,
    file_rows: Vec<FileRow>,
    ssh_hosts: Vec<SshHost>,
    git: Option<GitSnapshot>,
    work_dir: PathBuf,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let work_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let settings = persist::load_settings();
        let welcome_seen = settings.welcome_hint_seen;
        let zen = settings.zen_mode;
        theme::set_id(&settings.theme_id);
        let mut shell = Self {
            focus: cx.focus_handle(),
            library: None,
            git_open: false,
            agent_open: false,
            activity_open: false,
            transfers_open: false,
            dragging: false,
            focus_mode: false,
            zen_mode: zen,
            settings,
            file_filter: String::new(),
            filter_focus: false,
            editor: None,
            editor_active: false,
            agent_child: None,
            welcome_dismissed: welcome_seen,
            ssh_done: false,
            agent_done: false,
            modal: None,
            status: "Ready".into(),
            next_id: 1,
            home_group: 1,
            active_group: 1,
            groups: Vec::new(),
            sessions: std::collections::HashMap::new(),
            files_cwd: work_dir.clone(),
            file_rows: Vec::new(),
            ssh_hosts: Vec::new(),
            git: git_snapshot(&work_dir),
            work_dir,
        };
        let group_id = shell.alloc();
        shell.home_group = group_id;
        shell.active_group = group_id;
        let session = shell.spawn_session(cx);
        shell.groups.push(Group {
            id: group_id,
            name: "Group 1".into(),
            panes: vec![Pane {
                tabs: vec![session],
                active: session,
            }],
            active_pane: 0,
        });
        shell.reload_files();
        shell.reload_ssh();
        shell
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn spawn_session(&mut self, cx: &mut Context<Self>) -> u64 {
        let id = self.alloc();
        let title = shell_title();
        match TermView::spawn(&SpawnOpts::default()) {
            Ok((view, rx)) => {
                self.sessions.insert(
                    id,
                    Session {
                        title,
                        view,
                        origin_x: 0.0,
                        origin_y: 0.0,
                        cell_w: 8.0,
                        cell_h: 16.0,
                        remote: None,
                    },
                );
                pump_pty(id, rx, cx);
            }
            Err(err) => {
                let mut view = TermView::open(80, 24, 10_000);
                let message = format!("Could not open a local shell: {err}\r\n");
                view.push_bytes(message.as_bytes());
                view.note_exit(None);
                self.sessions.insert(
                    id,
                    Session {
                        title,
                        view,
                        origin_x: 0.0,
                        origin_y: 0.0,
                        cell_w: 8.0,
                        cell_h: 16.0,
                        remote: None,
                    },
                );
                self.status = "Local shell failed".into();
            }
        }
        if let Some(session) = self.sessions.get_mut(&id) {
            session.view.set_palette(theme::terminal_palette());
        }
        id
    }

    fn active_group_mut(&mut self) -> Option<&mut Group> {
        let id = self.active_group;
        self.groups.iter_mut().find(|group| group.id == id)
    }

    fn new_tab(&mut self, cx: &mut Context<Self>) {
        let session = self.spawn_session(cx);
        let Some(group) = self.active_group_mut() else {
            return;
        };
        if group.panes.is_empty() {
            group.panes.push(Pane {
                tabs: vec![session],
                active: session,
            });
            group.active_pane = 0;
        } else {
            let index = group.active_pane.min(group.panes.len() - 1);
            group.active_pane = index;
            let pane = &mut group.panes[index];
            pane.tabs.push(session);
            pane.active = session;
        }
        self.status = "Opened a local terminal".into();
    }

    fn start_remote(&mut self, host: &str, user: &str, port: u16, cx: &mut Context<Self>) {
        let link = crate::ssh_session::connect(host, user, port);
        let id = self.alloc();
        let mut view = TermView::open(80, 24, self.settings.prefs.scrollback as usize);
        view.set_palette(theme::terminal_palette());
        self.sessions.insert(
            id,
            Session {
                title: format!("{user}@{host}"),
                view,
                origin_x: 0.0,
                origin_y: 0.0,
                cell_w: 8.0,
                cell_h: 16.0,
                remote: Some(link.input),
            },
        );
        if let Some(group) = self.active_group_mut() {
            if group.panes.is_empty() {
                group.panes.push(Pane {
                    tabs: vec![id],
                    active: id,
                });
                group.active_pane = 0;
            } else {
                let index = group.active_pane.min(group.panes.len() - 1);
                let pane = &mut group.panes[index];
                pane.tabs.push(id);
                pane.active = id;
            }
        }
        pump_pty(id, link.output, cx);
        self.ssh_done = true;
        self.settings.first_run.imported_ssh = true;
        let _ = persist::save_settings(&self.settings);
        self.status = format!("Connecting to {user}@{host}:{port}");
    }

    fn split_right(&mut self, cx: &mut Context<Self>) {
        let session = self.spawn_session(cx);
        let Some(group) = self.active_group_mut() else {
            return;
        };
        group.panes.push(Pane {
            tabs: vec![session],
            active: session,
        });
        group.active_pane = group.panes.len() - 1;
        self.status = "Split the pane to the right".into();
    }

    fn new_group(&mut self, cx: &mut Context<Self>) {
        let id = self.alloc();
        let session = self.spawn_session(cx);
        let name = format!("Group {}", self.groups.len() + 1);
        self.groups.push(Group {
            id,
            name,
            panes: vec![Pane {
                tabs: vec![session],
                active: session,
            }],
            active_pane: 0,
        });
        self.active_group = id;
        self.status = "Opened a new group".into();
    }

    fn close_session(&mut self, id: u64) {
        self.sessions.remove(&id);
        for group in &mut self.groups {
            for pane in &mut group.panes {
                pane.tabs.retain(|tab| *tab != id);
                if !pane.tabs.is_empty() && !pane.tabs.contains(&pane.active) {
                    pane.active = *pane.tabs.last().unwrap();
                }
            }
            group.panes.retain(|pane| !pane.tabs.is_empty());
            if group.active_pane >= group.panes.len() {
                group.active_pane = group.panes.len().saturating_sub(1);
            }
        }
        self.status = "Closed terminal".into();
    }

    fn close_group(&mut self, id: u64) {
        if id == self.home_group {
            self.status = "The home group stays open".into();
            return;
        }
        let Some(index) = self.groups.iter().position(|group| group.id == id) else {
            return;
        };
        let group = self.groups.remove(index);
        for pane in group.panes {
            for tab in pane.tabs {
                self.sessions.remove(&tab);
            }
        }
        if self.active_group == id {
            self.active_group = self.home_group;
        }
    }

    fn select_tab(&mut self, pane_index: usize, session: u64) {
        self.editor_active = false;
        if let Some(group) = self.active_group_mut() {
            if let Some(pane) = group.panes.get_mut(pane_index) {
                if pane.tabs.contains(&session) {
                    pane.active = session;
                    group.active_pane = pane_index;
                }
            }
        }
    }

    fn write_active(&mut self, bytes: &[u8]) {
        let Some(id) = self.active_session_id() else {
            return;
        };
        if let Some(session) = self.sessions.get_mut(&id) {
            if let Some(remote) = &session.remote {
                let _ = remote.send(bytes.to_vec());
            } else {
                session.view.write(bytes);
            }
        }
    }

    fn note_size(
        &mut self,
        id: u64,
        cols: usize,
        rows: usize,
        pixel_width: u16,
        pixel_height: u16,
        origin_x: f32,
        origin_y: f32,
        cell_w: f32,
        cell_h: f32,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(&id) else {
            return false;
        };
        session.origin_x = origin_x;
        session.origin_y = origin_y;
        session.cell_w = cell_w;
        session.cell_h = cell_h;
        session.view.resize(cols, rows, pixel_width, pixel_height)
    }

    fn reload_files(&mut self) {
        self.file_rows = list_dir(&self.files_cwd);
    }

    fn reload_ssh(&mut self) {
        self.ssh_hosts = load_ssh_hosts();
        if !self.ssh_hosts.is_empty() {
            self.ssh_done = true;
        }
    }

    fn toggle_library(&mut self, library: Library) {
        self.library = if self.library == Some(library) {
            None
        } else {
            match library {
                Library::Files => self.reload_files(),
                Library::Connections => self.reload_ssh(),
                Library::Workspaces | Library::Snippets => {}
            }
            Some(library)
        };
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.to_ascii_lowercase();
        let mods = &event.keystroke.modifiers;
        if self.modal.is_some() {
            if key == "escape" {
                self.modal = None;
                cx.notify();
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if mods.control && !mods.alt && !mods.platform {
            let handled = if mods.shift {
                match key.as_str() {
                    "t" => {
                        self.new_tab(cx);
                        true
                    }
                    "n" => {
                        self.new_group(cx);
                        true
                    }
                    "e" => {
                        self.toggle_library(Library::Files);
                        true
                    }
                    "w" => {
                        if let Some(id) = self.active_session_id() {
                            self.close_session(id);
                        }
                        true
                    }
                    _ => false,
                }
            } else {
                match key.as_str() {
                    "k" => {
                        self.modal = Some(Modal::Palette);
                        true
                    }
                    "," => {
                        self.modal = Some(Modal::Settings);
                        true
                    }
                    "/" => {
                        self.modal = Some(Modal::Shortcuts);
                        true
                    }
                    _ => false,
                }
            };
            if handled {
                cx.notify();
                window.prevent_default();
                cx.stop_propagation();
                return;
            }
        }
        if mods.control && mods.alt && !mods.shift && key == "g" {
            self.git_open = !self.git_open;
            if self.git_open {
                self.git = git_snapshot(&self.work_dir);
            }
            cx.notify();
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        let hotkey = KeyEvent {
            ctrl: mods.control,
            meta: mods.platform,
            shift: mods.shift,
            alt: mods.alt,
            key: hotkey_name(&key),
        };
        if let Some(id) = match_hotkey(&hotkey, None) {
            if self.handle_hotkey(id, cx) {
                cx.notify();
                window.prevent_default();
                cx.stop_propagation();
                return;
            }
        }
        if self.filter_focus {
            self.filter_key(&key, event);
            window.prevent_default();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.editor_active {
            if self.editor_key(&key, event) {
                window.prevent_default();
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        if self.find_captures_keys() {
            self.find_key(&key, event);
            window.prevent_default();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if let Some(bytes) = pty_bytes(event) {
            self.write_active(&bytes);
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    fn handle_hotkey(
        &mut self,
        id: crate::logic::hotkeys::HotkeyId,
        cx: &mut Context<Self>,
    ) -> bool {
        use crate::logic::hotkeys::HotkeyId::*;
        match id {
            Palette | PaletteAlt => self.modal = Some(Modal::Palette),
            NewTerminal => self.new_tab(cx),
            NewGroup => self.new_group(cx),
            CloseTerminal => {
                if let Some(session) = self.active_session_id() {
                    self.close_session(session);
                }
            }
            ToggleSidebar => self.toggle_library(Library::Files),
            Find => {
                if let Some(session) = self.active_session_id() {
                    if let Some(view) = self.sessions.get_mut(&session) {
                        view.view.open_find();
                    }
                }
            }
            Settings => self.modal = Some(Modal::Settings),
            Shortcuts => self.modal = Some(Modal::Shortcuts),
            ToggleGit => {
                self.git_open = !self.git_open;
                if self.git_open {
                    self.git = git_snapshot(&self.work_dir);
                }
            }
            SplitRight => self.split_right(cx),
            ToggleFocus => {
                self.focus_mode = !self.focus_mode;
                if self.focus_mode {
                    self.zen_mode = false;
                }
            }
            ToggleZenMode => {
                self.zen_mode = !self.zen_mode;
                self.settings.zen_mode = self.zen_mode;
                crate::logic::settings::set_zen_mode(&mut self.settings, self.zen_mode);
                let _ = persist::save_settings(&self.settings);
            }
            SaveEditor => {
                if let Some(doc) = self.editor.as_mut() {
                    match editor::save_file(doc) {
                        Ok(()) => {
                            doc.dirty = false;
                            self.status = format!("Saved {}", doc.path.display());
                        }
                        Err(err) => self.status = err,
                    }
                }
            }
            PreviewMarkdown => {
                if let Some(doc) = self.editor.as_mut() {
                    editor::cycle_preview(doc);
                }
            }
            _ => return false,
        }
        true
    }

    fn find_captures_keys(&self) -> bool {
        self.active_session_id()
            .and_then(|id| self.sessions.get(&id))
            .map(|session| session.view.find_open())
            .unwrap_or(false)
    }

    fn find_key(&mut self, key: &str, event: &KeyDownEvent) {
        let Some(id) = self.active_session_id() else {
            return;
        };
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        match key {
            "escape" => session.view.close_find(),
            "enter" | "return" => {
                let _ = session.view.find_next(!event.keystroke.modifiers.shift);
            }
            "backspace" => {
                let mut query = session.view.find_query().to_string();
                query.pop();
                session.view.set_find_query(query);
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    if event.keystroke.modifiers.control
                        || event.keystroke.modifiers.alt
                        || event.keystroke.modifiers.platform
                    {
                        return;
                    }
                    let mut query = session.view.find_query().to_string();
                    query.push_str(text);
                    session.view.set_find_query(query);
                }
            }
        }
    }

    fn pointer(&mut self, id: u64, x: f32, y: f32, down: bool, motion: bool) -> bool {
        if motion && !self.dragging {
            return false;
        }
        if !motion {
            self.dragging = down;
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return false;
        };
        if session.cell_w < 1.0 || session.cell_h < 1.0 {
            return false;
        }
        let col = ((x - session.origin_x) / session.cell_w).floor().max(0.0) as usize;
        let row = ((y - session.origin_y) / session.cell_h).floor().max(0.0) as usize;
        session.view.mouse(
            term_view::MouseButton::Left,
            col,
            row,
            down && !motion,
            motion,
        )
    }

    fn filter_key(&mut self, key: &str, event: &KeyDownEvent) {
        match key {
            "escape" | "enter" | "return" => self.filter_focus = false,
            "backspace" => {
                self.file_filter.pop();
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform {
                        self.file_filter.push_str(text);
                    }
                }
            }
        }
    }

    fn editor_key(&mut self, key: &str, event: &KeyDownEvent) -> bool {
        let Some(doc) = self.editor.as_mut() else {
            return false;
        };
        if doc.mode == crate::logic::markdown_preview::MarkdownPreviewMode::Preview
            && key != "escape"
        {
            return false;
        }
        match key {
            "escape" => {
                self.editor_active = false;
                true
            }
            "backspace" => {
                doc.text.pop();
                doc.dirty = true;
                true
            }
            "enter" | "return" => {
                doc.text.push('\n');
                doc.dirty = true;
                true
            }
            "tab" => {
                doc.text.push_str("    ");
                doc.dirty = true;
                true
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    if event.keystroke.modifiers.control || event.keystroke.modifiers.platform {
                        return false;
                    }
                    doc.text.push_str(text);
                    doc.dirty = true;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn set_theme(&mut self, id: &str) {
        self.settings.theme_id = id.to_string();
        self.settings.first_run.picked_theme = true;
        theme::set_id(id);
        let palette = theme::terminal_palette();
        for session in self.sessions.values_mut() {
            session.view.set_palette(palette);
        }
        let _ = persist::save_settings(&self.settings);
    }

    fn open_agent(&mut self) {
        self.agent_done = true;
        self.agent_open = true;
        if self.agent_child.is_some() {
            self.status = format!("Agent already running ({})", self.settings.agent_kind);
            return;
        }
        let bridge = crate::logic::agent_launch::Bridge {
            url: "http://127.0.0.1:9/mcp".into(),
            token: "local".into(),
            port: 9,
        };
        let extras = crate::logic::agent_launch::LaunchExtras {
            native_local: true,
            spawn_cwd: Some(self.work_dir.display().to_string()),
            resume_sessions: self.settings.agent_preferences.resume_sessions,
            ..crate::logic::agent_launch::LaunchExtras::default()
        };
        let prepared =
            if self.settings.agent_kind == "devterm" || self.settings.agent_kind.is_empty() {
                crate::logic::agent_launch::prepare_builtin_agent_launch("", &bridge, &extras)
            } else {
                Err(crate::logic::agent_launch::assert_agent_bin_available(
                    &self.settings.agent_kind,
                )
                .err()
                .unwrap_or_else(|| {
                    format!("launch {} from the agent picker", self.settings.agent_kind)
                }))
            };
        match prepared {
            Ok(spec) => match std::process::Command::new(&spec.bin)
                .args(&spec.args)
                .current_dir(&spec.cwd)
                .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .spawn()
            {
                Ok(child) => {
                    self.agent_child = Some(child);
                    self.status = format!("Agent started ({})", spec.bin);
                }
                Err(err) => self.status = format!("Agent failed to start: {err}"),
            },
            Err(err) => self.status = err,
        }
    }

    fn stop_agent(&mut self) {
        if let Some(mut child) = self.agent_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.status = "Agent stopped".into();
    }

    fn save_workspace(&mut self) {
        let mut items = Vec::new();
        for (id, session) in &self.sessions {
            items.push(serde_json::json!({
                "id": id.to_string(),
                "title": session.title,
                "cwd": session.view.cwd(),
                "remote": session.remote.is_some(),
            }));
        }
        let path = persist::user_data_dir().join("workspaces.json");
        let mut existing = persist::read_text(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value.get("workspaces").cloned())
            .unwrap_or_else(|| serde_json::json!([]));
        if let Some(list) = existing.as_array_mut() {
            list.push(serde_json::json!({
                "name": "Saved group",
                "items": items,
            }));
        }
        let body = serde_json::json!({ "workspaces": existing });
        match persist::write_private(
            &path,
            &serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into()),
        ) {
            Ok(()) => self.status = format!("Saved workspace to {}", path.display()),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn settings_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.theme_id.clone();
        let shell = self.settings.default_shell.kind.clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        rows.push(
            div()
                .text_xs()
                .text_color(theme::muted())
                .child(format!("Shell · {shell}"))
                .into_any_element(),
        );
        for spec in crate::logic::themes::themes() {
            let id = spec.id.to_string();
            let name = spec.name.to_string();
            let selected = id == current;
            rows.push(
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .text_xs()
                    .text_color(if selected {
                        theme::fg()
                    } else {
                        theme::muted()
                    })
                    .when(selected, |el| el.bg(theme::accent_quiet()))
                    .hover(|style| style.bg(theme::hover()))
                    .child(name)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.set_theme(&id);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .text_xs()
                .text_color(theme::muted())
                .child("Export writes settings.json with secrets removed. Import merges through the same normalizers and does not bring the getting-started row back.")
                .into_any_element(),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(rows)
            .into_any_element()
    }

    fn active_session_id(&self) -> Option<u64> {
        let group = self
            .groups
            .iter()
            .find(|group| group.id == self.active_group)?;
        Some(group.panes.get(group.active_pane)?.active)
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let changes = self.git.as_ref().map(|git| git.changes.len()).unwrap_or(0);
        div()
            .w(px(40.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .bg(theme::panel())
            .border_r_1()
            .border_color(theme::border())
            .py(px(6.))
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(rail_button(Icon::Folder, self.library == Some(Library::Files), cx, |this, _| {
                        this.toggle_library(Library::Files);
                    }))
                    .child(rail_button(
                        Icon::Remote,
                        self.library == Some(Library::Connections),
                        cx,
                        |this, _| this.toggle_library(Library::Connections),
                    ))
                    .child(rail_button(
                        Icon::Group,
                        self.library == Some(Library::Workspaces),
                        cx,
                        |this, _| this.toggle_library(Library::Workspaces),
                    ))
                    .child(rail_button(
                        Icon::Keyboard,
                        self.library == Some(Library::Snippets),
                        cx,
                        |this, _| this.toggle_library(Library::Snippets),
                    )),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(div().w(px(18.)).h(px(1.)).bg(theme::border()))
                    .child(rail_button(Icon::Branch, self.git_open, cx, |this, _| {
                        this.git_open = !this.git_open;
                        if this.git_open {
                            this.git = git_snapshot(&this.work_dir);
                        }
                    }))
                    .when(changes > 0, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme::warn())
                                .child(changes.to_string()),
                        )
                    })
                    .child(rail_button(Icon::Mic, false, cx, |this, _| {
                        this.status = "Dictation is push-to-talk (Ctrl+Shift+M). Weights are downloaded on first use and are not in the package.".into();
                    }))
                    .child(rail_button(
                        Icon::Keyboard,
                        self.modal == Some(Modal::Shortcuts),
                        cx,
                        |this, _| {
                            this.modal = Some(Modal::Shortcuts);
                        },
                    ))
                    .child(rail_button(
                        Icon::Settings,
                        self.modal == Some(Modal::Settings),
                        cx,
                        |this, _| {
                            this.modal = Some(Modal::Settings);
                        },
                    )),
            )
    }

    fn render_library(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(library) = self.library else {
            return div().into_any_element();
        };
        let title = match library {
            Library::Files => "Files",
            Library::Connections => "Connections",
            Library::Workspaces => "Workspaces",
            Library::Snippets => "Snippets",
        };
        let mut body: Vec<AnyElement> = Vec::new();
        match library {
            Library::Files => {
                let filter = self.file_filter.clone();
                body.push(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .pb(px(4.))
                        .child(display_path(&self.files_cwd))
                        .into_any_element(),
                );
                body.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_xs()
                        .px(px(6.))
                        .py(px(4.))
                        .mb(px(6.))
                        .rounded(px(4.))
                        .bg(theme::panel2())
                        .border_1()
                        .border_color(if self.filter_focus {
                            theme::accent()
                        } else {
                            theme::border()
                        })
                        .child(glyph(Icon::Search, 12., theme::muted()))
                        .child(
                            div()
                                .text_color(if filter.is_empty() {
                                    theme::muted()
                                } else {
                                    theme::fg()
                                })
                                .child(if filter.is_empty() {
                                    "Filter files…".to_string()
                                } else {
                                    filter.clone()
                                }),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.filter_focus = true;
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .into_any_element(),
                );
                if self.files_cwd.parent().is_some() {
                    body.push(self.file_row("..", true, cx));
                }
                let needle = filter.to_lowercase();
                let rows: Vec<(String, bool)> = self
                    .file_rows
                    .iter()
                    .filter(|row| needle.is_empty() || row.name.to_lowercase().contains(&needle))
                    .map(|row| (row.name.clone(), row.dir))
                    .collect();
                for (name, dir) in rows {
                    body.push(self.file_row(&name, dir, cx));
                }
            }
            Library::Connections => {
                body.push(
                    button("Import SSH config", false, cx, |this, _| {
                        this.reload_ssh();
                        this.status = if this.ssh_hosts.is_empty() {
                            "No concrete hosts in ~/.ssh/config".into()
                        } else {
                            format!("Imported {} host(s)", this.ssh_hosts.len())
                        };
                    })
                    .into_any_element(),
                );
                if self.ssh_hosts.is_empty() {
                    body.push(empty_copy(
                        "No saved connections. Import reads concrete Host names from ~/.ssh/config.",
                    ));
                }
                let hosts = self.ssh_hosts.clone();
                for host in hosts {
                    let subtitle = match (&host.user, &host.host_name) {
                        (Some(user), Some(name)) => format!("{user}@{name}"),
                        (None, Some(name)) => name.clone(),
                        (Some(user), None) => user.clone(),
                        (None, None) => host.name.clone(),
                    };
                    let label = host.name.clone();
                    body.push(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(8.))
                            .py(px(8.))
                            .rounded(px(6.))
                            .hover(|style| style.bg(theme::hover()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(div().text_sm().child(label.clone()))
                                    .child(
                                        div().text_xs().text_color(theme::muted()).child(subtitle),
                                    ),
                            )
                            .child(button("Connect", true, cx, move |this, cx| {
                                let target =
                                    host.host_name.clone().unwrap_or_else(|| host.name.clone());
                                let user = host.user.clone().unwrap_or_else(|| {
                                    std::env::var("USER").unwrap_or_else(|_| "root".into())
                                });
                                this.start_remote(&target, &user, 22, cx);
                            }))
                            .into_any_element(),
                    );
                }
            }
            Library::Workspaces => {
                let path = persist::user_data_dir().join("workspaces.json");
                let saved = persist::read_text(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
                let rows = saved
                    .as_ref()
                    .and_then(|value| value.get("workspaces"))
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                if rows.is_empty() {
                    body.push(empty_copy(
                        "No workspaces yet. Save on the group bar writes workspaces.json.",
                    ));
                }
                for row in rows {
                    let name = row
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Workspace");
                    body.push(
                        div()
                            .px(px(8.))
                            .py(px(6.))
                            .text_sm()
                            .child(name.to_string())
                            .into_any_element(),
                    );
                }
            }
            Library::Snippets => {
                for (name, detail, body_text) in SNIPPETS {
                    let payload = (*body_text).to_string();
                    let name = (*name).to_string();
                    body.push(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(8.))
                            .py(px(8.))
                            .rounded(px(6.))
                            .hover(|style| style.bg(theme::hover()))
                            .child(
                                div().flex_1().child(div().text_sm().child(name)).child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::muted())
                                        .font_family("Cascadia Mono")
                                        .child((*detail).to_string()),
                                ),
                            )
                            .child(button("Run", true, cx, move |this, _| {
                                this.write_active(payload.as_bytes());
                                this.status = "Sent snippet to the active terminal".into();
                            }))
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .border_r_1()
            .border_color(theme::border())
            .p(px(12.))
            .gap(px(6.))
            .child(div().text_size(px(15.)).pb(px(6.)).child(title))
            .child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(theme::border())
                    .text_color(theme::muted())
                    .text_xs()
                    .child("Filter"),
            )
            .children(body)
            .into_any_element()
    }

    fn file_row(&self, name: &str, dir: bool, cx: &mut Context<Self>) -> AnyElement {
        let label = name.to_string();
        let cwd = self.files_cwd.clone();
        let kind = file_kind(&label, dir);
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(22.))
            .px(px(6.))
            .rounded(px(4.))
            .hover(|style| style.bg(theme::hover()))
            .child(if dir && label != ".." {
                glyph(Icon::ChevronDown, 13., theme::muted())
            } else {
                glyph(Icon::ChevronDown, 13., rgba(0x00000000))
            })
            .child(glyph(kind.icon(), 15., kind.color()))
            .child(div().text_xs().text_color(theme::fg()).child(label.clone()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if !dir {
                        let path = cwd.join(&label);
                        match editor::open_file(&path) {
                            Ok(doc) => {
                                this.editor = Some(doc);
                                this.editor_active = true;
                                this.status = format!("Editing {}", path.display());
                            }
                            Err(err) => this.status = err,
                        }
                        cx.notify();
                        return;
                    }
                    let next = if label == ".." {
                        cwd.parent().unwrap_or(&cwd).to_path_buf()
                    } else {
                        cwd.join(&label)
                    };
                    this.files_cwd = next;
                    this.reload_files();
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    fn render_center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let group_bar = self.render_group_bar(cx);
        let welcome = self.render_welcome(cx);
        let panes = self.render_panes(window, cx);
        let activity = self.activity_open.then(|| {
            dock_note(
                "Activity",
                "No agent activity yet. The bridge log arrives with the agent process.",
            )
        });
        let transfer_note = {
            let mut store = crate::logic::transfers::TransferStore::new(&persist::user_data_dir());
            let items = store.load();
            let running = items
                .iter()
                .filter(|item| {
                    !item.done && item.paused != Some(true) && item.canceled != Some(true)
                })
                .count();
            if items.is_empty() {
                "No transfers.".to_string()
            } else {
                format!("{} queued, {running} running", items.len())
            }
        };
        let transfers = self
            .transfers_open
            .then(|| dock_note("Transfers", transfer_note));
        let status = self.render_status(cx);
        let zen = self.zen_mode;
        let focus = self.focus_mode;
        let show_status = self.settings.show_status_bar && !zen && !focus;
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .when(!zen, move |el| el.child(group_bar))
            .when(!zen && !focus, move |el| el.children(welcome))
            .child(panes)
            .when(!zen && !focus, move |el| {
                el.children(activity).children(transfers)
            })
            .when(show_status, move |el| el.child(status))
    }

    fn render_group_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs: Vec<AnyElement> = Vec::new();
        let groups: Vec<(u64, String, usize, bool)> = self
            .groups
            .iter()
            .map(|group| {
                let count: usize = group.panes.iter().map(|pane| pane.tabs.len()).sum();
                (
                    group.id,
                    group.name.clone(),
                    count,
                    group.id == self.active_group,
                )
            })
            .collect();
        for (id, name, count, active) in groups {
            let closable = id != self.home_group;
            tabs.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .text_xs()
                    .text_color(if active { theme::fg() } else { theme::muted() })
                    .when(active, |el| el.bg(theme::group_active()))
                    .hover(|style| style.text_color(theme::fg()))
                    .child(glyph(
                        Icon::Group,
                        14.,
                        if active { theme::fg() } else { theme::muted() },
                    ))
                    .child(name)
                    .child(
                        div()
                            .px(px(6.))
                            .rounded(px(8.))
                            .bg(theme::hover())
                            .text_size(px(10.))
                            .child(count.to_string()),
                    )
                    .when(closable, |el| {
                        el.child(
                            div()
                                .text_color(theme::muted())
                                .hover(|style| style.text_color(theme::danger()))
                                .child(glyph(Icon::Close, 12., theme::muted()))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.close_group(id);
                                        cx.stop_propagation();
                                        cx.notify();
                                    }),
                                ),
                        )
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.active_group = id;
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }
        div()
            .h(px(26.))
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(6.))
            .bg(theme::bg())
            .border_b_1()
            .border_color(theme::border())
            .children(tabs)
            .child(
                div()
                    .px(px(8.))
                    .text_color(theme::muted())
                    .hover(|style| style.text_color(theme::fg()))
                    .child(glyph(Icon::Plus, 14., theme::muted()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.new_group(cx);
                            cx.notify();
                        }),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(theme::border())
                    .text_xs()
                    .text_color(theme::muted())
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(glyph(Icon::Save, 13., theme::muted()))
                    .child("Save")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.library = Some(Library::Workspaces);
                            this.save_workspace();
                            cx.notify();
                        }),
                    ),
            )
    }

    fn render_welcome(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.welcome_dismissed {
            return None;
        }
        let local_done = self
            .sessions
            .values()
            .any(|session| !session.view.shell().is_empty() || session.view.exited());
        let cards = vec![
            welcome_card(
                "Local terminal",
                "A shell in this group, ready to type.",
                local_done,
                "Open",
                cx,
                |this, cx| this.new_tab(cx),
            ),
            welcome_card(
                "SSH connection",
                "Import SSH config or save a host.",
                self.ssh_done,
                "Add",
                cx,
                |this, _| {
                    this.library = Some(Library::Connections);
                    this.reload_ssh();
                },
            ),
            welcome_card(
                "DevTerm Agent",
                "Runs on this PC and works over SSH.",
                self.agent_done,
                "Start",
                cx,
                |this, _| {
                    this.agent_open = true;
                    this.agent_done = true;
                    this.status = "Agent column open".into();
                },
            ),
        ];
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .border_b_1()
                .border_color(theme::border())
                .bg(theme::bg())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(div().text_sm().child("Getting started"))
                        .child(div().flex_1())
                        .child(hint_key("Ctrl+K", "palette"))
                        .child(hint_key("Ctrl+Shift+T", "new terminal"))
                        .child(hint_key("Ctrl+,", "settings"))
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(theme::border())
                                .text_xs()
                                .text_color(theme::muted())
                                .child("Dismiss")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.welcome_dismissed = true;
                                        this.settings.welcome_hint_seen = true;
                                        let _ = persist::save_settings(&this.settings);
                                        cx.notify();
                                    }),
                                ),
                        ),
                )
                .child(div().flex().gap(px(8.)).children(cards))
                .into_any_element(),
        )
    }

    fn render_panes(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let group = self
            .groups
            .iter()
            .find(|group| group.id == self.active_group)
            .cloned();
        let Some(group) = group else {
            return div().flex_1().into_any_element();
        };
        if group.panes.is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.))
                        .child(div().text_sm().child("Empty group"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .child("Open a terminal in this group."),
                        )
                        .child(button("New terminal", true, cx, |this, cx| {
                            this.new_tab(cx)
                        })),
                )
                .into_any_element();
        }
        let mut panes: Vec<AnyElement> = Vec::new();
        for (index, pane) in group.panes.iter().enumerate() {
            panes.push(self.render_pane(index, pane, index == group.active_pane, cx));
        }
        let agent = self.agent_open.then(|| self.render_agent(cx));
        div()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_hidden()
            .flex()
            .flex_row()
            .children(panes)
            .children(agent)
            .into_any_element()
    }

    fn render_pane(
        &self,
        index: usize,
        pane: &Pane,
        active_pane: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut tabs: Vec<AnyElement> = Vec::new();
        for session_id in &pane.tabs {
            let Some(session) = self.sessions.get(session_id) else {
                continue;
            };
            let selected = *session_id == pane.active;
            let title = session.title.clone();
            let id = *session_id;
            let dot = if session.view.exited() {
                theme::danger()
            } else {
                theme::ok()
            };
            tabs.push(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .text_xs()
                    .text_color(if selected {
                        theme::fg()
                    } else {
                        theme::muted()
                    })
                    .when(selected, |el| el.bg(theme::panel2()))
                    .child(div().w(px(6.)).h(px(6.)).rounded(px(3.)).bg(dot))
                    .child(title)
                    .child(
                        div()
                            .text_color(theme::muted())
                            .hover(|style| style.text_color(theme::danger()))
                            .child(glyph(Icon::Close, 12., theme::muted()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.close_session(id);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            window.focus(&this.focus);
                            this.select_tab(index, id);
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }
        if active_pane {
            if let Some(doc) = &self.editor {
                let name = doc
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("editor")
                    .to_string();
                let dirty = doc.dirty;
                let selected = self.editor_active;
                tabs.push(
                    div()
                        .h_full()
                        .flex()
                        .items_center()
                        .px(px(8.))
                        .text_xs()
                        .text_color(if selected {
                            theme::fg()
                        } else {
                            theme::muted()
                        })
                        .when(selected, |el| el.bg(theme::panel2()))
                        .child(if dirty { format!("{name} •") } else { name })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                window.focus(&this.focus);
                                this.editor_active = true;
                                cx.notify();
                            }),
                        )
                        .into_any_element(),
                );
            }
        }
        let session_id = pane.active;
        let find_open = self
            .sessions
            .get(&session_id)
            .map(|session| session.view.find_open())
            .unwrap_or(false);
        let find_query = self
            .sessions
            .get(&session_id)
            .map(|session| session.view.find_query().to_string())
            .unwrap_or_default();
        let editor_text = (active_pane && self.editor_active)
            .then(|| self.editor.as_ref())
            .flatten()
            .map(|doc| {
                let body = match doc.mode {
                    crate::logic::markdown_preview::MarkdownPreviewMode::Edit => doc.text.clone(),
                    _ => editor::preview_html(doc),
                };
                let count = body.chars().count();
                if count > 4000 {
                    body.chars().skip(count - 4000).collect()
                } else {
                    body
                }
            });
        let entity = cx.entity().clone();
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::term_bg())
            .border_1()
            .border_color(if active_pane {
                theme::accent()
            } else {
                theme::border()
            })
            .child(
                div()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .px(px(4.))
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(if active_pane {
                        theme::accent()
                    } else {
                        theme::border()
                    })
                    .children(tabs)
                    .child(div().flex_1())
                    .child(strip_button(Icon::Agent, cx, |this, _| {
                        this.open_agent();
                    }))
                    .child(strip_button(Icon::Split, cx, |this, cx| {
                        this.split_right(cx)
                    }))
                    .child(strip_button(Icon::Plus, cx, |this, cx| this.new_tab(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .relative()
                    .overflow_hidden()
                    .font_family("DejaVu Sans Mono")
                    .text_size(px(13.))
                    .text_color(theme::fg())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus);
                            this.select_tab(index, session_id);
                            if this.pointer(
                                session_id,
                                event.position.x.into(),
                                event.position.y.into(),
                                true,
                                false,
                            ) {
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        let dragging = this.dragging;
                        if this.pointer(
                            session_id,
                            event.position.x.into(),
                            event.position.y.into(),
                            dragging,
                            true,
                        ) {
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            if this.pointer(
                                session_id,
                                event.position.x.into(),
                                event.position.y.into(),
                                false,
                                false,
                            ) {
                                cx.notify();
                            }
                        }),
                    )
                    .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                        let lines = match event.delta {
                            gpui::ScrollDelta::Lines(delta) => delta.y,
                            gpui::ScrollDelta::Pixels(delta) => f32::from(delta.y) / 18.0,
                        };
                        if let Some(session) = this.sessions.get_mut(&session_id) {
                            session.view.scroll_lines(-lines as i32);
                        }
                        cx.notify();
                    }))
                    .when(find_open, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(4.))
                                .right(px(4.))
                                .px(px(8.))
                                .py(px(4.))
                                .bg(theme::panel())
                                .border_1()
                                .border_color(theme::border())
                                .text_xs()
                                .child(format!("Find: {find_query}")),
                        )
                    })
                    .child(
                        canvas(
                            {
                                let entity = entity.clone();
                                move |bounds, window, app| {
                                    let (cell_w, cell_h) = term_view::measure_cell(window, 13.0);
                                    let cols = (bounds.size.width / cell_w).floor() as usize;
                                    let rows = (bounds.size.height / cell_h).floor() as usize;
                                    let cols = cols.clamp(2, 500);
                                    let rows = rows.clamp(1, 400);
                                    let pixel_width = (f32::from(cell_w) * cols as f32) as u16;
                                    let pixel_height = (f32::from(cell_h) * rows as f32) as u16;
                                    entity.update(app, |shell, cx| {
                                        let changed = shell.note_size(
                                            session_id,
                                            cols,
                                            rows,
                                            pixel_width,
                                            pixel_height,
                                            f32::from(bounds.origin.x),
                                            f32::from(bounds.origin.y),
                                            f32::from(cell_w),
                                            f32::from(cell_h),
                                        );
                                        if changed {
                                            cx.notify();
                                        }
                                        shell
                                            .sessions
                                            .get(&session_id)
                                            .map(|session| (session.view.frame(), cell_w, cell_h))
                                    })
                                }
                            },
                            move |bounds, snap, window, app| {
                                if let Some((frame, cell_w, cell_h)) = snap {
                                    term_view::paint_frame(
                                        &frame,
                                        bounds.origin,
                                        cell_w,
                                        cell_h,
                                        13.0,
                                        window,
                                        app,
                                    );
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .when(editor_text.is_some(), |el| {
                        let text = editor_text.clone().unwrap_or_default();
                        el.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .p(px(8.))
                                .bg(theme::term_bg())
                                .overflow_hidden()
                                .font_family("DejaVu Sans Mono")
                                .text_xs()
                                .child(text)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_agent(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w(px(320.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .border_l_1()
            .border_color(theme::border())
            .child(
                div()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .px(px(10.))
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(div().text_color(theme::accent()).child("✦"))
                    .child(div().text_xs().child("DevTerm"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .child("Hide")
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                this.agent_open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::danger())
                            .child("Stop")
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                this.stop_agent();
                                this.agent_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .p(px(12.))
                    .text_xs()
                    .text_color(theme::muted())
                    .child("The bundled agent is the Node runtime. Open Agent on the tab strip starts it. Hide leaves the process running. Stop kills it."),
            )
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let exited = self
            .active_session_id()
            .and_then(|id| self.sessions.get(&id))
            .is_some_and(|session| session.view.exited());
        let left = if exited {
            "Local · Linux · exited".to_string()
        } else {
            format!("Local · Linux · {}", display_path(&self.work_dir))
        };
        div()
            .h(px(22.))
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .bg(theme::panel())
            .border_t_1()
            .border_color(theme::border())
            .text_size(px(11.))
            .text_color(theme::muted())
            .child(left)
            .when(!self.status.is_empty() && self.status != "Ready", |el| {
                el.child(self.status.clone())
            })
            .child(div().flex_1())
            .child(status_button(
                Icon::Activity,
                "Activity",
                self.activity_open,
                cx,
                |this, _| {
                    this.activity_open = !this.activity_open;
                },
            ))
            .child(status_button(
                Icon::Transfer,
                "Transfers",
                self.transfers_open,
                cx,
                |this, _| {
                    this.transfers_open = !this.transfers_open;
                },
            ))
            .child(status_button(
                Icon::Agent,
                "DevTerm",
                self.agent_open,
                cx,
                |this, _| {
                    this.agent_open = !this.agent_open;
                    this.agent_done = true;
                },
            ))
    }

    fn render_git(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.git_open {
            return div().into_any_element();
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(git) = &self.git {
            rows.push(
                div()
                    .text_sm()
                    .child(format!("⎇ {}", git.branch))
                    .into_any_element(),
            );
            if git.changes.is_empty() {
                rows.push(empty_copy("Working tree clean."));
            }
            for change in &git.changes {
                rows.push(
                    div()
                        .text_xs()
                        .font_family("Cascadia Mono")
                        .child(change.clone())
                        .into_any_element(),
                );
            }
        } else {
            rows.push(empty_copy("This folder is not a git repository."));
        }
        div()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .border_l_1()
            .border_color(theme::border())
            .p(px(12.))
            .gap(px(6.))
            .child(div().text_size(px(15.)).child("Git"))
            .child(button("Refresh", false, cx, |this, _| {
                this.git = git_snapshot(&this.work_dir);
            }))
            .children(rows)
            .into_any_element()
    }

    fn render_modal(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let modal = self.modal?;
        let (title, body) = match modal {
            Modal::Settings => ("Settings", self.settings_body(cx)),
            Modal::Shortcuts => ("Keyboard shortcuts", shortcuts_body()),
            Modal::Palette => ("Command palette", palette_body(cx)),
        };
        Some(
            div()
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .right(px(0.))
                .bottom(px(0.))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x101018cc))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.modal = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .w(px(460.))
                        .max_h(px(520.))
                        .bg(theme::panel())
                        .border_1()
                        .border_color(theme::border())
                        .rounded(px(8.))
                        .p(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(div().text_size(px(15.)).child(title))
                                .child(div().flex_1())
                                .child(glyph(Icon::Close, 14., theme::muted()).on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.modal = None;
                                        cx.notify();
                                    }),
                                )),
                        )
                        .child(body),
                )
                .into_any_element(),
        )
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hide_chrome = self.focus_mode || self.zen_mode;
        let rail = (!hide_chrome).then(|| self.render_rail(cx));
        let library = (!hide_chrome).then(|| self.render_library(cx));
        let center = self.render_center(window, cx);
        let git = (!hide_chrome && self.git_open).then(|| self.render_git(cx));
        let modal = self.render_modal(cx);
        div()
            .id("devterm")
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(theme::bg())
            .text_color(theme::fg())
            .font_family("DejaVu Sans")
            .text_sm()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event, window, cx| this.on_key(event, window, cx)))
            .children(rail)
            .children(library)
            .child(center)
            .children(git)
            .children(modal)
    }
}

pub fn open(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("DevTerm".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            app_id: Some("com.devterm.app".into()),
            ..Default::default()
        },
        |window, cx| {
            let shell = cx.new(|cx| Shell::new(cx));
            shell.update(cx, |shell, _| {
                window.focus(&shell.focus);
            });
            shell
        },
    )
    .expect("open DevTerm window");
    cx.activate(true);
}

fn pump_pty(id: u64, rx: std::sync::mpsc::Receiver<PtyMsg>, cx: &mut Context<Shell>) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |this, cx| loop {
        let rx = rx.clone();
        let msg = cx
            .background_executor()
            .spawn(async move { rx.lock().ok().and_then(|guard| guard.recv().ok()) })
            .await;
        let Some(msg) = msg else { break };
        let alive = this
            .update(cx, |shell, cx| {
                if let Some(session) = shell.sessions.get_mut(&id) {
                    match msg {
                        PtyMsg::Data(bytes) => {
                            session.view.push_bytes(&bytes);
                            if let Some(cwd) = session.view.cwd().map(str::to_string) {
                                let path = PathBuf::from(&cwd);
                                if path.is_dir() && shell.files_cwd != path {
                                    shell.files_cwd = path;
                                    shell.reload_files();
                                }
                            }
                        }
                        PtyMsg::Exit(code) => session.view.note_exit(code),
                    }
                }
                cx.notify();
            })
            .is_ok();
        if !alive {
            break;
        }
    })
    .detach();
}

fn hotkey_name(key: &str) -> String {
    match key {
        "pageup" | "page_up" => "PageUp".into(),
        "pagedown" | "page_down" => "PageDown".into(),
        "tab" => "Tab".into(),
        other => other.to_string(),
    }
}

fn shell_program() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
}

fn shell_title() -> String {
    Path::new(&shell_program())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("shell")
        .to_string()
}

fn pty_bytes(event: &KeyDownEvent) -> Option<Vec<u8>> {
    let key = event.keystroke.key.as_str();
    let mods = &event.keystroke.modifiers;
    if mods.control && !mods.alt && !mods.platform && !mods.shift {
        if let Some(ch) = key.chars().next() {
            if ch.is_ascii_lowercase() {
                return Some(vec![ch as u8 - b'a' + 1]);
            }
        }
    }
    match key {
        "enter" | "return" => Some(b"\r".to_vec()),
        "backspace" => Some(vec![0x7f]),
        "tab" => Some(b"\t".to_vec()),
        "escape" => Some(vec![0x1b]),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        _ => {
            if mods.control || mods.alt || mods.platform {
                None
            } else {
                event
                    .keystroke
                    .key_char
                    .clone()
                    .map(|text| text.into_bytes())
            }
        }
    }
}

fn list_dir(path: &Path) -> Vec<FileRow> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        rows.push(FileRow { name, dir });
        if rows.len() == 300 {
            break;
        }
    }
    rows.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
    rows
}

fn load_ssh_hosts() -> Vec<SshHost> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let path = PathBuf::from(home).join(".ssh").join("config");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_ssh_config(&text)
}

fn git_snapshot(cwd: &Path) -> Option<GitSnapshot> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-b"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut branch = "detached".to_string();
    let mut changes = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            branch = rest.split("...").next().unwrap_or(rest).to_string();
        } else if !line.is_empty() {
            changes.push(line.to_string());
        }
    }
    Some(GitSnapshot { branch, changes })
}

fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".into();
            }
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

fn rail_button(
    icon: Icon,
    active: bool,
    cx: &mut Context<Shell>,
    on_press: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> gpui::Div {
    let color = if active { theme::fg() } else { theme::muted() };
    div()
        .w(px(32.))
        .h(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_color(color)
        .when(active, |el| el.bg(theme::accent_quiet()))
        .hover(|style| style.bg(theme::hover()).text_color(theme::fg()))
        .child(glyph(icon, 16., color))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus);
                on_press(this, cx);
                cx.notify();
            }),
        )
}

fn button(
    label: &'static str,
    primary: bool,
    cx: &mut Context<Shell>,
    on_press: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> gpui::Div {
    div()
        .px(px(8.))
        .py(px(4.))
        .rounded(px(6.))
        .border_1()
        .border_color(if primary {
            theme::accent()
        } else {
            theme::border()
        })
        .text_xs()
        .when(primary, |el| el.bg(theme::accent_quiet()))
        .hover(|style| style.bg(theme::hover()))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                on_press(this, cx);
                cx.notify();
            }),
        )
}

fn strip_button(
    icon: Icon,
    cx: &mut Context<Shell>,
    on_press: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> gpui::Div {
    div()
        .w(px(24.))
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_color(theme::muted())
        .hover(|style| style.bg(theme::hover()).text_color(theme::fg()))
        .child(glyph(icon, 15., theme::muted()))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus);
                on_press(this, cx);
                cx.notify();
            }),
        )
}

fn status_button(
    icon: Icon,
    label: &'static str,
    active: bool,
    cx: &mut Context<Shell>,
    on_press: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> gpui::Div {
    let color = if active { theme::fg() } else { theme::muted() };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_color(color)
        .hover(|style| style.text_color(theme::fg()))
        .child(glyph(icon, 12., color))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                on_press(this, cx);
                cx.notify();
            }),
        )
}

fn welcome_card(
    title: &'static str,
    copy: &'static str,
    done: bool,
    action: &'static str,
    cx: &mut Context<Shell>,
    on_press: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(0.))
        .h(px(56.))
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .rounded(px(6.))
        .bg(theme::panel2())
        .border_1()
        .border_color(theme::border())
        .when(!done, |el| {
            el.hover(|style| style.border_color(theme::accent()))
        })
        .child(
            div()
                .w(px(28.))
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(if done {
                    rgba(0x9ece6a2e)
                } else {
                    theme::accent_quiet()
                })
                .text_color(if done { theme::ok() } else { theme::accent() })
                .child(if done { "✓" } else { "●" }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(div().text_xs().text_color(theme::fg()).child(title))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::muted())
                        .child(copy),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(if done { theme::ok() } else { theme::fg() })
                .child(if done { "Done" } else { action }),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if done {
                    return;
                }
                on_press(this, cx);
                cx.notify();
            }),
        )
        .into_any_element()
}

fn hint_key(chord: &'static str, label: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.))
        .text_color(theme::muted())
        .child(
            div()
                .px(px(5.))
                .rounded(px(4.))
                .bg(theme::panel2())
                .font_family("Cascadia Mono")
                .text_color(theme::fg())
                .child(chord),
        )
        .child(label)
}

fn dock_note(title: &'static str, body: impl Into<String>) -> AnyElement {
    div()
        .h(px(72.))
        .px(px(12.))
        .py(px(8.))
        .bg(theme::panel())
        .border_t_1()
        .border_color(theme::border())
        .child(div().text_xs().text_color(theme::fg()).child(title))
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::muted())
                .child(body.into()),
        )
        .into_any_element()
}

fn empty_copy(text: &'static str) -> AnyElement {
    div()
        .text_xs()
        .text_color(theme::muted())
        .child(text)
        .into_any_element()
}

fn term_line(line: &str, cursor: Option<usize>) -> AnyElement {
    let chars: Vec<char> = line.chars().collect();
    let row = div().h(px(18.)).flex().flex_row().whitespace_nowrap();
    let Some(col) = cursor else {
        let trimmed = line.trim_end();
        return row.child(trimmed.to_string()).into_any_element();
    };
    let before: String = chars.iter().take(col).collect();
    let current = chars.get(col).copied().unwrap_or(' ');
    let after: String = chars.iter().skip(col + 1).collect::<String>();
    let after = after.trim_end().to_string();
    row.child(before)
        .child(
            div()
                .bg(theme::accent())
                .text_color(theme::bg())
                .child(current.to_string()),
        )
        .child(after)
        .into_any_element()
}

fn shortcuts_body() -> AnyElement {
    let rows = [
        ("Ctrl+K", "Command palette"),
        ("Ctrl+Shift+T", "New terminal"),
        ("Ctrl+Shift+N", "New group"),
        ("Ctrl+Shift+W", "Close terminal"),
        ("Ctrl+Shift+E", "Files"),
        ("Ctrl+Alt+G", "Git panel"),
        ("Ctrl+,", "Settings"),
        ("Ctrl+/", "Keyboard shortcuts"),
        ("Esc", "Close dialog"),
    ];
    let mut list: Vec<AnyElement> = Vec::new();
    for (chord, label) in rows {
        list.push(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .w(px(120.))
                        .font_family("Cascadia Mono")
                        .text_color(theme::fg())
                        .child(chord),
                )
                .child(div().text_color(theme::muted()).child(label))
                .into_any_element(),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .text_xs()
        .children(list)
        .into_any_element()
}

fn palette_body(cx: &mut Context<Shell>) -> AnyElement {
    let actions: [(&str, fn(&mut Shell, &mut Context<Shell>)); 5] = [
        ("New terminal", |this, cx| this.new_tab(cx)),
        ("New group", |this, cx| this.new_group(cx)),
        ("Toggle files", |this, _| {
            this.toggle_library(Library::Files)
        }),
        ("Toggle git", |this, _| {
            this.git_open = !this.git_open;
            if this.git_open {
                this.git = git_snapshot(&this.work_dir);
            }
        }),
        ("Settings", |this, _| this.modal = Some(Modal::Settings)),
    ];
    let mut rows: Vec<AnyElement> = Vec::new();
    for (label, action) in actions {
        rows.push(
            div()
                .px(px(8.))
                .py(px(6.))
                .rounded(px(4.))
                .text_sm()
                .hover(|style| style.bg(theme::hover()))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        action(this, cx);
                        if !matches!(this.modal, Some(Modal::Settings)) {
                            this.modal = None;
                        }
                        cx.notify();
                    }),
                )
                .into_any_element(),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .children(rows)
        .into_any_element()
}
