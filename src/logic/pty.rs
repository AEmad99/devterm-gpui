//! Local PTY policy ported from DevTerm's main-process shell launcher.
//!
//! Covers ConPTY selection, the env scrub, PowerShell prompt args, the
//! "did the shell print anything?" check, startup-failure text, and shell
//! resolution. `TERM` is the name passed to node-pty (`xterm-256color`).

use std::collections::HashMap;

/// `name` passed to every local PTY.
pub const TERM: &str = "xterm-256color";

/// Settings scrollback clamp (`SettingsModal`: 100–100000, empty input → 10000).
pub const DEFAULT_SCROLLBACK: i64 = 10_000;
pub const MIN_SCROLLBACK: i64 = 100;
pub const MAX_SCROLLBACK: i64 = 100_000;

/// User's preferred local shell. Mirrors `DefaultShellPref` in `shared/types.ts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellPref {
    Auto,
    Pwsh,
    Powershell,
    Cmd,
    Custom(String),
}

/// Bundled ConPTY (`OpenConsole.exe`) is opt-in on Windows only.
///
/// Port of `shouldUseBundledConpty`.
pub fn should_use_bundled_conpty(
    platform: &str,
    bundled_available: bool,
    opt_in: Option<&str>,
) -> bool {
    platform == "win32" && bundled_available && opt_in == Some("1")
}

/// Environment passed to every local PTY.
///
/// Strips keys whose uppercase form starts with `ELECTRON_` or `NODE_`, or
/// equals `VITE_DEV_SERVER_URL` or `NO_COLOR`. Drops `FORCE_COLOR` when the
/// value is `"0"` or trim-empty. Sets `COLORTERM=truecolor`. Other keys,
/// including a non-zero `FORCE_COLOR`, keep their original casing.
pub fn build_pty_base_env(source: &HashMap<String, String>) -> HashMap<String, String> {
    let mut base = HashMap::new();
    for (key, value) in source {
        let upper = key.to_ascii_uppercase();
        if upper.starts_with("ELECTRON_")
            || upper.starts_with("NODE_")
            || upper == "VITE_DEV_SERVER_URL"
            || upper == "NO_COLOR"
        {
            continue;
        }
        if upper == "FORCE_COLOR" && (value == "0" || value.trim().is_empty()) {
            continue;
        }
        base.insert(key.clone(), value.clone());
    }
    base.insert("COLORTERM".to_string(), "truecolor".to_string());
    base
}

const POWERSHELL_PROMPT: &str = "function prompt { $e=[char]27; $b=[char]7; $p=$PWD.ProviderPath; $u=($p -replace '\\\\','/'); Write-Host -NoNewline ($e + ']133;A' + $b + $e + ']7;file:///' + $u + $b); ('PS ' + $p + '> ' + $e + ']133;B' + $b) }";

/// Startup args for the chosen shell. PowerShell gets the OSC 7 / OSC 133 prompt.
pub fn shell_args(shell: &str) -> Vec<&'static str> {
    if is_powershell_shell(shell) {
        vec!["-NoLogo", "-NoExit", "-Command", POWERSHELL_PROMPT]
    } else {
        Vec::new()
    }
}

fn is_powershell_shell(shell: &str) -> bool {
    let lower = shell.to_ascii_lowercase();
    lower.contains("powershell") || lower.contains("pwsh")
}

/// Whether a chunk has printable shell output after OSC, CSI, and C0 are stripped.
///
/// Same pattern as `ANSI_OR_CTRL` in `manager.ts`:
/// `\x1b\][^\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-9;?<=>]*[!-/]*[@-~]|[\x00-\x1f\x7f]`
pub fn has_real_output(data: &str) -> bool {
    let bytes = data.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(len) = match_ansi_or_ctrl(bytes, i) {
            i += len;
        } else {
            return true;
        }
    }
    false
}

fn match_ansi_or_ctrl(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes[i] == 0x1b {
        if let Some(len) = match_osc(bytes, i) {
            return Some(len);
        }
        if let Some(len) = match_csi(bytes, i) {
            return Some(len);
        }
    }
    if bytes[i] <= 0x1f || bytes[i] == 0x7f {
        Some(1)
    } else {
        None
    }
}

