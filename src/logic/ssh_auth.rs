//! SSH auth selection from `src/main/ssh/auth.ts`.
//!
//! The system agent is the default when a hop has neither a password nor a
//! private key. Windows uses the OpenSSH named pipe, not Pageant.

use std::fs;
use std::io;

/// OpenSSH's Windows named pipe (not Pageant).
pub const WINDOWS_SSH_AGENT_PIPE: &str = "\\\\.\\pipe\\openssh-ssh-agent";

pub const AGENT_NO_KEY_ERROR: &str = "system agent has no usable key";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshHop {
    pub host: String,
    pub port: i64,
    pub username: String,
    pub password: Option<String>,
    pub private_key_path: Option<String>,
    pub passphrase: Option<String>,
    pub use_agent: Option<bool>,
}

impl SshHop {
    pub fn new(host: impl Into<String>, port: i64, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            password: None,
            private_key_path: None,
            passphrase: None,
            use_agent: None,
        }
    }
}

/// Profile fields `establish()` needs. `jump` is preserved unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshProfile {
    pub hop: SshHop,
    pub jump: Option<JumpField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JumpField {
    One(SshHop),
    Many(Vec<SshHop>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EstablishedHop {
    pub hop: SshHop,
    pub jump: Option<JumpField>,
}

/// Auth fields handed to ssh2 `connect()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthConfig {
    pub private_key: Option<Vec<u8>>,
    pub passphrase: Option<String>,
    pub password: Option<String>,
    pub agent: Option<String>,
}

pub fn hop_uses_agent(hop: &SshHop) -> bool {
    match hop.use_agent {
        Some(true) => true,
        Some(false) => false,
        None => !truthy(&hop.password) && !truthy(&hop.private_key_path),
    }
}

fn truthy(value: &Option<String>) -> bool {
    match value {
        Some(text) => !text.is_empty(),
        None => false,
    }
}

/// `win32` always returns the OpenSSH pipe. Other platforms use `SSH_AUTH_SOCK`.
pub fn system_agent_path(ssh_auth_sock: Option<&str>, platform: &str) -> Option<String> {
    if platform == "win32" {
        return Some(WINDOWS_SSH_AGENT_PIPE.to_string());
    }
    let sock = ssh_auth_sock.unwrap_or("").trim();
    if sock.is_empty() {
        None
    } else {
        Some(sock.to_string())
    }
}

pub fn auth_config(
    hop: &SshHop,
    ssh_auth_sock: Option<&str>,
    platform: &str,
) -> io::Result<AuthConfig> {
    let mut cfg = AuthConfig {
        private_key: None,
        passphrase: None,
        password: None,
        agent: None,
    };
    if truthy(&hop.private_key_path) {
        let path = hop.private_key_path.as_deref().unwrap();
        cfg.private_key = Some(fs::read(path)?);
        if truthy(&hop.passphrase) {
            cfg.passphrase = hop.passphrase.clone();
        }
    }
    if truthy(&hop.password) {
        cfg.password = hop.password.clone();
    }
    if hop_uses_agent(hop) {
        if let Some(agent) = system_agent_path(ssh_auth_sock, platform) {
            cfg.agent = Some(agent);
        }
    }
    Ok(cfg)
}

/// Build the hop object `establish()` expects, including `use_agent`.
pub fn establish_profile(profile: &SshProfile) -> EstablishedHop {
    EstablishedHop {
        hop: profile.hop.clone(),
        jump: profile.jump.clone(),
    }
}

fn is_transport_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("host key")
        || lower.contains("timed out")
        || message.contains("ECONNREFUSED")
        || message.contains("ENOTFOUND")
        || message.contains("EHOSTUNREACH")
        || lower.contains("tcp connect")
}

fn looks_like_agent_auth_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("all configured authentication methods failed")
        || lower.contains("no more authentication methods")
        || lower.contains("cannot connect to the agent")
        || lower.contains("cannot connect to agent")
        || (lower.contains("agent")
            && (lower.contains("fail")
                || lower.contains("error")
                || lower.contains("unable")
                || lower.contains("enoent")
                || lower.contains("pipe")))
        || lower.contains("connect enoent")
}

/// Rewrite agent-auth failures so the UI names the system agent.
pub fn map_auth_error(message: &str, hop: &SshHop) -> String {
    if !hop_uses_agent(hop) {
        return message.to_string();
    }
    if is_transport_error(message) {
        return message.to_string();
    }
    let agent_only = !truthy(&hop.password) && !truthy(&hop.private_key_path);
    if agent_only {
        return AGENT_NO_KEY_ERROR.to_string();
    }
    if looks_like_agent_auth_failure(message) {
        return format!("{message} ({AGENT_NO_KEY_ERROR})");
    }
    message.to_string()
}

