//! Hidden-group terminal hibernate decisions.
//!
//! Hibernate disposes a hidden group's terminal surface after the delay and
//! replays a bounded output ring when the group is focused again.
//! Attention-needed sessions and dirty editors stay rendered.
//! The OS process is not hibernated.

#![allow(dead_code)]

pub const DEFAULT_HIBERNATE_AFTER_MS: u64 = 30_000;
pub const MIN_HIBERNATE_AFTER_MS: u64 = 1_000;
pub const MAX_HIBERNATE_AFTER_MS: u64 = 24 * 60 * 60 * 1000;
pub const DEFAULT_OUTPUT_RING_LINES: u32 = 10_000;
pub const MIN_OUTPUT_RING_LINES: u32 = 100;
pub const MAX_OUTPUT_RING_LINES: u32 = 100_000;

/// Hibernate never suspends or kills the OS process. It only drops the
/// hidden group's terminal surface.
pub const HIBERNATES_OS_PROCESS: bool = false;

pub fn normalize_hibernate_after_ms(value: Option<f64>) -> u64 {
    let Some(value) = value else {
        return DEFAULT_HIBERNATE_AFTER_MS;
    };
    if !value.is_finite() {
        return DEFAULT_HIBERNATE_AFTER_MS;
    }
    let n = value.floor() as i64;
    n.clamp(MIN_HIBERNATE_AFTER_MS as i64, MAX_HIBERNATE_AFTER_MS as i64) as u64
}

pub fn normalize_output_ring_lines(value: Option<f64>, fallback: u32) -> u32 {
    let Some(value) = value else {
        return fallback;
    };
    if !value.is_finite() {
        return fallback;
    }
    let n = value.floor() as i64;
    n.clamp(MIN_OUTPUT_RING_LINES as i64, MAX_OUTPUT_RING_LINES as i64) as u32
}

#[derive(Clone, Debug)]
pub struct HibernateCandidate<'a> {
    pub kind: &'a str,
    pub group_id: &'a str,
    pub needs_attention: bool,
    pub agent_pending_approval: bool,
}

/// True only for a terminal that is safe to hibernate right now.
/// A true result means the hidden group's terminal surface may be disposed
/// after the delay. It does not hibernate the OS process.
pub fn can_hibernate_session(
    session: &HibernateCandidate<'_>,
    active_group_id: &str,
    dirty_editor: bool,
) -> bool {
    if session.kind != "local" && session.kind != "remote" {
        return false;
    }
    let group = if session.group_id.is_empty() {
        "default"
    } else {
        session.group_id
    };
    if group == active_group_id {
        return false;
    }
    if session.needs_attention || session.agent_pending_approval {
        return false;
    }
    if dirty_editor {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_an_inactive_clean_terminal_after_the_delay() {
        assert!(can_hibernate_session(
            &HibernateCandidate {
                kind: "local",
                group_id: "background",
                needs_attention: false,
                agent_pending_approval: false,
            },
            "active",
            false,
        ));
    }

    #[test]
    fn does_not_hibernate_a_hidden_terminal_that_needs_attention() {
        assert!(!can_hibernate_session(
            &HibernateCandidate {
                kind: "remote",
                group_id: "background",
                needs_attention: true,
                agent_pending_approval: false,
            },
            "active",
            false,
        ));
    }

    #[test]
    fn does_not_hibernate_a_dirty_editor_or_a_browser_pane() {
        assert!(!can_hibernate_session(
            &HibernateCandidate {
                kind: "local",
                group_id: "background",
                needs_attention: false,
                agent_pending_approval: false,
            },
            "active",
            true,
        ));
        assert!(!can_hibernate_session(
            &HibernateCandidate {
                kind: "browser",
                group_id: "background",
                needs_attention: false,
                agent_pending_approval: false,
            },
            "active",
            false,
        ));
    }

    #[test]
    fn does_not_hibernate_the_active_group_or_the_os_process() {
        assert!(!can_hibernate_session(
            &HibernateCandidate {
                kind: "local",
                group_id: "active",
                needs_attention: false,
                agent_pending_approval: false,
            },
            "active",
            false,
        ));
        assert!(!can_hibernate_session(
            &HibernateCandidate {
                kind: "remote",
                group_id: "background",
                needs_attention: false,
                agent_pending_approval: true,
            },
            "active",
            false,
        ));
        assert!(!HIBERNATES_OS_PROCESS);
        assert_eq!(
            normalize_hibernate_after_ms(None),
            DEFAULT_HIBERNATE_AFTER_MS
        );
        assert_eq!(normalize_output_ring_lines(Some(50.0), 10), 100);
        assert_eq!(
            normalize_output_ring_lines(Some(500_000.0), DEFAULT_OUTPUT_RING_LINES),
            MAX_OUTPUT_RING_LINES
        );
    }
}