/// OSC: ESC ] non-ESC* (BEL | ST). Greedy, so the last BEL wins unless ST follows.
fn match_osc(bytes: &[u8], i: usize) -> Option<usize> {
    if i + 1 >= bytes.len() || bytes[i + 1] != b']' {
        return None;
    }
    let mut end = i + 2;
    while end < bytes.len() && bytes[end] != 0x1b {
        end += 1;
    }
    if end + 1 < bytes.len() && bytes[end] == 0x1b && bytes[end + 1] == b'\\' {
        return Some(end + 2 - i);
    }
    let mut bel = None;
    for (offset, byte) in bytes[i + 2..end].iter().enumerate() {
        if *byte == 0x07 {
            bel = Some(i + 2 + offset);
        }
    }
    bel.map(|at| at + 1 - i)
}

/// CSI subset: ESC [ [0-9;?<=>]* [!-/]* [@-~]
fn match_csi(bytes: &[u8], i: usize) -> Option<usize> {
    if i + 1 >= bytes.len() || bytes[i + 1] != b'[' {
        return None;
    }
    let mut j = i + 2;
    while j < bytes.len() && is_csi_param(bytes[j]) {
        j += 1;
    }
    while j < bytes.len() && is_csi_intermediate(bytes[j]) {
        j += 1;
    }
    if j < bytes.len() && is_csi_final(bytes[j]) {
        Some(j + 1 - i)
    } else {
        None
    }
}

fn is_csi_param(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b';' | b'?' | b'<' | b'=' | b'>')
}

fn is_csi_intermediate(byte: u8) -> bool {
    (0x21..=0x2f).contains(&byte)
}

fn is_csi_final(byte: u8) -> bool {
    (0x40..=0x7e).contains(&byte)
}

const EXIT_RESET: &str = "\u{1b}[?1049l\u{1b}[?1000l\u{1b}[?1002l\u{1b}[?1003l\u{1b}[?1006l\u{1b}[?2004l\u{1b}[?1004l\u{1b}[0m\u{1b}[?25h";

/// User-visible diagnostic written when a PTY exits before real output.
///
/// Same text as `TerminalView`'s startup-failure notice: mode reset, the shell
/// path, the exit code (`unknown` when the process did not report one), then
/// the Windows PowerShell 5.1 help or the generic help.
pub fn startup_failure_message(shell: &str, exit_code: Option<i32>) -> String {
    let code = match exit_code {
        Some(code) => code.to_string(),
        None => "unknown".to_string(),
    };
    let help = if is_windows_powershell_path(shell) {
        powershell_failure_help()
    } else {
        generic_failure_help(shell)
    };
    format!(
        "{EXIT_RESET}\r\n\u{1b}[31m[Shell failed to start]\u{1b}[0m\r\n\u{1b}[90m  {shell}\u{1b}[0m\r\n\u{1b}[90m  Exit code: {code}\u{1b}[0m\r\n{help}"
    )
}

fn is_windows_powershell_path(path: &str) -> bool {
    let norm = path.replace('\\', "/").to_ascii_lowercase();
    norm.ends_with("/windowspowershell/v1.0/powershell.exe")
}

fn powershell_failure_help() -> String {
    [
        "",
        "\u{1b}[33mWindows PowerShell failed to start.\u{1b}[0m",
        "\u{1b}[90mThis is almost always a managed-assembly signature failure\u{1b}[0m",
        "\u{1b}[90m(0x8009001d / NTE_BAD_SIGNATURE) \u{2014} commonly caused by antivirus\u{1b}[0m",
        "\u{1b}[90mquarantining a PowerShell DLL, a corrupted .NET Framework install,\u{1b}[0m",
        "\u{1b}[90mor an out-of-sync system clock.\u{1b}[0m",
        "",
        "\u{1b}[36mRecommended fix \u{2014} install PowerShell 7:\u{1b}[0m",
        "  winget install Microsoft.PowerShell",
        "",
        "\u{1b}[90mThen either:\u{1b}[0m",
        "  1. open a new terminal (DevTerm auto-detects pwsh.exe), or",
        "  2. Settings \u{2192} General \u{2192} Default local shell \u{2192} PowerShell 7",
        "",
        "\u{1b}[90mIf you need Windows PowerShell back, repair the .NET Framework:\u{1b}[0m",
        "  DISM /Online /Cleanup-Image /RestoreHealth",
        "  sfc /scannow",
        "",
    ]
    .join("\r\n")
        + "\r\n"
}

