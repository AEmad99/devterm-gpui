//! Strip ANSI/VT escape sequences and C0 controls at search ingest.
//!
//! Port of `src/main/search/ansi.ts`. Match order:
//! CSI (`ESC [` … final), OSC (`ESC ]` … BEL / ST / end of chunk),
//! then a stray ESC sequence. Remaining C0 controls except TAB, plus DEL,
//! are removed. CR and LF are removed so an index row stays one line.

/// Strip ANSI/VT sequences and C0 controls (except TAB) from `text`.
pub fn strip_ansi(text: &str) -> String {
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
        // CONTROL_RE: \x00-\x08, \x0a-\x1f, \x7f. TAB (\x09) is kept.
        if ch <= 0x08 || (0x0a..=0x1f).contains(&ch) || ch == 0x7f {
            i += 1;
            continue;
        }
        // Copy one UTF-8 scalar starting at i.
        let width = utf8_width(ch);
        let end = (i + width).min(bytes.len());
        if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
            out.push_str(s);
        }
        i = end;
    }
    out
}

fn utf8_width(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else if b & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

/// Returns the index just past a matched escape, if one starts at `i`.
fn consume_ansi(bytes: &[u8], i: usize) -> Option<usize> {
    if i >= bytes.len() || bytes[i] != 0x1b {
        return None;
    }
    let next = *bytes.get(i + 1)?;
    if next == b'[' {
        return consume_csi(bytes, i);
    }
    if next == b']' {
        return Some(consume_osc(bytes, i));
    }
    consume_stray_esc(bytes, i)
}

/// CSI: ESC [ [0-?]* [ -/]* [@-~]
fn consume_csi(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 2;
    while j < bytes.len() && (0x30..=0x3f).contains(&bytes[j]) {
        j += 1;
    }
    while j < bytes.len() && (0x20..=0x2f).contains(&bytes[j]) {
        j += 1;
    }
    if j < bytes.len() && (0x40..=0x7e).contains(&bytes[j]) {
        Some(j + 1)
    } else {
        None
    }
}

/// OSC: ESC ] [^\x07\x1b]* (?: BEL | ESC \ | end)
fn consume_osc(bytes: &[u8], i: usize) -> usize {
    let mut j = i + 2;
    while j < bytes.len() && bytes[j] != 0x07 && bytes[j] != 0x1b {
        j += 1;
    }
    if j >= bytes.len() {
        return bytes.len();
    }
    if bytes[j] == 0x07 {
        return j + 1;
    }
    // ST is ESC \
    if bytes[j] == 0x1b && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
        return j + 2;
    }
    // Unterminated relative to ST: the OSC alternative still matches through
    // the end of the chunk (`$`), which is how a trailing half-written OSC
    // is dropped. If ESC is not ST, consume through the end as well — the
    // TS pattern's `$` alternative only succeeds at the end, and a mid-chunk
    // ESC that is not `\` fails this alternative so a later pass can see it.
    // Practical chunks in the tests either end here or use ST. If we see a
    // non-ST ESC, stop *before* it so CSI/stray handling can run... but the
    // regex engine tries the whole match. A non-ST ESC inside OSC makes the
    // OSC alternative fail (the `*` cannot consume ESC, BEL/ST don't match,
    // `$` isn't at end), then the stray-ESC alternative matches from the
    // original ESC only if it fits. For `\x1b]...\x1b\\` ST is handled above.
    // For an ESC that isn't `\`, return the position of that ESC so the
    // caller does not skip it — BUT the regex is anchored at the original
    // ESC, so a failed OSC alternative falls through to stray ESC which
    // matches ESC + intermediates + one final. `]` is not in [ -/] (0x20-0x2F
    // is space through `/`; `]` is 0x5D). Stray is `\x1b[ -/]*[0-~]`. After
    // ESC, `]` is NOT in [ -/], and `]` IS in [0-~] (0x30-0x7E includes 0x5D).
    // So stray matches just `ESC ]` (empty intermediates + final `]`).
    // That would NOT swallow the OSC payload!
    //
    // The OSC alternative is tried first and succeeds when the payload runs
    // to BEL, ST, or end. A mid-string ESC that is not `\` fails OSC, then
    // stray matches only `ESC ]`. The payload would remain. The tests don't
    // cover a broken OSC with an embedded ESC that isn't ST. ST is handled.
    // Return j so the ESC is reconsidered only when it isn't ST — wait, if
    // we return j the caller skips nothing of the original match and would
    // infinite-loop if we don't advance. The function is "consume from i".
    // If OSC fails, return None from a different function. Here we already
    // decided it's OSC because next is `]`. The regex OSC alternative fails
    // and stray matches ESC + `]` as final (2 bytes). Consume 2 bytes.
    i + 2
}

/// Stray ESC: ESC [ -/]* [0-~]
fn consume_stray_esc(bytes: &[u8], i: usize) -> Option<usize> {
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
    use super::strip_ansi;

    #[test]
    fn leaves_plain_text_untouched() {
        assert_eq!(strip_ansi("total 42 files"), "total 42 files");
    }

    #[test]
    fn strips_sgr_color_codes() {
        assert_eq!(
            strip_ansi("\x1b[1;32muser@host\x1b[0m:\x1b[94m~\x1b[0m$ ls"),
            "user@host:~$ ls"
        );
    }

    #[test]
    fn strips_cursor_positioning_and_other_csi_finals() {
        assert_eq!(strip_ansi("\x1b[23;20Hresult\x1b[K"), "result");
        assert_eq!(strip_ansi("\x1b[?25ltyping\x1b[?25h"), "typing");
    }

    #[test]
    fn strips_osc_7_cwd_sequences_terminated_by_bel_and_st() {
        assert_eq!(
            strip_ansi("\x1b]7;file://DESKTOP-A1B2C3/D:/projects/DevTerm\x07"),
            ""
        );
        assert_eq!(
            strip_ansi("\x1b]7;file:///home/user\x1b\\rest of line"),
            "rest of line"
        );
    }

    #[test]
    fn strips_osc_133_prompt_markers() {
        assert_eq!(
            strip_ansi("\x1b]133;A\x07$ \x1b]133;B\x07ls -la"),
            "$ ls -la"
        );
    }

    #[test]
    fn cleans_a_powershell_prompt_line() {
        let raw = "\x1b[?25l\x1b[93mPS C:\\Users\\dev\x1b[0m\x1b[1;33m ❯\x1b[0m \x1b[?25h";
        assert_eq!(strip_ansi(raw), "PS C:\\Users\\dev ❯ ");
    }

    #[test]
    fn strips_stray_esc_sequences() {
        assert_eq!(strip_ansi("\x1b(B\x1b7hi\x1b8"), "hi");
    }

    #[test]
    fn strips_an_unterminated_osc_at_the_end_of_a_chunk() {
        assert_eq!(strip_ansi("output\x1b]0;half-written title"), "output");
    }

    #[test]
    fn drops_c0_control_chars_but_keeps_tabs() {
        assert_eq!(strip_ansi("a\x07b\x00c\td\x7fe"), "abc\tde");
    }

    #[test]
    fn drops_cr_lf_so_a_chunk_stays_a_single_result_row() {
        assert_eq!(strip_ansi("one\r\ntwo\rthree"), "onetwothree");
    }

    #[test]
    fn returns_empty_for_a_pure_escape_sequence_chunk() {
        assert_eq!(strip_ansi("\x1b]7;file:///home/user\x07\x1b[2K\r"), "");
    }
}
