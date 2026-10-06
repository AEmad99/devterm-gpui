//! Persisted session scrollback tail.
//!
//! Port of `src/main/terminal/scrollback.ts`. Lines keep their trailing newline
//! so replay does not invent or drop terminal line endings. Byte trimming uses
//! JavaScript UTF-16 indexes (`String.length` / `slice` / `charCodeAt`) and
//! `Buffer.byteLength`: a cut that lands on a low surrogate is skipped, and a
//! lone surrogate counts as U+FFFD (3 bytes) while searching.

/// Default persisted tail. The hard cap keeps a restore file from growing with xterm.
pub const DEFAULT_PERSISTED_SCROLLBACK_LINES: usize = 2_000;
pub const MAX_PERSISTED_SCROLLBACK_LINES: usize = 10_000;
pub const MAX_PERSISTED_SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;

/// Keep the newest raw lines while retaining ANSI bytes exactly.
///
/// `max_lines` / `max_bytes` of `None` use the documented defaults. Finite
/// values are floored into `1..=MAX_*` the same way as the TypeScript helper.
pub fn trim_session_scrollback(
    raw: &str,
    max_lines: Option<i64>,
    max_bytes: Option<i64>,
) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let line_limit = clamp_persisted(
        max_lines,
        DEFAULT_PERSISTED_SCROLLBACK_LINES as i64,
        MAX_PERSISTED_SCROLLBACK_LINES as i64,
    ) as usize;
    let byte_limit = clamp_persisted(
        max_bytes,
        MAX_PERSISTED_SCROLLBACK_BYTES as i64,
        MAX_PERSISTED_SCROLLBACK_BYTES as i64,
    );
    let lines = split_keep_newlines(raw);
    let start = lines.len().saturating_sub(line_limit);
    let joined = lines[start..].concat();
    suffix_within_bytes(&joined, byte_limit)
}

fn clamp_persisted(value: Option<i64>, fallback: i64, max: i64) -> i64 {
    // `Math.max(1, Math.min(MAX, Math.floor(Number.isFinite(n) ? n : fallback)))`.
    value.unwrap_or(fallback).clamp(1, max)
}

/// `/[^\n]*(?:\n|$)/g` with empty matches dropped.
fn split_keep_newlines(raw: &str) -> Vec<&str> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        if i < bytes.len() {
            i += 1;
            out.push(&raw[start..i]);
        } else if start < bytes.len() {
            out.push(&raw[start..]);
        }
    }
    out
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

/// `Buffer.byteLength(value.slice(start), 'utf8')`.
/// Unpaired surrogates encode as U+FFFD (3 bytes), matching Node.
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

    #[test]
    fn keeps_raw_ansi_and_only_the_newest_complete_lines() {
        let raw = "\u{1b}[32mone\u{1b}[0m\ntwo\nthree";
        assert_eq!(trim_session_scrollback(raw, Some(2), None), "two\nthree");
        assert!(trim_session_scrollback(raw, Some(3), None).contains("\u{1b}[32mone"));
    }

    #[test]
    fn uses_the_documented_default_and_hard_line_cap() {
        let lines: String = (0..DEFAULT_PERSISTED_SCROLLBACK_LINES + 5)
            .map(|i| format!("{i}\n"))
            .collect();
        let default_tail = trim_session_scrollback(&lines, None, None);
        assert_eq!(
            default_tail
                .split('\n')
                .filter(|line| !line.is_empty())
                .count(),
            DEFAULT_PERSISTED_SCROLLBACK_LINES
        );

        let hard_cap_input: String = (0..MAX_PERSISTED_SCROLLBACK_LINES + 1)
            .map(|i| format!("{i}\n"))
            .collect();
        let hard_tail = trim_session_scrollback(
            &hard_cap_input,
            Some((MAX_PERSISTED_SCROLLBACK_LINES + 1) as i64),
            None,
        );
        assert_eq!(
            hard_tail
                .split('\n')
                .filter(|line| !line.is_empty())
                .count(),
            MAX_PERSISTED_SCROLLBACK_LINES
        );
    }

    #[test]
    fn bounds_a_single_unterminated_line_by_bytes() {
        let tail = trim_session_scrollback(&"x".repeat(100), Some(10), Some(12));
        assert_eq!(tail.len(), 12);
        assert_eq!(tail, "x".repeat(12));
    }

    #[test]
    fn utf16_surrogate_suffix_matches_javascript() {
        // "é" is one UTF-16 unit / 2 bytes; "😀" is a surrogate pair / 4 bytes.
        // Cuts that land on the low surrogate skip the whole scalar (no U+FFFD).
        let raw = format!("{}{}z", "é".repeat(3), "😀");
        assert_eq!(trim_session_scrollback(&raw, Some(10), Some(1)), "z");
        assert_eq!(trim_session_scrollback(&raw, Some(10), Some(4)), "z");
        assert_eq!(trim_session_scrollback(&raw, Some(10), Some(5)), "😀z");
        assert_eq!(trim_session_scrollback(&raw, Some(10), Some(7)), "é😀z");
        let emoji = "😀".repeat(2);
        assert_eq!(trim_session_scrollback(&emoji, Some(10), Some(3)), "");
        assert_eq!(trim_session_scrollback(&emoji, Some(10), Some(4)), "😀");
    }
}