fn generic_failure_help(shell: &str) -> String {
    [
        "",
        "\u{1b}[33mThe shell exited before producing any output.\u{1b}[0m",
        &format!("\u{1b}[90m  Path: {shell}\u{1b}[0m"),
        "\u{1b}[90mCommon causes: missing executable, broken symlink, or a\u{1b}[0m",
        "\u{1b}[90m32/64-bit mismatch. Settings \u{2192} General \u{2192} Default local shell\u{1b}[0m",
        "\u{1b}[90mcan pick a different shell (cmd.exe, PowerShell 7, custom path).\u{1b}[0m",
        "",
    ]
    .join("\r\n")
        + "\r\n"
}

/// Map a shell preference (or an explicit one-off path) to a shell path.
///
/// Precedence matches `resolveShell`: explicit path, then `pref` (only when
/// that shell is installed, except `powershell` / `cmd` / `custom` which
/// return their path even if missing), then [`default_shell`].
/// `exists` is called with the candidate paths this platform would check.
pub fn resolve_shell(
    platform: &str,
    env: &HashMap<String, String>,
    pref: Option<&ShellPref>,
    explicit: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> String {
    if let Some(path) = explicit {
        if !path.is_empty() {
            return path.to_string();
        }
    }
    if let Some(pref) = pref {
        match pref {
            ShellPref::Custom(path) if !path.is_empty() => return path.clone(),
            ShellPref::Pwsh => {
                for candidate in pwsh_candidate_paths(platform, env) {
                    if exists(&candidate) {
                        return candidate;
                    }
                }
            }
            ShellPref::Powershell => {
                let fallback = default_shell(platform, env, &exists);
                if fallback.contains("WindowsPowerShell") {
                    return fallback;
                }
                return windows_powershell_path(platform, env);
            }
            ShellPref::Cmd => return comspec(platform, env),
            ShellPref::Auto | ShellPref::Custom(_) => {}
        }
    }
    default_shell(platform, env, &exists)
}

/// Best installed interactive shell. Windows: pwsh, then Windows PowerShell, then COMSPEC.
/// Anything else: `SHELL` or `/bin/bash`.
pub fn default_shell(
    platform: &str,
    env: &HashMap<String, String>,
    exists: impl Fn(&str) -> bool,
) -> String {
    if platform == "win32" {
        for candidate in pwsh_candidate_paths(platform, env) {
            if exists(&candidate) {
                return candidate;
            }
        }
        let win_ps = windows_powershell_path(platform, env);
        if exists(&win_ps) {
            return win_ps;
        }
        return comspec(platform, env);
    }
    match env_get(env, platform, "SHELL") {
        Some(shell) if !shell.is_empty() => shell.to_string(),
        _ => "/bin/bash".to_string(),
    }
}

/// PowerShell 7 install locations, in the order `pwshCandidatePaths` checks them.
pub fn pwsh_candidate_paths(platform: &str, env: &HashMap<String, String>) -> Vec<String> {
    if platform != "win32" {
        return Vec::new();
    }
    let program_files = env_or(env, platform, "ProgramFiles", r"C:\Program Files");
    let program_files_x86 = env_or(
        env,
        platform,
        "ProgramFiles(x86)",
        r"C:\Program Files (x86)",
    );
    let local_app_data = env_or(env, platform, "LOCALAPPDATA", "");
    let paths = [
        win_join(&[local_app_data, "Microsoft", "WindowsApps", "pwsh.exe"]),
        win_join(&[local_app_data, "Microsoft", "PowerShell", "7", "pwsh.exe"]),
        win_join(&[program_files, "PowerShell", "7", "pwsh.exe"]),
        win_join(&[program_files_x86, "PowerShell", "7", "pwsh.exe"]),
    ];
    paths.into_iter().filter(|path| !path.is_empty()).collect()
}

fn windows_powershell_path(platform: &str, env: &HashMap<String, String>) -> String {
    let root = env_or(env, platform, "SystemRoot", r"C:\Windows");
    win_join(&[
        root,
        "System32",
        "WindowsPowerShell",
        "v1.0",
        "powershell.exe",
    ])
}

fn comspec(platform: &str, env: &HashMap<String, String>) -> String {
    match env_get(env, platform, "COMSPEC") {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => "cmd.exe".to_string(),
    }
}

/// `??` : a missing key uses `default`; a present empty string is kept.
fn env_or<'a>(
    env: &'a HashMap<String, String>,
    platform: &str,
    key: &str,
    default: &'a str,
) -> &'a str {
    env_get(env, platform, key).unwrap_or(default)
}

