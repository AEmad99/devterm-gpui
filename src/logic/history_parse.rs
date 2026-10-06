//! Command-history parsing and merge.
//!
//! Port of `src/main/ipc/history-parse.ts`. PSReadLine trailing-backtick lines
//! are reassembled; multi-line records are dropped from the palette lists.

pub const MAX_OUT: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEntry {
    pub command: String,
    pub count: i64,
    pub last: i64,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandStat {
    pub command: String,
    pub count: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryResult {
    pub recent: Vec<String>,
    pub frequent: Vec<CommandStat>,
}

pub fn looks_sensitive(cmd: &str) -> bool {
    sensitive_keyword(cmd)
        || has_akia(cmd)
        || has_gh_token(cmd)
        || has_github_pat(cmd)
        || cmd.contains("-----BEGIN ") && cmd.contains("PRIVATE KEY-----")
}

fn sensitive_keyword(cmd: &str) -> bool {
    // \b(pass(?:word|wd)?|secret|token|api[_-]?key|access[_-]?key|client[_-]?secret|bearer)\b
    let lower = cmd.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !is_word(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && is_word(b[i]) {
            i += 1;
        }
        let word = &lower[start..i];
        if matches!(
            word,
            "pass" | "password" | "passwd" | "secret" | "token" | "bearer"
        ) || word == "apikey"
            || word == "api_key"
            || word == "api-key"
            || word == "accesskey"
            || word == "access_key"
            || word == "access-key"
            || word == "clientsecret"
            || word == "client_secret"
            || word == "client-secret"
        {
            return true;
        }
    }
    false
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn has_akia(cmd: &str) -> bool {
    let b = cmd.as_bytes();
    let mut i = 0;
    while i + 20 <= b.len() {
        let boundary_before = i == 0 || !is_word(b[i - 1]);
        if boundary_before && b[i..].starts_with(b"AKIA") {
            let rest = &b[i + 4..i + 20];
            if rest
                .iter()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                let after = i + 20;
                if after == b.len() || !is_word(b[after]) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn has_gh_token(cmd: &str) -> bool {
    // \bgh[pousr]_[A-Za-z0-9]{20,}\b
    let b = cmd.as_bytes();
    let mut i = 0;
    while i + 4 < b.len() {
        let boundary_before = i == 0 || !is_word(b[i - 1]);
        if boundary_before
            && b[i] == b'g'
            && b[i + 1] == b'h'
            && matches!(b[i + 2], b'p' | b'o' | b'u' | b's' | b'r')
            && b[i + 3] == b'_'
        {
            let mut j = i + 4;
            while j < b.len() && (b[j].is_ascii_alphanumeric()) {
                j += 1;
            }
            if j - (i + 4) >= 20 && (j == b.len() || !is_word(b[j])) {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn has_github_pat(cmd: &str) -> bool {
    let needle = "github_pat_";
    if let Some(idx) = cmd.find(needle) {
        let b = cmd.as_bytes();
        if idx > 0 && is_word(b[idx - 1]) {
            return false;
        }
        let mut j = idx + needle.len();
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        return j - (idx + needle.len()) >= 20;
    }
    false
}

pub fn clean(line: &str) -> String {
    line.trim_end_matches('\r').trim().to_string()
}

pub fn split_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\r' && i + 1 < chars.len() && chars[i + 1] == '\n' {
            out.push(std::mem::take(&mut cur));
            i += 2;
        } else if chars[i] == '\n' || chars[i] == '\r' {
            out.push(std::mem::take(&mut cur));
            i += 1;
        } else {
            cur.push(chars[i]);
            i += 1;
        }
    }
    out.push(cur);
    out
}

pub fn strip_zsh(line: &str) -> String {
    let bytes = line.as_bytes();
    if bytes.first() != Some(&b':') {
        return line.to_string();
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let dig_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == dig_start || i >= bytes.len() || bytes[i] != b':' {
        return line.to_string();
    }
    i += 1;
    let dig2 = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == dig2 || i >= bytes.len() || bytes[i] != b';' {
        return line.to_string();
    }
    line[i + 1..].to_string()
}

/// Reassemble PSReadLine trailing-backtick continuations.
/// A line ending in a backtick continues onto the next; exactly one backtick
/// is stripped. An unterminated continuation at EOF is dropped.
pub fn parse_ps_read_line(text: &str) -> Vec<String> {
    let mut records = Vec::new();
    let mut pending: Option<String> = None;
    for line in split_lines(text) {
        if line.ends_with('`') {
            let mut next = pending.take().unwrap_or_default();
            next.push_str(&line[..line.len() - 1]);
            next.push('\n');
            pending = Some(next);
        } else if let Some(p) = pending.take() {
            records.push(format!("{p}{line}"));
        } else {
            records.push(line);
        }
    }
    records
}

pub fn history_key(command: &str) -> String {
    let mut s = String::new();
    let mut prev_space = false;
    for c in command.chars() {
        if c == '\'' || c == '"' {
            continue;
        }
        let c = c.to_ascii_lowercase();
        if c.is_whitespace() {
            if !prev_space && !s.is_empty() {
                s.push(' ');
                prev_space = true;
            }
            continue;
        }
        prev_space = false;
        s.push(c);
    }
    let s = s.trim().to_string();
    s.trim_end_matches(['/', '\\']).to_string()
}

pub fn merge_history(external_chrono: &[String], in_app: &[StoredEntry]) -> HistoryResult {
    let mut counts: Vec<(String, String, i64)> = Vec::new();
    let bump = |counts: &mut Vec<(String, String, i64)>, display: &str, n: i64| {
        let key = history_key(display);
        if let Some(row) = counts.iter_mut().find(|(k, _, _)| k == &key) {
            row.1 = display.to_string();
            row.2 += n;
        } else {
            counts.push((key, display.to_string(), n));
        }
    };

    let mut ext = Vec::new();
    for raw in external_chrono {
        let c = clean(raw);
        if c.is_empty() || c.contains('\n') || looks_sensitive(&c) {
            continue;
        }
        ext.push(c.clone());
        bump(&mut counts, &c, 1);
    }
    let mut in_app_sorted = in_app.to_vec();
    in_app_sorted.sort_by_key(|e| e.last);
    for e in &in_app_sorted {
        if e.command.is_empty() || looks_sensitive(&e.command) {
            continue;
        }
        bump(&mut counts, &e.command, e.count);
    }

    let mut recent = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = in_app.to_vec();
    ordered.sort_by(|a, b| b.last.cmp(&a.last));
    let mut seq: Vec<String> = ordered.into_iter().map(|e| e.command).collect();
    let mut rev = ext.clone();
    rev.reverse();
    seq.extend(rev);
    for c in seq {
        if c.is_empty() || c.contains('\n') || looks_sensitive(&c) {
            continue;
        }
        let k = history_key(&c);
        if !seen.insert(k) {
            continue;
        }
        recent.push(c);
        if recent.len() >= MAX_OUT {
            break;
        }
    }

    let mut frequent: Vec<CommandStat> = counts
        .into_iter()
        .map(|(_, command, count)| CommandStat { command, count })
        .collect();
    frequent.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.command.cmp(&b.command))
    });
    frequent.truncate(MAX_OUT);
    HistoryResult { recent, frequent }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(command: &str, count: i64, last: i64) -> StoredEntry {
        StoredEntry {
            command: command.to_string(),
            count,
            last,
            scope: "local".into(),
        }
    }

    #[test]
    fn split_lines_handles_endings() {
        assert_eq!(split_lines("a\nb"), vec!["a", "b"]);
        assert_eq!(split_lines("a\r\nb"), vec!["a", "b"]);
        assert_eq!(split_lines("a\rb"), vec!["a", "b"]);
        assert_eq!(split_lines("a\r\nb\r\nc"), vec!["a", "b", "c"]);
        assert_eq!(split_lines("a\n"), vec!["a", ""]);
    }

    #[test]
    fn parse_ps_read_line_cases() {
        assert_eq!(
            parse_ps_read_line("git status\r\nnpm run dev\r\n"),
            vec!["git status", "npm run dev", ""]
        );
        assert_eq!(
            parse_ps_read_line("git status\r\nnpm run dev"),
            vec!["git status", "npm run dev"]
        );
        let file = "cd D:\\projects\\my-term`\r\nD:\\projects\\my-term\r\ngit status\r\n";
        assert_eq!(
            parse_ps_read_line(file),
            vec![
                "cd D:\\projects\\my-term\nD:\\projects\\my-term",
                "git status",
                ""
            ]
        );
        let file = "cd D:\\projects\\my-term``\r\nD:\\projects\\my-term\r\n";
        assert_eq!(
            parse_ps_read_line(file),
            vec!["cd D:\\projects\\my-term`\nD:\\projects\\my-term", ""]
        );
        let file = "foreach ($f in $files) `\r\n{\r\n  echo $f `\r\n}\r\n";
        assert_eq!(
            parse_ps_read_line(file),
            vec!["foreach ($f in $files) \n{", "  echo $f \n}", ""]
        );
        assert_eq!(
            parse_ps_read_line("git status\r\ncd D:\\projects\\my-term`"),
            vec!["git status"]
        );
    }

    #[test]
    fn history_key_normalizes() {
        assert_eq!(
            history_key("CD D:\\projects\\my-term"),
            history_key("cd d:\\projects\\my-term")
        );
        assert_eq!(
            history_key("cd 'D:\\projects\\my-term'"),
            history_key("cd D:\\projects\\my-term")
        );
        assert_eq!(
            history_key("cd \"D:\\projects\\my-term\""),
            history_key("cd D:\\projects\\my-term")
        );
        assert_eq!(
            history_key("CD D:\\projects\\my-term\\"),
            history_key("cd 'D:\\projects\\my-term'")
        );
        assert_eq!(history_key("npm  run   dev"), history_key("npm run dev"));
        assert_ne!(history_key("git status"), history_key("git stash"));
    }

    #[test]
    fn merge_drops_multiline_junk_and_collapses_variants() {
        let file = [
            "git status",
            "cd D:\\projects\\my-term`",
            "D:\\projects\\my-term",
            "CD D:\\projects\\my-term\\",
            "cd 'D:\\projects\\my-term'",
            "npm run dev",
        ]
        .join("\r\n");
        let HistoryResult { recent, frequent } = merge_history(&parse_ps_read_line(&file), &[]);
        let mut all = recent.clone();
        all.extend(frequent.iter().map(|f| f.command.clone()));
        assert!(all.iter().all(|c| !c.ends_with('`')));
        assert!(all.iter().all(|c| !c.contains('\n')));
        assert!(all
            .iter()
            .all(|c| c != "cd D:\\projects\\my-termD:\\projects\\my-term"));
        let cds: Vec<_> = all
            .iter()
            .filter(|c| history_key(c) == history_key("cd D:\\projects\\my-term"))
            .collect();
        let mut uniq = cds.clone();
        uniq.dedup();
        assert_eq!(uniq.len(), 1);
    }

    #[test]
    fn collapse_keeps_most_recent_display() {
        let external = vec![
            "CD D:\\projects\\my-term\\".to_string(),
            "cd 'D:\\projects\\my-term'".to_string(),
        ];
        let h = merge_history(&external, &[]);
        assert_eq!(h.recent, vec!["cd 'D:\\projects\\my-term'"]);
        assert_eq!(
            h.frequent,
            vec![CommandStat {
                command: "cd 'D:\\projects\\my-term'".into(),
                count: 2
            }]
        );
    }

    #[test]
    fn newer_in_app_variant_wins() {
        let h = merge_history(&["git status".into()], &[entry("GIT status", 1, 10)]);
        assert_eq!(h.recent, vec!["GIT status"]);
        assert_eq!(
            h.frequent,
            vec![CommandStat {
                command: "GIT status".into(),
                count: 2
            }]
        );
    }

    #[test]
    fn dedupes_recent_by_key() {
        let h = merge_history(&["npm test".into()], &[entry("NPM TEST", 1, 5)]);
        assert_eq!(h.recent, vec!["NPM TEST"]);
    }

    #[test]
    fn excludes_multiline() {
        let h = merge_history(&["echo one\necho two".into(), "echo three".into()], &[]);
        assert_eq!(h.recent, vec!["echo three"]);
        assert_eq!(
            h.frequent,
            vec![CommandStat {
                command: "echo three".into(),
                count: 1
            }]
        );
    }

    #[test]
    fn filters_sensitive_and_blanks() {
        let h = merge_history(
            &[
                "".into(),
                "   ".into(),
                "export TOKEN=abc123".into(),
                "git status".into(),
            ],
            &[entry("set password=hunter2", 1, 0)],
        );
        assert_eq!(h.recent, vec!["git status"]);
        assert_eq!(
            h.frequent,
            vec![CommandStat {
                command: "git status".into(),
                count: 1
            }]
        );
    }

    #[test]
    fn sums_counts_across_variants() {
        let in_app = vec![entry("git pull", 3, 1), entry("GIT PULL", 2, 2)];
        let h = merge_history(&["git pull".into()], &in_app);
        assert_eq!(h.recent, vec!["GIT PULL"]);
        assert_eq!(
            h.frequent,
            vec![CommandStat {
                command: "GIT PULL".into(),
                count: 6
            }]
        );
    }

    #[test]
    fn clean_strip_zsh_sensitive() {
        assert_eq!(clean("  git status \r"), "git status");
        assert_eq!(strip_zsh(": 1700000000:0;git status"), "git status");
        assert_eq!(strip_zsh("git status"), "git status");
        assert!(looks_sensitive(
            "curl -H \"Authorization: Bearer abc\" example.com"
        ));
        assert!(!looks_sensitive("git status"));
    }
}
