//! Bounded raw terminal output retained by the main process.
//!
//! Port of `src/main/terminal/output-ring.ts`. Completed lines sit in a circular
//! queue; the current unterminated line is kept separately. Byte lengths are
//! UTF-8 (`Buffer.byteLength`). A suffix cut uses JavaScript UTF-16 indexes, so
//! a boundary on a low surrogate skips that scalar instead of emitting U+FFFD.

use std::collections::HashMap;

pub const DEFAULT_OUTPUT_RING_LINES: usize = 10_000;
pub const MAX_OUTPUT_RING_LINES: usize = 100_000;
pub const DEFAULT_OUTPUT_RING_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_OUTPUT_RING_BYTES: usize = 64 * 1024 * 1024;

/// `max_lines` / `max_bytes` of `None` means "use the fallback" (constructor
/// defaults, or the ring's current limit inside `set_limits`).
#[derive(Clone, Copy, Debug, Default)]
pub struct OutputRingOptions {
    pub max_lines: Option<i64>,
    pub max_bytes: Option<i64>,
}

pub struct OutputRingBuffer {
    max_lines: usize,
    max_bytes: usize,
    slots: Vec<Option<String>>,
    start: usize,
    size: usize,
    partial: String,
    bytes: usize,
}

impl OutputRingBuffer {
    pub fn new(options: OutputRingOptions) -> Self {
        let max_lines = clamp_ring(
            options.max_lines,
            DEFAULT_OUTPUT_RING_LINES,
            MAX_OUTPUT_RING_LINES,
        );
        let max_bytes = clamp_ring(
            options.max_bytes,
            DEFAULT_OUTPUT_RING_BYTES,
            MAX_OUTPUT_RING_BYTES,
        );
        Self {
            max_lines,
            max_bytes,
            slots: vec![None; max_lines],
            start: 0,
            size: 0,
            partial: String::new(),
            bytes: 0,
        }
    }

    pub fn line_capacity(&self) -> usize {
        self.max_lines
    }

    pub fn byte_capacity(&self) -> usize {
        self.max_bytes
    }

    /// Complete lines plus the current unterminated line, if any.
    pub fn line_count(&self) -> usize {
        self.size + usize::from(!self.partial.is_empty())
    }

    pub fn byte_length(&self) -> usize {
        self.bytes
    }

    /// Append raw terminal bytes. Newlines are retained exactly.
    pub fn append(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        let mut combined = std::mem::take(&mut self.partial);
        self.bytes -= combined.len();
        combined.push_str(data);

        let mut line_start = 0usize;
        while let Some(rel) = combined[line_start..].find('\n') {
            let newline = line_start + rel;
            self.push_complete_line(combined[line_start..=newline].to_string());
            line_start = newline + 1;
        }
        self.partial = combined[line_start..].to_string();
        self.bytes += self.partial.len();
        self.trim();
    }

    /// Retained raw stream in chronological order.
    pub fn replay(&self) -> String {
        if self.size == 0 {
            return self.partial.clone();
        }
        let mut out = String::new();
        for line in self.complete_lines() {
            out.push_str(&line);
        }
        out.push_str(&self.partial);
        out
    }

