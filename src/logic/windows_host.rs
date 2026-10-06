//! Windows OpenSSH helpers from `src/main/ssh/windows-host.ts`, plus the
//! second-channel policy from `SSHManager`.
//!
//! Win32-OpenSSH often resets the whole transport when a second channel opens
//! on the interactive connection. Command, SFTP, and port-forward traffic use
//! dedicated auxiliary clients (legacy KEX allowed). The shell channel stays
//! alone. A dropped PowerShell channel is recovered in place; the SSH
//! connection is ended only when that recovery itself fails.

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsDrivePath {
    pub drive: String,
    pub rest: String,
}

/// Low-latency KEX order for auxiliary clients once the target is known to be Windows.
/// Group1 and group-exchange/SHA-1 stay disabled.
pub const LEGACY_WINDOWS_KEX: &[&str] = &[
    "curve25519-sha256@libssh.org",
    "curve25519-sha256",
    "ecdh-sha2-nistp256",
    "ecdh-sha2-nistp384",
    "ecdh-sha2-nistp521",
    "diffie-hellman-group14-sha256",
    "diffie-hellman-group15-sha512",
    "diffie-hellman-group16-sha512",
    "diffie-hellman-group17-sha512",
    "diffie-hellman-group18-sha512",
    "diffie-hellman-group14-sha1",
    "diffie-hellman-group-exchange-sha256",
];

/// Idle time before an unused Windows forwarding transport is closed.
pub const WINDOWS_FORWARD_IDLE_MS: u64 = 30_000;

pub const SHELL_RECOVERY_WINDOW_MS: u64 = 60_000;
pub const SHELL_RECOVERY_DELAYS_MS: [u64; 3] = [250, 1000, 3000];

pub const WINDOWS_SHELL_RECOVERY_STOPPED: &str =
    "\r\n\u{1b}[31m[Windows shell repeatedly closed; automatic recovery stopped]\u{1b}[0m\r\n";

pub const POWERSHELL_CHANNEL_NOT_OPENED: &str = "PowerShell exec channel was not opened";

pub fn parse_windows_remote_path(path: &str) -> Option<WindowsDrivePath> {
    let text = path.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(parsed) = match_sftp_drive_colon(text) {
        return Some(parsed);
    }
    if let Some(parsed) = match_sftp_drive_slash(text) {
        return Some(parsed);
    }
    if let Some(parsed) = match_sftp_drive_only(text) {
        return Some(parsed);
    }
    if let Some(parsed) = match_fs_drive(text) {
        return Some(parsed);
    }
    match_fs_drive_only(text)
}

fn drive_of(text: &str, index: usize) -> String {
    text[index..index + 1].to_ascii_uppercase()
}

fn match_sftp_drive_colon(text: &str) -> Option<WindowsDrivePath> {
    let bytes = text.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && (bytes[3] == b'\\' || bytes[3] == b'/')
    {
        let mut index = 3;
        while index < bytes.len() && (bytes[index] == b'\\' || bytes[index] == b'/') {
            index += 1;
        }
        return Some(WindowsDrivePath {
            drive: drive_of(text, 1),
            rest: text[index..].to_string(),
        });
    }
    None
}

fn match_sftp_drive_slash(text: &str) -> Option<WindowsDrivePath> {
    let bytes = text.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
        return Some(WindowsDrivePath {
            drive: drive_of(text, 1),
            rest: text[3..].to_string(),
        });
    }
    None
}

fn match_sftp_drive_only(text: &str) -> Option<WindowsDrivePath> {
    let bytes = text.as_bytes();
    if bytes.len() == 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() {
        return Some(WindowsDrivePath {
            drive: drive_of(text, 1),
            rest: String::new(),
        });
    }
    None
}

fn match_fs_drive(text: &str) -> Option<WindowsDrivePath> {
    let bytes = text.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        let mut index = 2;
        while index < bytes.len() && (bytes[index] == b'\\' || bytes[index] == b'/') {
            index += 1;
        }
        return Some(WindowsDrivePath {
            drive: drive_of(text, 0),
            rest: text[index..].to_string(),
        });
    }
    None
}

