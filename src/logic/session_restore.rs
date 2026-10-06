//! Session-restore snapshot build / normalize.
//!
//! Port of the pure parts of `src/renderer/lib/session-restore.ts`,
//! `src/main/ipc/session-restore.ts`, and `src/main/terminal/scrollback.ts`.
//!
//! Boot order (documented as data): auto-launch workspaces, then the last
//! session snapshot, then one local terminal.
//!
//! A local PTY is not restored as a live process — only a bounded raw ANSI
//! tail. Passwords are never written into the restore JSON. An ad-hoc SSH
//! draft whose secret is missing comes back `needs-auth`.

pub const DEFAULT_PERSISTED_SCROLLBACK_LINES: usize = 2_000;
pub const MAX_PERSISTED_SCROLLBACK_LINES: usize = 10_000;
pub const MAX_PERSISTED_SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_TOTAL_PERSISTED_SCROLLBACK_BYTES: usize = 32 * 1024 * 1024;

/// Documented startup order from `App.tsx`.
pub const BOOT_ORDER: &[&str] = &[
    "auto-launch workspaces",
    "session snapshot",
    "one local terminal",
];

/// Local shells are not respawned from the snapshot. Only the ANSI tail is kept.
pub fn restores_live_local_pty() -> bool {
    false
}

pub fn trim_session_scrollback(
    raw: &str,
    max_lines: Option<f64>,
    max_bytes: Option<f64>,
) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let line_limit = clamp_limit(
        max_lines,
        DEFAULT_PERSISTED_SCROLLBACK_LINES,
        MAX_PERSISTED_SCROLLBACK_LINES,
    );
    let byte_limit = clamp_limit(
        max_bytes,
        MAX_PERSISTED_SCROLLBACK_BYTES,
        MAX_PERSISTED_SCROLLBACK_BYTES,
    );
    let lines = lines_keeping_newlines(raw);
    let start = lines.len().saturating_sub(line_limit);
    let joined = lines[start..].concat();
    suffix_within_bytes(&joined, byte_limit)
}

fn clamp_limit(value: Option<f64>, default_value: usize, hard_max: usize) -> usize {
    let n = match value {
        Some(v) if v.is_finite() => v.floor() as i64,
        _ => default_value as i64,
    };
    n.clamp(1, hard_max as i64) as usize
}

fn lines_keeping_newlines(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            out.push(&raw[start..=i]);
            start = i + 1;
        }
    }
    if start < raw.len() {
        out.push(&raw[start..]);
    }
    out.retain(|l| !l.is_empty());
    out
}

fn suffix_within_bytes(value: &str, max_bytes: usize) -> String {
    if value.is_empty() || max_bytes == 0 {
        return String::new();
    }
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let chars: Vec<usize> = value.char_indices().map(|(i, _)| i).collect();
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high) / 2;
        let byte_idx = chars[mid];
        if value[byte_idx..].len() <= max_bytes {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    let mut start = chars.get(low).copied().unwrap_or(value.len());
    // JS adjusts if `start` lands on a low surrogate. Rust chars are scalars,
    // so the index is already a boundary.
    if start > value.len() {
        start = value.len();
    }
    value[start..].to_string()
}

