//! MCP policy decisions and the tool-name lists the registration tests lock.
//!
//! Port of `src/main/mcp/policy.ts` plus the name lists asserted by
//! `tools-register.test.ts`, `tools-list.test.ts`, and the handoff prompt
//! builder in `tools-agent.ts`.
//!
//! Launch policy mode is `"full"`. Host tools are remote only. Workspace
//! tools are `git_status`, `git_diff`, `search_terminals` — no git_commit,
//! git_push, and no port-forward tools. Local handoff tools are local only.
//! Browser and preview tools exist behind a toggle that defaults on.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyMode {
    ReadOnly,
    Confirm,
    Full,
}

impl PolicyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyMode::ReadOnly => "read_only",
            PolicyMode::Confirm => "confirm",
            PolicyMode::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "read_only" => PolicyMode::ReadOnly,
            "confirm" => PolicyMode::Confirm,
            "full" => PolicyMode::Full,
            _ => PolicyMode::Full,
        }
    }
}

/// Shipped launch mode. There is no per-session policy picker.
pub const LAUNCH_POLICY_MODE: &str = "full";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyVerdict {
    pub allow: bool,
    pub need_confirm: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleOutcome {
    Allow,
    Deny,
    Ask,
}

pub struct Policy {
    pub mode: PolicyMode,
}

impl Policy {
    pub fn new(mode: PolicyMode) -> Self {
        Self { mode }
    }

    pub fn evaluate_command(&self, cmd: &str) -> PolicyVerdict {
        let scan = strip_quoted(cmd);
        let destructive = is_destructive(&scan);
        let mutating = is_mutating(&scan);
        match self.mode {
            PolicyMode::ReadOnly => {
                if destructive || mutating {
                    PolicyVerdict {
                        allow: false,
                        need_confirm: false,
                        reason: Some("host is read-only".into()),
                    }
                } else {
                    PolicyVerdict {
                        allow: true,
                        need_confirm: false,
                        reason: None,
                    }
                }
            }
            PolicyMode::Confirm => PolicyVerdict {
                allow: true,
                need_confirm: destructive || mutating,
                reason: None,
            },
            PolicyMode::Full => PolicyVerdict {
                allow: true,
                need_confirm: false,
                reason: None,
            },
        }
    }

    pub fn evaluate_write(&self) -> PolicyVerdict {
        match self.mode {
            PolicyMode::ReadOnly => PolicyVerdict {
                allow: false,
                need_confirm: false,
                reason: Some("host is read-only".into()),
            },
            PolicyMode::Confirm => PolicyVerdict {
                allow: true,
                need_confirm: true,
                reason: None,
            },
            PolicyMode::Full => PolicyVerdict {
                allow: true,
                need_confirm: false,
                reason: None,
            },
        }
    }

    /// Pre-check: allow/deny/ask short-circuit the mode. `None` means no rule.
    pub fn evaluate_command_precheck(&self, rule: Option<RuleOutcome>, cmd: &str) -> PolicyVerdict {
        if let Some(v) = rule_verdict(rule) {
            return v;
        }
        self.evaluate_command(cmd)
    }

    pub fn evaluate_browser(&self, rule: Option<RuleOutcome>, mutating: bool) -> PolicyVerdict {
        if let Some(v) = rule_verdict(rule) {
            return v;
        }
        if !mutating {
            return PolicyVerdict {
                allow: true,
                need_confirm: false,
                reason: None,
            };
        }
        self.evaluate_write()
    }
}

fn rule_verdict(rule: Option<RuleOutcome>) -> Option<PolicyVerdict> {
    Some(match rule? {
        RuleOutcome::Allow => PolicyVerdict {
            allow: true,
            need_confirm: false,
            reason: None,
        },
        RuleOutcome::Deny => PolicyVerdict {
            allow: false,
            need_confirm: false,
            reason: Some("denied by approval rule".into()),
        },
        RuleOutcome::Ask => PolicyVerdict {
            allow: true,
            need_confirm: true,
            reason: Some("approval rule requires confirmation".into()),
        },
    })
}

fn strip_quoted(cmd: &str) -> String {
    let mut out = String::new();
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' || c == '"' {
            let q = c;
            let mut closed = false;
            for n in chars.by_ref() {
                if n == q {
                    closed = true;
                    break;
                }
            }
            if closed {
                out.push(' ');
            } else {
                out.push(q);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn boundary_before(b: &[u8], i: usize) -> bool {
    i == 0 || !is_word(b[i - 1])
}

fn boundary_after(b: &[u8], i: usize) -> bool {
    i >= b.len() || !is_word(b[i])
}

fn has_word(b: &[u8], word: &[u8]) -> bool {
    let mut i = 0;
    while i + word.len() <= b.len() {
        if boundary_before(b, i)
            && b[i..i + word.len()].eq_ignore_ascii_case(word)
            && boundary_after(b, i + word.len())
        {
            return true;
        }
        i += 1;
    }
    false
}

fn is_destructive(scan: &str) -> bool {
    let lower = scan.to_ascii_lowercase();
    let b = lower.as_bytes();
    rm_flag(b)
        || has_word(b, b"mkfs")
        || dd_of(b)
        || has_word(b, b"shred")
        || has_word_then(b, b"find", b"-delete")
        || has_word_then(b, b"rsync", b"--delete")
        || fork_bomb(&lower)
        || has_word(b, b"shutdown")
        || has_word(b, b"reboot")
        || systemctl_sub(b, &["stop", "disable", "mask"])
        || verb_sub(b, &["oc", "kubectl"], &["delete"])
        || drop_sql(b)
        || redirect_dev(b)
}

fn is_mutating(scan: &str) -> bool {
    let lower = scan.to_ascii_lowercase();
    let b = lower.as_bytes();
    const VERBS: &[&[u8]] = &[
        b"rm",
        b"mv",
        b"cp",
        b"touch",
        b"mkdir",
        b"rmdir",
        b"chmod",
        b"chown",
        b"chattr",
        b"ln",
        b"kill",
        b"pkill",
        b"tee",
        b"truncate",
        b"unlink",
    ];
    if VERBS.iter().any(|w| has_word(b, w)) {
        return true;
    }
    if sed_i(b) {
        return true;
    }
    if bare_redirect(b) {
        return true;
    }
    if pkg_sub(b) {
        return true;
    }
    if systemctl_sub(b, &["start", "restart", "enable"]) {
        return true;
    }
    if verb_sub(
        b,
        &["oc", "kubectl"],
        &["apply", "create", "scale", "edit", "patch"],
    ) {
        return true;
    }
    if verb_sub(b, &["git"], &["push", "reset", "clean"]) {
        return true;
    }
    if verb_sub(b, &["npm"], &["publish"]) {
        return true;
    }
    false
}

fn rm_flag(b: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 <= b.len() {
        if boundary_before(b, i) && b[i..].starts_with(b"rm") && boundary_after(b, i + 2) {
            let mut j = i + 2;
            let ws_start = j;
            while j < b.len() && is_ws(b[j]) {
                j += 1;
            }
            if j > ws_start && j < b.len() && b[j] == b'-' {
                j += 1;
                let flag_start = j;
                while j < b.len() && b[j].is_ascii_lowercase() {
                    j += 1;
                }
                if j > flag_start {
                    let last = b[j - 1];
                    if last == b'r' || last == b'f' {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b)
}

fn dd_of(b: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 <= b.len() {
        if boundary_before(b, i) && b[i..].starts_with(b"dd") && boundary_after(b, i + 2) {
            let mut j = i + 2;
            while j < b.len() && b[j] != b'|' && b[j] != b';' && b[j] != b'&' {
                if j + 3 <= b.len() && b[j..].starts_with(b"of=") && boundary_before(b, j) {
                    return true;
                }
                j += 1;
            }
        }
        i += 1;
    }
    false
}

/// `\bWORD\b.*\sFLAG\b` on a single line (`.` does not cross newlines).
fn has_word_then(b: &[u8], word: &[u8], flag: &[u8]) -> bool {
    let mut i = 0;
    while i + word.len() <= b.len() {
        if boundary_before(b, i)
            && b[i..i + word.len()] == *word
            && boundary_after(b, i + word.len())
        {
            let rest_end = b[i + word.len()..]
                .iter()
                .position(|c| *c == b'\n')
                .map(|p| i + word.len() + p)
                .unwrap_or(b.len());
            let rest = &b[i + word.len()..rest_end];
            if let Some(pos) = find_sub(rest, flag) {
                let abs = i + word.len() + pos;
                // flag must be preceded by whitespace and be a word (boundary after)
                if abs > 0 && is_ws(b[abs - 1]) && boundary_after(b, abs + flag.len()) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn fork_bomb(s: &str) -> bool {
    // :()\s*{
    let b = s.as_bytes();
    let mut i = 0;
    while i + 3 < b.len() {
        if b[i] == b':' && b[i + 1] == b'(' && b[i + 2] == b')' {
            let mut j = i + 3;
            while j < b.len() && is_ws(b[j]) {
                j += 1;
            }
            if j < b.len() && b[j] == b'{' {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn systemctl_sub(b: &[u8], subs: &[&str]) -> bool {
    verb_sub(b, &["systemctl"], subs)
}

fn verb_sub(b: &[u8], verbs: &[&str], subs: &[&str]) -> bool {
    for verb in verbs {
        let vb = verb.as_bytes();
        let mut i = 0;
        while i + vb.len() <= b.len() {
            if boundary_before(b, i) && &b[i..i + vb.len()] == vb && boundary_after(b, i + vb.len())
            {
                let mut j = i + vb.len();
                let ws = j;
                while j < b.len() && is_ws(b[j]) {
                    j += 1;
                }
                if j > ws {
                    for sub in subs {
                        let sb = sub.as_bytes();
                        if j + sb.len() <= b.len()
                            && &b[j..j + sb.len()] == sb
                            && boundary_after(b, j + sb.len())
                        {
                            return true;
                        }
                    }
                }
            }
            i += 1;
        }
    }
    false
}

fn drop_sql(b: &[u8]) -> bool {
    verb_sub(b, &["drop"], &["database", "table"])
}

fn redirect_dev(b: &[u8]) -> bool {
    // >\s*/dev/(sd|nvme|hd|vd|mapper)
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'>' {
            let mut j = i + 1;
            while j < b.len() && is_ws(b[j]) {
                j += 1;
            }
            if b[j..].starts_with(b"/dev/") {
                let rest = &b[j + 5..];
                for p in [b"sd".as_slice(), b"nvme", b"hd", b"vd", b"mapper"] {
                    if rest.starts_with(p) {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

fn sed_i(b: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 <= b.len() {
        if boundary_before(b, i) && b[i..].starts_with(b"sed") && boundary_after(b, i + 3) {
            let mut j = i + 3;
            let ws = j;
            while j < b.len() && is_ws(b[j]) {
                j += 1;
            }
            if j > ws && b[j..].starts_with(b"-i") {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn bare_redirect(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'>' {
            let prev_ok = i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'&');
            if prev_ok {
                let mut j = i + 1;
                if j < b.len() && b[j] == b'>' {
                    j += 1;
                }
                let next_ok = j >= b.len() || b[j] != b'&';
                if next_ok {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn pkg_sub(b: &[u8]) -> bool {
    // apt-get is not a single word; match the literal with boundaries around the ends.
    let tools: &[&[u8]] = &[b"yum", b"dnf", b"apt-get", b"apt", b"pip", b"npm"];
    let subs: &[&[u8]] = &[b"install", b"remove", b"update", b"upgrade"];
    for tool in tools {
        let mut i = 0;
        while i + tool.len() <= b.len() {
            let end = i + tool.len();
            let before_ok = boundary_before(b, i);
            let after_ok = boundary_after(b, end);
            if before_ok && &b[i..end] == *tool && after_ok {
                let mut j = end;
                let ws = j;
                while j < b.len() && is_ws(b[j]) {
                    j += 1;
                }
                if j > ws {
                    for sub in subs {
                        if j + sub.len() <= b.len()
                            && &b[j..j + sub.len()] == *sub
                            && boundary_after(b, j + sub.len())
                        {
                            return true;
                        }
                    }
                }
            }
            i += 1;
        }
    }
    false
}

// ---- tool registration -------------------------------------------------------

pub const HOST_TOOLS: &[&str] = &[
    "ping",
    "get_host_context",
    "run_command",
    "list_dir",
    "read_file",
    "write_file",
];

pub const WORKSPACE_TOOLS: &[&str] = &["git_status", "git_diff", "search_terminals"];

pub const PREVIEW_TOOLS: &[&str] = &["preview_open", "preview_snapshot", "preview_comments"];

pub const HANDOFF_TOOLS: &[&str] = &["agent_list", "agent_delegate", "agent_message"];

pub const BROWSER_TOOLS: &[&str] = &[
    "browser_list",
    "browser_open",
    "browser_navigate",
    "browser_snapshot",
    "browser_click",
    "browser_type",
    "browser_fill",
    "browser_select",
    "browser_scroll",
    "browser_hover",
    "browser_wait",
    "browser_focus",
    "browser_press_key",
    "browser_screenshot",
    "browser_attach",
    "browser_detach",
    "browser_close",
];

/// Browser and preview registration defaults on; host tools are remote-only.
pub fn registered_tools(
    host_tools: bool,
    browser_enabled: bool,
    handoff_enabled: bool,
) -> Vec<&'static str> {
    let mut names = Vec::new();
    if host_tools {
        names.extend_from_slice(HOST_TOOLS);
    } else if handoff_enabled {
        names.extend_from_slice(HANDOFF_TOOLS);
    }
    if browser_enabled {
        names.extend_from_slice(BROWSER_TOOLS);
        names.extend_from_slice(PREVIEW_TOOLS);
    }
    names.extend_from_slice(WORKSPACE_TOOLS);
    names
}

pub struct ToolSchema {
    pub type_name: &'static str,
    pub properties: &'static [&'static str],
}

pub fn tool_input_schema(name: &str) -> ToolSchema {
    let properties: &'static [&'static str] = match name {
        "run_command" => &["command", "timeout_ms"],
        "list_dir" => &["path"],
        "read_file" => &["path"],
        "write_file" => &["path", "content"],
        "browser_open" | "browser_navigate" => &["url"],
        "git_status" | "git_diff" => &["cwd"],
        "search_terminals" => &["query"],
        "agent_delegate" => &["kind", "prompt"],
        "agent_message" => &["sessionId", "text"],
        _ => &[],
    };
    ToolSchema {
        type_name: "object",
        properties,
    }
}

pub fn cli_label(kind: &str) -> String {
    match kind {
        "devterm" => "DevTerm Agent".into(),
        "muse" => "Muse Code".into(),
        "cursor" => "Cursor".into(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => format!("{}{}", f.to_uppercase(), c.as_str()),
                None => String::new(),
            }
        }
    }
}

pub struct HandoffPromptInput<'a> {
    pub source_kind: &'a str,
    pub source_session_id: &'a str,
    pub session_id: &'a str,
    pub cwd: &'a str,
    pub kind: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub model_note: Option<&'a str>,
    pub prompt: &'a str,
}

pub fn build_agent_handoff_prompt(input: &HandoffPromptInput) -> String {
    let model_effort = [input.model.unwrap_or(""), input.effort.unwrap_or("")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    let mut lines = vec![
        "You were opened by DevTerm to take over a task from another local agent.".to_string(),
        String::new(),
        format!(
            "- Source: {} (session {})",
            cli_label(input.source_kind),
            input.source_session_id
        ),
        format!("- Your session: {}", input.session_id),
        format!("- Working directory: {}", input.cwd),
        format!("- Your CLI: {}", cli_label(input.kind)),
    ];
    if !model_effort.is_empty() {
        lines.push(format!("- Model / effort: {model_effort}"));
    }
    if let Some(note) = input.model_note {
        lines.push(format!("- {note}"));
    }
    lines.push(String::new());
    lines.push("Do the work in this directory. Do not open another agent, do not launch".into());
    lines.push("agent CLIs yourself, and do not read DevTerm config or bridge files.".into());
    lines.push(String::new());
    lines.push(format!(
        "When finished (or blocked), send ONE summary back with your `agent_message`"
    ));
    lines.push(format!(
        "tool to session {}, then stop and wait for the operator.",
        input.source_session_id
    ));
    lines.push(String::new());
    lines.push("## Task".into());
    lines.push(input.prompt.to_string());
    lines.join("\n")
}

pub struct DelegateResult {
    pub is_error: bool,
    pub text: String,
}

pub fn agent_delegate_error(message: &str) -> DelegateResult {
    DelegateResult {
        is_error: true,
        text: format!("agent_delegate failed: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(cmd: &str, mode: PolicyMode) -> PolicyVerdict {
        Policy::new(mode).evaluate_command(cmd)
    }

    #[test]
    fn destructive_ops() {
        let cases = [
            "rm -rf /",
            "rm -fr /tmp/x",
            "dd if=/dev/zero of=/dev/sda",
            "shred -u secrets.txt",
            "find . -delete",
            "rsync -avz --delete src/ dst/",
            "systemctl stop nginx",
            "systemctl disable nginx",
            "oc delete pod x",
            "kubectl delete namespace x",
            "drop database prod",
            "drop table users",
            "shutdown -h now",
            "reboot",
        ];
        for cmd in cases {
            let v = r(cmd, PolicyMode::ReadOnly);
            assert!(!v.allow, "expected block for {cmd}");
            assert!(v.reason.unwrap_or_default().contains("read-only"));
            let v = r(cmd, PolicyMode::Confirm);
            assert!(v.allow && v.need_confirm);
            let v = r(cmd, PolicyMode::Full);
            assert!(v.allow && !v.need_confirm);
        }
    }

    #[test]
    fn mutating_ops() {
        let cases = [
            "rm /tmp/x",
            "mv a b",
            "cp a b",
            "touch x",
            "mkdir new",
            "chmod 777 file",
            "chown root file",
            "sed -i s/x/y/ file",
            "tee file",
            "truncate -s 0 file",
            "unlink file",
            "yum install nginx",
            "apt-get update",
            "npm install",
            "pip install requests",
            "npm publish",
            "git push origin main",
            "git reset --hard",
            "git clean -fd",
            "systemctl start nginx",
            "oc apply -f x.yaml",
            "kubectl create deploy x --image=y",
            "echo hi > /etc/hosts",
            "echo hi >> file",
        ];
        for cmd in cases {
            let v = r(cmd, PolicyMode::ReadOnly);
            assert!(!v.allow, "expected block for {cmd}");
            let v = r(cmd, PolicyMode::Confirm);
            assert!(v.allow && v.need_confirm, "{cmd}");
            let v = r(cmd, PolicyMode::Full);
            assert!(v.allow && !v.need_confirm);
        }
    }

    #[test]
    fn benign_commands() {
        let benign = [
            "ls -la",
            "cat file.txt",
            "grep -r pattern .",
            "find . -name \"*.ts\"",
            "echo hello",
            "ps aux",
            "df -h",
            "uptime",
            "git status",
            "git log --oneline",
            "git diff",
            "kubectl get pods",
            "oc get pods",
        ];
        for cmd in benign {
            for mode in [PolicyMode::ReadOnly, PolicyMode::Confirm, PolicyMode::Full] {
                let v = r(cmd, mode);
                assert!(v.allow, "{cmd} blocked in {}", mode.as_str());
                assert!(!v.need_confirm, "{cmd} asked confirm");
            }
        }
    }

    #[test]
    fn heredoc_and_piped_rm() {
        let v = r(
            "cat <<EOF | dd of=/dev/sda\nstuff\nEOF",
            PolicyMode::ReadOnly,
        );
        assert!(!v.allow);
        let v = r("find . -name x | xargs rm -f", PolicyMode::ReadOnly);
        assert!(!v.allow);
    }

    #[test]
    fn kubectlized_is_not_destructive() {
        let v = r("kubectlized foo", PolicyMode::ReadOnly);
        assert!(v.allow);
    }

    #[test]
    fn evaluate_write_ladder() {
        assert!(!Policy::new(PolicyMode::ReadOnly).evaluate_write().allow);
        let c = Policy::new(PolicyMode::Confirm).evaluate_write();
        assert!(c.allow && c.need_confirm);
        let f = Policy::new(PolicyMode::Full).evaluate_write();
        assert!(f.allow && !f.need_confirm);
    }

    #[test]
    fn precheck_rules() {
        let p = Policy::new(PolicyMode::ReadOnly);
        let v = p.evaluate_command_precheck(Some(RuleOutcome::Allow), "rm -rf /");
        assert!(v.allow && !v.need_confirm);
        let p = Policy::new(PolicyMode::Full);
        let v = p.evaluate_command_precheck(Some(RuleOutcome::Deny), "ls");
        assert!(!v.allow);
        assert!(v.reason.unwrap().contains("denied by approval rule"));
        let v = p.evaluate_command_precheck(Some(RuleOutcome::Ask), "ls");
        assert!(v.allow && v.need_confirm);
        assert!(v.reason.unwrap().contains("approval rule"));
        let v = p.evaluate_command_precheck(None, "rm -rf /");
        assert!(v.allow && !v.need_confirm);
        assert_eq!(LAUNCH_POLICY_MODE, "full");
    }

    #[test]
    fn registers_host_tools_for_remote_sessions() {
        let names = registered_tools(true, false, false);
        for tool in HOST_TOOLS {
            assert!(names.contains(tool), "missing {tool}");
        }
        assert!(!names.iter().any(|n| n.starts_with("browser_")));
        for tool in ["git_status", "git_diff", "search_terminals"] {
            assert!(names.contains(&tool), "missing {tool}");
        }
        assert!(!names.iter().any(|n| *n == "git_commit" || *n == "git_push"));
    }

    #[test]
    fn skips_host_tools_locally_and_keeps_browser_when_enabled() {
        let names = registered_tools(false, true, false);
        for tool in HOST_TOOLS {
            assert!(!names.contains(tool), "unexpected host tool {tool}");
        }
        assert!(names.iter().any(|n| n.starts_with("browser_")));
        for tool in PREVIEW_TOOLS {
            assert!(names.contains(tool), "missing {tool}");
        }
        for tool in WORKSPACE_TOOLS {
            assert!(names.contains(tool));
        }
    }

    #[test]
    fn registers_local_handoff_only_when_enabled() {
        let names = registered_tools(false, false, true);
        for tool in HANDOFF_TOOLS {
            assert!(names.contains(tool), "missing {tool}");
        }
        assert!(registered_tools(false, false, false)
            .iter()
            .filter(|n| n.starts_with("agent_"))
            .collect::<Vec<_>>()
            .is_empty());
        assert!(registered_tools(true, false, true)
            .iter()
            .filter(|n| n.starts_with("agent_"))
            .collect::<Vec<_>>()
            .is_empty());
    }

    #[test]
    fn lists_host_and_browser_schemas() {
        let tools = registered_tools(true, false, false);
        for tool in HOST_TOOLS {
            assert!(tools.contains(tool));
        }
        let schema = tool_input_schema("run_command");
        assert_eq!(schema.type_name, "object");
        assert!(schema.properties.contains(&"command"));
        let tools = registered_tools(false, true, false);
        for expected in BROWSER_TOOLS {
            assert!(tools.contains(expected), "missing {expected}");
        }
        for tool in &tools {
            assert_eq!(tool_input_schema(tool).type_name, "object");
        }
        for name in ["preview_open", "git_status", "search_terminals"] {
            assert!(tools.contains(&name));
        }
    }

    #[test]
    fn handoff_prompt_preserves_the_raw_task() {
        let task = "Implement these files:\n- src/main/ipc/agent.ts\nKeep the task intact.";
        let prompt = build_agent_handoff_prompt(&HandoffPromptInput {
            source_kind: "grok",
            source_session_id: "local-source-1",
            session_id: "local-agent-abc123",
            cwd: "D:\\projects\\DevTerm",
            kind: "codex",
            model: Some("luna"),
            effort: Some("max"),
            model_note: None,
            prompt: task,
        });
        assert!(prompt.contains("Source: Grok (session local-source-1)"));
        assert!(prompt.contains("Your session: local-agent-abc123"));
        assert!(prompt.contains("Working directory: D:\\projects\\DevTerm"));
        assert!(prompt.contains("Your CLI: Codex"));
        assert!(prompt.contains("Model / effort: luna / max"));
        assert!(prompt.contains("send ONE summary back with your `agent_message`"));
        assert!(prompt.contains("to session local-source-1"));
        assert!(prompt.contains("Do not open another agent"));
        assert!(prompt.ends_with(task));
    }

    #[test]
    fn dropped_model_note_does_not_touch_the_task() {
        let prompt = build_agent_handoff_prompt(&HandoffPromptInput {
            source_kind: "grok",
            source_session_id: "local-source-1",
            session_id: "local-agent-xyz",
            cwd: "D:\\projects\\DevTerm",
            kind: "opencode",
            model: None,
            effort: None,
            model_note: Some(
                "Requested model \"muse spark 1.3 free\" is not a valid OpenCode model — starting with the operator default model.",
            ),
            prompt: "do the thing",
        });
        assert!(!prompt.contains("Model / effort"));
        assert!(prompt.contains("Requested model \"muse spark 1.3 free\""));
        assert!(prompt.ends_with("do the thing"));
    }

    #[test]
    fn delegate_cap_error() {
        let result = agent_delegate_error("Delegate cap reached");
        assert!(result.is_error);
        assert!(result.text.contains("Delegate cap reached"));
    }
}
