//! Snippet `{{placeholder}}` substitution and a session-only value cache.
//!
//! Placeholder values are not written to a durable store by these helpers.
//! The frecency helpers live here too because the snippet tests score history
//! against the saved snippet set.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const PLACEHOLDER_CACHE_TTL_MS: u64 = 5 * 60 * 1000;
const CACHE_PREFIX: &str = "devterm.placeholder.v1.";
const CACHE_INDEX_KEY: &str = "devterm.placeholder.v1.__index__";

pub const RECENCY_WEIGHT: f64 = 0.6;
pub const COUNT_WEIGHT: f64 = 0.4;

pub trait KvStore {
    fn get_item(&self, key: &str) -> Option<String>;
    fn set_item(&mut self, key: &str, value: &str);
    fn remove_item(&mut self, key: &str);
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    map: BTreeMap<String, String>,
}

impl KvStore for MemoryStore {
    fn get_item(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }
    fn set_item(&mut self, key: &str, value: &str) {
        self.map.insert(key.to_string(), value.to_string());
    }
    fn remove_item(&mut self, key: &str) {
        self.map.remove(key);
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// Unique placeholder names in `command`, in first-seen order.
pub fn extract_placeholders(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut seen = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '{' && chars[i + 1] == '{' {
            let mut j = i + 2;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let start = j;
            while j < chars.len() && is_name_char(chars[j]) {
                j += 1;
            }
            let name: String = chars[start..j].iter().collect();
            let mut k = j;
            while k < chars.len() && chars[k].is_whitespace() {
                k += 1;
            }
            if !name.is_empty() && k + 1 < chars.len() && chars[k] == '}' && chars[k + 1] == '}' {
                if !seen.iter().any(|s: &String| s == &name) {
                    seen.push(name);
                }
                i = k + 2;
                continue;
            }
        }
        i += 1;
    }
    seen
}

/// Substitute placeholder values. Unfilled tokens are left as-is.
pub fn apply_placeholders(command: &str, values: &HashMap<String, String>) -> String {
    let chars: Vec<char> = command.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '{' && chars[i + 1] == '{' {
            let mut j = i + 2;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let start = j;
            while j < chars.len() && is_name_char(chars[j]) {
                j += 1;
            }
            let name: String = chars[start..j].iter().collect();
            let mut k = j;
            while k < chars.len() && chars[k].is_whitespace() {
                k += 1;
            }
            if !name.is_empty() && k + 1 < chars.len() && chars[k] == '}' && chars[k + 1] == '}' {
                if let Some(value) = values.get(&name) {
                    out.push_str(value);
                } else {
                    for c in &chars[i..=k + 1] {
                        out.push(*c);
                    }
                }
                i = k + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn cache_key(snippet_id: &str, placeholder_name: &str) -> String {
    format!("{CACHE_PREFIX}{snippet_id}::{placeholder_name}")
}

pub fn get_cached_placeholder(
    snippet_id: &str,
    placeholder_name: &str,
    storage: &mut dyn KvStore,
    now_ms: u64,
) -> Option<String> {
    let key = cache_key(snippet_id, placeholder_name);
    let raw = storage.get_item(&key)?;
    let parsed = parse_cache_entry(&raw);
    let Some((value, t)) = parsed else {
        return None;
    };
    if now_ms.saturating_sub(t) > PLACEHOLDER_CACHE_TTL_MS {
        storage.remove_item(&key);
        return None;
    }
    Some(value)
}

fn parse_cache_entry(raw: &str) -> Option<(String, u64)> {
    let v_key = raw.find("\"v\"")?;
    let after = &raw[v_key + 3..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let mut value = String::new();
    let bytes = rest.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            value.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            break;
        }
        value.push(bytes[i] as char);
        i += 1;
    }
    let t_key = raw.find("\"t\"")?;
    let t_after = &raw[t_key + 3..];
    let t_colon = t_after.find(':')?;
    let t_rest = t_after[t_colon + 1..].trim_start();
    let mut num = String::new();
    for c in t_rest.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            break;
        }
    }
    let t = num.parse().ok()?;
    Some((value, t))
}

pub fn set_cached_placeholder(
    snippet_id: &str,
    placeholder_name: &str,
    value: &str,
    storage: &mut dyn KvStore,
    now_ms: u64,
) {
    let key = cache_key(snippet_id, placeholder_name);
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    storage.set_item(&key, &format!("{{\"v\":\"{escaped}\",\"t\":{now_ms}}}"));
    let index_raw = storage.get_item(CACHE_INDEX_KEY);
    let mut index: Vec<String> = index_raw
        .as_deref()
        .and_then(parse_string_array)
        .unwrap_or_default();
    if !index.iter().any(|k| k == &key) {
        index.push(key);
    }
    storage.set_item(CACHE_INDEX_KEY, &format_string_array(&index));
}

fn parse_string_array(raw: &str) -> Option<Vec<String>> {
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return None;
    }
    let inner = &raw[1..raw.len() - 1];
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for part in inner.split(',') {
        let part = part.trim().trim_matches('"');
        out.push(part.to_string());
    }
    Some(out)
}