/// Trim each session's tail, then the 32 MiB total budget, newest-first order
/// as supplied (caller passes sessions in snapshot order; the budget is
/// consumed from the front, matching the main-process save loop).
pub fn apply_scrollback_budget(raws: &[String]) -> Vec<Option<String>> {
    let mut remaining = MAX_TOTAL_PERSISTED_SCROLLBACK_BYTES;
    let mut out = Vec::with_capacity(raws.len());
    for raw in raws {
        if remaining == 0 {
            out.push(None);
            continue;
        }
        let cap = remaining.min(MAX_PERSISTED_SCROLLBACK_BYTES);
        let trimmed = trim_session_scrollback(
            raw,
            Some(DEFAULT_PERSISTED_SCROLLBACK_LINES as f64),
            Some(cap as f64),
        );
        if trimmed.is_empty() {
            out.push(None);
        } else {
            remaining = remaining.saturating_sub(trimmed.len());
            out.push(Some(trimmed));
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshDraft {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub private_key_path: Option<String>,
    pub has_passphrase: bool,
    pub use_agent: Option<bool>,
    /// Runtime only. Never serialized into session-restore.json.
    pub password: Option<String>,
    pub passphrase: Option<String>,
    pub secret_id: Option<String>,
}

/// Ad-hoc SSH without the secret it needs cannot reconnect silently.
pub fn needs_auth(draft: &SshDraft) -> bool {
    let missing_password = draft.auth_method == "password"
        && draft.password.as_deref().map(str::is_empty).unwrap_or(true);
    let missing_passphrase = draft.auth_method == "key"
        && draft.has_passphrase
        && draft
            .passphrase
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true);
    let missing_none = draft.auth_method == "none"
        && draft
            .private_key_path
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true)
        && draft.use_agent != Some(true);
    missing_password || missing_passphrase || missing_none
}

#[derive(Clone, Debug)]
pub struct RestoreItem {
    pub id: String,
    pub kind: String,
    pub connection_id: Option<String>,
    pub ssh_draft: Option<SshDraft>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    /// Raw ANSI tail. Local items keep this instead of a live PTY.
    pub scrollback: Option<String>,
    pub live_session_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RestoreGroup {
    pub name: String,
    pub items: Vec<RestoreItem>,
}

#[derive(Clone, Debug)]
pub struct RestoreSnapshot {
    pub saved_at: i64,
    pub groups: Vec<RestoreGroup>,
    pub active_group_index: Option<usize>,
}

/// JSON written to session-restore.json. Passwords, passphrases, and
/// `liveSessionId` are omitted. Scrollback is capped.
pub fn snapshot_json_for_disk(snap: &RestoreSnapshot) -> String {
    let mut scroll_inputs = Vec::new();
    let mut index = Vec::new();
    for (gi, g) in snap.groups.iter().enumerate() {
        for (ii, it) in g.items.iter().enumerate() {
            scroll_inputs.push(it.scrollback.clone().unwrap_or_default());
            index.push((gi, ii));
        }
    }
    let trimmed = apply_scrollback_budget(&scroll_inputs);
    let by_item: Vec<Option<String>> = trimmed;
    let _ = index;

    let mut out = String::from("{\n  \"version\": 1,\n");
    out.push_str(&format!("  \"savedAt\": {},\n", snap.saved_at));
    if let Some(idx) = snap.active_group_index {
        out.push_str(&format!("  \"activeGroupIndex\": {idx},\n"));
    }
    out.push_str("  \"groups\": [\n");
    let mut flat = 0usize;
    for (gi, g) in snap.groups.iter().enumerate().take(20) {
        if gi > 0 {
            out.push_str(",\n");
        }
        out.push_str("    {\n");
        let name = if g.name.trim().is_empty() {
            "Group 1".to_string()
        } else {
            g.name.trim().to_string()
        };
        out.push_str(&format!("      \"name\": {},\n", json_str(&name)));
        out.push_str("      \"items\": [\n");
        for (ii, it) in g.items.iter().take(64).enumerate() {
            if ii > 0 {
                out.push_str(",\n");
            }
            let scroll = by_item.get(flat).cloned().flatten();
            flat += 1;
            out.push_str("        {\n");
            out.push_str(&format!("          \"id\": {},\n", json_str(&it.id)));
            let kind = if it.kind == "remote" || it.kind == "browser" {
                it.kind.as_str()
            } else {
                "local"
            };
            out.push_str(&format!("          \"kind\": {}\n", json_str(kind)));
            if let Some(cid) = &it.connection_id {
                out.push_str(",\n");
                out.push_str(&format!("          \"connectionId\": {}", json_str(cid)));
            }
            if let Some(draft) = &it.ssh_draft {
                out.push_str(",\n          \"sshDraft\": {\n");
                out.push_str(&format!(
                    "            \"host\": {},\n",
                    json_str(&draft.host)
                ));
                out.push_str(&format!("            \"port\": {},\n", draft.port));
                out.push_str(&format!(
                    "            \"username\": {},\n",
                    json_str(&draft.username)
                ));
                out.push_str(&format!(
                    "            \"authMethod\": {}",
                    json_str(&draft.auth_method)
                ));
                if let Some(path) = &draft.private_key_path {
                    out.push_str(",\n");
                    out.push_str(&format!(
                        "            \"privateKeyPath\": {}",
                        json_str(path)
                    ));
                }
                if draft.has_passphrase {
                    out.push_str(",\n            \"hasPassphrase\": true");
                }
                if let Some(agent) = draft.use_agent {
                    out.push_str(&format!(",\n            \"useAgent\": {agent}"));
                }
                if let Some(sid) = &draft.secret_id {
                    out.push_str(",\n");
                    out.push_str(&format!("            \"secretId\": {}", json_str(sid)));
                }
                // password / passphrase / restoreSecret are intentionally absent.
                out.push_str("\n          }");
            }
            if let Some(cwd) = &it.cwd {
                out.push_str(",\n");
                out.push_str(&format!("          \"cwd\": {}", json_str(cwd)));
            }
            if let Some(title) = &it.title {
                out.push_str(",\n");
                out.push_str(&format!("          \"title\": {}", json_str(title)));
            }
            if let Some(scroll) = scroll.as_ref().filter(|s| !s.is_empty()) {
                out.push_str(",\n");
                out.push_str(&format!("          \"scrollback\": {}", json_str(scroll)));
            }
            out.push_str("\n        }");
        }
        out.push_str("\n      ]\n    }");
    }
    out.push_str("\n  ]\n}");
    out
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_raw_ansi_and_only_the_newest_lines() {
        let raw = "\u{1b}[32mone\u{1b}[0m\ntwo\nthree";
        assert_eq!(trim_session_scrollback(raw, Some(2.0), None), "two\nthree");
        assert!(trim_session_scrollback(raw, Some(3.0), None).contains("\u{1b}[32mone"));
    }

    #[test]
    fn uses_the_documented_default_and_hard_line_cap() {
        let lines = (0..DEFAULT_PERSISTED_SCROLLBACK_LINES + 5)
            .map(|i| format!("{i}\n"))
            .collect::<String>();
        let default_tail = trim_session_scrollback(&lines, None, None);
        assert_eq!(
            default_tail.split('\n').filter(|s| !s.is_empty()).count(),
            DEFAULT_PERSISTED_SCROLLBACK_LINES
        );
        let hard = (0..MAX_PERSISTED_SCROLLBACK_LINES + 1)
            .map(|i| format!("{i}\n"))
            .collect::<String>();
        let hard_tail = trim_session_scrollback(
            &hard,
            Some((MAX_PERSISTED_SCROLLBACK_LINES + 1) as f64),
            None,
        );
        assert_eq!(
            hard_tail.split('\n').filter(|s| !s.is_empty()).count(),
            MAX_PERSISTED_SCROLLBACK_LINES
        );
    }

    #[test]
    fn bounds_a_single_unterminated_line_by_bytes() {
        let tail = trim_session_scrollback(&"x".repeat(100), Some(10.0), Some(12.0));
        assert_eq!(tail.len(), 12);
        assert_eq!(tail, "x".repeat(12));
    }

    #[test]
    fn total_budget_is_32_mib_and_per_session_2_mib() {
        let huge = "y".repeat(3 * 1024 * 1024);
        let trimmed = apply_scrollback_budget(&[huge.clone(), huge]);
        assert!(trimmed[0].as_ref().unwrap().len() <= MAX_PERSISTED_SCROLLBACK_BYTES);
        assert!(trimmed[1].as_ref().unwrap().len() <= MAX_PERSISTED_SCROLLBACK_BYTES);
        let sum = trimmed.iter().flatten().map(|s| s.len()).sum::<usize>();
        assert!(sum <= MAX_TOTAL_PERSISTED_SCROLLBACK_BYTES);
    }

    #[test]
    fn password_is_never_written_and_missing_secret_needs_auth() {
        let secret = "s3cret-password";
        let snap = RestoreSnapshot {
            saved_at: 1,
            active_group_index: Some(0),
            groups: vec![RestoreGroup {
                name: "Group 1".into(),
                items: vec![RestoreItem {
                    id: "sr-1".into(),
                    kind: "remote".into(),
                    connection_id: None,
                    ssh_draft: Some(SshDraft {
                        host: "db.internal".into(),
                        port: 22,
                        username: "root".into(),
                        auth_method: "password".into(),
                        private_key_path: None,
                        has_passphrase: false,
                        use_agent: None,
                        password: Some(secret.into()),
                        passphrase: None,
                        secret_id: Some("sec-1".into()),
                    }),
                    cwd: Some("/root".into()),
                    title: None,
                    scrollback: Some("tail".into()),
                    live_session_id: Some("live-pty-9".into()),
                }],
            }],
        };
        let json = snapshot_json_for_disk(&snap);
        assert!(!json.contains(secret), "{json}");
        assert!(!json.contains("\"password\":"));
        assert!(!json.contains("\"passphrase\":"));
        assert!(json.contains("\"authMethod\": \"password\""));
        assert!(!json.contains("liveSessionId"));
        assert!(!json.contains("live-pty-9"));
        assert!(json.contains("\"secretId\": \"sec-1\""));
        assert!(!restores_live_local_pty());
        let mut draft = snap.groups[0].items[0].ssh_draft.clone().unwrap();
        draft.password = None;
        assert!(needs_auth(&draft));
        draft.password = Some(secret.into());
        assert!(!needs_auth(&draft));
    }

    #[test]
    fn boot_order_is_workspaces_then_snapshot_then_one_local() {
        assert_eq!(
            BOOT_ORDER,
            [
                "auto-launch workspaces",
                "session snapshot",
                "one local terminal"
            ]
        );
    }

    #[test]
    fn ad_hoc_key_without_passphrase_secret_needs_auth() {
        let draft = SshDraft {
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_method: "key".into(),
            private_key_path: Some("/home/u/.ssh/id_ed25519".into()),
            has_passphrase: true,
            use_agent: None,
            password: None,
            passphrase: None,
            secret_id: None,
        };
        assert!(needs_auth(&draft));
    }
}