/// `Some(AGENT_NO_KEY_ERROR)` when agent auth is required but no socket/pipe exists.
pub fn missing_agent_path_error(
    hop: &SshHop,
    ssh_auth_sock: Option<&str>,
    platform: &str,
) -> Option<String> {
    if !hop_uses_agent(hop) {
        return None;
    }
    if system_agent_path(ssh_auth_sock, platform).is_some() {
        return None;
    }
    Some(AGENT_NO_KEY_ERROR.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop() -> SshHop {
        SshHop::new("example.test", 22, "op")
    }

    #[test]
    fn defaults_on_when_no_key_or_password_is_set() {
        assert!(hop_uses_agent(&hop()));
    }

    #[test]
    fn defaults_off_when_a_password_or_key_path_is_present() {
        let mut with_password = hop();
        with_password.password = Some("x".into());
        assert!(!hop_uses_agent(&with_password));
        let mut with_key = hop();
        with_key.private_key_path = Some(r"C:\Users\me\.ssh\id".into());
        assert!(!hop_uses_agent(&with_key));
    }

    #[test]
    fn honors_an_explicit_flag_over_the_default() {
        let mut forced = hop();
        forced.password = Some("x".into());
        forced.use_agent = Some(true);
        assert!(hop_uses_agent(&forced));
        let mut off = hop();
        off.use_agent = Some(false);
        assert!(!hop_uses_agent(&off));
    }

    #[test]
    fn uses_the_openssh_named_pipe_on_windows() {
        assert_eq!(
            system_agent_path(None, "win32").as_deref(),
            Some(WINDOWS_SSH_AGENT_PIPE)
        );
    }

    #[test]
    fn uses_ssh_auth_sock_on_posix() {
        assert_eq!(
            system_agent_path(Some("/tmp/ssh-agent.sock"), "linux").as_deref(),
            Some("/tmp/ssh-agent.sock")
        );
        assert_eq!(system_agent_path(None, "linux"), None);
        assert_eq!(system_agent_path(Some("   "), "linux"), None);
    }

    #[test]
    fn sets_the_windows_agent_pipe_when_agent_auth_is_on() {
        let mut hop = hop();
        hop.use_agent = Some(true);
        let cfg = auth_config(&hop, None, "win32").unwrap();
        assert_eq!(cfg.agent.as_deref(), Some(WINDOWS_SSH_AGENT_PIPE));
        assert_eq!(cfg.password, None);
        assert_eq!(cfg.private_key, None);
    }

    #[test]
    fn keeps_password_auth_when_the_operator_chose_a_password() {
        let mut hop = hop();
        hop.password = Some("secret".into());
        hop.use_agent = Some(false);
        let cfg = auth_config(&hop, None, "linux").unwrap();
        assert_eq!(cfg.password.as_deref(), Some("secret"));
        assert_eq!(cfg.agent, None);
    }

    #[test]
    fn names_a_missing_usable_key_when_agent_only_auth_fails() {
        let err = map_auth_error("All configured authentication methods failed", &hop());
        assert_eq!(err, AGENT_NO_KEY_ERROR);
    }

    #[test]
    fn does_not_rewrite_host_key_or_tcp_failures() {
        let host = map_auth_error("Host key for example.test mismatch", &hop());
        assert!(host.contains("Host key"));
        let tcp = map_auth_error("TCP connect to example.test:22 timed out", &hop());
        assert!(tcp.contains("TCP connect"));
    }

    #[test]
    fn leaves_password_only_failures_unchanged() {
        let mut hop = hop();
        hop.password = Some("x".into());
        hop.use_agent = Some(false);
        let err = map_auth_error("All configured authentication methods failed", &hop);
        assert!(err.contains("All configured"));
    }

    #[test]
    fn appends_the_agent_name_when_agent_is_combined_with_another_method() {
        let mut hop = hop();
        hop.password = Some("x".into());
        hop.use_agent = Some(true);
        let err = map_auth_error("All configured authentication methods failed", &hop);
        assert_eq!(
            err,
            format!("All configured authentication methods failed ({AGENT_NO_KEY_ERROR})")
        );
    }

    #[test]
    fn missing_agent_path_is_silent_when_a_path_exists() {
        assert_eq!(missing_agent_path_error(&hop(), None, "win32"), None);
        assert_eq!(
            missing_agent_path_error(&hop(), None, "linux").as_deref(),
            Some(AGENT_NO_KEY_ERROR)
        );
        assert_eq!(
            missing_agent_path_error(&hop(), Some("/tmp/agent.sock"), "linux"),
            None
        );
    }

    #[test]
    fn establish_profile_keeps_use_agent_and_jump() {
        let mut hop = hop();
        hop.use_agent = Some(true);
        let profile = SshProfile {
            hop: hop.clone(),
            jump: Some(JumpField::One(SshHop::new("bastion", 22, "jump"))),
        };
        let established = establish_profile(&profile);
        assert_eq!(established.hop.use_agent, Some(true));
        assert!(matches!(established.jump, Some(JumpField::One(_))));
    }

    #[test]
    fn reads_a_private_key_and_keeps_its_passphrase() {
        let dir = std::env::temp_dir().join(format!(
            "devterm-auth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("id");
        fs::write(&path, b"KEYDATA").unwrap();
        let mut hop = hop();
        hop.private_key_path = Some(path.to_string_lossy().into_owned());
        hop.passphrase = Some("phrase".into());
        hop.use_agent = Some(false);
        let cfg = auth_config(&hop, None, "linux").unwrap();
        assert_eq!(cfg.private_key.as_deref(), Some(&b"KEYDATA"[..]));
        assert_eq!(cfg.passphrase.as_deref(), Some("phrase"));
        assert_eq!(cfg.agent, None);
        let _ = fs::remove_dir_all(&dir);
    }
}