fn format_string_array(keys: &[String]) -> String {
    let body = keys
        .iter()
        .map(|k| format!("\"{k}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

pub fn clear_cached_placeholders(storage: &mut dyn KvStore) -> usize {
    let raw = storage.get_item(CACHE_INDEX_KEY);
    let keys = raw
        .as_deref()
        .and_then(parse_string_array)
        .unwrap_or_default();
    let mut removed = 0;
    for k in &keys {
        storage.remove_item(k);
        removed += 1;
    }
    storage.remove_item(CACHE_INDEX_KEY);
    removed
}

pub fn prefilled_values(
    snippet_id: &str,
    command: &str,
    storage: &mut dyn KvStore,
    now_ms: u64,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for name in extract_placeholders(command) {
        if let Some(v) = get_cached_placeholder(snippet_id, &name, storage, now_ms) {
            out.insert(name, v);
        }
    }
    out
}

pub fn persist_values(
    snippet_id: &str,
    command: &str,
    values: &HashMap<String, String>,
    storage: &mut dyn KvStore,
    now_ms: u64,
) {
    let names: BTreeSet<String> = extract_placeholders(command).into_iter().collect();
    for (name, value) in values {
        if names.contains(name) {
            set_cached_placeholder(snippet_id, name, value, storage, now_ms);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandStat {
    pub command: String,
    pub count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryResult {
    pub recent: Vec<String>,
    pub frequent: Vec<CommandStat>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrecencyEntry {
    pub command: String,
    pub count: u32,
    pub score: f64,
    pub recency_index: i32,
}

pub fn build_frecency(hist: Option<&HistoryResult>, _now: u64) -> Vec<FrecencyEntry> {
    let Some(hist) = hist else {
        return Vec::new();
    };
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut recent_indexes: BTreeMap<String, i32> = BTreeMap::new();
    for f in &hist.frequent {
        if f.command.is_empty() {
            continue;
        }
        let slot = counts.entry(f.command.clone()).or_insert(0);
        *slot = (*slot).max(f.count);
    }
    for (i, cmd) in hist.recent.iter().enumerate() {
        if cmd.is_empty() {
            continue;
        }
        counts.entry(cmd.clone()).or_insert(1);
        recent_indexes.entry(cmd.clone()).or_insert(i as i32);
    }
    let mut all: BTreeSet<String> = BTreeSet::new();
    all.extend(counts.keys().cloned());
    all.extend(recent_indexes.keys().cloned());
    let mut out = Vec::new();
    for cmd in all {
        let count = counts.get(&cmd).copied().unwrap_or(0);
        let recency_index = recent_indexes.get(&cmd).copied().unwrap_or(-1);
        let age_days = if recency_index < 0 {
            30.0
        } else {
            recency_index as f64
        };
        let recency_term = 1.0 / (age_days + 1.0);
        let count_term = ((count as f64) + 1.0).log10();
        let score = RECENCY_WEIGHT * recency_term + COUNT_WEIGHT * count_term;
        out.push(FrecencyEntry {
            command: cmd,
            count,
            score,
            recency_index,
        });
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.recency_index.cmp(&b.recency_index))
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.command.cmp(&b.command))
    });
    out
}

pub fn normalize_for_dedupe(command: &str) -> String {
    let trimmed_end = command.trim_end_matches(|c: char| c.is_whitespace());
    trimmed_end
        .trim_start_matches(|c: char| c.is_whitespace())
        .to_string()
}

pub fn snippet_command_set(snippets: &[impl AsRef<str>]) -> BTreeSet<String> {
    snippets
        .iter()
        .map(|s| normalize_for_dedupe(s.as_ref()))
        .collect()
}

pub fn filter_history(
    entries: &[FrecencyEntry],
    snippet_cmds: &BTreeSet<String>,
    query: &str,
    max: usize,
) -> Vec<FrecencyEntry> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    let mut out = Vec::new();
    for e in entries {
        if snippet_cmds.contains(&normalize_for_dedupe(&e.command)) {
            continue;
        }
        if !terms.is_empty() {
            let hay = e.command.to_lowercase();
            if terms.iter().any(|t| !hay.contains(t)) {
                continue;
            }
        }
        out.push(e.clone());
        if out.len() >= max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_placeholders_cases() {
        assert!(extract_placeholders("ls -la").is_empty());
        assert_eq!(
            extract_placeholders("git log {{ref}} --grep {{pattern}}"),
            vec!["ref", "pattern"]
        );
        assert_eq!(
            extract_placeholders("{{name}} {{name}} {{other}}"),
            vec!["name", "other"]
        );
        assert!(extract_placeholders("{{}} {{ }}").is_empty());
        assert_eq!(extract_placeholders("{{HOST_NAME_2}}"), vec!["HOST_NAME_2"]);
    }

    #[test]
    fn apply_placeholders_cases() {
        let mut values = HashMap::new();
        values.insert("msg".into(), "hi".into());
        assert_eq!(
            apply_placeholders("echo {{msg}} {{msg}}", &values),
            "echo hi hi"
        );
        let mut values = HashMap::new();
        values.insert("a".into(), "x".into());
        assert_eq!(
            apply_placeholders("echo {{a}} {{b}}", &values),
            "echo x {{b}}"
        );
        assert_eq!(apply_placeholders("ls -la", &HashMap::new()), "ls -la");
    }

    #[test]
    fn normalize_for_dedupe_cases() {
        assert_eq!(normalize_for_dedupe("  ls -la  "), "ls -la");
        assert_eq!(normalize_for_dedupe("ls\n"), "ls");
        assert_eq!(normalize_for_dedupe("ls\n\n\n"), "ls");
        assert_eq!(normalize_for_dedupe("  Git   Status "), "Git   Status");
    }

    #[test]
    fn build_frecency_ranks_recent_commands() {
        let hist = HistoryResult {
            recent: vec![
                "ls".into(),
                "docker ps".into(),
                "git status".into(),
                "pwd".into(),
                "ls -la".into(),
                "echo hi".into(),
                "cat foo".into(),
            ],
            frequent: vec![CommandStat {
                command: "ls".into(),
                count: 1,
            }],
        };
        let ranked = build_frecency(Some(&hist), 0);
        assert_eq!(ranked[0].command, "ls");
        assert!(build_frecency(None, 0).is_empty());
    }

    #[test]
    fn build_frecency_keeps_frequent_only_commands() {
        let hist = HistoryResult {
            recent: vec!["ls".into()],
            frequent: vec![CommandStat {
                command: "kubectl get pods".into(),
                count: 100,
            }],
        };
        let ranked = build_frecency(Some(&hist), 0);
        let cmds: Vec<_> = ranked.into_iter().map(|e| e.command).collect();
        assert!(cmds.iter().any(|c| c == "kubectl get pods"));
        assert!(cmds.iter().any(|c| c == "ls"));
    }

    #[test]
    fn filter_history_removes_saved_snippets() {
        let snippets = ["git status", "ls -la"];
        let dedupe = snippet_command_set(&snippets);
        let ranked = build_frecency(
            Some(&HistoryResult {
                recent: vec![
                    "git status".into(),
                    "docker ps".into(),
                    "ls -la".into(),
                    "pwd".into(),
                ],
                frequent: vec![],
            }),
            0,
        );
        let filtered = filter_history(&ranked, &dedupe, "", 50);
        let cmds: Vec<_> = filtered.into_iter().map(|e| e.command).collect();
        assert_eq!(cmds, vec!["docker ps", "pwd"]);
    }
}
