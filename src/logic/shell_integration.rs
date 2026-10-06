//! POSIX shell integration and detached tmux bootstrap from `manager.ts`
//! (`buildPosixShellIntegrationSetup`, `consumeShellIntegrationReady`,
//! `buildQuietStartCwd`) and `tmux.ts` (`buildDetachedSessionBootstrap`).
//!
//! The Windows PowerShell startup command (Set-Location + OSC 7 / OSC 133)
//! is built here too; `windows_host` owns the generic PowerShell wrappers.

/// Rows the POSIX inject must reclaim after writeQuiet.
pub const SHELL_INTEGRATION_RECLAIM_LINES: usize = 3;

/// How long shell output must stay quiet before OSC hooks are injected.
pub const SHELL_INTEGRATION_IDLE_MS: u64 = 450;

/// Cap waiting for MOTD idle — still inject so hooks eventually land.
pub const SHELL_INTEGRATION_MAX_WAIT_MS: u64 = 8000;

/// Failsafe that restores echo if the ready marker never comes back.
pub const ECHO_RESTORE_FAILSAFE_MS: u64 = 4000;

/// OSC the quiet inject prints once `stty echo` has run.
pub const SHELL_INTEGRATION_READY_MARK: &str = "\u{1b}]633;P;DevTermReady\u{7}";

/// Same marker inside a tmux DCS passthrough wrapper.
pub const SHELL_INTEGRATION_READY_MARK_TMUX: &str =
    "\u{1b}Ptmux;\u{1b}\u{1b}]633;P;DevTermReady\u{7}\u{1b}\\";

/// First half of a quiet inject: turn off PTY echo.
pub const STTY_DISABLE_ECHO: &str = "\u{15}stty -echo 2>/dev/null\n";

/// Failsafe so operator typing is visible if the inject never confirms.
pub const STTY_ENABLE_ECHO: &str =
    "\u{15}stty echo 2>/dev/null; printf \"\\033[1A\\r\\033[2K\"\n";

/// Wait for `stty -echo` to run before sending the payload on a slow SSH link.
pub const QUIET_WRITE_GAP_MS: u64 = 180;

