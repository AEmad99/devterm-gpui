//! Recent `host:port:user` targets from `src/main/ssh/quick-connect.ts`.
//!
//! The store keeps at most [`MAX_ENTRIES`] triples. Recording the same
//! host, port, and username again bumps `lastUsedAt` and moves that row to
//! the newest end. Oldest rows are dropped first. No secrets are stored.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const MAX_ENTRIES: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickConnectEntry {
    pub host: String,
    pub port: i64,
    pub username: String,
    pub last_used_at: i64,
}

pub struct QuickConnect {
    path: PathBuf,
    cache: Option<Vec<QuickConnectEntry>>,
}

impl QuickConnect {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Newest first.
    pub fn list(&mut self) -> Vec<QuickConnectEntry> {
        let mut entries = self.load().clone();
        entries.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
        entries
    }

    pub fn record(&mut self, host: &str, port: i64, username: &str, now_ms: i64) -> io::Result<()> {
        let entries = self.load();
        let key = format!("{host}|{port}|{username}");
        if let Some(index) = entries
            .iter()
            .position(|entry| format!("{}|{}|{}", entry.host, entry.port, entry.username) == key)
        {
            entries.remove(index);
        }
        entries.push(QuickConnectEntry {
            host: host.to_string(),
            port,
            username: username.to_string(),
            last_used_at: now_ms,
        });
        while entries.len() > MAX_ENTRIES {
            entries.remove(0);
        }
        self.persist()
    }

    fn load(&mut self) -> &mut Vec<QuickConnectEntry> {
        if self.cache.is_none() {
            self.cache = Some(read_entries(&self.path));
        }
        self.cache.as_mut().unwrap()
    }

    fn persist(&self) -> io::Result<()> {
        let Some(entries) = self.cache.as_ref() else {
            return Ok(());
        };
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        // `storeFile() + '.tmp'` keeps the original name: `quick-connect.json.tmp`.
        let tmp_path = PathBuf::from(format!("{}.tmp", self.path.display()));
        {
            let mut file = File::create(&tmp_path)?;
            file.write_all(stringify_entries(entries).as_bytes())?;
        }
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

fn read_entries(path: &Path) -> Vec<QuickConnectEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_entries(&text).unwrap_or_default()
}

fn stringify_entries(entries: &[QuickConnectEntry]) -> String {
    if entries.is_empty() {
        return "{\n  \"entries\": []\n}".to_string();
    }
    let mut out = String::from("{\n  \"entries\": [\n");
    for (index, entry) in entries.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"host\": {},\n", json_string(&entry.host)));
        out.push_str(&format!("      \"port\": {},\n", entry.port));
        out.push_str(&format!(
            "      \"username\": {},\n",
            json_string(&entry.username)
        ));
        out.push_str(&format!("      \"lastUsedAt\": {}\n", entry.last_used_at));
        out.push_str("    }");
        if index + 1 != entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}");
    out
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn parse_entries(text: &str) -> Option<Vec<QuickConnectEntry>> {
    let value = parse_json(text)?;
    let Json::Object(pairs) = value else {
        return Some(Vec::new());
    };
    let entries = pairs.into_iter().find(|(key, _)| key == "entries")?;
    let Json::Array(items) = entries.1 else {
        return Some(Vec::new());
    };
    let mut out = Vec::new();
    for item in items {
        if let Some(entry) = entry_from_json(item) {
            out.push(entry);
        }
    }
    Some(out)
}

