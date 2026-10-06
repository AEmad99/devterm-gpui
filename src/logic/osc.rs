//! Streaming OSC 7 (cwd) and OSC 133 A/B parser.
//!
//! Accepts BEL (`ESC ] … BEL`) and ST (`ESC ] … ESC \`) terminators, including
//! when a sequence is split across chunks. Tmux passthrough is the DCS form
//! DevTerm emits for remote shells: `ESC P tmux ; <OSC with ESC doubled> ESC \`.
//! OSC 9 / OSC 99 are ignored.

/// Last OSC 7 working directory and last OSC 133 prompt marker seen on a stream.
#[derive(Debug, Default)]
pub struct OscParser {
    state: State,
    cwd: Option<String>,
    marker: Option<&'static str>,
}

#[derive(Debug)]
enum State {
    Ground,
    Esc,
    Osc(Vec<u8>),
    /// Saw ESC inside an OSC; `\` would complete ST.
    OscEsc(Vec<u8>),
    Dcs(Vec<u8>),
    /// Saw ESC inside a DCS. `\` ends it; a second ESC is a doubled escape.
    DcsEsc(Vec<u8>),
}

impl Default for State {
    fn default() -> Self {
        State::Ground
    }
}

impl OscParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next chunk of terminal output. Chunks may split a sequence anywhere.
    pub fn push(&mut self, chunk: &str) {
        self.push_bytes(chunk.as_bytes());
    }

    pub fn push_bytes(&mut self, chunk: &[u8]) {
        for &byte in chunk {
            self.feed_byte(byte);
        }
    }

    /// Latest cwd decoded from OSC 7, if a `file://` URI parsed.
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// Latest OSC 133 marker, `"A"` or `"B"`.
    pub fn marker(&self) -> Option<&str> {
        self.marker
    }

    fn feed_byte(&mut self, byte: u8) {
        match std::mem::replace(&mut self.state, State::Ground) {
            State::Ground => {
                if byte == 0x1b {
                    self.state = State::Esc;
                }
            }
            State::Esc => {
                if byte == b']' {
                    self.state = State::Osc(Vec::new());
                } else if byte == b'P' {
                    self.state = State::Dcs(Vec::new());
                } else if byte == 0x1b {
                    self.state = State::Esc;
                }
            }
            State::Osc(mut payload) => {
                if byte == 0x07 {
                    self.finish_osc(&payload);
                } else if byte == 0x1b {
                    self.state = State::OscEsc(payload);
                } else {
                    payload.push(byte);
                    self.state = State::Osc(payload);
                }
            }
            State::OscEsc(payload) => {
                if byte == b'\\' {
                    self.finish_osc(&payload);
                } else if byte == 0x1b {
                    self.state = State::Esc;
                } else {
                    self.feed_byte(byte);
                }
            }
            State::Dcs(mut payload) => {
                if byte == 0x1b {
                    self.state = State::DcsEsc(payload);
                } else {
                    payload.push(byte);
                    self.state = State::Dcs(payload);
                }
            }
            State::DcsEsc(mut payload) => {
                if byte == b'\\' {
                    self.finish_dcs(&payload);
                } else if byte == 0x1b {
                    // ESC ESC inside tmux DCS is one literal ESC of passthrough data.
                    payload.push(0x1b);
                    self.state = State::Dcs(payload);
                } else {
                    payload.push(0x1b);
                    payload.push(byte);
                    self.state = State::Dcs(payload);
                }
            }
        }
    }

    fn finish_osc(&mut self, payload: &[u8]) {
        let Ok(text) = std::str::from_utf8(payload) else {
            return;
        };
        if let Some(rest) = text.strip_prefix("7;") {
            if let Some(path) = parse_osc7(rest) {
                self.cwd = Some(path);
            }
        } else if let Some(rest) = text.strip_prefix("133;") {
            if rest.starts_with('A') {
                self.marker = Some("A");
            } else if rest.starts_with('B') {
                self.marker = Some("B");
            }
        }
    }

    fn finish_dcs(&mut self, payload: &[u8]) {
        const PREFIX: &[u8] = b"tmux;";
        if !payload.starts_with(PREFIX) {
            return;
        }
        let inner = payload[PREFIX.len()..].to_vec();
        for byte in inner {
            self.feed_byte(byte);
        }
    }
}

/// Parse an OSC 7 payload (`file://host/path`) into a usable path.
///
/// Windows paths arrive as `/C:/Users/...` and become `C:\Users\...`.
/// Port of `src/renderer/lib/osc7.ts`.
pub fn parse_osc7(data: &str) -> Option<String> {
    let data = data.trim();
    let rest = data.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let path = &rest[slash..];
    let decoded = decode_uri_component(path).unwrap_or_else(|| path.to_string());
    Some(windows_file_path(&decoded))
}

fn windows_file_path(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        path[1..].replace('/', "\\")
    } else {
        path.to_string()
    }
}

