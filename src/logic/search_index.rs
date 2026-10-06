//! In-memory search ring per session, plus optional JSONL persist path logic.
//!
//! Port of `src/main/search/index.ts` and the path/cap pieces of `persist.ts`.
//! Default ring is 2000 lines (clamped to 200..=10_000). Persist tails cap at
//! 5000 lines and sanitize the session id so it cannot escape the search dir.

pub const DEFAULT_SEARCH_INDEX_LINES: usize = 2000;
pub const MIN_SEARCH_INDEX_LINES: usize = 200;
pub const MAX_SEARCH_INDEX_LINES: usize = 10_000;
/// On-disk JSONL tail cap (`persist.ts` MAX_LINES).
pub const PERSIST_MAX_LINES: usize = 5000;
pub const PERSIST_MAX_PENDING: usize = 2000;
/// A command that never emits LF is truncated to this many chars.
pub const PENDING_CAP: usize = 16_384;

pub fn normalize_search_index_lines(value: Option<f64>, fallback: usize) -> usize {
    let Some(value) = value else {
        return fallback;
    };
    if !value.is_finite() {
        return fallback;
    }
    let floored = value.floor() as i64;
    floored.clamp(MIN_SEARCH_INDEX_LINES as i64, MAX_SEARCH_INDEX_LINES as i64) as usize
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub session_id: String,
    pub session_title: String,
    pub line_number: usize,
    pub text: String,
    pub kind: &'static str,
    pub total_lines: usize,
}

struct StoredLine {
    text: String,
    line_number: usize,
}

struct SessionRecord {
    title: String,
    lines: Vec<Option<StoredLine>>,
    capacity: usize,
    start: usize,
    size: usize,
    next_line_number: usize,
    pending: String,
}

pub struct SearchIndex {
    index: Vec<(String, SessionRecord)>,
    max_lines: usize,
}

impl SearchIndex {
    pub fn new(max_lines: usize) -> Self {
        Self {
            index: Vec::new(),
            max_lines: normalize_search_index_lines(Some(max_lines as f64), DEFAULT_SEARCH_INDEX_LINES),
        }
    }

    pub fn get_max_lines(&self) -> usize {
        self.max_lines
    }

    pub fn set_max_lines(&mut self, value: Option<f64>) {
        let next = normalize_search_index_lines(value, self.max_lines);
        if next == self.max_lines {
            return;
        }
        self.max_lines = next;
        for (_, rec) in &mut self.index {
            rebuild_record(rec, next);
        }
    }

    pub fn set_session_title(&mut self, session_id: &str, title: &str) {
        if let Some((_, rec)) = self.index.iter_mut().find(|(id, _)| id == session_id) {
            rec.title = title.to_string();
        } else {
            self.index
                .push((session_id.to_string(), self.create_record(title)));
        }
    }

    pub fn push_line(&mut self, session_id: &str, text: &str, title_fallback: Option<&str>) {
        if !self.index.iter().any(|(id, _)| id == session_id) {
            let title = title_fallback.unwrap_or(session_id);
            self.index
                .push((session_id.to_string(), self.create_record(title)));
        }
        let max_lines = self.max_lines;
        let rec = self
            .index
            .iter_mut()
            .find(|(id, _)| id == session_id)
            .map(|(_, r)| r)
            .unwrap();
        let _ = max_lines;
        let normalized = text.replace("\r\n", "\n");
        let parts: Vec<&str> = normalized.split('\n').collect();
        let last = parts.len() - 1;
        for (i, part) in parts.iter().enumerate() {
            if i == last {
                break;
            }
            let logical = format!("{}{}", rec.pending, part);
            let visible = match logical.rfind('\r') {
                Some(idx) => &logical[idx + 1..],
                None => logical.as_str(),
            };
            let clean = strip_ansi(visible);
            if !clean.is_empty() {
                append(rec, clean);
            }
            rec.pending.clear();
        }
        let tail = parts[last];
        let combined = format!("{}{}", rec.pending, tail);
        rec.pending = match combined.rfind('\r') {
            Some(idx) => combined[idx + 1..].to_string(),
            None => combined,
        };
        if rec.pending.chars().count() > PENDING_CAP {
            let skip = rec.pending.chars().count() - PENDING_CAP;
            rec.pending = rec.pending.chars().skip(skip).collect();
        }
    }