fn entry_from_json(value: Json) -> Option<QuickConnectEntry> {
    let Json::Object(pairs) = value else {
        return None;
    };
    let mut host = None;
    let mut port = None;
    let mut username = None;
    let mut last_used_at = None;
    for (key, value) in pairs {
        match (key.as_str(), value) {
            ("host", Json::String(text)) => host = Some(text),
            ("port", Json::Number(number)) if number.fract() == 0.0 => port = Some(number as i64),
            ("username", Json::String(text)) => username = Some(text),
            ("lastUsedAt", Json::Number(number)) if number.fract() == 0.0 => {
                last_used_at = Some(number as i64)
            }
            _ => {}
        }
    }
    Some(QuickConnectEntry {
        host: host?,
        port: port?,
        username: username?,
        last_used_at: last_used_at?,
    })
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

fn parse_json(text: &str) -> Option<Json> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        index: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.index != parser.bytes.len() {
        return None;
    }
    Some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.index += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.index += 1;
        Some(b)
    }

    fn parse_value(&mut self) -> Option<Json> {
        self.skip_ws();
        match self.peek()? {
            b'n' => self.consume(b"null").then_some(Json::Null),
            b't' => self.consume(b"true").then_some(Json::Bool(true)),
            b'f' => self.consume(b"false").then_some(Json::Bool(false)),
            b'"' => self.parse_string().map(Json::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => None,
        }
    }

    fn consume(&mut self, literal: &[u8]) -> bool {
        if self.bytes[self.index..].starts_with(literal) {
            self.index += literal.len();
            true
        } else {
            false
        }
    }

    fn parse_object(&mut self) -> Option<Json> {
        self.bump()?;
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.bump();
                break;
            }
            if !pairs.is_empty() && self.bump()? != b',' {
                return None;
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bump()? != b':' {
                return None;
            }
            pairs.push((key, self.parse_value()?));
        }
        Some(Json::Object(pairs))
    }

    fn parse_array(&mut self) -> Option<Json> {
        self.bump()?;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.bump();
                break;
            }
            if !items.is_empty() && self.bump()? != b',' {
                return None;
            }
            items.push(self.parse_value()?);
        }
        Some(Json::Array(items))
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.bump()? != b'"' {
            return None;
        }
        let mut out = String::new();
        loop {
            match self.bump()? {
                b'"' => return Some(out),
                b'\\' => match self.bump()? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let mut hex = [0u8; 4];
                        for slot in &mut hex {
                            *slot = self.bump()?;
                        }
                        let text = std::str::from_utf8(&hex).ok()?;
                        let code = u32::from_str_radix(text, 16).ok()?;
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    }
                    _ => return None,
                },
                byte if byte < 0x80 => out.push(byte as char),
                _ => {
                    self.index -= 1;
                    let text = std::str::from_utf8(&self.bytes[self.index..]).ok()?;
                    let ch = text.chars().next()?;
                    out.push(ch);
                    self.index += ch.len_utf8();
                }
            }
        }
    }

    fn parse_number(&mut self) -> Option<Json> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.index += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index]).ok()?;
        Some(Json::Number(text.parse().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("devterm-qc-{}-{nanos}.json", std::process::id()))
    }

    #[test]
    fn records_mru_dedupes_and_caps_at_20() {
        let path = temp_path();
        let mut store = QuickConnect::open(&path);
        assert!(store.list().is_empty());
        store.record("a.example", 22, "op", 1).unwrap();
        store.record("b.example", 22, "op", 5).unwrap();
        store.record("a.example", 22, "op", 10).unwrap();
        let listed = store.list();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].host, "a.example");
        assert_eq!(listed[0].last_used_at, 10);
        assert_eq!(listed[1].host, "b.example");
        store.record("a.example", 2222, "op", 11).unwrap();
        assert_eq!(store.list().len(), 3);
        for n in 0..20 {
            store.record(&format!("h{n}"), 22, "user", 100 + n).unwrap();
        }
        let listed = store.list();
        assert_eq!(MAX_ENTRIES, 20);
        assert_eq!(listed.len(), 20);
        assert_eq!(listed[0].host, "h19");
        assert!(listed.iter().all(|entry| entry.host.starts_with('h')));
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"lastUsedAt\""));
        assert!(body.contains("\"entries\""));
        assert!(!path.with_extension("json.tmp").exists());
        let mut reloaded = QuickConnect::open(&path);
        assert_eq!(reloaded.list().len(), 20);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn corrupt_or_partial_entries_are_dropped() {
        let path = temp_path();
        fs::write(
            &path,
            r#"{"entries":[{"host":"ok","port":22,"username":"u","lastUsedAt":1},{"host":"bad"},null]}"#,
        )
        .unwrap();
        let mut store = QuickConnect::open(&path);
        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].host, "ok");
        fs::write(&path, "nope").unwrap();
        let mut broken = QuickConnect::open(&path);
        assert!(broken.list().is_empty());
        let _ = fs::remove_file(&path);
    }
}