const READY_MARKS: [&str; 2] = [
    SHELL_INTEGRATION_READY_MARK,
    SHELL_INTEGRATION_READY_MARK_TMUX,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellIntegrationReady {
    pub ready: bool,
    pub text: String,
    pub carry: String,
}

/// Drop a start directory that would split the one-line inject.
pub fn sanitize_start_cwd(cwd: Option<&str>) -> Option<String> {
    let cwd = cwd?;
    let trimmed = cwd.trim();
    if trimmed.is_empty() || trimmed.chars().any(|c| c == '\0' || c == '\r' || c == '\n') {
        return None;
    }
    Some(trimmed.to_string())
}

/// Single-quote a string for a POSIX shell.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

/// `cd` fragment for the quiet inject. Empty when there is nothing safe to enter.
pub fn quiet_posix_cd(cwd: Option<&str>) -> String {
    let Some(clean) = sanitize_start_cwd(cwd) else {
        return String::new();
    };
    format!("cd -- {} 2>/dev/null || true; ", sh_quote(&clean))
}

fn shell_integration_ready_command() -> String {
    "if [ -n \"${TMUX-}\" ]; then printf '\\033Ptmux;\\033\\033]633;P;DevTermReady\\007\\033\\\\'; else printf '\\033]633;P;DevTermReady\\007'; fi".to_string()
}

/// Echo-off `cd` for hosts that do not get the full POSIX hook script.
pub fn build_quiet_start_cwd(cwd: Option<&str>) -> String {
    let cd = quiet_posix_cd(cwd);
    if cd.is_empty() {
        return String::new();
    }
    format!(
        "{cd}stty echo 2>/dev/null; printf '\\033[{}A\\r\\033[J'; {}\n",
        SHELL_INTEGRATION_RECLAIM_LINES,
        shell_integration_ready_command()
    )
}

/// Pull ready markers out of a shell chunk.
///
/// A marker split across SSH reads stays in `carry` so it is neither shown
/// nor missed. Pass `flush` to release a partial carry (channel close).
pub fn consume_shell_integration_ready(carry: &str, chunk: &str) -> ShellIntegrationReady {
    consume_shell_integration_ready_flush(carry, chunk, false)
}

pub fn consume_shell_integration_ready_flush(
    carry: &str,
    chunk: &str,
    flush: bool,
) -> ShellIntegrationReady {
    let mut buf = format!("{carry}{chunk}");
    let mut ready = false;
    loop {
        let mut at: Option<usize> = None;
        let mut len = 0;
        for mark in READY_MARKS {
            if let Some(index) = buf.find(mark) {
                if at.map(|current| index < current).unwrap_or(true) {
                    at = Some(index);
                    len = mark.len();
                }
            }
        }
        let Some(at) = at else { break };
        ready = true;
        buf = format!("{}{}", &buf[..at], &buf[at + len..]);
    }
    if flush {
        return ShellIntegrationReady {
            ready,
            text: buf,
            carry: String::new(),
        };
    }
    let max_hold = READY_MARKS
        .iter()
        .map(|mark| mark.chars().count())
        .max()
        .unwrap_or(1)
        - 1;
    let total = buf.chars().count();
    let limit = max_hold.min(total);
    let mut hold = 0;
    for n in (1..=limit).rev() {
        let suffix = char_suffix(&buf, n);
        if READY_MARKS.iter().any(|mark| mark.starts_with(suffix)) {
            hold = n;
            break;
        }
    }
    if hold == 0 {
        return ShellIntegrationReady {
            ready,
            text: buf,
            carry: String::new(),
        };
    }
    let carry_text = char_suffix(&buf, hold).to_string();
    let text_chars = total - hold;
    let text: String = buf.chars().take(text_chars).collect();
    ShellIntegrationReady {
        ready,
        text,
        carry: carry_text,
    }
}

fn char_suffix(value: &str, n: usize) -> &str {
    let count = value.chars().count();
    if n >= count {
        return value;
    }
    let mut start = value.len();
    for (seen, (index, _)) in value.char_indices().rev().enumerate() {
        if seen + 1 == n {
            start = index;
            break;
        }
    }
    &value[start..]
}

/// One-liner injected into a remote POSIX shell so DevTerm can track cwd
/// (OSC 7) and command-input anchors (OSC 133 ;A/;B).
pub fn build_posix_shell_integration_setup(start_cwd: Option<&str>) -> String {
    let osc7_tmux =
        r#"printf '\033Ptmux;\033\033]7;file://%s%s\007\033\\' "${HOSTNAME:-h}" "$PWD""#;
    let osc7_plain = r#"printf '\033]7;file://%s%s\007' "${HOSTNAME:-h}" "$PWD""#;
    let osc133_a_tmux = r#"printf '\033Ptmux;\033\033]133;A\007\033\\'"#;
    let osc133_b_tmux = r#"printf '\033Ptmux;\033\033]133;B\007\033\\'"#;
    let osc133_a_plain = r#"printf '\033]133;A\007'"#;
    let osc133_b_plain = r#"printf '\033]133;B\007'"#;
    let cd = quiet_posix_cd(start_cwd);
    format!(
        "{cd}[ -n \"${{TMUX-}}\" ] && tmux set-option allow-passthrough on 2>/dev/null; \
__dt7() {{ if [ -n \"${{TMUX-}}\" ]; then {osc7_tmux}; else {osc7_plain}; fi; }}; \
if [ -n \"${{TMUX-}}\" ]; then __dtA=$({osc133_a_tmux}); __dtB=$({osc133_b_tmux}); else __dtA=$({osc133_a_plain}); __dtB=$({osc133_b_plain}); fi; \
if [ -n \"$ZSH_VERSION\" ]; then case \" ${{precmd_functions[*]}} \" in *\" __dt7 \"*) ;; *) precmd_functions+=(__dt7);; esac; \
if [ -n \"${{TMUX-}}\" ]; then case \"$PROMPT\" in *Ptmux*133*) ;; *) PROMPT=\"%{{$__dtA%}}$PROMPT%{{$__dtB%}}\";; esac; else case \"$PROMPT\" in *133*) ;; *) PROMPT=\"%{{$__dtA%}}$PROMPT%{{$__dtB%}}\";; esac; fi; else \
case \":$PROMPT_COMMAND:\" in *__dt7*) ;; *) PROMPT_COMMAND=\"__dt7${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\";; esac; \
if [ -n \"$BASH_VERSION\" ]; then case \"$PS1\" in *'${{__dtA}}'*) ;; *) PS1='\\[${{__dtA}}\\]'\"$PS1\"'\\[${{__dtB}}\\]';; esac; fi; fi; \
stty echo 2>/dev/null; printf '\\033[{reclaim}A\\r\\033[J'; {ready}\n",
        reclaim = SHELL_INTEGRATION_RECLAIM_LINES,
        ready = shell_integration_ready_command(),
    )
}

fn sanitize_tmux_name(raw: &str) -> String {
    let mut cleaned = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            cleaned.push(ch);
        } else {
            cleaned.push('-');
        }
    }
    let mut collapsed = String::new();
    let mut prev_dash = false;
    for ch in cleaned.chars() {
        if ch == '-' {
            if !prev_dash {
                collapsed.push('-');
            }
            prev_dash = true;
        } else {
            prev_dash = false;
            collapsed.push(ch);
        }
    }
    let trimmed = collapsed.trim_matches('-');
    trimmed.chars().take(48).collect()
}

