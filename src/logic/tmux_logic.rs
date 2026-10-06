//! tmux picker, attach, and kill helpers from `src/main/ssh/tmux.ts`.
//!
//! Attach is always a child process. A failed `tmux -V` (the probe prints
//! `__DT_TMUX_MISSING` and never `__DT_TMUX_OK`) skips the picker.

/// Format string for `tmux list-sessions -F`. Tabs keep names with spaces intact.
pub const TMUX_LIST_FORMAT: &str = "#{session_name}\t#{session_windows}\t#{session_attached}\t#{session_created}\t#{session_activity}\t#{window_name}\t#{pane_current_command}\t#{pane_current_path}";

/// Compact window labels for the picker (`0:vim*`).
pub const TMUX_WIN_FORMAT: &str = "#{window_index}:#{window_name}#{?window_active,*,}";

pub const TMUX_LIST_CLIENTS_FORMAT: &str = "#{client_tty}\t#{client_session}\t#{client_activity}";

pub const TMUX_LIST_CLIENTS: &str =
    "tmux list-clients -F '#{client_tty}\t#{client_session}\t#{client_activity}' 2>/dev/null || true";

/// One-shot probe + list + visible-pane capture.
/// `__DT_TMUX_OK` / `__DT_TMUX_MISSING` is the only signal we trust — `tmux -V`
/// failing is treated as "not available".
pub fn tmux_probe_and_list() -> String {
    format!(
        "if command -v tmux >/dev/null 2>&1 && tmux -V >/dev/null 2>&1; then \
printf '__DT_VER=%s\\n' \"$(tmux -V)\"; printf '__DT_TMUX_OK\\n'; \
tmux list-sessions -F '{TMUX_LIST_FORMAT}' 2>/dev/null || true; \
printf '__DT_PREVIEWS\\n'; \
tmux list-sessions -F '#{{session_name}}' 2>/dev/null | while IFS= read -r __dt_n; do \
[ -n \"$__dt_n\" ] || continue; \
printf '__DT_P_BEGIN %s\\n' \"$__dt_n\"; \
printf '__DT_WINS '; \
tmux list-windows -t \"$__dt_n\" -F '{TMUX_WIN_FORMAT}' 2>/dev/null | awk '{{printf \"%s%s\", p, $0; p=\"|\"}}'; \
printf '\\n'; \
tmux capture-pane -pt \"$__dt_n:\" -p -J -S -48 2>/dev/null || true; \
printf '\\n__DT_P_END\\n'; \
done; \
else printf '__DT_TMUX_MISSING\\n'; fi"
    )
}

/// Kept as a function so the format constants are interpolated the same way
/// the TypeScript template builds `TMUX_PROBE_AND_LIST`.
pub const TMUX_PROBE_AND_LIST_HAS_FORMATS: bool = true;