fn match_fs_drive_only(text: &str) -> Option<WindowsDrivePath> {
    let bytes = text.as_bytes();
    if (bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || (bytes.len() == 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\')
    {
        return Some(WindowsDrivePath {
            drive: drive_of(text, 0),
            rest: String::new(),
        });
    }
    None
}

pub fn is_windows_remote_path(path: &str) -> bool {
    parse_windows_remote_path(path).is_some()
}

/// Convert to a native Windows path for PowerShell Set-Location.
pub fn to_windows_fs_path(path: &str) -> String {
    let Some(parsed) = parse_windows_remote_path(path) else {
        return path.to_string();
    };
    let rest = collapse_separators(&parsed.rest);
    if rest.is_empty() {
        format!("{}:\\", parsed.drive)
    } else {
        format!("{}:\\{rest}", parsed.drive)
    }
}

fn collapse_separators(rest: &str) -> String {
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' || ch == '/' {
            out.push('\\');
            while matches!(chars.peek(), Some('\\' | '/')) {
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Convert to Win32-OpenSSH SFTP form (`/C/Users/...`).
pub fn to_windows_sftp_path(path: &str) -> String {
    let Some(parsed) = parse_windows_remote_path(path) else {
        return posix_normalize(&path.replace('\\', "/"));
    };
    let rest = parsed.rest.replace('\\', "/");
    if rest.is_empty() {
        posix_normalize(&format!("/{}", parsed.drive))
    } else {
        posix_normalize(&format!("/{}/{}", parsed.drive, rest))
    }
}

pub fn posix_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let is_absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if !parts.is_empty() && parts.last() != Some(&"..") {
                parts.pop();
            } else if !is_absolute {
                parts.push("..");
            }
        } else {
            parts.push(segment);
        }
    }
    let mut result = parts.join("/");
    if is_absolute {
        result.insert(0, '/');
    }
    if trailing && result != "/" && !result.ends_with('/') {
        result.push('/');
    }
    if result.is_empty() {
        ".".into()
    } else {
        result
    }
}

/// Resolve a possibly-relative path against a Windows cwd.
pub fn resolve_windows_relative(cwd: Option<&str>, path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "." {
        return cwd.unwrap_or(trimmed).to_string();
    }
    if is_windows_remote_path(trimmed) {
        return trimmed.to_string();
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return trimmed.to_string();
    }
    let Some(cwd) = cwd else {
        return trimmed.to_string();
    };
    let mut rel = trimmed.to_string();
    if rel.starts_with('/') || rel.starts_with('\\') {
        rel = rel.trim_start_matches(['/', '\\']).to_string();
    }
    if let Some(stripped) = rel.strip_prefix("./") {
        rel = stripped.to_string();
    }
    let fs_cwd = to_windows_fs_path(cwd).trim_end_matches('\\').to_string();
    format!("{fs_cwd}\\{}", rel.replace('/', "\\"))
}

/// PowerShell single-argument escape: double embedded single quotes.
pub fn ps_quote(value: &str) -> String {
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

pub fn encode_powershell_script(script: &str) -> String {
    let mut bytes = Vec::new();
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    base64_encode(&bytes)
}

pub fn powershell_encoded_command(script: &str) -> String {
    format!(
        "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}",
        encode_powershell_script(script)
    )
}

pub fn powershell_command(script: &str) -> String {
    let encoded = encode_powershell_script(script);
    format!(
        "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \"[ScriptBlock]::Create([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{encoded}'))).Invoke()\""
    )
}

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
        ps_quote(&to_windows_fs_path(trimmed))
    )
}

pub fn windows_powershell_interactive_command(script: &str) -> String {
    let encoded = encode_powershell_script(script);
    format!(
        "powershell.exe -NoLogo -NoExit -Command \"[ScriptBlock]::Create([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{encoded}'))).Invoke()\""
    )
}

/// xterm Backspace (DEL `0x7f`) becomes BS (`0x08`) for Windows PTYs.
pub fn normalize_windows_interactive_input(data: &str) -> String {
    data.replace('\u{7f}', "\u{8}")
}

pub fn is_already_windows_wrapped(command: &str) -> bool {
    contains_powershell_exe(command)
        && (contains_ci(command, "EncodedCommand") || has_powershell_script_block(command))
}

