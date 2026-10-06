//! Lightweight highlighter for the OSC 133 command editor.
//!
//! This is not drawn on the live terminal (known limit). The library stays
//! so an editor surface can still color a command line.

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellDialect {
    Shell,
    Powershell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellTokenKind {
    Text,
    Command,
    Flag,
    String,
    Comment,
    Operator,
    Number,
    Path,
}

impl ShellTokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Command => "command",
            Self::Flag => "flag",
            Self::String => "string",
            Self::Comment => "comment",
            Self::Operator => "operator",
            Self::Number => "number",
            Self::Path => "path",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellToken {
    pub kind: ShellTokenKind,
    pub text: String,
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn take_string(src: &str) -> (&str, &str) {
    let bytes = src.as_bytes();
    let quote = bytes[0];
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            i += 1;
            break;
        }
        i += 1;
    }
    (&src[..i], &src[i..])
}

fn match_operator(src: &str) -> Option<usize> {
    if src.starts_with("&&")
        || src.starts_with("||")
        || src.starts_with(";;")
        || src.starts_with("<<")
        || src.starts_with(">>")
    {
        return Some(2);
    }
    let b = src.as_bytes().first().copied()?;
    if matches!(b, b'|' | b';' | b'&' | b'<' | b'>') {
        return Some(1);
    }
    None
}

fn match_flag(src: &str) -> Option<usize> {
    let b = src.as_bytes();
    if b.first() != Some(&b'-') {
        return None;
    }
    let mut i = 1;
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    if i >= b.len() || !is_word_byte(b[i]) {
        return None;
    }
    i += 1;
    while i < b.len() && (is_word_byte(b[i]) || b[i] == b'-') {
        i += 1;
    }
    Some(i)
}

fn match_number(src: &str) -> Option<usize> {
    let b = src.as_bytes();
    if b.first().map(|c| c.is_ascii_digit()) != Some(true) {
        return None;
    }
    let mut i = 1;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    Some(i)
}

fn path_class(b: u8) -> bool {
    is_word_byte(b) || matches!(b, b'.' | b'+' | b'@' | b'-')
}

fn match_posix_path(src: &str) -> Option<usize> {
    let b = src.as_bytes();
    let mut i = 0;
    if src.starts_with('~') {
        i = 1;
    } else if src.starts_with("..") {
        i = 2;
    } else if src.starts_with('.') {
        i = 1;
    }
    let start_segs = i;
    if i >= b.len() || b[i] != b'/' {
        return None;
    }
    let mut segs = 0;
    while i < b.len() && b[i] == b'/' {
        let seg = i + 1;
        let mut j = seg;
        while j < b.len() && path_class(b[j]) {
            j += 1;
        }
        if j == seg {
            break;
        }
        i = j;
        segs += 1;
        if i < b.len() && b[i] == b'/' && (i + 1 == b.len() || !path_class(b[i + 1])) {
            i += 1;
            break;
        }
    }
    if segs == 0 || i == start_segs {
        return None;
    }
    Some(i)
}

fn win_class(b: u8) -> bool {
    is_word_byte(b) || matches!(b, b'.' | b'\\' | b' ' | b'-')
}

fn match_win_path(src: &str) -> Option<usize> {
    let b = src.as_bytes();
    let mut i = if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        3
    } else if src.starts_with("\\\\") {
        2
    } else {
        return None;
    };
    let start = i;
    while i < b.len() && win_class(b[i]) {
        i += 1;
    }
    if i == start {
        return None;
    }
    Some(i)
}

fn match_word(src: &str) -> Option<usize> {
    let b = src.as_bytes();
    if b.is_empty() {
        return None;
    }
    if b[0].is_ascii_whitespace()
        || matches!(b[0], b'\'' | b'"' | b'#' | b';' | b'|' | b'&' | b'<' | b'>')
    {
        return None;
    }
    let mut i = 1;
    while i < b.len()
        && !b[i].is_ascii_whitespace()
        && !matches!(b[i], b'\'' | b'"' | b'#' | b';' | b'|' | b'&' | b'<' | b'>')
    {
        i += 1;
    }
    Some(i)
}

