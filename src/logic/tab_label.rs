//! Dynamic tab label derivation.
//!
//! Context precedence (highest first):
//!   1. Terminal/agent status (closed, reconnecting, bridge error, ...)
//!   2. Agent task from live bridge activity
//!   3. Current command running in the shell
//!   4. Current working directory folder
//!
//! A manually renamed tab keeps the user's chosen title and still receives a
//! dynamic context suffix.

#![allow(dead_code)]

pub const TAB_CONTEXT_MAX: usize = 42;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Local,
    Remote,
    Browser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentBridgeState {
    Starting,
    Listening,
    Connected,
    Disconnected,
    Stopped,
    Error,
}

const BRIDGE_LABELS: [(AgentBridgeState, &str); 6] = [
    (AgentBridgeState::Starting, "agent starting…"),
    (AgentBridgeState::Listening, "agent waiting…"),
    (AgentBridgeState::Connected, "agent connected"),
    (AgentBridgeState::Disconnected, "agent disconnected"),
    (AgentBridgeState::Stopped, "agent stopped"),
    (AgentBridgeState::Error, "agent bridge error"),
];

#[derive(Clone, Debug, Default)]
pub struct TabLabelInput {
    pub id: Option<String>,
    pub kind: Option<TabKind>,
    pub title: Option<String>,
    pub custom_title: bool,
    pub local_num: Option<u32>,
    pub cwd: Option<String>,
    pub status: Option<String>,
    pub closed: bool,
    pub current_command: Option<String>,
    pub agent_task: Option<String>,
    pub agent_kind: Option<String>,
    pub agent_bridge_state: Option<AgentBridgeState>,
    pub agent_pending_approval: bool,
    pub exit_code: Option<i32>,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabLabel {
    pub title: String,
    pub context: Option<String>,
    pub tooltip: String,
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for c in s.chars() {
        if c.is_whitespace() {
            pending = true;
        } else {
            if pending && !out.is_empty() {
                out.push(' ');
            }
            pending = false;
            out.push(c);
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let head: String = s.chars().take(max - 1).collect();
    format!("{}…", head.trim_end())
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn strip_command_prefix(s: &str) -> &str {
    let lower = s.to_ascii_lowercase();
    for p in ["command=", "cmd=", "script="] {
        if lower.starts_with(p) {
            return &s[p.len()..];
        }
    }
    s
}

fn contains_spaced_assignment(val: &str) -> bool {
    let b = val.as_bytes();
    for i in 0..b.len() {
        if b[i].is_ascii_whitespace() && i + 1 < b.len() && is_word_byte(b[i + 1]) {
            let mut j = i + 2;
            while j < b.len() && is_word_byte(b[j]) {
                j += 1;
            }
            if j < b.len() && b[j] == b'=' {
                return true;
            }
        }
    }
    false
}

fn lone_kv_value(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return None;
    }
    let mut i = 1;
    while i < b.len() && is_word_byte(b[i]) {
        i += 1;
    }
    if i >= b.len() || b[i] != b'=' {
        return None;
    }
    let val = &s[i + 1..];
    let first = val.chars().next()?;
    if first.is_whitespace() {
        return None;
    }
    if contains_spaced_assignment(val) {
        return None;
    }
    Some(val.trim())
}

fn match_heredoc(s: &str) -> Option<(String, String)> {
    let b = s.as_bytes();
    let mut i = 0;
    loop {
        let rest = &s[i..];
        let mut matched = false;
        for kw in ["sudo", "doas", "env"] {
            if rest.starts_with(kw) {
                let after = i + kw.len();
                if after < b.len() && b[after].is_ascii_whitespace() {
                    let mut j = after;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    i = j;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            break;
        }
    }
    loop {
        if i >= b.len() || !(is_word_byte(b[i]) || b[i] == b'.') {
            break;
        }
        let mut j = i;
        while j < b.len() && (is_word_byte(b[j]) || b[j] == b'.') {
            j += 1;
        }
        if j >= b.len() || b[j] != b'=' {
            break;
        }
        j += 1;
        if j >= b.len() || b[j].is_ascii_whitespace() {
            break;
        }
        while j < b.len() && !b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() || !b[j].is_ascii_whitespace() {
            break;
        }
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        i = j;
    }
    let g1 = s[..i].to_string();
    if i >= b.len() || b[i].is_ascii_whitespace() || b[i] == b'<' {
        return None;
    }
    let prog = i;
    let mut j = i;
    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'<' {
        j += 1;
    }
    loop {
        let mut k = j;
        if k >= b.len() || !b[k].is_ascii_whitespace() {
            break;
        }
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if k >= b.len() || b[k] != b'-' {
            break;
        }
        let mut m = k + 1;
        if m >= b.len() || b[m].is_ascii_whitespace() || b[m] == b'<' {
            break;
        }
        while m < b.len() && !b[m].is_ascii_whitespace() && b[m] != b'<' {
            m += 1;
        }
        j = m;
    }
    let g2 = s[prog..j].to_string();
    let mut k = j;
    while k < b.len() && b[k].is_ascii_whitespace() {
        k += 1;
    }
    if !s[k..].starts_with("<<") {
        return None;
    }
    k += 2;
    if k < b.len() && b[k] == b'-' {
        k += 1;
    }
    while k < b.len() && b[k].is_ascii_whitespace() {
        k += 1;
    }
    if k < b.len() && (b[k] == b'\'' || b[k] == b'"') {
        k += 1;
    }
    if k >= b.len() || !is_word_byte(b[k]) {
        return None;
    }
    Some((g1, g2))
}

fn find_chain(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            let rest = &s[i + 1..];
            let op_len = if rest.starts_with("&&") || rest.starts_with("||") {
                2
            } else if rest.starts_with(';') || rest.starts_with('|') {
                1
            } else {
                0
            };
            if op_len > 0 {
                let after = i + 1 + op_len;
                if after < b.len() && b[after].is_ascii_whitespace() {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

fn find_powershell_here(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(1) {
        if b[i] == b'@' && (b[i + 1] == b'\'' || b[i + 1] == b'"') {
            return Some(i);
        }
    }
    None
}

pub fn summarize_command(cmd: &str) -> String {
    summarize_command_limited(cmd, TAB_CONTEXT_MAX)
}

pub fn summarize_command_limited(cmd: &str, max_len: usize) -> String {
    let collapsed = collapse_ws(cmd);
    if collapsed.is_empty() {
        return collapsed;
    }
    let stripped = strip_command_prefix(&collapsed);
    let mut s = if let Some(val) = lone_kv_value(stripped) {
        val.trim().to_string()
    } else {
        stripped.to_string()
    };
    if s.is_empty() {
        return s;
    }
    if let Some((g1, g2)) = match_heredoc(&s) {
        let head = collapse_ws(format!("{g1}{g2}").trim());
        let marked = if head.is_empty() {
            "<<…".to_string()
        } else {
            format!("{head} <<…")
        };
        return truncate(&marked, max_len);
    }
    if let Some(idx) = find_powershell_here(&s) {
        if s.chars().count() > max_len {
            let head = s[..idx].trim();
            let marked = if head.is_empty() {
                s.clone()
            } else {
                format!("{head} @…")
            };
            return truncate(&marked, max_len);
        }
    }
    if s.chars().count() > max_len {
        let cut: String = s.chars().take(max_len - 1).collect();
        if let Some(chain) = find_chain(&cut) {
            if chain > 12 {
                let clause = cut[..chain].trim_end();
                return format!("{clause} …");
            }
        }
    }
    let owned = std::mem::take(&mut s);
    truncate(&owned, max_len)
}

fn folder_name(cwd: &str) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let idx_slash = trimmed.rfind('/');
    let idx_bslash = trimmed.rfind('\\');
    let idx = match (idx_slash, idx_bslash) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let name = if let Some(i) = idx {
        &trimmed[i + 1..]
    } else {
        trimmed
    };
    if !name.is_empty() {
        name.to_string()
    } else if cwd.starts_with('/') {
        "/".to_string()
    } else {
        cwd.to_string()
    }
}

fn agent_label(kind: Option<&str>) -> String {
    match kind {
        None => "Agent".to_string(),
        Some("devterm") => "DevTerm".to_string(),
        Some("opencode") => "OpenCode".to_string(),
        Some("antigravity") => "Antigravity".to_string(),
        Some("muse") => "Muse Code".to_string(),
        Some("cursor") => "Cursor".to_string(),
        Some(kind) => {
            let mut chars = kind.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
    }
}

fn strip_trailing_scalar(rest: &str) -> String {
    let b = rest.as_bytes();
    if let Some(start) = rest.rfind(|c: char| c.is_whitespace()) {
        let tail = rest[start..].trim_start();
        let tb = tail.as_bytes();
        if !tb.is_empty() && is_word_byte(tb[0]) {
            let mut j = 1;
            while j < tb.len() && is_word_byte(tb[j]) {
                j += 1;
            }
            if j < tb.len() && tb[j] == b'=' {
                let mut k = j + 1;
                if k < tb.len() && !tb[k].is_ascii_whitespace() {
                    while k < tb.len() && !tb[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if k == tb.len() {
                        return rest[..start].to_string();
                    }
                }
            }
        }
    }
    let _ = b;
    rest.to_string()
}

pub fn summarize_agent_task(task: &str) -> String {
    summarize_agent_task_limited(task, TAB_CONTEXT_MAX)
}

pub fn summarize_agent_task_limited(task: &str, max_len: usize) -> String {
    let s = collapse_ws(task);
    if s.is_empty() {
        return s;
    }
    let bytes = s.as_bytes();
    let mut tool_end = 0;
    if bytes.first().map(|c| c.is_ascii_alphabetic()) != Some(true) {
        return summarize_command_limited(&s, max_len);
    }
    while tool_end < bytes.len() && is_word_byte(bytes[tool_end]) {
        tool_end += 1;
    }
    if tool_end >= bytes.len() || bytes[tool_end] != b':' {
        return summarize_command_limited(&s, max_len);
    }
    let tool = &s[..tool_end];
    let mut rest = s[tool_end + 1..].trim_start();
    if rest.is_empty() {
        return truncate(tool, max_len);
    }
    if let Some(idx) = find_key(rest, "command=") {
        let value = &rest[idx + "command=".len()..];
        let cmd = strip_trailing_scalar(value);
        let cmd = cmd.trim();
        let budget = 12.max(max_len.saturating_sub(tool.len() + 1));
        let summarized = summarize_command_limited(cmd, budget);
        return truncate(&format!("{tool} {summarized}"), max_len);
    }
    if let Some(idx) = find_key(rest, "path=") {
        let value_start = idx + "path=".len();
        let value = &rest[value_start..];
        let end = value.find(char::is_whitespace).unwrap_or(value.len());
        let path = &value[..end];
        let base = folder_name(path);
        let base = if base.is_empty() {
            path.to_string()
        } else {
            base
        };
        return truncate(&format!("{tool} {base}"), max_len);
    }
    if let Some((name_len, value)) = first_kv(rest) {
        let _ = name_len;
        let budget = 12.max(max_len.saturating_sub(tool.len() + 1));
        let summarized = summarize_command_limited(value, budget);
        return truncate(&format!("{tool} {summarized}"), max_len);
    }
    let budget = 12.max(max_len.saturating_sub(tool.len() + 1));
    let summarized = summarize_command_limited(rest, budget);
    let _ = &mut rest;
    truncate(&format!("{tool} {summarized}"), max_len)
}

fn find_key(rest: &str, key: &str) -> Option<usize> {
    if rest.starts_with(key) {
        return Some(0);
    }
    let b = rest.as_bytes();
    let kb = key.as_bytes();
    for i in 0..b.len() {
        if b[i].is_ascii_whitespace() && rest[i + 1..].starts_with(key) {
            return Some(i + 1);
        }
        let _ = kb;
    }
    None
}

fn first_kv(rest: &str) -> Option<(usize, &str)> {
    let b = rest.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return None;
    }
    let mut i = 1;
    while i < b.len() && is_word_byte(b[i]) {
        i += 1;
    }
    if i >= b.len() || b[i] != b'=' {
        return None;
    }
    Some((i, &rest[i + 1..]))
}

fn bridge_state_label(state: AgentBridgeState) -> &'static str {
    BRIDGE_LABELS
        .iter()
        .find(|(s, _)| *s == state)
        .map(|(_, label)| *label)
        .unwrap_or("")
}

fn derive_context(s: &TabLabelInput) -> Option<String> {
    if s.closed {
        if let Some(code) = s.exit_code {
            if code != 0 {
                return Some(format!("exit {code}"));
            }
            return Some(
                s.status
                    .clone()
                    .filter(|st| !st.is_empty())
                    .unwrap_or_else(|| "exited".to_string()),
            );
        }
        return Some(
            s.status
                .clone()
                .filter(|st| !st.is_empty())
                .unwrap_or_else(|| "closed".to_string()),
        );
    }
    if let Some(status) = &s.status {
        let lower = status.to_lowercase();
        if lower.starts_with("reconnecting") || lower.starts_with("failed:") {
            return Some(status.clone());
        }
        if lower.starts_with("waiting") || lower.starts_with("connecting") {
            return Some(status.clone());
        }
    }
    if s.agent_pending_approval {
        return Some(format!(
            "{}: awaiting approval",
            agent_label(s.agent_kind.as_deref())
        ));
    }
    if let Some(task) = &s.agent_task {
        let prefix = if s.agent_kind.is_some() {
            format!("{}: ", agent_label(s.agent_kind.as_deref()))
        } else {
            String::new()
        };
        let budget = 16.max(TAB_CONTEXT_MAX.saturating_sub(prefix.chars().count()));
        let summarized = summarize_agent_task_limited(task, budget);
        return Some(format!("{prefix}{summarized}"));
    }
    if let Some(state) = s.agent_bridge_state {
        if state != AgentBridgeState::Connected && state != AgentBridgeState::Stopped {
            return Some(bridge_state_label(state).to_string());
        }
    }
    if let Some(cmd) = &s.current_command {
        if !cmd.is_empty() {
            return Some(summarize_command(cmd));
        }
    }
    if let Some(cwd) = &s.cwd {
        let folder = folder_name(cwd);
        if !folder.is_empty() {
            return Some(folder);
        }
    }
    if let Some(status) = &s.status {
        if !status.to_lowercase().starts_with("connected") {
            return Some(status.clone());
        }
    }
    None
}

fn build_tooltip(s: &TabLabelInput, title: &str, context: Option<&str>) -> String {
    let mut parts = vec![title.to_string()];
    if let Some(task) = &s.agent_task {
        let full = collapse_ws(task);
        if s.agent_kind.is_some() {
            parts.push(format!("{}: {full}", agent_label(s.agent_kind.as_deref())));
        } else {
            parts.push(full);
        }
    } else if let Some(cmd) = &s.current_command {
        if !cmd.is_empty() {
            parts.push(collapse_ws(cmd));
        }
    } else if let Some(context) = context {
        if context != title {
            parts.push(context.to_string());
        }
    }
    if let Some(cwd) = &s.cwd {
        parts.push(format!("cwd: {cwd}"));
    }
    if let Some(code) = s.exit_code {
        parts.push(format!("exit: {code}"));
    }
    if let Some(status) = &s.status {
        if !parts.iter().any(|p| p == status) {
            parts.push(status.clone());
        }
    }
    if s.kind == Some(TabKind::Remote) {
        if let Some(host) = &s.hostname {
            parts.push(format!("host: {host}"));
        }
    }
    parts.join("\n")
}

fn default_base_title(s: &TabLabelInput) -> String {
    if let Some(title) = &s.title {
        if !title.starts_with("pending-") {
            return title.clone();
        }
    }
    match s.kind {
        Some(TabKind::Local) => format!(
            "Local {}",
            s.local_num
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".to_string())
        ),
        Some(TabKind::Remote) => {
            if let Some(host) = &s.hostname {
                format!("remote · {host}")
            } else {
                "Remote".to_string()
            }
        }
        Some(TabKind::Browser) => "Browser".to_string(),
        None => "Terminal".to_string(),
    }
}

pub fn derive_tab_label(s: &TabLabelInput) -> TabLabel {
    let title = if s.custom_title {
        if let Some(title) = &s.title {
            if !title.is_empty() {
                title.clone()
            } else {
                default_base_title(s)
            }
        } else {
            default_base_title(s)
        }
    } else {
        default_base_title(s)
    };
    let context = derive_context(s);
    let shown = context.as_ref().filter(|c| c.as_str() != title).cloned();
    let tooltip = build_tooltip(s, &title, context.as_deref());
    TabLabel {
        title,
        context: shown,
        tooltip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_command_cases() {
        assert_eq!(summarize_command(""), "");
        assert_eq!(summarize_command("   "), "");
        assert_eq!(summarize_command("ls -la"), "ls -la");
        assert_eq!(summarize_command("command=ls -la"), "ls -la");
        assert_eq!(summarize_command("cmd=ls -la"), "ls -la");
        assert_eq!(
            summarize_command("path=/etc/nginx/nginx.conf"),
            "/etc/nginx/nginx.conf"
        );
        let cmd = "sudo nginx -t <<EOF\nserver {\nlisten 80;\n}\nEOF";
        let out = summarize_command(cmd);
        assert_eq!(out, "sudo nginx -t <<…");
        let piped = "docker build -t myimage . && docker push myimage && docker rmi myimage";
        let out = summarize_command_limited(piped, 30);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.chars().count() <= 30);
        let long = "a".repeat(200);
        let out = summarize_command_limited(&long, 50);
        assert!(out.chars().count() <= 50);
    }

    #[test]
    fn summarize_agent_task_cases() {
        assert_eq!(summarize_agent_task("ls -la"), "ls -la");
        assert_eq!(summarize_agent_task("ping:"), "ping");
        assert_eq!(
            summarize_agent_task("run_command: command=ls -la"),
            "run_command ls -la"
        );
        assert_eq!(
            summarize_agent_task("read_file: path=/etc/nginx/nginx.conf"),
            "read_file nginx.conf"
        );
        assert_eq!(
            summarize_agent_task("write_file: content=hello world"),
            "write_file hello world"
        );
    }

    #[test]
    fn derive_tab_label_exit_codes() {
        let label = derive_tab_label(&TabLabelInput {
            kind: Some(TabKind::Local),
            title: Some("Local 1".into()),
            closed: true,
            exit_code: Some(127),
            ..TabLabelInput::default()
        });
        assert_eq!(label.context.as_deref(), Some("exit 127"));
        assert!(label.tooltip.contains("exit: 127"));

        let label = derive_tab_label(&TabLabelInput {
            kind: Some(TabKind::Local),
            title: Some("Local 1".into()),
            closed: true,
            exit_code: Some(0),
            ..TabLabelInput::default()
        });
        assert_eq!(label.context.as_deref(), Some("exited"));
    }
}