pub fn wrap_windows_remote_command(command: &str, cwd: Option<&str>) -> String {
    if is_already_windows_wrapped(command) {
        return command.to_string();
    }
    let mut parts = vec!["$ErrorActionPreference = \"Continue\"".to_string()];
    if let Some(cwd) = cwd {
        parts.push(format!(
            "Set-Location -LiteralPath {}",
            ps_quote(&to_windows_fs_path(cwd))
        ));
    }
    parts.push(command.to_string());
    parts.push("if ($null -ne $LASTEXITCODE) { exit $LASTEXITCODE }".to_string());
    powershell_command(&parts.join("; "))
}

pub fn wrap_windows_git_command(cwd: &str, git_args_quoted: &str) -> String {
    let script = format!(
        "Set-Location -LiteralPath {}; git {}; exit $LASTEXITCODE",
        ps_quote(&to_windows_fs_path(cwd)),
        git_args_quoted
    );
    powershell_command(&script)
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn contains_powershell_exe(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let needle = b"powershell.exe";
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if &bytes[index..index + needle.len()] == needle {
            let before = index == 0 || !is_word_byte(bytes[index - 1]);
            let after = index + needle.len();
            let after_ok = after == bytes.len() || !is_word_byte(bytes[after]);
            if before && after_ok {
                return true;
            }
        }
        index += 1;
    }
    false
}

/// Faithful to `/\b-Command\b[\s\S]*FromBase64String\('/i`.
/// The leading word-boundary only matches when `-Command` is glued to a word character.
fn has_powershell_script_block(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let needle = b"-command";
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if &bytes[index..index + needle.len()] == needle {
            let before_boundary = index > 0 && is_word_byte(bytes[index - 1]);
            let after = index + needle.len();
            let after_boundary = after == bytes.len() || !is_word_byte(bytes[after]);
            if before_boundary && after_boundary && lower[after..].contains("frombase64string('") {
                return true;
            }
        }
        index += 1;
    }
    false
}

pub fn normalize_remote_path(path: &str) -> String {
    if is_windows_remote_path(path) {
        to_windows_sftp_path(path)
    } else {
        posix_normalize(&path.replace('\\', "/"))
    }
}

