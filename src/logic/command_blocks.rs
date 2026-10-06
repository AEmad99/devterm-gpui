//! OSC 133 command-block tracking and comments.
//!
//! A completed command is A then B, closed by the next prompt A.
//! D stores the exit status. C does not complete a block.
//! Gutters mark finished commands. There are no header bars.

#![allow(dead_code)]

use std::collections::BTreeMap;

pub const MAX_COMMAND_GUTTERS: usize = 48;
pub const MAX_COMMENTS_PER_SESSION: usize = 80;
pub const COMMENTS_KEY: &str = "devterm.block-comments.v1";

/// Command blocks are gutter decorations, not header bars.
pub const COMMAND_BLOCK_HEADER_BARS: bool = false;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscKind {
    A,
    B,
    C,
    D,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Osc133Event {
    pub kind: OscKind,
    pub line: i64,
    pub x: i64,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedBlock {
    pub id: String,
    pub start_line: i64,
    pub input_line: i64,
    pub input_x: i64,
    pub end_line: i64,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingBlock {
    pub start_line: i64,
    pub input_line: Option<i64>,
    pub input_x: Option<i64>,
    pub saw_b: bool,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockTrackerState {
    pub pending: Option<PendingBlock>,
    pub next_id: u64,
}

pub fn empty_block_tracker() -> BlockTrackerState {
    BlockTrackerState {
        pending: None,
        next_id: 1,
    }
}

pub fn reduce_osc133(
    state: &BlockTrackerState,
    event: &Osc133Event,
) -> (BlockTrackerState, Option<CompletedBlock>) {
    if event.kind == OscKind::D {
        let Some(pending) = state.pending.clone() else {
            return (state.clone(), None);
        };
        return (
            BlockTrackerState {
                pending: Some(PendingBlock {
                    exit_code: event.exit_code,
                    ..pending
                }),
                next_id: state.next_id,
            },
            None,
        );
    }
    if event.kind == OscKind::B {
        let mut pending = state.pending.clone().unwrap_or(PendingBlock {
            start_line: event.line,
            input_line: None,
            input_x: None,
            saw_b: false,
            exit_code: None,
        });
        pending.saw_b = true;
        pending.input_line = Some(event.line);
        pending.input_x = Some(event.x);
        return (
            BlockTrackerState {
                pending: Some(pending),
                next_id: state.next_id,
            },
            None,
        );
    }
    if event.kind != OscKind::A {
        return (state.clone(), None);
    }
    let mut completed = None;
    let mut next_id = state.next_id;
    if let Some(pending) = &state.pending {
        if pending.saw_b && pending.input_line.is_some() {
            completed = Some(CompletedBlock {
                id: format!("b{}", state.next_id),
                start_line: pending.start_line,
                input_line: pending.input_line.unwrap(),
                input_x: pending.input_x.unwrap_or(0),
                end_line: event.line,
                exit_code: pending.exit_code,
            });
            next_id += 1;
        }
    }
    (
        BlockTrackerState {
            pending: Some(PendingBlock {
                start_line: event.line,
                input_line: None,
                input_x: None,
                saw_b: false,
                exit_code: None,
            }),
            next_id,
        },
        completed,
    )
}

/// Entries that should be disposed so only the newest `max` remain.
pub fn gutters_to_release<T: Clone>(items: &[T], max: usize) -> Vec<T> {
    if items.len() <= max {
        return Vec::new();
    }
    items[..items.len() - max].to_vec()
}

pub fn comment_key(command: &str) -> String {
    let mut collapsed = String::new();
    let mut prev_space = false;
    for c in command.chars() {
        if c.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
                prev_space = true;
            }
        } else {
            collapsed.push(c);
            prev_space = false;
        }
    }
    collapsed.trim().chars().take(240).collect()
}

#[derive(Clone, Debug, Default)]
pub struct CommentStore {
    sessions: BTreeMap<String, BTreeMap<String, (String, String)>>,
}

impl CommentStore {
    pub fn save(&mut self, session_id: &str, command: &str, comment: &str) {
        let key = comment_key(command);
        if key.is_empty() {
            return;
        }
        let session = self.sessions.entry(session_id.to_string()).or_default();
        let trimmed = comment.trim();
        if trimmed.is_empty() {
            session.remove(&key);
        } else {
            session.insert(key, (comment_key(command), trimmed.to_string()));
        }
        if session.len() > MAX_COMMENTS_PER_SESSION {
            let extra: Vec<String> = session
                .keys()
                .take(session.len() - MAX_COMMENTS_PER_SESSION)
                .cloned()
                .collect();
            for k in extra {
                session.remove(&k);
            }
        }
    }

    pub fn load(&self, session_id: &str, command: &str) -> Option<String> {
        let key = comment_key(command);
        self.sessions
            .get(session_id)
            .and_then(|s| s.get(&key))
            .map(|(_, comment)| comment.clone())
    }

    pub fn comments_for_prompt(&self, session_id: &str) -> String {
        let Some(session) = self.sessions.get(session_id) else {
            return String::new();
        };
        let rows: Vec<_> = session
            .values()
            .filter(|(_, comment)| !comment.trim().is_empty())
            .collect();
        if rows.is_empty() {
            return String::new();
        }
        let start = rows.len().saturating_sub(MAX_COMMENTS_PER_SESSION);
        let body = rows[start..]
            .iter()
            .map(|(command, comment)| format!("- `{command}`: {comment}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Operator comments on this pane:\n{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: OscKind, line: i64, x: i64, exit_code: Option<i32>) -> Osc133Event {
        Osc133Event {
            kind,
            line,
            x,
            exit_code,
        }
    }

    #[test]
    fn completes_a_block_only_after_a_then_b_then_the_next_a() {
        let mut state = empty_block_tracker();
        let (next, completed) = reduce_osc133(&state, &ev(OscKind::A, 10, 0, None));
        state = next;
        assert!(completed.is_none());

        let (next, completed) = reduce_osc133(&state, &ev(OscKind::B, 10, 8, None));
        state = next;
        assert!(completed.is_none());

        let (_next, completed) = reduce_osc133(&state, &ev(OscKind::A, 24, 0, None));
        let block = completed.expect("completed");
        assert_eq!(block.start_line, 10);
        assert_eq!(block.input_line, 10);
        assert_eq!(block.input_x, 8);
        assert_eq!(block.end_line, 24);
        assert_eq!(block.id, "b1");
        assert_eq!(block.exit_code, None);
    }

    #[test]
    fn does_not_complete_a_prompt_that_never_saw_b() {
        let mut state = empty_block_tracker();
        state = reduce_osc133(&state, &ev(OscKind::A, 1, 0, None)).0;
        let (_next, completed) = reduce_osc133(&state, &ev(OscKind::A, 4, 0, None));
        assert!(completed.is_none());
    }

    #[test]
    fn keeps_a_d_exit_code_until_the_next_a_completes_the_block() {
        let mut state = empty_block_tracker();
        state = reduce_osc133(&state, &ev(OscKind::A, 1, 0, None)).0;
        state = reduce_osc133(&state, &ev(OscKind::B, 1, 2, None)).0;
        let (next, completed) = reduce_osc133(&state, &ev(OscKind::C, 2, 0, None));
        assert!(completed.is_none());
        let (next, completed) = reduce_osc133(&next, &ev(OscKind::D, 8, 0, Some(1)));
        assert!(completed.is_none());
        let (_next, completed) = reduce_osc133(&next, &ev(OscKind::A, 9, 0, None));
        let block = completed.expect("completed");
        assert_eq!(block.exit_code, Some(1));
        assert_eq!(block.start_line, 1);
        assert_eq!(block.end_line, 9);
    }

    #[test]
    fn gutters_keep_the_newest() {
        let items: Vec<i32> = (0..(MAX_COMMAND_GUTTERS as i32 + 3)).collect();
        assert_eq!(
            gutters_to_release(&items, MAX_COMMAND_GUTTERS),
            vec![0, 1, 2]
        );
        assert!(gutters_to_release(&[1, 2, 3], 3).is_empty());
    }

    #[test]
    fn comment_key_collapses_whitespace_and_caps_length() {
        assert_eq!(comment_key("  git   status  "), "git status");
        assert_eq!(comment_key(&"x".repeat(400)).chars().count(), 240);
    }

    #[test]
    fn no_header_bars() {
        assert!(!COMMAND_BLOCK_HEADER_BARS);
    }
}