fn default_tmux_name(session_id: &str) -> String {
    format!("devterm-{}", sanitize_tmux_name(session_id))
}

/// Create-or-reuse `devterm-<sessionId>` and attach as a child process.
/// Detach returns to the login shell because this never `exec`s tmux.
pub fn build_detached_session_bootstrap(session_id: &str) -> String {
    let name = default_tmux_name(session_id);
    let quoted = sh_quote(&name);
    format!(
        "tmux has-session -t {quoted} 2>/dev/null || tmux new-session -Ad -s {quoted}; \
tmux set-option -t {quoted} allow-passthrough on 2>/dev/null; \
tmux attach-session -t {quoted}; _dt_ec=$?; \
stty echo 2>/dev/null; \
if [ \"$_dt_ec\" -eq 0 ]; then \
printf '\\r\\n[DevTerm: detached from tmux; back in a normal shell]\\r\\n'; \
else printf '\\r\\n[DevTerm: tmux attach failed; staying in a normal shell]\\r\\n'; \
fi\n"
    )
}

fn ps_quote(value: &str) -> String {
    let mut out = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn to_windows_fs_for_start(path: &str) -> String {
    let trimmed = path.trim();
    if let Some((drive, rest)) = split_windows_drive(trimmed) {
        let mut normalized = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' || ch == '/' {
                if !normalized.ends_with('\\') {
                    normalized.push('\\');
                }
                while matches!(chars.peek(), Some('\\' | '/')) {
                    chars.next();
                }
            } else {
                normalized.push(ch);
            }
        }
        if normalized.is_empty() {
            format!("{drive}:\\")
        } else if normalized.starts_with('\\') {
            format!("{drive}:{normalized}")
        } else {
            format!("{drive}:\\{normalized}")
        }
    } else {
        path.to_string()
    }
}

fn split_windows_drive(path: &str) -> Option<(String, &str)> {
    let bytes = path.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && (bytes[3] == b'\\' || bytes[3] == b'/')
    {
        let mut i = 3;
        while i < bytes.len() && (bytes[i] == b'\\' || bytes[i] == b'/') {
            i += 1;
        }
        return Some((path[1..2].to_ascii_uppercase(), &path[i..]));
    }
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/'
    {
        return Some((path[1..2].to_ascii_uppercase(), &path[3..]));
    }
    if bytes.len() == 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() {
        return Some((path[1..2].to_ascii_uppercase(), ""));
    }
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        let mut i = 2;
        while i < bytes.len() && (bytes[i] == b'\\' || bytes[i] == b'/') {
            i += 1;
        }
        return Some((path[0..1].to_ascii_uppercase(), &path[i..]));
    }
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some((path[0..1].to_ascii_uppercase(), ""));
    }
    if bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
    {
        return Some((path[0..1].to_ascii_uppercase(), ""));
    }
    None
}

/// PowerShell `Set-Location` prefix. Empty when the path would break the
/// one-line startup script.
pub fn windows_start_location_prefix(start_cwd: Option<&str>) -> String {
    let Some(start_cwd) = start_cwd else {
        return String::new();
    };
    let trimmed = start_cwd.trim();
    if trimmed.is_empty() || trimmed.chars().any(|c| c == '\0' || c == '\r' || c == '\n') {
        return String::new();
    }
    format!(
        "Set-Location -LiteralPath {}; ",
        ps_quote(&to_windows_fs_for_start(trimmed))
    )
}