/// `sftp.realpath('.')`, then the Windows/POSIX normalizer used by `sftpHome`.
pub fn sftp_home_from_realpath<E>(
    realpath: impl FnOnce(&str) -> Result<String, E>,
) -> Result<String, E> {
    realpath(".").map(|path| normalize_remote_path(&path))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxPurpose {
    Exec,
    Sftp,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuxClientDecision {
    pub use_dedicated_client: bool,
    pub prefer_legacy_windows_kex: bool,
    /// Auxiliary failures belong to the operation, not the visible shell.
    pub isolate_status_from_shell: bool,
}

/// Windows remotes never open a second channel on the shell transport.
pub fn auxiliary_client_decision(os: &str, _purpose: AuxPurpose) -> AuxClientDecision {
    if os == "windows" {
        AuxClientDecision {
            use_dedicated_client: true,
            prefer_legacy_windows_kex: true,
            isolate_status_from_shell: true,
        }
    } else {
        AuxClientDecision {
            use_dedicated_client: false,
            prefer_legacy_windows_kex: false,
            isolate_status_from_shell: false,
        }
    }
}

pub fn opens_on_shell_transport(os: &str, purpose: &str) -> bool {
    if os == "windows" {
        purpose == "shell"
    } else {
        true
    }
}

pub fn second_channel_resets_shell_transport(os: &str) -> bool {
    os == "windows"
}

/// Windows command channels are serialized (MaxSessions is often 1).
pub fn windows_exec_is_serialized(os: &str) -> bool {
    os == "windows"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellLaunch {
    PtyShell,
    /// Interactive PowerShell exec, falling back to `cmd.exe`. Never `client.shell()`.
    WindowsExec {
        fallback_cmd_exe: bool,
    },
}

pub fn shell_launch(os: &str) -> ShellLaunch {
    if os == "windows" {
        ShellLaunch::WindowsExec {
            fallback_cmd_exe: true,
        }
    } else {
        ShellLaunch::PtyShell
    }
}

pub fn windows_shell_open_error(powershell_message: &str, fallback_error: Option<&str>) -> String {
    match fallback_error {
        Some(message) => message.to_string(),
        None => format!("Windows shell could not be opened: {powershell_message}"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellSessionFlags {
    pub os: String,
    pub closing: bool,
    pub reconnecting: bool,
    pub client_is_current: bool,
    pub session_is_current: bool,
    pub tmux_client_running: bool,
    pub shell_open: bool,
    pub received_exit: bool,
    pub has_shell_request: bool,
    pub recovery_timer_pending: bool,
    pub recovery_attempts: u32,
    pub recovery_window_started_at: Option<u64>,
}

impl Default for ShellSessionFlags {
    fn default() -> Self {
        Self {
            os: "windows".into(),
            closing: false,
            reconnecting: false,
            client_is_current: true,
            session_is_current: true,
            tmux_client_running: false,
            shell_open: false,
            received_exit: false,
            has_shell_request: true,
            recovery_timer_pending: false,
            recovery_attempts: 0,
            recovery_window_started_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellCloseOutcome {
    Ignored,
    ResumeAfterTmux,
    ReportExit,
    ScheduleRecovery { attempt: u32, delay_ms: u64 },
    StopRecovery { message: String },
}

impl ShellCloseOutcome {
    pub fn drops_connection(&self) -> bool {
        false
    }

    pub fn reports_exit(&self) -> bool {
        matches!(self, Self::ReportExit | Self::StopRecovery { .. })
    }
}

pub fn on_shell_channel_close(session: &mut ShellSessionFlags, now_ms: u64) -> ShellCloseOutcome {
    if !session.session_is_current || !session.client_is_current || session.closing {
        return ShellCloseOutcome::Ignored;
    }
    if session.tmux_client_running
        && session.client_is_current
        && !session.closing
        && !session.reconnecting
    {
        return ShellCloseOutcome::ResumeAfterTmux;
    }
    if session.os == "windows" && !session.received_exit {
        return schedule_windows_shell_recovery(session, now_ms);
    }
    ShellCloseOutcome::ReportExit
}

fn schedule_windows_shell_recovery(
    session: &mut ShellSessionFlags,
    now_ms: u64,
) -> ShellCloseOutcome {
    if session.recovery_timer_pending
        || session.closing
        || session.reconnecting
        || !session.client_is_current
    {
        return ShellCloseOutcome::Ignored;
    }
    if !session.has_shell_request {
        return ShellCloseOutcome::ReportExit;
    }
    let reset = session
        .recovery_window_started_at
        .map(|started| now_ms.saturating_sub(started) > SHELL_RECOVERY_WINDOW_MS)
        .unwrap_or(true);
    if reset {
        session.recovery_window_started_at = Some(now_ms);
        session.recovery_attempts = 0;
    }
    let attempt = session.recovery_attempts + 1;
    session.recovery_attempts = attempt;
    if attempt > 3 {
        return ShellCloseOutcome::StopRecovery {
            message: WINDOWS_SHELL_RECOVERY_STOPPED.to_string(),
        };
    }
    session.recovery_timer_pending = true;
    ShellCloseOutcome::ScheduleRecovery {
        attempt,
        delay_ms: SHELL_RECOVERY_DELAYS_MS[(attempt - 1) as usize],
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryTimerOutcome {
    Aborted,
    Reopen { message: String, attempt: u32 },
}

pub fn on_shell_recovery_timer(
    session: &mut ShellSessionFlags,
    attempt: u32,
) -> RecoveryTimerOutcome {
    session.recovery_timer_pending = false;
    if !session.session_is_current
        || session.closing
        || session.reconnecting
        || !session.client_is_current
        || session.shell_open
    {
        return RecoveryTimerOutcome::Aborted;
    }
    RecoveryTimerOutcome::Reopen {
        attempt,
        message: format!(
            "\r\n\u{1b}[90m[DevTerm: Windows shell channel closed; recovering ({attempt}/3)\u{2026}]\u{1b}[0m\r\n"
        ),
    }
}

/// When the replacement shell cannot be opened, end the client so the normal
/// transport-close path can reconnect. Returns the banner, or `None` when the
/// session is no longer the current one.
pub fn on_shell_recovery_open_failed(session: &ShellSessionFlags, err: &str) -> Option<String> {
    if !session.session_is_current
        || session.closing
        || session.reconnecting
        || !session.client_is_current
    {
        return None;
    }
    Some(format!(
        "\r\n\u{1b}[31m[Windows shell recovery failed: {err}; reconnecting SSH]\u{1b}[0m\r\n"
    ))
}

pub fn recovery_open_failure_ends_transport() -> bool {
    true
}

#[derive(Clone, Debug)]
pub struct ForwardPool {
    open: bool,
    generation: u64,
    refs: u32,
    idle_deadline: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardLease {
    generation: u64,
    active: bool,
}

impl ForwardPool {
    pub fn new() -> Self {
        Self {
            open: true,
            generation: 1,
            refs: 0,
            idle_deadline: None,
        }
    }

    pub fn refs(&self) -> u32 {
        self.refs
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn idle_deadline(&self) -> Option<u64> {
        self.idle_deadline
    }

    pub fn acquire(&mut self, _now: u64) -> Option<ForwardLease> {
        if !self.open {
            return None;
        }
        self.idle_deadline = None;
        self.refs += 1;
        Some(ForwardLease {
            generation: self.generation,
            active: true,
        })
    }

    pub fn release(&mut self, lease: &mut ForwardLease, now: u64) {
        if !lease.active {
            return;
        }
        lease.active = false;
        if lease.generation != self.generation || !self.open {
            return;
        }
        self.refs = self.refs.saturating_sub(1);
        if self.refs > 0 || self.idle_deadline.is_some() {
            return;
        }
        self.idle_deadline = Some(now.saturating_add(WINDOWS_FORWARD_IDLE_MS));
    }

    /// Returns true when the idle timer closed the dedicated transport.
    pub fn poll(&mut self, now: u64) -> bool {
        if let Some(deadline) = self.idle_deadline {
            if self.refs == 0 && now >= deadline {
                self.close();
                return true;
            }
        }
        false
    }

    pub fn close(&mut self) {
        self.open = false;
        self.generation = self.generation.wrapping_add(1);
        self.refs = 0;
        self.idle_deadline = None;
    }
}

impl Default for ForwardPool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardClientChoice {
    Unavailable,
    Primary,
    DedicatedWindows,
}

pub fn forward_client_choice(
    os: &str,
    has_primary: bool,
    closing: bool,
    reconnecting: bool,
) -> ForwardClientChoice {
    if !has_primary || closing || reconnecting {
        return ForwardClientChoice::Unavailable;
    }
    if os == "windows" {
        ForwardClientChoice::DedicatedWindows
    } else {
        ForwardClientChoice::Primary
    }
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
        let remaining = data.len() - index;
        let mut n = (data[index] as u32) << 16;
        if remaining == 2 {
            n |= (data[index + 1] as u32) << 8;
        }
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if remaining == 2 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

pub fn base64_decode(input: &str) -> Vec<u8> {
    fn val(byte: u8) -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let bytes: Vec<u8> = input.bytes().filter(|byte| *byte != b'=').collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        let n = ((val(bytes[index]) as u32) << 18)
            | ((val(bytes[index + 1]) as u32) << 12)
            | ((val(bytes[index + 2]) as u32) << 6)
            | (val(bytes[index + 3]) as u32);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
        index += 4;
    }
    if index < bytes.len() {
        let rest = &bytes[index..];
        let mut n = (val(rest[0]) as u32) << 18;
        if rest.len() > 1 {
            n |= (val(rest[1]) as u32) << 12;
        }
        out.push((n >> 16) as u8);
        if rest.len() > 2 {
            n |= (val(rest[2]) as u32) << 6;
            out.push((n >> 8) as u8);
        }
    }
    out
}

pub fn utf16le_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk.get(1).copied().unwrap_or(0)]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Silence an unused-import warning if a caller only needs path conversion.
pub fn path_is_absolute_hint(path: &str) -> bool {
    Path::new(path).is_absolute() || is_windows_remote_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_script(wrapped: &str) -> String {
        let b64 = wrapped
            .split("FromBase64String('")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        utf16le_to_string(&base64_decode(b64))
    }

    #[test]
    fn parses_drive_sftp_and_mixed_slash_forms() {
        assert_eq!(
            parse_windows_remote_path(r"C:\Users\x"),
            Some(WindowsDrivePath {
                drive: "C".into(),
                rest: r"Users\x".into(),
            })
        );
        assert_eq!(
            parse_windows_remote_path("C:/Users/x"),
            Some(WindowsDrivePath {
                drive: "C".into(),
                rest: "Users/x".into(),
            })
        );
        assert_eq!(
            parse_windows_remote_path("/C/Users/x"),
            Some(WindowsDrivePath {
                drive: "C".into(),
                rest: "Users/x".into(),
            })
        );
        assert_eq!(
            parse_windows_remote_path("/C:/Users/x"),
            Some(WindowsDrivePath {
                drive: "C".into(),
                rest: "Users/x".into(),
            })
        );
        assert!(!is_windows_remote_path("/home/op"));
        assert!(is_windows_remote_path(r"C:\Users"));
    }

    #[test]
    fn converts_to_native_fs_and_win32_openssh_sftp_paths() {
        assert_eq!(
            to_windows_fs_path("/C/Users/Administrator"),
            r"C:\Users\Administrator"
        );
        assert_eq!(
            to_windows_fs_path("C:/Users/Administrator"),
            r"C:\Users\Administrator"
        );
        assert_eq!(
            to_windows_sftp_path(r"C:\Users\Administrator"),
            "/C/Users/Administrator"
        );
        assert_eq!(
            to_windows_sftp_path("C:/Users/Administrator"),
            "/C/Users/Administrator"
        );
        assert_eq!(
            to_windows_sftp_path("/C:/Users/Administrator"),
            "/C/Users/Administrator"
        );
    }

    #[test]
    fn resolves_relative_paths_against_a_windows_cwd() {
        assert_eq!(
            resolve_windows_relative(Some(r"C:\Users\Administrator"), "Documents"),
            r"C:\Users\Administrator\Documents"
        );
        assert_eq!(
            resolve_windows_relative(Some("/C/Users/Administrator"), "Documents/file.txt"),
            r"C:\Users\Administrator\Documents\file.txt"
        );
        assert_eq!(
            resolve_windows_relative(Some(r"C:\Users\Administrator"), r"C:\Windows"),
            r"C:\Windows"
        );
    }

    #[test]
    fn wraps_run_command_in_a_quote_safe_powershell_command() {
        let wrapped = wrap_windows_remote_command("Get-Location", Some(r"C:\Users\Administrator"));
        assert!(wrapped.starts_with("powershell.exe "));
        assert!(wrapped.contains("-Command "));
        let script = decode_script(&wrapped);
        assert!(script.contains("Set-Location -LiteralPath"));
        assert!(script.contains("Get-Location"));
        assert!(script.contains(r"Users\Administrator"));
    }

    #[test]
    fn wraps_git_as_powershell_set_location_plus_git() {
        let wrapped = wrap_windows_git_command(r"C:\repo", "'status'");
        let script = decode_script(&wrapped);
        assert!(script.contains("Set-Location -LiteralPath"));
        assert!(script.contains("git "));
        assert!(script.contains("status"));
    }

    #[test]
    fn builds_a_quiet_set_location_prefix() {
        assert_eq!(windows_start_location_prefix(None), "");
        assert_eq!(
            windows_start_location_prefix(Some(r"C:\Users\root")),
            "Set-Location -LiteralPath 'C:\\Users\\root'; "
        );
        assert_eq!(
            windows_start_location_prefix(Some("C:\\Users\\O'Brien")),
            "Set-Location -LiteralPath 'C:\\Users\\O''Brien'; "
        );
        assert_eq!(windows_start_location_prefix(Some("C:\\Users\nroot")), "");
    }

    #[test]
    fn uses_the_quote_safe_noninteractive_powershell_wrapper() {
        let command = powershell_command("Write-Output ok");
        assert!(command.contains("-Command "));
        assert!(!command.contains("-EncodedCommand"));
        assert_eq!(decode_script(&command), "Write-Output ok");
    }

    #[test]
    fn keeps_the_legacy_encoded_command_helper() {
        let command = powershell_encoded_command("Write-Output ok");
        assert!(command.contains("-EncodedCommand "));
        assert!(command.contains("VwByAGkAdABlAC0ATwB1AHQAcAB1AHQAIABvAGsA"));
    }

    #[test]
    fn starts_interactive_powershell_without_typing_the_prompt_hook() {
        let setup = "function prompt { Write-Host -NoNewline 'hook' }";
        let command = windows_powershell_interactive_command(setup);
        assert!(command.starts_with("powershell.exe -NoLogo -NoExit -Command "));
        assert!(!command.contains("-EncodedCommand"));
        assert_eq!(decode_script(&command), setup);
    }

    #[test]
    fn translates_xterm_backspace_to_the_byte_windows_ptys_expect() {
        assert_eq!(
            normalize_windows_interactive_input("abc\u{7f}\u{7f}"),
            "abc\u{8}\u{8}"
        );
        assert_eq!(
            normalize_windows_interactive_input("\t\u{1b}[3~\u{8}"),
            "\t\u{1b}[3~\u{8}"
        );
    }

    #[test]
    fn canonicalizes_the_windows_sftp_home() {
        let home = sftp_home_from_realpath(|path| {
            assert_eq!(path, ".");
            Ok::<_, ()>(r"C:\Users\dev".to_string())
        })
        .unwrap();
        assert_eq!(home, "/C/Users/dev");
    }

    #[test]
    fn windows_auxiliary_clients_stay_off_the_shell_transport() {
        for purpose in [AuxPurpose::Exec, AuxPurpose::Sftp, AuxPurpose::Forward] {
            let decision = auxiliary_client_decision("windows", purpose);
            assert!(decision.use_dedicated_client);
            assert!(decision.prefer_legacy_windows_kex);
            assert!(decision.isolate_status_from_shell);
        }
        let posix = auxiliary_client_decision("linux", AuxPurpose::Exec);
        assert!(!posix.use_dedicated_client);
        assert!(!posix.prefer_legacy_windows_kex);
        assert!(second_channel_resets_shell_transport("windows"));
        assert!(!second_channel_resets_shell_transport("linux"));
        assert!(!opens_on_shell_transport("windows", "exec"));
        assert!(!opens_on_shell_transport("windows", "sftp"));
        assert!(!opens_on_shell_transport("windows", "forward"));
        assert!(opens_on_shell_transport("windows", "shell"));
        assert!(opens_on_shell_transport("linux", "exec"));
        assert!(windows_exec_is_serialized("windows"));
        assert!(!windows_exec_is_serialized("linux"));
        assert!(LEGACY_WINDOWS_KEX.contains(&"diffie-hellman-group14-sha1"));
        assert!(!LEGACY_WINDOWS_KEX
            .iter()
            .any(|name| *name == "diffie-hellman-group1-sha1"));
        assert!(!LEGACY_WINDOWS_KEX
            .iter()
            .any(|name| *name == "diffie-hellman-group-exchange-sha1"));
        assert!(matches!(
            shell_launch("windows"),
            ShellLaunch::WindowsExec {
                fallback_cmd_exe: true
            }
        ));
        assert_eq!(
            windows_shell_open_error(POWERSHELL_CHANNEL_NOT_OPENED, None),
            "Windows shell could not be opened: PowerShell exec channel was not opened"
        );
        assert_eq!(
            forward_client_choice("windows", true, false, false),
            ForwardClientChoice::DedicatedWindows
        );
        assert_eq!(
            forward_client_choice("linux", true, false, false),
            ForwardClientChoice::Primary
        );
        assert_eq!(
            forward_client_choice("windows", false, false, false),
            ForwardClientChoice::Unavailable
        );
    }

    #[test]
    fn shell_channel_close_recovers_without_dropping_the_connection() {
        let mut session = ShellSessionFlags::default();
        match on_shell_channel_close(&mut session, 1_000) {
            ShellCloseOutcome::ScheduleRecovery { attempt, delay_ms } => {
                assert_eq!(attempt, 1);
                assert_eq!(delay_ms, 250);
            }
            other => panic!("{other:?}"),
        }
        assert!(!on_shell_channel_close(&mut session, 1_000).drops_connection());
        match on_shell_recovery_timer(&mut session, 1) {
            RecoveryTimerOutcome::Reopen { message, attempt } => {
                assert_eq!(attempt, 1);
                assert!(message.contains("recovering (1/3)"));
                assert!(message.contains('\u{2026}'));
            }
            other => panic!("{other:?}"),
        }
        session.shell_open = false;
        let _ = on_shell_channel_close(&mut session, 1_100);
        session.recovery_timer_pending = false;
        let _ = on_shell_channel_close(&mut session, 1_200);
        session.recovery_timer_pending = false;
        match on_shell_channel_close(&mut session, 1_300) {
            ShellCloseOutcome::StopRecovery { message } => {
                assert_eq!(message, WINDOWS_SHELL_RECOVERY_STOPPED);
            }
            other => panic!("expected stop, got {other:?}"),
        }
        let stopped = on_shell_channel_close(&mut session, 1_300);
        // The fourth close inside the window already consumed the stop above;
        // a fresh window after 60s starts again at attempt 1.
        let mut session = ShellSessionFlags {
            recovery_attempts: 3,
            recovery_window_started_at: Some(0),
            ..ShellSessionFlags::default()
        };
        match on_shell_channel_close(&mut session, 60_000) {
            ShellCloseOutcome::StopRecovery { .. } => {}
            other => panic!("boundary is exclusive, got {other:?}"),
        }
        let mut session = ShellSessionFlags {
            recovery_attempts: 3,
            recovery_window_started_at: Some(0),
            ..ShellSessionFlags::default()
        };
        match on_shell_channel_close(&mut session, 60_001) {
            ShellCloseOutcome::ScheduleRecovery { attempt, delay_ms } => {
                assert_eq!(attempt, 1);
                assert_eq!(delay_ms, 250);
            }
            other => panic!("{other:?}"),
        }
        let _ = stopped;
        let mut exited = ShellSessionFlags {
            received_exit: true,
            ..ShellSessionFlags::default()
        };
        assert!(matches!(
            on_shell_channel_close(&mut exited, 0),
            ShellCloseOutcome::ReportExit
        ));
        let mut tmux = ShellSessionFlags {
            tmux_client_running: true,
            os: "linux".into(),
            ..ShellSessionFlags::default()
        };
        assert!(matches!(
            on_shell_channel_close(&mut tmux, 0),
            ShellCloseOutcome::ResumeAfterTmux
        ));
        let mut session = ShellSessionFlags::default();
        let _ = on_shell_channel_close(&mut session, 5);
        let failed = on_shell_recovery_open_failed(&session, "no shell channel").unwrap();
        assert!(
            failed.contains("Windows shell recovery failed: no shell channel; reconnecting SSH")
        );
        assert!(recovery_open_failure_ends_transport());
        session.closing = true;
        assert!(on_shell_recovery_open_failed(&session, "no shell channel").is_none());
    }

    #[test]
    fn forwarding_transport_lease_closes_after_idle_and_ignores_a_second_release() {
        let mut pool = ForwardPool::new();
        let mut first = pool.acquire(0).unwrap();
        let mut second = pool.acquire(10).unwrap();
        assert_eq!(pool.refs(), 2);
        pool.release(&mut first, 20);
        assert_eq!(pool.refs(), 1);
        assert!(pool.idle_deadline().is_none());
        pool.release(&mut second, 30);
        assert_eq!(pool.idle_deadline(), Some(30 + WINDOWS_FORWARD_IDLE_MS));
        pool.release(&mut second, 40);
        assert_eq!(pool.idle_deadline(), Some(30 + WINDOWS_FORWARD_IDLE_MS));
        assert!(!pool.poll(30 + WINDOWS_FORWARD_IDLE_MS - 1));
        assert!(pool.is_open());
        assert!(pool.poll(30 + WINDOWS_FORWARD_IDLE_MS));
        assert!(!pool.is_open());
        let mut stale = ForwardLease {
            generation: 1,
            active: true,
        };
        pool.release(&mut stale, 99_000);
        assert_eq!(pool.refs(), 0);

        let mut pool = ForwardPool::new();
        let mut lease = pool.acquire(0).unwrap();
        pool.release(&mut lease, 0);
        assert!(pool.idle_deadline().is_some());
        let again = pool.acquire(1_000).unwrap();
        assert!(pool.idle_deadline().is_none());
        assert_eq!(pool.refs(), 1);
        drop(again);
    }

    #[test]
    fn does_not_double_wrap_an_encoded_command() {
        let command = powershell_encoded_command("Get-Location");
        assert_eq!(
            wrap_windows_remote_command(&command, Some(r"C:\Users")),
            command
        );
        assert!(path_is_absolute_hint(r"C:\Users"));
    }
}