fn env_get<'a>(env: &'a HashMap<String, String>, platform: &str, key: &str) -> Option<&'a str> {
    if let Some(value) = env.get(key) {
        return Some(value.as_str());
    }
    if platform == "win32" {
        return env
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(key))
            .min_by(|a, b| a.0.cmp(b.0))
            .map(|(_, value)| value.as_str());
    }
    None
}

/// `path.win32.join` for the shell candidates. Separators stay backslashes on any host.
fn win_join(parts: &[&str]) -> String {
    let mut joined = String::new();
    let mut any = false;
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if any {
            joined.push('\\');
        }
        joined.push_str(part);
        any = true;
    }
    if !any {
        return ".".to_string();
    }
    win_normalize(&joined)
}

fn win_normalize(path: &str) -> String {
    let path = path.replace('/', "\\");
    let is_unc = path.starts_with("\\\\");
    let mut drive = String::new();
    let mut rooted = false;
    let body: &str = if is_unc {
        rooted = true;
        path.trim_start_matches('\\')
    } else if path.len() >= 2 && path.as_bytes()[1] == b':' {
        drive = path[..2].to_string();
        let rest = &path[2..];
        rooted = rest.starts_with('\\');
        rest.trim_start_matches('\\')
    } else if path.starts_with('\\') {
        rooted = true;
        path.trim_start_matches('\\')
    } else {
        path.as_str()
    };

    let mut stack: Vec<&str> = Vec::new();
    for part in body.split('\\') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if stack.is_empty() {
                if !rooted && drive.is_empty() && !is_unc {
                    stack.push(part);
                }
            } else if *stack.last().unwrap() == ".." {
                stack.push(part);
            } else {
                stack.pop();
            }
            continue;
        }
        stack.push(part);
    }

    let mut out = String::new();
    if is_unc {
        out.push_str("\\\\");
    } else {
        out.push_str(&drive);
        if rooted {
            out.push('\\');
        }
    }
    for (index, part) in stack.iter().enumerate() {
        if index > 0 {
            out.push('\\');
        }
        out.push_str(part);
    }
    if out.is_empty() {
        ".".to_string()
    } else {
        out
    }
}