    /// Change limits without discarding newer output unnecessarily.
    pub fn set_limits(&mut self, options: OutputRingOptions) {
        let next_lines = clamp_ring(options.max_lines, self.max_lines, MAX_OUTPUT_RING_LINES);
        let next_bytes = clamp_ring(options.max_bytes, self.max_bytes, MAX_OUTPUT_RING_BYTES);
        if next_lines != self.max_lines {
            let retained = self.complete_lines();
            let start = retained.len().saturating_sub(next_lines);
            let retained = retained[start..].to_vec();
            self.max_lines = next_lines;
            self.slots = vec![None; next_lines];
            self.start = 0;
            self.size = 0;
            self.bytes = self.partial.len();
            for line in retained {
                self.push_complete_line(line);
            }
        }
        self.max_bytes = next_bytes;
        self.trim();
    }

    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.start = 0;
        self.size = 0;
        self.partial.clear();
        self.bytes = 0;
    }

    fn complete_lines(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.size);
        for i in 0..self.size {
            let line = self.slots[(self.start + i) % self.max_lines]
                .clone()
                .unwrap_or_default();
            out.push(line);
        }
        out
    }

    fn push_complete_line(&mut self, line: String) {
        let line_bytes = line.len();
        if self.size < self.max_lines {
            let index = (self.start + self.size) % self.max_lines;
            self.slots[index] = Some(line);
            self.size += 1;
        } else {
            let old = self.slots[self.start].take().unwrap_or_default();
            self.bytes -= old.len();
            self.slots[self.start] = Some(line);
            self.start = (self.start + 1) % self.max_lines;
        }
        self.bytes += line_bytes;
    }

    fn remove_oldest(&mut self) {
        if self.size == 0 {
            return;
        }
        let old = self.slots[self.start].take().unwrap_or_default();
        self.start = (self.start + 1) % self.max_lines;
        self.size -= 1;
        self.bytes -= old.len();
    }

    fn replace_oldest(&mut self, line: String) {
        if self.size == 0 {
            return;
        }
        let old = self.slots[self.start].take().unwrap_or_default();
        let new_len = line.len();
        self.slots[self.start] = Some(line);
        self.bytes = self.bytes + new_len - old.len();
    }

    fn trim(&mut self) {
        while self.line_count() > self.max_lines {
            self.remove_oldest();
        }
        while self.bytes > self.max_bytes && self.size > 0 {
            let oldest = self.slots[self.start].clone().unwrap_or_default();
            let oldest_bytes = oldest.len();
            let bytes_without_oldest = self.bytes - oldest_bytes;
            if bytes_without_oldest >= self.max_bytes {
                self.remove_oldest();
                continue;
            }
            let allowed = self.max_bytes - bytes_without_oldest;
            if oldest_bytes > allowed {
                self.replace_oldest(suffix_within_bytes(&oldest, allowed as i64));
            }
            break;
        }
        if self.bytes > self.max_bytes && self.size == 0 {
            self.partial = suffix_within_bytes(&self.partial, self.max_bytes as i64);
            self.bytes = self.partial.len();
        }
    }
}

/// Namespace prefixes keep a PTY id from colliding with an SSH session id.
pub fn output_stream_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// Registry of live terminal streams. Forwarding defaults to on.
pub struct OutputRingStore {
    rings: HashMap<String, OutputRingBuffer>,
    forwarding: HashMap<String, bool>,
    session_streams: HashMap<String, String>,
    max_lines: usize,
    max_bytes: usize,
}

impl OutputRingStore {
    pub fn new(options: OutputRingOptions) -> Self {
        let ring = OutputRingBuffer::new(options);
        Self {
            max_lines: ring.line_capacity(),
            max_bytes: ring.byte_capacity(),
            rings: HashMap::new(),
            forwarding: HashMap::new(),
            session_streams: HashMap::new(),
        }
    }

    pub fn append(&mut self, id: &str, data: &str) {
        self.get_or_create(id).append(data);
    }

    pub fn replay(&self, id: &str) -> String {
        self.rings
            .get(id)
            .map(|ring| ring.replay())
            .unwrap_or_default()
    }

    pub fn set_limits(&mut self, options: OutputRingOptions) {
        if options.max_lines.is_some() {
            self.max_lines = OutputRingBuffer::new(OutputRingOptions {
                max_lines: options.max_lines,
                max_bytes: None,
            })
            .line_capacity();
        }
        if options.max_bytes.is_some() {
            self.max_bytes = OutputRingBuffer::new(OutputRingOptions {
                max_lines: None,
                max_bytes: options.max_bytes,
            })
            .byte_capacity();
        }
        let applied = OutputRingOptions {
            max_lines: Some(self.max_lines as i64),
            max_bytes: Some(self.max_bytes as i64),
        };
        for ring in self.rings.values_mut() {
            ring.set_limits(applied);
        }
    }