    pub fn clear_session(&mut self, session_id: &str) {
        self.index.retain(|(id, _)| id != session_id);
    }

    pub fn seed_lines(&mut self, session_id: &str, lines: &[String], title: &str) {
        let mut rec = self.create_record(title);
        let start = lines.len().saturating_sub(self.max_lines);
        for text in &lines[start..] {
            let clean = strip_ansi(text);
            if !clean.is_empty() {
                append(&mut rec, clean);
            }
        }
        if let Some(slot) = self.index.iter_mut().find(|(id, _)| id == session_id) {
            *slot = (session_id.to_string(), rec);
        } else {
            self.index.push((session_id.to_string(), rec));
        }
    }

    pub fn query(&self, q: &str, limit: usize) -> Vec<SearchResult> {
        if q.trim().is_empty() {
            return Vec::new();
        }
        let lower = q.to_lowercase();
        let mut out = Vec::new();
        for (sid, rec) in &self.index {
            for i in 0..rec.size {
                let Some(ln) = rec.lines[(rec.start + i) % rec.capacity].as_ref() else {
                    continue;
                };
                if ln.text.to_lowercase().contains(&lower) {
                    out.push(SearchResult {
                        session_id: sid.clone(),
                        session_title: rec.title.clone(),
                        line_number: ln.line_number,
                        text: ln.text.clone(),
                        kind: "live",
                        total_lines: rec.next_line_number,
                    });
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }

    fn create_record(&self, title: &str) -> SessionRecord {
        SessionRecord {
            title: title.to_string(),
            lines: (0..self.max_lines).map(|_| None).collect(),
            capacity: self.max_lines,
            start: 0,
            size: 0,
            next_line_number: 1,
            pending: String::new(),
        }
    }
}

impl Default for SearchIndex {
    fn default() -> Self {
        Self::new(DEFAULT_SEARCH_INDEX_LINES)
    }
}

fn rebuild_record(rec: &mut SessionRecord, capacity: usize) {
    let mut kept = Vec::new();
    for i in 0..rec.size {
        if let Some(ln) = rec.lines[(rec.start + i) % rec.capacity].take() {
            kept.push(ln);
        }
    }
    let start = kept.len().saturating_sub(capacity);
    let slice = kept.split_off(start);
    rec.lines = (0..capacity).map(|_| None).collect();
    rec.capacity = capacity;
    rec.start = 0;
    rec.size = slice.len();
    for (i, ln) in slice.into_iter().enumerate() {
        rec.lines[i] = Some(ln);
    }
}

fn append(rec: &mut SessionRecord, text: String) {
    if text.is_empty() {
        return;
    }
    let stored = StoredLine {
        text,
        line_number: rec.next_line_number,
    };
    rec.next_line_number += 1;
    if rec.size < rec.capacity {
        rec.lines[(rec.start + rec.size) % rec.capacity] = Some(stored);
        rec.size += 1;
        return;
    }
    rec.lines[rec.start] = Some(stored);
    rec.start = (rec.start + 1) % rec.capacity;
}

/// Safe file name for `userData/search/<sessionId>.jsonl`.
pub fn persist_file_name(session_id: &str) -> String {
    let safe: String = session_id
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    format!("{safe}.jsonl")
}

/// FIFO-cap a persisted tail at `PERSIST_MAX_LINES`.
pub fn cap_persisted_lines(lines: &mut Vec<String>) {
    if lines.len() > PERSIST_MAX_LINES {
        let drop = lines.len() - PERSIST_MAX_LINES;
        lines.drain(0..drop);
    }
}

fn strip_ansi(text: &str) -> String {
    // Same ingest cleaner as search_ansi.rs (standalone file, so copied).
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if let Some(end) = consume_ansi(bytes, i) {
                i = end;
                continue;
            }
        }
        let ch = bytes[i];
        if ch <= 0x08 || (0x0a..=0x1f).contains(&ch) || ch == 0x7f {
            i += 1;
            continue;
        }
        let width = if ch < 0x80 {
            1
        } else if ch & 0xE0 == 0xC0 {
            2
        } else if ch & 0xF0 == 0xE0 {
            3
        } else if ch & 0xF8 == 0xF0 {
            4
        } else {
            1
        };
        let end = (i + width).min(bytes.len());
        if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
            out.push_str(s);
        }
        i = end;
    }
    out
}