/// Tokenize one command line. Not applied to the live terminal buffer.
pub fn tokenize_shell(input: &str, dialect: ShellDialect) -> Vec<ShellToken> {
    let mut tokens = Vec::new();
    let mut rest = input;
    let mut first_word = true;
    while !rest.is_empty() {
        let space_len = rest
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| c.len_utf8())
            .sum::<usize>();
        if space_len > 0 {
            tokens.push(ShellToken {
                kind: ShellTokenKind::Text,
                text: rest[..space_len].to_string(),
            });
            rest = &rest[space_len..];
            continue;
        }
        if dialect == ShellDialect::Shell && rest.starts_with('#') {
            tokens.push(ShellToken {
                kind: ShellTokenKind::Comment,
                text: rest.to_string(),
            });
            break;
        }
        if dialect == ShellDialect::Powershell && rest.starts_with("<#") {
            let end = rest.find("#>");
            let (chunk, next) = if let Some(end) = end {
                (&rest[..end + 2], &rest[end + 2..])
            } else {
                (rest, "")
            };
            tokens.push(ShellToken {
                kind: ShellTokenKind::Comment,
                text: chunk.to_string(),
            });
            rest = next;
            continue;
        }
        if (dialect == ShellDialect::Powershell && rest.starts_with('#')) || rest.starts_with("<#")
        {
            tokens.push(ShellToken {
                kind: ShellTokenKind::Comment,
                text: rest.to_string(),
            });
            break;
        }
        let b0 = rest.as_bytes()[0];
        if b0 == b'\'' || b0 == b'"' {
            let (text, next) = take_string(rest);
            tokens.push(ShellToken {
                kind: ShellTokenKind::String,
                text: text.to_string(),
            });
            rest = next;
            first_word = false;
            continue;
        }
        if let Some(n) = match_operator(rest) {
            tokens.push(ShellToken {
                kind: ShellTokenKind::Operator,
                text: rest[..n].to_string(),
            });
            rest = &rest[n..];
            first_word = true;
            continue;
        }
        if let Some(n) = match_flag(rest) {
            tokens.push(ShellToken {
                kind: ShellTokenKind::Flag,
                text: rest[..n].to_string(),
            });
            rest = &rest[n..];
            first_word = false;
            continue;
        }
        let path = if dialect == ShellDialect::Powershell {
            match_win_path(rest)
        } else {
            match_posix_path(rest)
        };
        if let Some(n) = path {
            tokens.push(ShellToken {
                kind: ShellTokenKind::Path,
                text: rest[..n].to_string(),
            });
            rest = &rest[n..];
            first_word = false;
            continue;
        }
        if !first_word {
            if let Some(n) = match_number(rest) {
                tokens.push(ShellToken {
                    kind: ShellTokenKind::Number,
                    text: rest[..n].to_string(),
                });
                rest = &rest[n..];
                continue;
            }
        }
        if let Some(n) = match_word(rest) {
            tokens.push(ShellToken {
                kind: if first_word {
                    ShellTokenKind::Command
                } else {
                    ShellTokenKind::Text
                },
                text: rest[..n].to_string(),
            });
            rest = &rest[n..];
            first_word = false;
            continue;
        }
        let ch = rest.chars().next().unwrap();
        tokens.push(ShellToken {
            kind: ShellTokenKind::Text,
            text: ch.to_string(),
        });
        rest = &rest[ch.len_utf8()..];
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str, dialect: ShellDialect) -> Vec<String> {
        tokenize_shell(input, dialect)
            .into_iter()
            .filter(|t| t.kind != ShellTokenKind::Text || !t.text.trim().is_empty())
            .map(|t| format!("{}:{}", t.kind.as_str(), t.text))
            .collect()
    }

    #[test]
    fn highlights_git_commit_style_tokens() {
        let got = kinds("git commit -m \"fix bug\"", ShellDialect::Shell);
        assert_eq!(
            got,
            vec![
                "command:git",
                "text:commit",
                "flag:-m",
                "string:\"fix bug\""
            ]
        );
    }

    #[test]
    fn treats_pipes_as_a_new_command() {
        let got = kinds("ls -la | grep foo", ShellDialect::Shell);
        assert_eq!(
            got,
            vec![
                "command:ls",
                "flag:-la",
                "operator:|",
                "command:grep",
                "text:foo"
            ]
        );
    }

    #[test]
    fn marks_comments() {
        let got = kinds("echo hi # note", ShellDialect::Shell);
        assert!(got.iter().any(|t| t.starts_with("comment:")));
    }

    #[test]
    fn highlights_powershell_flags_and_quoted_strings() {
        let got = kinds("Get-ChildItem -Path \"C:\\tmp\"", ShellDialect::Powershell);
        assert_eq!(got[0], "command:Get-ChildItem");
        assert!(got.iter().any(|t| t.starts_with("flag:-Path")));
        assert!(got.iter().any(|t| t.starts_with("string:")));
    }
}
