//! Approval-rule pre-check for the agent guardrail.
//!
//! Port of `src/main/agent/approval-rules.ts`. Longest-prefix match at a token
//! boundary. There is no per-session policy picker: launches use policy mode
//! `full` (see mcp_policy). An explicit allow/deny/ask rule is a pre-check
//! on top of that. A confirm that nobody answers resolves to the string
//! `"timeout"` after 120 seconds — not `"denied"`.

/// Operator did not answer the confirm prompt. Distinct from an explicit deny.
pub const CONFIRM_TIMEOUT_RESULT: &str = "timeout";
/// `src/main/ipc/agent.ts` confirm timer.
pub const CONFIRM_TIMEOUT_MS: u64 = 120_000;

/// Result string when the 120s confirm timer fires.
pub fn confirm_timeout_result() -> &'static str {
    CONFIRM_TIMEOUT_RESULT
}

/// Launches do not offer a per-session policy picker. The shipped mode is `full`.
pub const LAUNCH_POLICY_MODE: &str = "full";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalRule {
    pub id: String,
    pub command_prefix: String,
    pub outcome: String,
    pub session_id: Option<String>,
    pub created_at: i64,
}

/// Longest-prefix, token-boundary match.
///
/// Specificity is `command_prefix.len()`. A longer global rule beats a shorter
/// session rule. A session rule beats a global rule of the same length.
/// The prefix must end at a token boundary (whitespace, `|`, `&`, `;`, `>`,
/// `<`, `(`, or end of string) so `kubectl` matches `kubectl get pods` but
/// not `kubectlized`.
pub fn match_rules(
    rules: &[ApprovalRule],
    session_id: &str,
    command: &str,
) -> Option<ApprovalRule> {
    let cmd = command.trim_start();
    if cmd.is_empty() {
        return None;
    }
    let cmd_bytes = cmd.as_bytes();
    let mut best: Option<ApprovalRule> = None;
    for r in rules {
        if let Some(sid) = r.session_id.as_deref() {
            if sid != session_id {
                continue;
            }
        }
        let prefix = r.command_prefix.as_bytes();
        if cmd_bytes.len() < prefix.len() {
            continue;
        }
        if !cmd_bytes.starts_with(prefix) {
            continue;
        }
        let is_boundary = if prefix.len() == cmd_bytes.len() {
            true
        } else {
            matches!(
                cmd_bytes[prefix.len()],
                0x20 | 0x09 | 0x0a | b'|' | b'&' | b';' | b'>' | b'<' | b'('
            )
        };
        if !is_boundary {
            continue;
        }
        let replace = match &best {
            None => true,
            Some(cur) if r.command_prefix.len() > cur.command_prefix.len() => true,
            Some(cur)
                if r.command_prefix.len() == cur.command_prefix.len()
                    && r.session_id.as_deref() == Some(session_id)
                    && cur.session_id.is_none() =>
            {
                true
            }
            Some(_) => false,
        };
        if replace {
            best = Some(r.clone());
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(prefix: &str, outcome: &str, session_id: Option<&str>) -> ApprovalRule {
        ApprovalRule {
            id: prefix.to_string(),
            command_prefix: prefix.to_string(),
            outcome: outcome.to_string(),
            session_id: session_id.map(|s| s.to_string()),
            created_at: 0,
        }
    }

    #[test]
    fn matches_the_prefix_as_the_entire_command() {
        let r = rule("kubectl", "allow", None);
        assert!(match_rules(&[r], "s", "kubectl").is_some());
    }

    #[test]
    fn matches_the_prefix_followed_by_space() {
        let r = rule("kubectl", "allow", None);
        assert!(match_rules(&[r], "s", "kubectl get pods").is_some());
    }

    #[test]
    fn matches_tab_newline_pipe_amp_semicolon() {
        let r = rule("ls", "allow", None);
        for cmd in [
            "ls\t-la",
            "ls\n-la",
            "ls | grep x",
            "ls && echo ok",
            "ls; echo ok",
        ] {
            assert!(
                match_rules(&[r.clone()], "s", cmd).is_some(),
                "should match: {cmd}"
            );
        }
    }

    #[test]
    fn matches_redirect_or_paren() {
        let r = rule("echo", "allow", None);
        assert!(match_rules(&[r.clone()], "s", "echo hi > out").is_some());
        assert!(match_rules(&[r.clone()], "s", "echo hi < in").is_some());
        assert!(match_rules(&[r], "s", "echo(hi").is_some());
    }

    #[test]
    fn does_not_match_a_longer_word() {
        let r = rule("kubectl", "allow", None);
        assert!(match_rules(&[r], "s", "kubectlized get pods").is_none());
    }

    #[test]
    fn does_not_match_when_prefix_is_not_a_prefix() {
        let r = rule("kubectl", "allow", None);
        assert!(match_rules(&[r], "s", "docker ps").is_none());
    }

    #[test]
    fn trims_leading_whitespace() {
        let r = rule("kubectl", "allow", None);
        assert!(match_rules(&[r], "s", "   kubectl get pods").is_some());
    }

    #[test]
    fn empty_command_is_none() {
        let r = rule("kubectl", "allow", None);
        assert!(match_rules(&[r.clone()], "s", "").is_none());
        assert!(match_rules(&[r], "s", "   ").is_none());
    }

    #[test]
    fn longer_prefix_wins() {
        let a = rule("kubectl", "allow", None);
        let b = rule("kubectl delete", "deny", None);
        let m = match_rules(&[a, b], "s", "kubectl delete pod x").unwrap();
        assert_eq!(m.outcome, "deny");
    }

    #[test]
    fn shorter_prefix_when_longer_does_not_match() {
        let a = rule("kubectl", "allow", None);
        let b = rule("kubectl delete", "deny", None);
        let m = match_rules(&[a, b], "s", "kubectl get pods").unwrap();
        assert_eq!(m.outcome, "allow");
    }

    #[test]
    fn longer_global_beats_shorter_session() {
        let longer = rule("kubectl delete", "deny", None);
        let shorter = rule("kubectl", "allow", Some("s1"));
        let m = match_rules(&[longer, shorter], "s1", "kubectl delete pod x").unwrap();
        assert_eq!(m.outcome, "deny");
    }

    #[test]
    fn same_length_session_beats_global() {
        let global = rule("kubectl", "allow", None);
        let session = rule("kubectl", "deny", Some("s1"));
        let m = match_rules(&[global, session], "s1", "kubectl get pods").unwrap();
        assert_eq!(m.outcome, "deny");
    }

    #[test]
    fn session_rule_only_matches_its_session() {
        let r = rule("kubectl", "allow", Some("s1"));
        assert!(match_rules(&[r.clone()], "s1", "kubectl get pods").is_some());
        assert!(match_rules(&[r], "s2", "kubectl get pods").is_none());
    }

    #[test]
    fn global_rule_matches_any_session() {
        let r = rule("kubectl", "allow", None);
        assert!(match_rules(&[r.clone()], "s1", "kubectl get pods").is_some());
        assert!(match_rules(&[r], "s2", "kubectl get pods").is_some());
    }

    #[test]
    fn global_rule_for_a_different_session() {
        let session = rule("kubectl", "deny", Some("s1"));
        let global = rule("kubectl", "allow", None);
        let m = match_rules(&[session, global], "s2", "kubectl get pods").unwrap();
        assert_eq!(m.outcome, "allow");
    }

    #[test]
    fn confirm_timeout_is_the_string_timeout_after_120s() {
        assert_eq!(confirm_timeout_result(), "timeout");
        assert_eq!(CONFIRM_TIMEOUT_MS, 120_000);
        assert_eq!(LAUNCH_POLICY_MODE, "full");
        assert_ne!(CONFIRM_TIMEOUT_RESULT, "denied");
    }
}