const PREVIEW_MAX_LINES: usize = 48;
const PREVIEW_MAX_COLS: usize = 220;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxSessionInfo {
    pub name: String,
    pub windows: i64,
    pub attached: i64,
    pub created: Option<i64>,
    pub activity: Option<i64>,
    pub current_window: Option<String>,
    pub current_command: Option<String>,
    pub current_path: Option<String>,
    pub window_list: Option<Vec<String>>,
    pub preview: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxListing {
    pub available: bool,
    pub version: Option<String>,
    pub sessions: Vec<TmuxSessionInfo>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxClientInfo {
    pub tty: String,
    pub session: String,
    pub activity: i64,
}

/// Plain-text pane snapshot for the picker; drops VT noise, keeps line breaks.
pub fn sanitize_tmux_preview(raw: &str) -> String {
    let cleaned = strip_preview_ansi(raw);
    let cleaned = strip_preview_controls(&cleaned);
    let cleaned = cleaned.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = cleaned
        .split('\n')
        .map(|line| {
            let trimmed = line.trim_end_matches([' ', '\t']);
            let count = trimmed.chars().count();
            if count > PREVIEW_MAX_COLS {
                let head: String = trimmed.chars().take(PREVIEW_MAX_COLS - 1).collect();
                format!("{head}\u{2026}")
            } else {
                trimmed.to_string()
            }
        })
        .collect();
    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    if lines.len() > PREVIEW_MAX_LINES {
        lines = lines.split_off(lines.len() - PREVIEW_MAX_LINES);
    }
    lines.join("\n")
}

fn strip_preview_ansi(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\u{1b}' {
            if let Some(consumed) = match_ansi(&chars[i..]) {
                i += consumed;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn match_ansi(chars: &[char]) -> Option<usize> {
    if chars.first() != Some(&'\u{1b}') {
        return None;
    }
    if chars.get(1) == Some(&'[') {
        // CSI: ESC [ [0-?]* [ -/]* [@-~]
        let mut i = 2;
        while i < chars.len() && ('\u{30}'..='\u{3f}').contains(&chars[i]) {
            i += 1;
        }
        while i < chars.len() && ('\u{20}'..='\u{2f}').contains(&chars[i]) {
            i += 1;
        }
        if i < chars.len() && ('\u{40}'..='\u{7e}').contains(&chars[i]) {
            return Some(i + 1);
        }
    }
    if chars.get(1) == Some(&']') {
        // OSC: ESC ] [^\x07\x1b]* (BEL | ESC \ | end)
        let mut i = 2;
        while i < chars.len() && chars[i] != '\u{7}' && chars[i] != '\u{1b}' {
            i += 1;
        }
        if i >= chars.len() {
            return Some(chars.len());
        }
        if chars[i] == '\u{7}' {
            return Some(i + 1);
        }
        if chars[i] == '\u{1b}' && chars.get(i + 1) == Some(&'\\') {
            return Some(i + 2);
        }
        return Some(i);
    }
    // Other ESC: ESC [ -/]* [0-~]
    let mut i = 1;
    while i < chars.len() && ('\u{20}'..='\u{2f}').contains(&chars[i]) {
        i += 1;
    }
    if i < chars.len() && ('\u{30}'..='\u{7e}').contains(&chars[i]) {
        return Some(i + 1);
    }
    None
}

fn strip_preview_controls(raw: &str) -> String {
    raw.chars()
        .filter(|ch| {
            let code = *ch as u32;
            !((code <= 0x08) || code == 0x0b || code == 0x0c || (0x0e..=0x1f).contains(&code) || code == 0x7f)
        })
        .collect()
}

/// tmux treats `.` and `:` as hierarchy separators; keep new names boring.
pub fn sanitize_tmux_name(raw: &str) -> String {
    let mut cleaned = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            cleaned.push(ch);
        } else {
            cleaned.push('-');
        }
    }
    let mut collapsed = String::new();
    let mut prev_dash = false;
    for ch in cleaned.chars() {
        if ch == '-' {
            if !prev_dash {
                collapsed.push('-');
            }
            prev_dash = true;
        } else {
            prev_dash = false;
            collapsed.push(ch);
        }
    }
    collapsed
        .trim_matches('-')
        .chars()
        .take(48)
        .collect()
}

pub fn default_tmux_name(session_id: &str) -> String {
    format!("devterm-{}", sanitize_tmux_name(session_id))
}

fn optional_epoch(raw: Option<&str>) -> Option<i64> {
    let raw = raw?;
    let number: f64 = raw.parse().ok()?;
    if number.is_finite() && number > 0.0 && number.fract() == 0.0 {
        Some(number as i64)
    } else {
        None
    }
}

fn optional_text(raw: Option<&str>) -> Option<String> {
    let text = raw?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn js_number_or_zero(raw: Option<&str>) -> i64 {
    let Some(raw) = raw else { return 0 };
    let Ok(number) = raw.parse::<f64>() else {
        return 0;
    };
    if number.is_finite() && number.fract() == 0.0 {
        number as i64
    } else if number.is_finite() {
        number as i64
    } else {
        0
    }
}

pub fn parse_tmux_list_sessions(stdout: &str) -> Vec<TmuxSessionInfo> {
    let listing = stdout.split("__DT_PREVIEWS").next().unwrap_or(stdout);
    let mut sessions = Vec::new();
    for line in listing.split('\n') {
        let trimmed = line.trim_end_matches('\r').trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("__DT_") {
            continue;
        }
        if trimmed.to_ascii_lowercase().contains("no server running") {
            continue;
        }
        if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("error") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('\t').collect();
        let Some(name) = parts.first().copied().filter(|name| !name.is_empty()) else {
            continue;
        };
        let mut session = TmuxSessionInfo {
            name: name.to_string(),
            windows: js_number_or_zero(parts.get(1).copied()),
            attached: js_number_or_zero(parts.get(2).copied()),
            created: None,
            activity: None,
            current_window: None,
            current_command: None,
            current_path: None,
            window_list: None,
            preview: None,
        };
        if let Some(created) = optional_epoch(parts.get(3).copied()) {
            session.created = Some(created);
        }
        if let Some(activity) = optional_epoch(parts.get(4).copied()) {
            session.activity = Some(activity);
        }
        if let Some(window) = optional_text(parts.get(5).copied()) {
            session.current_window = Some(window);
        }
        if let Some(command) = optional_text(parts.get(6).copied()) {
            session.current_command = Some(command);
        }
        if let Some(path) = optional_text(parts.get(7).copied()) {
            session.current_path = Some(path);
        }
        sessions.push(session);
    }
    sessions
}

pub fn apply_tmux_previews(sessions: &mut [TmuxSessionInfo], stdout: &str) {
    let Some(marker) = stdout.find("__DT_PREVIEWS") else {
        return;
    };
    let blocks: Vec<&str> = stdout[marker..].split("__DT_P_BEGIN ").collect();
    for block in blocks.into_iter().skip(1) {
        let Some(nl) = block.find('\n') else { continue };
        let name = block[..nl].trim_end_matches('\r').trim();
        let Some(session) = sessions.iter_mut().find(|session| session.name == name) else {
            continue;
        };
        let end = block.find("__DT_P_END");
        let body = if let Some(end) = end {
            &block[nl + 1..end]
        } else {
            &block[nl + 1..]
        };
        let body = body.replace('\r', "");
        let lines: Vec<&str> = body.split('\n').collect();
        let mut start = 0;
        if lines.first().is_some_and(|line| line.starts_with("__DT_WINS ")) {
            let raw = lines[0]["__DT_WINS ".len()..].trim();
            if !raw.is_empty() {
                session.window_list = Some(
                    raw.split('|')
                        .map(str::trim)
                        .filter(|part| !part.is_empty())
                        .map(|part| part.to_string())
                        .collect(),
                );
            }
            start = 1;
        }
        let preview = sanitize_tmux_preview(&lines[start..].join("\n"));
        if !preview.is_empty() {
            session.preview = Some(preview);
        }
    }
}

pub fn parse_tmux_listing(stdout: &str, stderr: &str) -> TmuxListing {
    if !stdout.contains("__DT_TMUX_OK") {
        let error = stderr.trim();
        return TmuxListing {
            available: false,
            version: None,
            sessions: Vec::new(),
            error: if error.is_empty() {
                None
            } else {
                Some(error.to_string())
            },
        };
    }
    let version = stdout.split('\n').find_map(|line| {
        let line = line.trim_end_matches('\r');
        line.strip_prefix("__DT_VER=")
            .map(|value| value.trim().to_string())
    });
    let mut sessions = parse_tmux_list_sessions(stdout);
    apply_tmux_previews(&mut sessions, stdout);
    TmuxListing {
        available: true,
        version,
        sessions,
        error: None,
    }
}

/// Single-quote a string for a POSIX shell.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

/// Attach the current login shell to tmux as a child process. Never `exec`.
pub fn build_tmux_attach_command(session_name: &str, create: bool) -> String {
    let quoted = sh_quote(session_name);
    let create = if create {
        format!("tmux has-session -t {quoted} 2>/dev/null || tmux new-session -Ad -s {quoted}; ")
    } else {
        String::new()
    };
    format!(
        "{create}tmux set-option -t {quoted} allow-passthrough on 2>/dev/null; \
tmux attach-session -t {quoted}; _dt_ec=$?; \
stty echo 2>/dev/null; \
if [ \"$_dt_ec\" -eq 0 ]; then \
printf '\\r\\n[DevTerm: detached from tmux; back in a normal shell]\\r\\n'; \
else printf '\\r\\n[DevTerm: tmux attach failed; staying in a normal shell]\\r\\n'; \
fi\n"
    )
}

/// Banner tmux prints when the client leaves a session.
pub fn tmux_client_left(text: &str) -> bool {
    if text.contains("[exited]") {
        return true;
    }
    let bytes = text.as_bytes();
    let needle = b"[detached";
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if &bytes[index..index + needle.len()] == needle {
            let rest = &text[index + needle.len()..];
            if rest.starts_with(']') {
                return true;
            }
            if let Some(inner) = rest.strip_prefix(" (") {
                if let Some(end) = inner.find(')') {
                    if inner.as_bytes().get(end + 1) == Some(&b']') && !inner[..end].contains(')') {
                        return true;
                    }
                }
            }
        }
        index += 1;
    }
    false
}

pub fn build_tmux_kill_command(session_name: &str) -> String {
    format!("tmux kill-session -t {}", sh_quote(session_name))
}

pub fn parse_tmux_clients(stdout: &str) -> Vec<TmuxClientInfo> {
    let mut out = Vec::new();
    for line in stdout.split('\n') {
        let trimmed = line.trim_end_matches('\r').trim();
        if trimmed.is_empty() || trimmed.starts_with("__DT_") {
            continue;
        }
        if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("error") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('\t').collect();
        let Some(tty) = parts.first().map(|part| part.trim()).filter(|part| !part.is_empty()) else {
            continue;
        };
        let Some(session) = parts.get(1).map(|part| part.trim()).filter(|part| !part.is_empty())
        else {
            continue;
        };
        let activity = parts
            .get(2)
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .filter(|number| number.is_finite())
            .map(|number| number as i64)
            .unwrap_or(0);
        out.push(TmuxClientInfo {
            tty: tty.to_string(),
            session: session.to_string(),
            activity,
        });
    }
    out
}

/// Prefer the most recently active client attached to `session_name`.
pub fn pick_client_tty(clients: &[TmuxClientInfo], session_name: &str) -> Option<String> {
    let mut best: Option<&TmuxClientInfo> = None;
    for client in clients {
        if client.session != session_name {
            continue;
        }
        if best.map(|current| client.activity > current.activity).unwrap_or(true) {
            best = Some(client);
        }
    }
    best.map(|client| client.tty.clone())
}

pub fn build_tmux_switch_command(tty: &str, session_name: &str) -> String {
    format!(
        "tmux switch-client -c {} -t {}",
        sh_quote(tty),
        sh_quote(session_name)
    )
}

pub fn build_tmux_detach_client_command(tty: &str) -> String {
    format!("tmux detach-client -t {}", sh_quote(tty))
}

pub fn build_tmux_ensure_session_command(session_name: &str) -> String {
    let quoted = sh_quote(session_name);
    format!("tmux has-session -t {quoted} 2>/dev/null || tmux new-session -Ad -s {quoted}")
}

/// True when kill-session's failure is "already gone", not a real error.
pub fn is_tmux_session_gone(stdout: &str, stderr: &str, code: Option<i32>) -> bool {
    if code == Some(0) {
        return true;
    }
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    text.contains("can't find session")
        || text.contains("session not found")
        || text.contains("no server running")
}

/// Legacy helper: create-or-reuse `devterm-<sessionId>` and attach without
/// replacing the login shell.
pub fn build_detached_session_bootstrap(session_id: &str) -> String {
    build_tmux_attach_command(&default_tmux_name(session_id), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_exec_tmux(script: &str) -> bool {
        let bytes = script.as_bytes();
        let needle = b"exec tmux";
        let mut index = 0;
        while index + needle.len() <= bytes.len() {
            if &bytes[index..index + needle.len()] == needle {
                let before = index == 0 || !is_word(bytes[index - 1]);
                let after = index + needle.len();
                let after_ok = after == bytes.len() || !is_word(bytes[after]);
                if before && after_ok {
                    return true;
                }
            }
            index += 1;
        }
        false
    }

    fn is_word(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_'
    }

    #[test]
    fn uses_a_stable_sanitized_tmux_session_name() {
        assert_eq!(
            default_tmux_name("35148259-faae-4338-b3dc-0146a4b93a79"),
            "devterm-35148259-faae-4338-b3dc-0146a4b93a79"
        );
    }

    #[test]
    fn strips_characters_tmux_treats_as_hierarchy_separators() {
        assert_eq!(
            sanitize_tmux_name("sess/with spaces!and*junk"),
            "sess-with-spaces-and-junk"
        );
        let name = sanitize_tmux_name("a.b:c");
        assert!(!name.contains('.') && !name.contains(':'));
    }

    #[test]
    fn reports_unavailable_when_the_probe_marker_is_missing() {
        let listing = parse_tmux_listing("tmux: command not found\n", "nope");
        assert!(!listing.available);
        assert!(listing.sessions.is_empty());
        assert_eq!(listing.error.as_deref(), Some("nope"));
    }

    #[test]
    fn parses_version_and_session_rows_after_a_successful_probe() {
        let listing = parse_tmux_listing(
            &[
                "__DT_VER=tmux 3.4",
                "__DT_TMUX_OK",
                "ops\t3\t1\t1710000000",
                "dev\t1\t0\t1710001000",
                "",
            ]
            .join("\n"),
            "",
        );
        assert!(listing.available);
        assert_eq!(listing.version.as_deref(), Some("tmux 3.4"));
        assert_eq!(listing.sessions.len(), 2);
        assert_eq!(
            listing.sessions[0],
            TmuxSessionInfo {
                name: "ops".into(),
                windows: 3,
                attached: 1,
                created: Some(1710000000),
                activity: None,
                current_window: None,
                current_command: None,
                current_path: None,
                window_list: None,
                preview: None,
            }
        );
        assert_eq!(listing.sessions[1].name, "dev");
        assert_eq!(listing.sessions[1].attached, 0);
    }

    #[test]
    fn parses_activity_pane_command_cwd_window_list_and_preview() {
        let listing = parse_tmux_listing(
            &[
                "__DT_VER=tmux 3.4",
                "__DT_TMUX_OK",
                "ops\t3\t1\t1710000000\t1710002000\tmain\tnvim\t/home/ops/app",
                "__DT_PREVIEWS",
                "__DT_P_BEGIN ops",
                "__DT_WINS 0:main*|1:logs|2:ssh",
                "\u{1b}[32mops@host\u{1b}[0m:~/app$ \u{1b}[1mnvim README.md\u{1b}[0m",
                "editing README.md",
                "__DT_P_END",
                "",
            ]
            .join("\n"),
            "",
        );
        let ops = &listing.sessions[0];
        assert_eq!(ops.activity, Some(1710002000));
        assert_eq!(ops.current_window.as_deref(), Some("main"));
        assert_eq!(ops.current_command.as_deref(), Some("nvim"));
        assert_eq!(ops.current_path.as_deref(), Some("/home/ops/app"));
        assert_eq!(
            ops.window_list.as_deref(),
            Some(&["0:main*".to_string(), "1:logs".into(), "2:ssh".into()][..])
        );
        assert_eq!(
            ops.preview.as_deref(),
            Some("ops@host:~/app$ nvim README.md\nediting README.md")
        );
    }

    #[test]
    fn does_not_treat_preview_markers_as_session_rows() {
        let listing = parse_tmux_listing(
            &[
                "__DT_TMUX_OK",
                "dev\t1\t0\t1710001000",
                "__DT_PREVIEWS",
                "__DT_P_BEGIN dev",
                "hi",
                "__DT_P_END",
            ]
            .join("\n"),
            "",
        );
        assert_eq!(listing.sessions.len(), 1);
        assert_eq!(listing.sessions[0].name, "dev");
        assert_eq!(listing.sessions[0].preview.as_deref(), Some("hi"));
    }

    #[test]
    fn treats_no_server_running_as_an_empty_session_list() {
        let listing = parse_tmux_listing(
            "__DT_TMUX_OK\nno server running on /tmp/tmux-1000/default\n",
            "",
        );
        assert!(listing.available);
        assert!(listing.sessions.is_empty());
    }

    #[test]
    fn attaches_without_exec_so_detach_returns_to_the_login_shell() {
        let script = build_tmux_attach_command("ops", false);
        assert!(script.contains("tmux attach-session -t 'ops'"));
        assert!(!contains_exec_tmux(&script));
        assert!(script.contains("allow-passthrough on"));
        assert!(script.contains("detached from tmux"));
    }

    #[test]
    fn optionally_creates_the_session_before_attaching() {
        let script = build_tmux_attach_command("new-one", true);
        assert!(script.contains("tmux new-session -Ad -s 'new-one'"));
        assert!(script.contains("tmux attach-session -t 'new-one'"));
        assert!(!contains_exec_tmux(&script));
    }

    #[test]
    fn single_quotes_names_so_a_hostile_session_name_cannot_break_out() {
        let script = build_tmux_attach_command("foo'$(reboot)", false);
        assert!(script.contains("'foo'\\''$(reboot)'"));
    }

    #[test]
    fn detached_bootstrap_targets_a_stable_name_and_never_execs() {
        let script = build_detached_session_bootstrap("35148259-faae-4338-b3dc-0146a4b93a79");
        assert!(script.contains(
            "tmux new-session -Ad -s 'devterm-35148259-faae-4338-b3dc-0146a4b93a79'"
        ));
        assert!(script.contains(
            "tmux attach-session -t 'devterm-35148259-faae-4338-b3dc-0146a4b93a79'"
        ));
        assert!(!contains_exec_tmux(&script));
    }

    #[test]
    fn probes_stay_on_the_listing_helper() {
        let probe = tmux_probe_and_list();
        assert!(TMUX_PROBE_AND_LIST_HAS_FORMATS);
        assert!(probe.contains("command -v tmux"));
        assert!(probe.contains("tmux -V"));
        assert!(probe.contains("__DT_TMUX_OK"));
        assert!(probe.contains("__DT_TMUX_MISSING"));
        assert!(probe.contains("capture-pane"));
        assert!(probe.contains("-S -48"));
        assert!(probe.contains("__DT_PREVIEWS"));
        assert!(probe.contains(TMUX_LIST_FORMAT));
        assert!(probe.contains(TMUX_WIN_FORMAT));
    }

    #[test]
    fn strips_ansi_and_control_bytes_but_keeps_line_breaks() {
        assert_eq!(
            sanitize_tmux_preview("\u{1b}[31mred\u{1b}[0m\nnext\r\nlast"),
            "red\nnext\nlast"
        );
    }

    #[test]
    fn drops_leading_and_trailing_blank_lines() {
        assert_eq!(sanitize_tmux_preview("\n\nfoo\n\n"), "foo");
    }

    #[test]
    fn quotes_the_target_session_for_kill_session() {
        let script = build_tmux_kill_command("ops'$(reboot)");
        assert!(script.contains("tmux kill-session -t"));
        assert!(script.contains("'ops'\\''$(reboot)'"));
    }

    #[test]
    fn treats_missing_sessions_as_already_gone() {
        assert!(is_tmux_session_gone("", "can't find session: ops", Some(1)));
        assert!(is_tmux_session_gone(
            "",
            "no server running on /tmp/tmux-1000/default",
            Some(1)
        ));
        assert!(is_tmux_session_gone("", "", Some(0)));
        assert!(!is_tmux_session_gone("", "permission denied", Some(1)));
    }

    #[test]
    fn picks_the_most_recently_active_client_for_a_session() {
        let clients = parse_tmux_clients("/dev/pts/3\tops\t10\n/dev/pts/5\tops\t99\n/dev/pts/2\tdev\t50\n");
        assert_eq!(pick_client_tty(&clients, "ops").as_deref(), Some("/dev/pts/5"));
        assert_eq!(pick_client_tty(&clients, "missing"), None);
    }

    #[test]
    fn client_left_regex_matches_detach_and_exit_banners() {
        assert!(tmux_client_left("[detached (from session ops)]"));
        assert!(tmux_client_left("[detached]"));
        assert!(tmux_client_left("[exited]"));
        assert!(!tmux_client_left("detached something in user output"));
    }

    #[test]
    fn switch_detach_and_ensure_commands_quote_their_arguments() {
        assert_eq!(
            build_tmux_switch_command("/dev/pts/5", "ops"),
            "tmux switch-client -c '/dev/pts/5' -t 'ops'"
        );
        assert_eq!(
            build_tmux_detach_client_command("/dev/pts/5"),
            "tmux detach-client -t '/dev/pts/5'"
        );
        assert!(build_tmux_ensure_session_command("ops").contains("tmux new-session -Ad -s 'ops'"));
        assert!(TMUX_LIST_CLIENTS.contains(TMUX_LIST_CLIENTS_FORMAT));
    }
}