    pub fn set_forwarding(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.forwarding.remove(id);
        } else {
            self.forwarding.insert(id.to_string(), false);
        }
    }

    pub fn is_forwarding(&self, id: &str) -> bool {
        self.forwarding.get(id).copied().unwrap_or(true)
    }

    pub fn bind_session(&mut self, session_id: &str, stream_id: &str) {
        self.session_streams
            .insert(session_id.to_string(), stream_id.to_string());
    }

    pub fn unbind_session(&mut self, session_id: &str, stream_id: Option<&str>) {
        if let Some(stream_id) = stream_id {
            if self.session_streams.get(session_id).map(String::as_str) != Some(stream_id) {
                return;
            }
        }
        self.session_streams.remove(session_id);
    }

    pub fn set_forwarding_for_session(&mut self, session_id: &str, enabled: bool) -> bool {
        let Some(stream_id) = self.session_streams.get(session_id).cloned() else {
            return false;
        };
        self.set_forwarding(&stream_id, enabled);
        true
    }

    /// Capture the retained bytes, then reopen the live forwarding gate.
    pub fn replay_and_resume(&mut self, session_id: &str) -> String {
        let Some(stream_id) = self.session_streams.get(session_id).cloned() else {
            return String::new();
        };
        let replay = self.replay(&stream_id);
        self.set_forwarding(&stream_id, true);
        replay
    }

    /// Read the retained bytes without changing the forwarding gate.
    pub fn replay_for_session(&self, session_id: &str) -> String {
        match self.session_streams.get(session_id) {
            Some(stream_id) => self.replay(stream_id),
            None => String::new(),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.rings.remove(id);
        self.forwarding.remove(id);
        self.session_streams.retain(|_, stream| stream != id);
    }

    fn get_or_create(&mut self, id: &str) -> &mut OutputRingBuffer {
        if !self.rings.contains_key(id) {
            let ring = OutputRingBuffer::new(OutputRingOptions {
                max_lines: Some(self.max_lines as i64),
                max_bytes: Some(self.max_bytes as i64),
            });
            self.rings.insert(id.to_string(), ring);
        }
        self.rings.get_mut(id).expect("ring inserted")
    }
}

fn clamp_ring(value: Option<i64>, fallback: usize, max: usize) -> usize {
    let Some(value) = value else {
        return fallback;
    };
    value.clamp(1, max as i64) as usize
}