/// Interactive PowerShell prompt hook: OSC 133 A, OSC 7, prompt, OSC 133 B.
/// `start_cwd` is applied with `Set-Location` before the prompt function.
pub fn build_windows_powershell_prompt_setup(start_cwd: Option<&str>) -> String {
    format!(
        "{prefix}function prompt {{ $e=[char]27; $b=[char]7; $p=$PWD.ProviderPath; \
$u=($p -replace '\\\\','/'); \
Write-Host -NoNewline ($e + ']133;A' + $b + $e + ']7;file:///' + $u + $b); \
('PS ' + $p + '> ' + $e + ']133;B' + $b) }}",
        prefix = windows_start_location_prefix(start_cwd)
    )
}

fn encode_powershell_utf16_base64(script: &str) -> String {
    let mut bytes = Vec::new();
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    base64_encode(&bytes)
}

/// `windowsPowerShellInteractiveCommand` around the prompt hook.
/// Quote-safe `-Command` ScriptBlock, not `-EncodedCommand` (that emits CLIXML
/// on an interactive PTY).
pub fn build_windows_powershell_startup_command(start_cwd: Option<&str>) -> String {
    let script = build_windows_powershell_prompt_setup(start_cwd);
    let encoded = encode_powershell_utf16_base64(&script);
    format!(
        "powershell.exe -NoLogo -NoExit -Command \"[ScriptBlock]::Create([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{encoded}'))).Invoke()\""
    )
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        let n = ((data[index] as u32) << 16)
            | ((data[index + 1] as u32) << 8)
            | (data[index + 2] as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        index += 3;
    }
    if index < data.len() {
        let rem = data.len() - index;
        let mut n = (data[index] as u32) << 16;
        if rem == 2 {
            n |= (data[index + 1] as u32) << 8;
        }
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if rem == 2 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

fn contains_word_phrase(haystack: &str, phrase: &str) -> bool {
    let bytes = haystack.as_bytes();
    let needle = phrase.as_bytes();
    if needle.is_empty() || needle.len() > bytes.len() {
        return false;
    }
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if &bytes[index..index + needle.len()] == needle {
            let before_ok = index == 0 || !is_word(bytes[index - 1]);
            let after = index + needle.len();
            let after_ok = after == bytes.len() || !is_word(bytes[after]);
            if before_ok && after_ok {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_uses_a_stable_sanitized_tmux_session_name() {
        let script = build_detached_session_bootstrap("35148259-faae-4338-b3dc-0146a4b93a79");
        assert!(script.contains(
            "tmux new-session -Ad -s 'devterm-35148259-faae-4338-b3dc-0146a4b93a79'"
        ));
        assert!(script.contains(
            "tmux attach-session -t 'devterm-35148259-faae-4338-b3dc-0146a4b93a79'"
        ));
    }

    #[test]
    fn bootstrap_enables_allow_passthrough() {
        let script = build_detached_session_bootstrap("abc");
        assert!(script.contains("allow-passthrough on"));
        assert!(script.contains("set-option -t 'devterm-abc' allow-passthrough on"));
    }

    #[test]
    fn bootstrap_does_not_exec_tmux() {
        let script = build_detached_session_bootstrap("abc");
        assert!(!contains_word_phrase(&script, "exec tmux"));
        assert!(script.contains("detached from tmux"));
    }

    #[test]
    fn bootstrap_sanitizes_unsafe_characters() {
        let script = build_detached_session_bootstrap("sess/with spaces!and*junk");
        assert!(script.contains("-s 'devterm-sess-with-spaces-and-junk'"));
        assert!(!script.contains("sess/with"));
    }

    #[test]
    fn posix_setup_installs_dt7_and_osc_markers() {
        let script = build_posix_shell_integration_setup(None);
        assert!(script.contains("__dt7()"));
        assert!(script.contains("PROMPT_COMMAND="));
        assert!(script.contains("precmd_functions+=(__dt7)"));
        assert!(script.contains("]7;file://"));
        assert!(script.contains("]133;A"));
        assert!(script.contains("]133;B"));
    }

    #[test]
    fn posix_setup_wraps_osc_in_tmux_dcs_when_tmux_is_set() {
        let script = build_posix_shell_integration_setup(None);
        assert!(script.contains("[ -n \"${TMUX-}\" ] && tmux set-option allow-passthrough on"));
        assert!(script.contains("\\033Ptmux;\\033\\033]7;file://"));
        assert!(script.contains("\\033Ptmux;\\033\\033]133;A"));
        assert!(script.contains("\\033Ptmux;\\033\\033]133;B"));
        assert!(script.contains("else printf '\\033]7;file://"));
    }

    #[test]
    fn posix_setup_defers_bash_prompt_marker_vars() {
        let script = build_posix_shell_integration_setup(None);
        assert!(script.contains("PS1='\\[${__dtA}\\]'\"$PS1\"'\\[${__dtB}\\]'"));
        assert!(!script.contains("PS1=\"\\[$__dtA\\]"));
    }

    #[test]
    fn posix_setup_does_not_clear_the_screen() {
        let script = build_posix_shell_integration_setup(None);
        assert!(!contains_word_phrase(&script, "clear"));
        assert!(script.contains("stty echo"));
    }

    #[test]
    fn posix_setup_reclaims_leftover_inject_rows() {
        let script = build_posix_shell_integration_setup(None);
        assert_eq!(SHELL_INTEGRATION_RECLAIM_LINES, 3);
        assert!(script.contains("printf '\\033[3A\\r\\033[J'"));
        assert!(!script.contains("stty echo 2>/dev/null; __dt7"));
    }

    #[test]
    fn exports_motd_idle_timing() {
        assert!(SHELL_INTEGRATION_IDLE_MS >= 300);
        assert_eq!(SHELL_INTEGRATION_IDLE_MS, 450);
        assert!(SHELL_INTEGRATION_MAX_WAIT_MS >= SHELL_INTEGRATION_IDLE_MS * 4);
        assert_eq!(SHELL_INTEGRATION_MAX_WAIT_MS, 8000);
        assert!(ECHO_RESTORE_FAILSAFE_MS >= 3000);
        assert_eq!(ECHO_RESTORE_FAILSAFE_MS, 4000);
        assert!(STTY_ENABLE_ECHO.contains("stty echo 2>/dev/null"));
        assert!(STTY_ENABLE_ECHO.contains("\\033[1A\\r\\033[2K"));
        assert_eq!(QUIET_WRITE_GAP_MS, 180);
        assert!(STTY_DISABLE_ECHO.contains("stty -echo"));
    }

    #[test]
    fn prints_a_ready_marker_only_after_echo_is_restored() {
        let script = build_posix_shell_integration_setup(None);
        let echo_at = script.rfind("stty echo").unwrap();
        let mark_at = script.find("633;P;DevTermReady").unwrap();
        assert!(mark_at > echo_at);
        assert!(script.contains("printf '\\033]633;P;DevTermReady\\007'"));
        assert!(script.contains(
            "printf '\\033Ptmux;\\033\\033]633;P;DevTermReady\\007\\033\\\\'"
        ));
        assert!(!script.contains("cd --"));
    }

    #[test]
    fn changes_to_the_start_directory_inside_the_quiet_inject() {
        let script = build_posix_shell_integration_setup(Some("/root"));
        assert!(script.starts_with("cd -- '/root' 2>/dev/null || true; "));
        let evil = build_posix_shell_integration_setup(Some("/tmp/$(reboot)"));
        assert!(evil.starts_with("cd -- '/tmp/$(reboot)' 2>/dev/null || true; "));
        assert!(evil.contains("$(reboot)"));
        let injected = build_posix_shell_integration_setup(Some("/tmp\nrm -rf /"));
        assert!(!injected.starts_with("cd "));
        assert!(!injected.contains("rm -rf"));
    }

    #[test]
    fn quiet_start_cwd_cds_without_installing_prompt_hooks() {
        let script = build_quiet_start_cwd(Some("/root"));
        assert!(script.starts_with("cd -- '/root' 2>/dev/null || true; "));
        assert!(!script.contains("__dt7"));
        assert!(script.contains("633;P;DevTermReady"));
        assert_eq!(build_quiet_start_cwd(Some("/tmp\nid")), "");
    }

    #[test]
    fn consume_strips_the_ready_marker() {
        let out = consume_shell_integration_ready(
            "",
            &format!("banner{SHELL_INTEGRATION_READY_MARK}[root@host ~]# "),
        );
        assert!(out.ready);
        assert_eq!(out.text, "banner[root@host ~]# ");
        assert_eq!(out.carry, "");
    }

    #[test]
    fn consume_strips_the_tmux_dcs_form() {
        let out = consume_shell_integration_ready(
            "",
            &format!("x{SHELL_INTEGRATION_READY_MARK_TMUX}y"),
        );
        assert!(out.ready);
        assert_eq!(out.text, "xy");
    }

    #[test]
    fn consume_holds_a_marker_split_across_chunks() {
        let mark = SHELL_INTEGRATION_READY_MARK;
        let mid = 6;
        let prefix: String = mark.chars().take(mid).collect();
        let suffix: String = mark.chars().skip(mid).collect();
        let first = consume_shell_integration_ready("", &format!("prompt{prefix}"));
        assert!(!first.ready);
        assert_eq!(first.text, "prompt");
        assert_eq!(first.carry, prefix);
        let second = consume_shell_integration_ready(&first.carry, &format!("{suffix}# "));
        assert!(second.ready);
        assert_eq!(second.text, "# ");
        assert_eq!(second.carry, "");
    }

    #[test]
    fn consume_releases_a_held_escape_that_is_not_the_ready_marker() {
        let held = consume_shell_integration_ready("", "hi\u{1b}");
        assert_eq!(held.text, "hi");
        assert_eq!(held.carry, "\u{1b}");
        let next = consume_shell_integration_ready(&held.carry, "[31mOK");
        assert!(!next.ready);
        assert_eq!(next.text, "\u{1b}[31mOK");
        assert_eq!(next.carry, "");
    }

    #[test]
    fn consume_does_not_swallow_a_normal_osc_133_prompt() {
        let prompt = "[root@newaiops ~]# \u{1b}]133;B\u{7}";
        let out = consume_shell_integration_ready("", prompt);
        assert!(!out.ready);
        assert_eq!(out.text, prompt);
        assert_eq!(out.carry, "");
    }

    #[test]
    fn flush_releases_a_partial_marker() {
        let mark = SHELL_INTEGRATION_READY_MARK;
        let prefix: String = mark.chars().take(4).collect();
        let out = consume_shell_integration_ready_flush("", &prefix, true);
        assert!(!out.ready);
        assert_eq!(out.text, prefix);
        assert_eq!(out.carry, "");
    }

    #[test]
    fn windows_prompt_sets_location_and_emits_osc_7_and_133() {
        let script = build_windows_powershell_prompt_setup(Some(r"C:\Users\root"));
        assert!(script.starts_with("Set-Location -LiteralPath 'C:\\Users\\root'; "));
        assert!(script.contains("]133;A"));
        assert!(script.contains("]133;B"));
        assert!(script.contains("]7;file:///"));
        assert!(script.contains("-replace '\\\\','/'"));
        let quoted = build_windows_powershell_prompt_setup(Some("C:\\Users\\O'Brien"));
        assert!(quoted.contains("Set-Location -LiteralPath 'C:\\Users\\O''Brien'; "));
        let broken = build_windows_powershell_prompt_setup(Some("C:\\Users\nroot"));
        assert!(!broken.contains("Set-Location"));
        assert!(broken.starts_with("function prompt"));
    }

    #[test]
    fn windows_startup_command_is_a_quote_safe_interactive_script_block() {
        let command = build_windows_powershell_startup_command(None);
        assert!(command.starts_with("powershell.exe -NoLogo -NoExit -Command "));
        assert!(!command.contains("-EncodedCommand"));
        let b64 = command
            .split("FromBase64String('")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        let bytes = base64_decode(b64);
        let script = utf16le_to_string(&bytes);
        assert_eq!(script, build_windows_powershell_prompt_setup(None));
        assert!(script.contains("]7;file:///"));
    }

    fn base64_decode(input: &str) -> Vec<u8> {
        fn val(c: u8) -> u8 {
            match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => 0,
            }
        }
        let bytes: Vec<u8> = input.bytes().filter(|b| *b != b'=').collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i + 4 <= bytes.len() {
            let n = ((val(bytes[i]) as u32) << 18)
                | ((val(bytes[i + 1]) as u32) << 12)
                | ((val(bytes[i + 2]) as u32) << 6)
                | (val(bytes[i + 3]) as u32);
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
            out.push(n as u8);
            i += 4;
        }
        if i < bytes.len() {
            let rem = &bytes[i..];
            let mut n = (val(rem[0]) as u32) << 18;
            if rem.len() > 1 {
                n |= (val(rem[1]) as u32) << 12;
            }
            out.push((n >> 16) as u8);
            if rem.len() > 2 {
                n |= (val(rem[2]) as u32) << 6;
                out.push((n >> 8) as u8);
            }
        }
        out
    }

    fn utf16le_to_string(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk.get(1).copied().unwrap_or(0)]))
            .collect();
        String::from_utf16_lossy(&units)
    }
}
