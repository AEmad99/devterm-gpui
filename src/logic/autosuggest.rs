//! History-driven inline autocomplete.
//!
//! OSC 133 `;B` is the command-input anchor: it fires where command input
//! begins. Suggestions come from history. Accepting returns the keystrokes
//! that complete the line. It does not write the terminal buffer.

#![allow(dead_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandStat {
    pub command: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuggestView {
    pub items: Vec<String>,
    pub index: usize,
    pub prefix: String,
    pub left: f64,
    pub top: f64,
    pub above: bool,
}

/// Move a listbox selection with wraparound.
pub fn move_selection(index: i32, delta: i32, length: i32) -> i32 {
    if length <= 0 {
        return 0;
    }
    (index + delta + length) % length
}

/// History commands that continue `prefix` (case-insensitive), recency first
/// then by frequency, deduped. Only commands longer than the prefix count.
pub fn suggestions_for(
    prefix: &str,
    recent: &[String],
    frequent: &[CommandStat],
    limit: usize,
) -> Vec<String> {
    if prefix.trim().is_empty() {
        return Vec::new();
    }
    let lp = prefix.to_lowercase();
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut consider = |cmd: &str| {
        if out.len() >= limit || cmd.is_empty() || seen.contains(cmd) {
            return;
        }
        if cmd.len() <= prefix.len() {
            return;
        }
        if !cmd.to_lowercase().starts_with(&lp) {
            return;
        }
        seen.insert(cmd.to_string());
        out.push(cmd.to_string());
    };
    for c in recent {
        consider(c);
    }
    for f in frequent {
        consider(&f.command);
    }
    out
}

/// Keystrokes that turn the typed `prefix` into the full `command`.
/// This is the text to send. It is not a buffer write.
pub fn accept_keys(prefix: &str, command: &str) -> String {
    if let Some(rest) = command.strip_prefix(prefix) {
        return rest.to_string();
    }
    format!("{}{command}", "\u{7f}".repeat(prefix.len()))
}

/// Cursor recorded when OSC 133 `;B` arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputAnchor {
    pub line: i64,
    pub x: i64,
}

/// `;B` sets the command-input anchor. `;C` and `;D` drop it.
/// `;A` refreshes history and leaves the anchor in place.
pub fn reduce_input_anchor(
    anchor: Option<InputAnchor>,
    kind: char,
    line: i64,
    x: i64,
) -> Option<InputAnchor> {
    match kind {
        'B' => Some(InputAnchor { line, x }),
        'C' | 'D' => None,
        _ => anchor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_are_history_driven() {
        let recent = vec![
            "git status".into(),
            "git stash".into(),
            "ls".into(),
            "git".into(),
        ];
        let frequent = vec![CommandStat {
            command: "git show".into(),
            count: 4,
        }];
        assert!(suggestions_for("   ", &recent, &frequent, 6).is_empty());
        assert_eq!(
            suggestions_for("git s", &recent, &frequent, 6),
            vec!["git status", "git stash", "git show"]
        );
        assert_eq!(
            suggestions_for("GIT S", &recent, &frequent, 1),
            vec!["git status"]
        );
    }

    #[test]
    fn accepting_returns_keystrokes_and_does_not_write_the_buffer() {
        assert_eq!(accept_keys("git st", "git status"), "atus");
        let keys = accept_keys("Git", "git status");
        assert_eq!(keys, format!("{}git status", "\u{7f}".repeat(3)));
        assert!(!keys.contains('\n'));
    }

    #[test]
    fn osc_133_b_is_the_command_input_anchor() {
        let anchor = reduce_input_anchor(None, 'B', 10, 8);
        assert_eq!(anchor, Some(InputAnchor { line: 10, x: 8 }));
        assert_eq!(reduce_input_anchor(anchor.clone(), 'A', 11, 0), anchor);
        assert_eq!(reduce_input_anchor(anchor.clone(), 'C', 12, 0), None);
        assert_eq!(reduce_input_anchor(anchor, 'D', 13, 0), None);
    }

    #[test]
    fn selection_wraps() {
        assert_eq!(move_selection(0, -1, 3), 2);
        assert_eq!(move_selection(2, 1, 3), 0);
        assert_eq!(move_selection(0, 1, 0), 0);
    }
}