fn suffix_within_bytes(value: &str, max_bytes: i64) -> String {
    if value.is_empty() || max_bytes <= 0 {
        return String::new();
    }
    let max_bytes = max_bytes as usize;
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let units: Vec<u16> = value.encode_utf16().collect();
    let mut low = 0usize;
    let mut high = units.len();
    while low < high {
        let mid = (low + high) / 2;
        if utf8_len_from_utf16(&units, mid) <= max_bytes {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    let mut start = low;
    if start > 0 && start < units.len() {
        let code = units[start];
        if (0xDC00..=0xDFFF).contains(&code) {
            start += 1;
        }
    }
    String::from_utf16(&units[start..]).unwrap_or_default()
}

fn utf8_len_from_utf16(units: &[u16], start: usize) -> usize {
    let mut i = start;
    let mut bytes = 0usize;
    while i < units.len() {
        let unit = units[i];
        if (0xD800..=0xDBFF).contains(&unit) {
            if i + 1 < units.len() {
                let low = units[i + 1];
                if (0xDC00..=0xDFFF).contains(&low) {
                    let cp = 0x10000 + (((unit as u32) - 0xD800) << 10) + ((low as u32) - 0xDC00);
                    bytes += char::from_u32(cp).map(|ch| ch.len_utf8()).unwrap_or(3);
                    i += 2;
                    continue;
                }
            }
            bytes += 3;
            i += 1;
            continue;
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            bytes += 3;
            i += 1;
            continue;
        }
        bytes += char::from_u32(unit as u32)
            .map(|ch| ch.len_utf8())
            .unwrap_or(3);
        i += 1;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(max_lines: i64, max_bytes: i64) -> OutputRingOptions {
        OutputRingOptions {
            max_lines: Some(max_lines),
            max_bytes: Some(max_bytes),
        }
    }

    #[test]
    fn drops_the_oldest_lines_while_retaining_the_newest_output() {
        let mut ring = OutputRingBuffer::new(opts(2, 1024));
        ring.append("one\n");
        ring.append("two\n");
        ring.append("three\n");
        assert_eq!(ring.replay(), "two\nthree\n");
        assert_eq!(ring.line_count(), 2);
    }

    #[test]
    fn concatenates_arbitrary_chunks_and_keeps_an_unterminated_tail() {
        let mut ring = OutputRingBuffer::new(opts(4, 1024));
        ring.append("\u{1b}[32mhel");
        ring.append("lo\u{1b}[0m\nnext");
        ring.append(" line");
        assert_eq!(ring.replay(), "\u{1b}[32mhello\u{1b}[0m\nnext line");
        assert_eq!(ring.line_count(), 2);
        assert_eq!(ring.byte_length(), ring.replay().len());
    }

    #[test]
    fn bounds_a_long_unterminated_line_by_bytes() {
        let mut ring = OutputRingBuffer::new(opts(10, 5));
        ring.append("0123456789");
        assert_eq!(ring.replay(), "56789");
        assert!(ring.byte_length() <= 5);
    }

    #[test]
    fn trims_a_single_huge_complete_line_to_a_utf8_suffix() {
        let mut ring = OutputRingBuffer::new(opts(4, 10));
        ring.append(&"a".repeat(20));
        ring.append("\n");
        ring.append("z");
        assert_eq!(ring.replay(), "aaaaaaaa\nz");
        assert!(ring.byte_length() <= 10);
    }

    #[test]
    fn surrogate_suffix_of_an_unterminated_line_matches_javascript() {
        let mut ring = OutputRingBuffer::new(opts(10, 3));
        ring.append("😀😀");
        assert_eq!(ring.replay(), "");
        assert!(ring.byte_length() <= 3);

        let mut ring = OutputRingBuffer::new(opts(10, 4));
        ring.append("😀😀");
        assert_eq!(ring.replay(), "😀");
        assert_eq!(ring.byte_length(), "😀".len());
    }

    #[test]
    fn defaults_to_forwarding_and_can_gate_one_stream_without_stopping_its_ring() {
        let mut store = OutputRingStore::new(opts(2, 1024));
        assert!(store.is_forwarding("pty:one"));
        store.set_forwarding("pty:one", false);
        store.append("pty:one", "hidden\n");
        assert!(!store.is_forwarding("pty:one"));
        assert_eq!(store.replay("pty:one"), "hidden\n");
        store.set_forwarding("pty:one", true);
        assert!(store.is_forwarding("pty:one"));
    }

    #[test]
    fn replays_a_hibernated_session_and_resumes_its_mapped_stream() {
        let mut store = OutputRingStore::new(opts(4, 1024));
        store.bind_session("session-1", "pty:one");
        store.append("pty:one", "before\n");
        assert!(store.set_forwarding_for_session("session-1", false));
        store.append("pty:one", "while hidden\n");
        assert_eq!(
            store.replay_and_resume("session-1"),
            "before\nwhile hidden\n"
        );
        assert!(store.is_forwarding("pty:one"));
    }

    #[test]
    fn captures_a_mapped_session_without_reopening_a_hibernation_gate() {
        let mut store = OutputRingStore::new(opts(4, 1024));
        store.bind_session("session-1", "ssh:one");
        store.set_forwarding_for_session("session-1", false);
        store.append("ssh:one", "\u{1b}[32mkept\u{1b}[0m\n");
        assert_eq!(
            store.replay_for_session("session-1"),
            "\u{1b}[32mkept\u{1b}[0m\n"
        );
        assert!(!store.is_forwarding("ssh:one"));
    }

    #[test]
    fn output_stream_key_prefixes_the_kind() {
        assert_eq!(output_stream_key("pty", "abc"), "pty:abc");
        assert_eq!(output_stream_key("ssh", "abc"), "ssh:abc");
    }
}