/// `decodeURIComponent`. A bad escape yields `None` so the caller keeps the raw path.
fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_osc_7_file_uri_updates_cwd() {
        let mut parser = OscParser::new();
        parser.push("\u{1b}]7;file:///home/ada\u{7}");
        assert_eq!(parser.cwd(), Some("/home/ada"));
        assert_eq!(parser.marker(), None);
    }

    #[test]
    fn osc_7_windows_file_uri_becomes_a_drive_path() {
        let mut parser = OscParser::new();
        parser.push("\u{1b}]7;file:///C:/Users/ada\u{7}");
        assert_eq!(parser.cwd(), Some(r"C:\Users\ada"));
        parser.push("\u{1b}]7;file:///C:/Users/a%20b\u{1b}\\");
        assert_eq!(parser.cwd(), Some(r"C:\Users\a b"));
    }

    #[test]
    fn osc_133_a_and_b_markers() {
        let mut parser = OscParser::new();
        parser.push("\u{1b}]133;A\u{7}");
        assert_eq!(parser.marker(), Some("A"));
        parser.push("\u{1b}]133;B\u{7}");
        assert_eq!(parser.marker(), Some("B"));
        assert_eq!(parser.cwd(), None);
    }

    #[test]
    fn bel_and_st_terminators() {
        let mut parser = OscParser::new();
        parser.push("\u{1b}]7;file:///home/ada\u{7}");
        assert_eq!(parser.cwd(), Some("/home/ada"));
        parser.push("\u{1b}]7;file:///tmp/work\u{1b}\\");
        assert_eq!(parser.cwd(), Some("/tmp/work"));
        parser.push("\u{1b}]133;A\u{1b}\\");
        assert_eq!(parser.marker(), Some("A"));
        parser.push("\u{1b}]133;B\u{7}");
        assert_eq!(parser.marker(), Some("B"));
    }

    #[test]
    fn dcs_wrapped_tmux_form() {
        let mut parser = OscParser::new();
        // ESC P tmux ; ESC ESC ] 7 ; file:///home/ada BEL ESC \
        parser.push("\u{1b}Ptmux;\u{1b}\u{1b}]7;file:///home/ada\u{7}\u{1b}\\");
        assert_eq!(parser.cwd(), Some("/home/ada"));
        parser.push("\u{1b}Ptmux;\u{1b}\u{1b}]133;A\u{7}\u{1b}\\");
        assert_eq!(parser.marker(), Some("A"));
        parser.push("\u{1b}Ptmux;\u{1b}\u{1b}]133;B\u{7}\u{1b}\\");
        assert_eq!(parser.marker(), Some("B"));
        parser.push("\u{1b}Ptmux;\u{1b}\u{1b}]7;file:///C:/Users/ada\u{7}\u{1b}\\");
        assert_eq!(parser.cwd(), Some(r"C:\Users\ada"));
    }

    #[test]
    fn sequences_split_across_chunks() {
        let mut parser = OscParser::new();
        parser.push("\u{1b}]7;file://");
        assert_eq!(parser.cwd(), None);
        parser.push("/home/ada\u{7}");
        assert_eq!(parser.cwd(), Some("/home/ada"));

        parser.push("\u{1b}");
        parser.push("]133;");
        parser.push("B\u{7}");
        assert_eq!(parser.marker(), Some("B"));

        parser.push("\u{1b}Ptmux;\u{1b}");
        parser.push("\u{1b}]133;A\u{7}\u{1b}\\");
        assert_eq!(parser.marker(), Some("A"));

        let wrapped = "\u{1b}Ptmux;\u{1b}\u{1b}]7;file:///var/log\u{7}\u{1b}\\";
        for chunk in wrapped.as_bytes().chunks(3) {
            parser.push_bytes(chunk);
        }
        assert_eq!(parser.cwd(), Some("/var/log"));
    }

    #[test]
    fn ignores_osc_9_and_99() {
        let mut parser = OscParser::new();
        parser.push("\u{1b}]9;file:///home/ada\u{7}");
        parser.push("\u{1b}]99;A\u{7}");
        assert_eq!(parser.cwd(), None);
        assert_eq!(parser.marker(), None);
        parser.push("noise \u{1b}]7;file:///home/ada\u{7} more \u{1b}]133;A\u{7}");
        assert_eq!(parser.cwd(), Some("/home/ada"));
        assert_eq!(parser.marker(), Some("A"));
    }

    #[test]
    fn parse_osc7_decodes_uri_and_windows_drives() {
        assert_eq!(parse_osc7("file:///home/ada").as_deref(), Some("/home/ada"));
        assert_eq!(
            parse_osc7("file:///C:/Users/ada").as_deref(),
            Some(r"C:\Users\ada")
        );
        assert_eq!(
            parse_osc7("file://localhost/home/ada").as_deref(),
            Some("/home/ada")
        );
        assert_eq!(
            parse_osc7("file:///home/ada%20lovelace").as_deref(),
            Some("/home/ada lovelace")
        );
        assert_eq!(parse_osc7("file:///home/%ZZ").as_deref(), Some("/home/%ZZ"));
        assert_eq!(
            parse_osc7(" file:///home/ada ").as_deref(),
            Some("/home/ada")
        );
        assert_eq!(
            parse_osc7("file:///home/caf%C3%A9").as_deref(),
            Some("/home/café")
        );
        assert_eq!(parse_osc7("not a uri"), None);
    }
}