fn consume_ansi(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&0x1b) {
        return None;
    }
    let next = *bytes.get(i + 1)?;
    if next == b'[' {
        let mut j = i + 2;
        while j < bytes.len() && (0x30..=0x3f).contains(&bytes[j]) {
            j += 1;
        }
        while j < bytes.len() && (0x20..=0x2f).contains(&bytes[j]) {
            j += 1;
        }
        return if j < bytes.len() && (0x40..=0x7e).contains(&bytes[j]) {
            Some(j + 1)
        } else {
            None
        };
    }
    if next == b']' {
        let mut j = i + 2;
        while j < bytes.len() && bytes[j] != 0x07 && bytes[j] != 0x1b {
            j += 1;
        }
        if j >= bytes.len() {
            return Some(bytes.len());
        }
        if bytes[j] == 0x07 {
            return Some(j + 1);
        }
        if bytes[j] == 0x1b && bytes.get(j + 1) == Some(&b'\\') {
            return Some(j + 2);
        }
        return Some(i + 2);
    }
    let mut j = i + 1;
    while j < bytes.len() && (0x20..=0x2f).contains(&bytes[j]) {
        j += 1;
    }
    if j < bytes.len() && (0x30..=0x7e).contains(&bytes[j]) {
        Some(j + 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_lines_across_transport_chunks() {
        let mut index = SearchIndex::new(DEFAULT_SEARCH_INDEX_LINES);
        index.set_session_title("s1", "Shell");
        index.push_line("s1", "first par", None);
        index.push_line("s1", "t\r\nsecond\n", None);
        assert_eq!(
            index
                .query("part", 50)
                .into_iter()
                .map(|x| x.text)
                .collect::<Vec<_>>(),
            vec!["first part"]
        );
        assert_eq!(
            index
                .query("second", 50)
                .into_iter()
                .map(|x| x.text)
                .collect::<Vec<_>>(),
            vec!["second"]
        );
    }

    #[test]
    fn keeps_only_the_final_carriage_return_redraw() {
        let mut index = SearchIndex::default();
        index.push_line("s1", "10%\r50%\r100%\n", None);
        assert_eq!(index.query("10%", 50).len(), 0);
        assert_eq!(index.query("50%", 50).len(), 0);
        assert_eq!(index.query("100%", 50)[0].text, "100%");
    }

    #[test]
    fn retains_a_fixed_size_ring_with_monotonic_line_numbers() {
        let mut index = SearchIndex::default();
        for i in 0..2_010 {
            index.push_line("s1", &format!("row-{i}\n"), None);
        }
        let hit = &index.query("row-", 3)[0];
        assert_eq!(hit.text, "row-10");
        assert_eq!(hit.line_number, 11);
        assert_eq!(index.query("row-0", 50).len(), 0);
    }

    #[test]
    fn shrinks_the_ring_when_the_cap_is_lowered() {
        let mut index = SearchIndex::new(2000);
        for i in 0..1_200 {
            index.push_line("s1", &format!("n-{i}\n"), None);
        }
        index.set_max_lines(Some(1000.0));
        assert_eq!(index.query("n-199", 50).len(), 0);
        assert_eq!(index.query("n-200", 50)[0].text, "n-200");
        assert_eq!(index.get_max_lines(), 1000);
    }

    #[test]
    fn clamps_the_cap_and_sanitizes_persist_paths() {
        assert_eq!(normalize_search_index_lines(Some(10.0), 2000), 200);
        assert_eq!(normalize_search_index_lines(Some(50_000.0), 2000), 10_000);
        assert_eq!(normalize_search_index_lines(None, 2000), 2000);
        assert_eq!(
            persist_file_name("../../etc/passwd"),
            ".._.._etc_passwd.jsonl"
        );
        let mut lines: Vec<String> = (0..PERSIST_MAX_LINES + 3)
            .map(|i| i.to_string())
            .collect();
        cap_persisted_lines(&mut lines);
        assert_eq!(lines.len(), PERSIST_MAX_LINES);
        assert_eq!(lines[0], "3");
    }
}
