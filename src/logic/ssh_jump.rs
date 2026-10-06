//! ProxyJump normalization from `src/shared/ssh-jump.ts`.
//!
//! Extra hops besides the target are capped at [`MAX_JUMP_HOPS`] (2), so the
//! target plus jumps is at most 3. A longer list is truncated, matching the
//! TypeScript helper (it does not throw).

/// Extra hops besides the target. Target + jumps ≤ 3.
pub const MAX_JUMP_HOPS: usize = 2;

/// One SSH hop. Optional auth fields are preserved across normalize/encode
/// the same way the TypeScript object spread keeps them.
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

/// Stored `jump` field: a legacy single hop or a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JumpSpec {
    One(SshHop),
    Many(Vec<SshHop>),
}

/// Normalize a stored `jump` field (legacy single hop or a list) into hops.
///
/// Empty or whitespace-only hosts are dropped. At most [`MAX_JUMP_HOPS`] hops
/// are kept. `port` falls back to 22 when missing, zero, or not a finite number.
/// `username` is trimmed and defaults to an empty string.
pub fn list_jump_hops(jump: Option<&JumpSpec>) -> Vec<SshHop> {
    let Some(jump) = jump else {
        return Vec::new();
    };
    let hops: Vec<&SshHop> = match jump {
        JumpSpec::One(hop) => vec![hop],
        JumpSpec::Many(hops) => hops.iter().collect(),
    };
    hops.into_iter()
        .filter(|hop| !hop.host.trim().is_empty())
        .take(MAX_JUMP_HOPS)
        .map(|hop| SshHop {
            host: hop.host.trim().to_string(),
            port: normalize_port(hop.port),
            username: hop.username.trim().to_string(),
            password: hop.password.clone(),
            private_key_path: hop.private_key_path.clone(),
            passphrase: hop.passphrase.clone(),
            use_agent: hop.use_agent,
        })
        .collect()
}

/// `Number(port) || 22` — zero and non-finite values become 22.
fn normalize_port(port: i64) -> i64 {
    if port == 0 {
        22
    } else {
        port
    }
}

/// Persist one hop as an object so existing connections.json stays compatible.
/// Two hops stay an array. Empty input is `None`.
pub fn encode_jump(hops: &[SshHop]) -> Option<JumpSpec> {
    let list = list_jump_hops(Some(&JumpSpec::Many(hops.to_vec())));
    match list.len() {
        0 => None,
        1 => Some(JumpSpec::One(list.into_iter().next().unwrap())),
        _ => Some(JumpSpec::Many(list)),
    }
}

/// `user@host` labels joined with ` → `. Empty when there are no hops.
pub fn jump_label(jump: Option<&JumpSpec>) -> String {
    let hops = list_jump_hops(jump);
    if hops.is_empty() {
        return String::new();
    }
    hops.iter()
        .map(|hop| format!("{}@{}", hop.username, hop.host))
        .collect::<Vec<_>>()
        .join(" \u{2192} ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_legacy_single_hop_object() {
        let hops = list_jump_hops(Some(&JumpSpec::One(SshHop::new("bastion", 22, "jump"))));
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0].host, "bastion");
    }

    #[test]
    fn reads_a_hop_list_and_caps_at_two_extra_hops() {
        let hops = list_jump_hops(Some(&JumpSpec::Many(vec![
            SshHop::new("j1", 22, "a"),
            SshHop::new("j2", 22, "b"),
            SshHop::new("j3", 22, "c"),
        ])));
        assert_eq!(MAX_JUMP_HOPS, 2);
        assert_eq!(hops.len(), 2);
        assert_eq!(hops[1].host, "j2");
    }

    #[test]
    fn encodes_a_single_hop_as_an_object() {
        let encoded = encode_jump(&[SshHop::new("bastion", 22, "j")]);
        match encoded {
            Some(JumpSpec::One(hop)) => assert_eq!(hop.host, "bastion"),
            other => panic!("expected a single hop object, got {other:?}"),
        }
    }

    #[test]
    fn labels_a_chain() {
        assert_eq!(
            jump_label(Some(&JumpSpec::Many(vec![
                SshHop::new("a", 22, "u"),
                SshHop::new("b", 22, "v"),
            ]))),
            "u@a \u{2192} v@b"
        );
    }

    #[test]
    fn drops_blank_hosts_trims_fields_and_defaults_port() {
        let mut blank = SshHop::new("   ", 22, "x");
        blank.host = "   ".into();
        let mut spaced = SshHop::new("  bastion  ", 0, "  jump  ");
        spaced.password = Some("secret".into());
        let hops = list_jump_hops(Some(&JumpSpec::Many(vec![blank, spaced])));
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0].host, "bastion");
        assert_eq!(hops[0].port, 22);
        assert_eq!(hops[0].username, "jump");
        assert_eq!(hops[0].password.as_deref(), Some("secret"));
    }

    #[test]
    fn empty_jump_encodes_to_none_and_labels_empty() {
        assert!(encode_jump(&[]).is_none());
        assert_eq!(jump_label(None), "");
        assert!(list_jump_hops(None).is_empty());
    }
}