/// Scrollback line count for the settings field: `100..=100_000`.
///
/// `0` is the empty number input (`Number(value) || 10000`) and uses
/// [`DEFAULT_SCROLLBACK`]. Other values clamp into range.
pub fn clamp_scrollback(n: i64) -> i64 {
    if n == 0 {
        DEFAULT_SCROLLBACK
    } else {
        n.clamp(MIN_SCROLLBACK, MAX_SCROLLBACK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn uses_the_inbox_conpty_by_default_even_when_bundled_binaries_exist() {
        assert!(!should_use_bundled_conpty("win32", true, None));
    }

    #[test]
    fn allows_the_bundled_helper_only_through_an_explicit_windows_opt_in() {
        assert!(should_use_bundled_conpty("win32", true, Some("1")));
        assert!(!should_use_bundled_conpty("win32", false, Some("1")));
        assert!(!should_use_bundled_conpty("linux", true, Some("1")));
    }

    #[test]
    fn advertises_truecolor_and_drops_parent_no_color_signals() {
        let source = env(&[
            ("PATH", "/bin"),
            ("NO_COLOR", "1"),
            ("FORCE_COLOR", "0"),
            ("COLORTERM", "mono"),
            ("ELECTRON_RUN_AS_NODE", "1"),
            ("NODE_OPTIONS", "--no-warnings"),
            ("VITE_DEV_SERVER_URL", "http://127.0.0.1:5173"),
        ]);
        let got = build_pty_base_env(&source);
        assert_eq!(got.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(got.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(got.get("NO_COLOR"), None);
        assert_eq!(got.get("FORCE_COLOR"), None);
        assert_eq!(got.get("ELECTRON_RUN_AS_NODE"), None);
        assert_eq!(got.get("NODE_OPTIONS"), None);
        assert_eq!(got.get("VITE_DEV_SERVER_URL"), None);
    }

    #[test]
    fn keeps_a_nonzero_force_color_from_the_parent() {
        let source = env(&[("FORCE_COLOR", "3"), ("HOME", "/home/op")]);
        let got = build_pty_base_env(&source);
        assert_eq!(got.get("FORCE_COLOR").map(String::as_str), Some("3"));
        assert_eq!(got.get("HOME").map(String::as_str), Some("/home/op"));
        assert_eq!(got.get("COLORTERM").map(String::as_str), Some("truecolor"));
    }

    #[test]
    fn preserves_original_key_casing_and_only_exact_force_color_zero() {
        let source = env(&[
            ("Path", "/bin"),
            ("ColorTerm", "mono"),
            ("Force_Color", "2"),
            ("electron_run_as_node", "1"),
            ("no_color", "1"),
            ("FORCE_COLOR", " 0 "),
        ]);
        let got = build_pty_base_env(&source);
        assert_eq!(got.get("Path").map(String::as_str), Some("/bin"));
        assert_eq!(got.get("ColorTerm").map(String::as_str), Some("mono"));
        assert_eq!(got.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(got.get("Force_Color").map(String::as_str), Some("2"));
        assert_eq!(got.get("FORCE_COLOR").map(String::as_str), Some(" 0 "));
        assert_eq!(got.get("electron_run_as_node"), None);
        assert_eq!(got.get("no_color"), None);
    }

    #[test]
    fn shell_args_inject_the_powershell_prompt_function() {
        let args = shell_args("pwsh.exe");
        assert_eq!(
            args,
            vec![
                "-NoLogo",
                "-NoExit",
                "-Command",
                "function prompt { $e=[char]27; $b=[char]7; $p=$PWD.ProviderPath; $u=($p -replace '\\\\','/'); Write-Host -NoNewline ($e + ']133;A' + $b + $e + ']7;file:///' + $u + $b); ('PS ' + $p + '> ' + $e + ']133;B' + $b) }"
            ]
        );
        assert_eq!(
            shell_args(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            args
        );
        assert_eq!(shell_args("PowerShell"), args);
        assert!(shell_args("/bin/bash").is_empty());
        assert!(shell_args("cmd.exe").is_empty());
    }

    #[test]
    fn has_real_output_ignores_the_conpty_handshake() {
        assert!(!has_real_output(
            "\u{1b}[1t\u{1b}[c\u{1b}[?1004h\u{1b}[?9001h"
        ));
        assert!(!has_real_output("\u{1b}]133;A\u{7}"));
        assert!(!has_real_output("\u{1b}]7;file:///tmp\u{1b}\\"));
        assert!(has_real_output("\u{1b}[32mhi\u{1b}[0m"));
        assert!(has_real_output("prompt> "));
        assert!(!has_real_output(""));
        assert!(has_real_output(" \n"));
    }

    #[test]
    fn startup_failure_message_for_windows_powershell_includes_path_and_code() {
        let shell = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
        let msg = startup_failure_message(shell, Some(1));
        assert!(msg.starts_with("\u{1b}[?1049l"));
        assert!(msg.contains("\u{1b}[31m[Shell failed to start]\u{1b}[0m"));
        assert!(msg.contains(shell));
        assert!(msg.contains("Exit code: 1"));
        assert!(msg.contains("Windows PowerShell failed to start."));
        assert!(msg.contains("0x8009001d / NTE_BAD_SIGNATURE"));
        assert!(msg.contains("winget install Microsoft.PowerShell"));
        assert!(msg.contains(
            "Settings \u{2192} General \u{2192} Default local shell \u{2192} PowerShell 7"
        ));
        assert!(msg.contains("DISM /Online /Cleanup-Image /RestoreHealth"));
        assert!(msg.contains("sfc /scannow"));
        assert!(msg.ends_with("\r\n"));

        let forward = "C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";
        let unknown = startup_failure_message(forward, None);
        assert!(unknown.contains(forward));
        assert!(unknown.contains("Exit code: unknown"));
        assert!(unknown.contains("Windows PowerShell failed to start."));
    }

    #[test]
    fn startup_failure_message_for_other_shells_names_the_path() {
        let msg = startup_failure_message("/bin/bash", Some(127));
        assert!(msg.contains("\u{1b}[31m[Shell failed to start]\u{1b}[0m"));
        assert!(msg.contains("\u{1b}[90m  /bin/bash\u{1b}[0m"));
        assert!(msg.contains("Exit code: 127"));
        assert!(msg.contains("The shell exited before producing any output."));
        assert!(msg.contains("\u{1b}[90m  Path: /bin/bash\u{1b}[0m"));
        assert!(msg.contains("cmd.exe, PowerShell 7, custom path"));
        assert!(!msg.contains("Windows PowerShell failed to start."));
    }

    #[test]
    fn term_is_xterm_256color() {
        assert_eq!(TERM, "xterm-256color");
    }

    #[test]
    fn clamp_scrollback_uses_the_settings_range() {
        assert_eq!(clamp_scrollback(0), 10_000);
        assert_eq!(clamp_scrollback(50), 100);
        assert_eq!(clamp_scrollback(-5), 100);
        assert_eq!(clamp_scrollback(100), 100);
        assert_eq!(clamp_scrollback(2_000), 2_000);
        assert_eq!(clamp_scrollback(10_000), 10_000);
        assert_eq!(clamp_scrollback(100_000), 100_000);
        assert_eq!(clamp_scrollback(100_001), 100_000);
        assert_eq!(DEFAULT_SCROLLBACK, 10_000);
    }

    #[test]
    fn linux_resolve_uses_shell_or_bash() {
        let zsh = env(&[("SHELL", "/bin/zsh")]);
        assert_eq!(
            resolve_shell("linux", &zsh, None, None, |_| false),
            "/bin/zsh"
        );
        assert_eq!(
            resolve_shell("linux", &HashMap::new(), None, None, |_| false),
            "/bin/bash"
        );
        assert_eq!(
            resolve_shell(
                "linux",
                &zsh,
                Some(&ShellPref::Auto),
                Some("/bin/fish"),
                |_| false
            ),
            "/bin/fish"
        );
        assert_eq!(
            resolve_shell("linux", &zsh, None, Some(""), |_| false),
            "/bin/zsh"
        );
        assert_eq!(
            resolve_shell(
                "linux",
                &zsh,
                Some(&ShellPref::Custom("/usr/bin/nu".into())),
                None,
                |_| false
            ),
            "/usr/bin/nu"
        );
        assert_eq!(
            resolve_shell("linux", &zsh, Some(&ShellPref::Pwsh), None, |_| true),
            "/bin/zsh"
        );
        assert_eq!(
            resolve_shell(
                "linux",
                &HashMap::new(),
                Some(&ShellPref::Cmd),
                None,
                |_| false
            ),
            "cmd.exe"
        );
    }

    #[test]
    fn windows_resolve_prefers_pwsh_then_windows_powershell_then_comspec() {
        let base = env(&[
            ("LOCALAPPDATA", r"C:\Users\ada\AppData\Local"),
            ("ProgramFiles", r"C:\Program Files"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("SystemRoot", r"C:\Windows"),
        ]);
        let store = r"C:\Users\ada\AppData\Local\Microsoft\WindowsApps\pwsh.exe";
        let user = r"C:\Users\ada\AppData\Local\Microsoft\PowerShell\7\pwsh.exe";
        let system = r"C:\Program Files\PowerShell\7\pwsh.exe";
        let x86 = r"C:\Program Files (x86)\PowerShell\7\pwsh.exe";
        let win_ps = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

        assert_eq!(
            pwsh_candidate_paths("win32", &base),
            vec![
                store.to_string(),
                user.to_string(),
                system.to_string(),
                x86.to_string(),
            ]
        );
        assert!(pwsh_candidate_paths("linux", &base).is_empty());

        assert_eq!(
            resolve_shell("win32", &base, None, None, |path| path == x86),
            x86
        );
        assert_eq!(
            resolve_shell("win32", &base, None, None, |path| path == store
                || path == system),
            store
        );
        assert_eq!(
            resolve_shell("win32", &base, None, None, |path| path == user),
            user
        );
        assert_eq!(
            resolve_shell("win32", &base, None, None, |path| path == win_ps),
            win_ps
        );
        assert_eq!(
            resolve_shell("win32", &base, None, None, |_| false),
            "cmd.exe"
        );

        let mut with_comspec = base.clone();
        with_comspec.insert("COMSPEC".into(), r"C:\Windows\System32\cmd.exe".into());
        assert_eq!(
            resolve_shell("win32", &with_comspec, None, None, |_| false),
            r"C:\Windows\System32\cmd.exe"
        );
        assert_eq!(
            resolve_shell("win32", &with_comspec, Some(&ShellPref::Cmd), None, |_| {
                false
            }),
            r"C:\Windows\System32\cmd.exe"
        );

        // `powershell` asks for 5.1 even when pwsh is what auto would pick.
        assert_eq!(
            resolve_shell(
                "win32",
                &base,
                Some(&ShellPref::Powershell),
                None,
                |path| path == system
            ),
            win_ps
        );
        // When auto already resolved to Windows PowerShell, keep that path.
        assert_eq!(
            resolve_shell(
                "win32",
                &base,
                Some(&ShellPref::Powershell),
                None,
                |path| path == win_ps
            ),
            win_ps
        );
        // Missing pwsh falls through to the same default as auto.
        assert_eq!(
            resolve_shell("win32", &base, Some(&ShellPref::Pwsh), None, |path| path
                == win_ps),
            win_ps
        );
        // An explicit path beats the preference.
        assert_eq!(
            resolve_shell(
                "win32",
                &base,
                Some(&ShellPref::Pwsh),
                Some(r"D:\tools\bash.exe"),
                |path| path == system
            ),
            r"D:\tools\bash.exe"
        );
    }

    #[test]
    fn windows_defaults_fill_missing_env_and_keep_empty_localappdata() {
        let bare = env(&[("SystemRoot", r"C:\Windows")]);
        assert_eq!(
            resolve_shell("win32", &bare, Some(&ShellPref::Pwsh), None, |path| {
                path == r"Microsoft\WindowsApps\pwsh.exe"
            }),
            r"Microsoft\WindowsApps\pwsh.exe"
        );
        assert_eq!(
            resolve_shell("win32", &bare, None, None, |path| {
                path == r"C:\Program Files\PowerShell\7\pwsh.exe"
            }),
            r"C:\Program Files\PowerShell\7\pwsh.exe"
        );
        let slash = env(&[("ProgramFiles", "C:/Program Files")]);
        assert_eq!(
            resolve_shell("win32", &slash, Some(&ShellPref::Pwsh), None, |path| {
                path == r"C:\Program Files\PowerShell\7\pwsh.exe"
            }),
            r"C:\Program Files\PowerShell\7\pwsh.exe"
        );
    }
}
