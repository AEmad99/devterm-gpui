//! SSH port-forward bookkeeping from `src/main/ssh/port-forward.ts`.
//!
//! `local` (-L) and `dynamic` (-D) SOCKS5 are supported. SOCKS5 is no-auth and
//! CONNECT only. Specs survive a dropped transport: listeners are suspended
//! and [`PortForwardManager::rebind`] brings back the ones whose local port
//! is still free. A bind failure during rebind drops that spec.

use std::fs::File;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SSH_SESSION_NOT_CONNECTED: &str = "SSH session not connected";
pub const LOCAL_FORWARD_REQUIRES_TARGET: &str = "Local forwards require a remote host and port";

/// SOCKS5 success reply: VER=5 REP=0 RSV=0 ATYP=IPv4 0.0.0.0:0.
pub const SOCKS5_SUCCESS_REPLY: [u8; 10] = [0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
/// SOCKS5 generic failure reply used when `forwardOut` fails.
pub const SOCKS5_FAILURE_REPLY: [u8; 10] = [0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortForwardKind {
    Local,
    Dynamic,
}

impl PortForwardKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Dynamic => "dynamic",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortForward {
    pub id: String,
    pub session_id: String,
    pub kind: PortForwardKind,
    pub local_port: u16,
    pub remote_host: Option<String>,
    pub remote_port: Option<u16>,
    pub created_at: i64,
    pub bytes: u64,
}

struct ForwardEntry {
    forward: PortForward,
    listening: bool,
    bytes_in: u64,
    bytes_out: u64,
}

pub struct PortForwardManager {
    forwards: Vec<ForwardEntry>,
    now_ms: i64,
    seq: u64,
}

impl PortForwardManager {
    pub fn new() -> Self {
        Self {
            forwards: Vec::new(),
            now_ms: 0,
            seq: 0,
        }
    }

    pub fn set_now(&mut self, now_ms: i64) {
        self.now_ms = now_ms;
    }

    pub fn add(
        &mut self,
        connected: bool,
        session_id: &str,
        kind: PortForwardKind,
        local_port: u16,
        remote_host: Option<&str>,
        remote_port: Option<u16>,
        bind: impl FnOnce(u16) -> Result<u16, String>,
    ) -> Result<PortForward, String> {
        if !connected {
            return Err(SSH_SESSION_NOT_CONNECTED.to_string());
        }
        let remote_host = remote_host.and_then(|host| {
            if host.is_empty() {
                None
            } else {
                Some(host.to_string())
            }
        });
        if kind == PortForwardKind::Local && (remote_host.is_none() || remote_port.is_none()) {
            return Err(LOCAL_FORWARD_REQUIRES_TARGET.to_string());
        }
        let actual_port = bind(local_port)?;
        self.seq += 1;
        let forward = PortForward {
            id: format!("pf-{}", uuid_v4()),
            session_id: session_id.to_string(),
            kind,
            local_port: actual_port,
            remote_host,
            remote_port,
            created_at: self.now_ms,
            bytes: 0,
        };
        let listed = forward.clone();
        self.forwards.push(ForwardEntry {
            forward,
            listening: true,
            bytes_in: 0,
            bytes_out: 0,
        });
        let _ = self.seq;
        Ok(listed)
    }

    pub fn note_bytes_in(&mut self, id: &str, count: u64) {
        if let Some(entry) = self.forwards.iter_mut().find(|entry| entry.forward.id == id) {
            entry.bytes_in += count;
        }
    }

    pub fn note_bytes_out(&mut self, id: &str, count: u64) {
        if let Some(entry) = self.forwards.iter_mut().find(|entry| entry.forward.id == id) {
            entry.bytes_out += count;
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.forwards.len();
        self.forwards.retain(|entry| entry.forward.id != id);
        before != self.forwards.len()
    }

    pub fn remove_by_session(&mut self, session_id: &str) {
        self.forwards
            .retain(|entry| entry.forward.session_id != session_id);
    }

    /// Close listeners but keep the specs so a reconnect can rebind them.
    pub fn suspend_by_session(&mut self, session_id: &str) {
        for entry in &mut self.forwards {
            if entry.forward.session_id == session_id && entry.listening {
                entry.listening = false;
            }
        }
    }

    /// Re-establish suspended listeners. Specs whose port can no longer be bound are dropped.
    pub fn rebind(&mut self, session_id: &str, mut bind: impl FnMut(u16) -> Result<u16, String>) {
        let mut index = 0;
        while index < self.forwards.len() {
            let entry = &self.forwards[index];
            if entry.forward.session_id != session_id || entry.listening {
                index += 1;
                continue;
            }
            let requested = entry.forward.local_port;
            match bind(requested) {
                Ok(actual) => {
                    let entry = &mut self.forwards[index];
                    entry.listening = true;
                    entry.forward.local_port = actual;
                    index += 1;
                }
                Err(_) => {
                    self.forwards.remove(index);
                }
            }
        }
    }

    pub fn list(&self, session_id: Option<&str>) -> Vec<PortForward> {
        self.forwards
            .iter()
            .filter(|entry| session_id.map(|id| entry.forward.session_id == id).unwrap_or(true))
            .map(|entry| {
                let mut forward = entry.forward.clone();
                forward.bytes = entry.bytes_in + entry.bytes_out;
                forward
            })
            .collect()
    }

    pub fn is_listening(&self, id: &str) -> bool {
        self.forwards
            .iter()
            .find(|entry| entry.forward.id == id)
            .map(|entry| entry.listening)
            .unwrap_or(false)
    }
}

impl Default for PortForwardManager {
    fn default() -> Self {
        Self::new()
    }
}

fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = File::open("/dev/urandom") {
        let _ = file.read_exact(&mut bytes);
    } else {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        bytes.copy_from_slice(&nanos.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocksTarget {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Greeting,
    Request,
}

/// Incremental SOCKS5 no-auth + CONNECT handshake.
///
/// Greeting and request routinely arrive in separate TCP segments. Replies
/// already written are returned from [`Socks5Handshake::push`] and are not
/// repeated.
pub struct Socks5Handshake {
    buf: Vec<u8>,
    phase: Phase,
    done: Option<Option<SocksTarget>>,
}

impl Socks5Handshake {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            phase: Phase::Greeting,
            done: None,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done.is_some()
    }

    pub fn target(&self) -> Option<&SocksTarget> {
        self.done.as_ref().and_then(|value| value.as_ref())
    }

    /// Feed the next TCP chunk. Returned buffers are written to the client
    /// (the method-selection reply). `None` in [`Self::target`] after
    /// [`Self::is_done`] means the handshake failed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        if self.done.is_some() {
            return Vec::new();
        }
        self.buf.extend_from_slice(chunk);
        let mut replies = Vec::new();
        if self.phase == Phase::Greeting {
            if self.buf.len() < 2 {
                return replies;
            }
            if self.buf[0] != 0x05 {
                self.done = Some(None);
                return replies;
            }
            let n_methods = self.buf[1] as usize;
            if self.buf.len() < 2 + n_methods {
                return replies;
            }
            replies.push(vec![0x05, 0x00]);
            self.buf.drain(..2 + n_methods);
            self.phase = Phase::Request;
        }
        if self.buf.len() < 4 {
            return replies;
        }
        if self.buf[0] != 0x05 {
            self.done = Some(None);
            return replies;
        }
        if self.buf[1] != 0x01 {
            self.done = Some(None);
            return replies;
        }
        let atyp = self.buf[3];
        let parsed = if atyp == 0x01 {
            if self.buf.len() < 10 {
                return replies;
            }
            let host = format!(
                "{}.{}.{}.{}",
                self.buf[4], self.buf[5], self.buf[6], self.buf[7]
            );
            let port = u16::from_be_bytes([self.buf[8], self.buf[9]]);
            Some(SocksTarget { host, port })
        } else if atyp == 0x03 {
            if self.buf.len() < 5 {
                return replies;
            }
            let len = self.buf[4] as usize;
            if self.buf.len() < 4 + 1 + len + 2 {
                return replies;
            }
            let host = String::from_utf8_lossy(&self.buf[5..5 + len]).into_owned();
            let port_at = 5 + len;
            let port = u16::from_be_bytes([self.buf[port_at], self.buf[port_at + 1]]);
            Some(SocksTarget { host, port })
        } else if atyp == 0x04 {
            if self.buf.len() < 22 {
                return replies;
            }
            let mut parts = Vec::new();
            for group in 0..8 {
                let offset = 4 + group * 2;
                let value = u16::from_be_bytes([self.buf[offset], self.buf[offset + 1]]);
                parts.push(format!("{value:x}"));
            }
            let port = u16::from_be_bytes([self.buf[20], self.buf[21]]);
            Some(SocksTarget {
                host: parts.join(":"),
                port,
            })
        } else {
            self.done = Some(None);
            return replies;
        };
        self.done = Some(parsed);
        replies
    }
}

impl Default for Socks5Handshake {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_forward_when_the_session_is_down_and_local_needs_a_target() {
        let mut manager = PortForwardManager::new();
        let err = manager
            .add(false, "s1", PortForwardKind::Dynamic, 1080, None, None, |_| Ok(1080))
            .unwrap_err();
        assert_eq!(err, SSH_SESSION_NOT_CONNECTED);
        let err = manager
            .add(
                true,
                "s1",
                PortForwardKind::Local,
                8080,
                None,
                Some(80),
                |_| Ok(8080),
            )
            .unwrap_err();
        assert_eq!(err, LOCAL_FORWARD_REQUIRES_TARGET);
        let err = manager
            .add(
                true,
                "s1",
                PortForwardKind::Local,
                8080,
                Some(""),
                Some(80),
                |_| Ok(8080),
            )
            .unwrap_err();
        assert_eq!(err, LOCAL_FORWARD_REQUIRES_TARGET);
    }

    #[test]
    fn suspend_keeps_specs_and_rebind_drops_ports_that_cannot_be_bound() {
        let mut manager = PortForwardManager::new();
        manager.set_now(50);
        let local = manager
            .add(
                true,
                "s1",
                PortForwardKind::Local,
                0,
                Some("10.0.0.5"),
                Some(80),
                |_| Ok(40000),
            )
            .unwrap();
        let dynamic = manager
            .add(true, "s1", PortForwardKind::Dynamic, 1080, None, None, |port| {
                Ok(port)
            })
            .unwrap();
        assert!(local.id.starts_with("pf-"));
        assert_eq!(local.local_port, 40000);
        assert_eq!(local.created_at, 50);
        assert_eq!(dynamic.kind, PortForwardKind::Dynamic);
        manager.note_bytes_in(&local.id, 5);
        manager.note_bytes_out(&local.id, 7);
        assert_eq!(manager.list(Some("s1"))[0].bytes, 12);
        manager.suspend_by_session("s1");
        assert!(!manager.is_listening(&local.id));
        assert!(!manager.is_listening(&dynamic.id));
        assert_eq!(manager.list(Some("s1")).len(), 2);
        manager.rebind("s1", |port| {
            if port == 40000 {
                Err("address in use".into())
            } else {
                Ok(port)
            }
        });
        let left = manager.list(None);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, dynamic.id);
        assert!(manager.is_listening(&dynamic.id));
        assert_eq!(left[0].bytes, 0);
        manager.remove_by_session("s1");
        assert!(manager.list(None).is_empty());
    }

    #[test]
    fn socks5_connect_handles_split_packets_ipv4_domain_and_ipv6() {
        let mut handshake = Socks5Handshake::new();
        assert!(handshake.push(&[0x05]).is_empty());
        assert!(!handshake.is_done());
        let greeting = handshake.push(&[0x01, 0x00]);
        assert_eq!(greeting, vec![vec![0x05, 0x00]]);
        assert!(!handshake.is_done());
        let replies = handshake.push(&[0x05, 0x01, 0x00, 0x01, 1, 2, 3, 4, 0x00, 0x50]);
        assert!(replies.is_empty());
        assert_eq!(
            handshake.target(),
            Some(&SocksTarget {
                host: "1.2.3.4".into(),
                port: 80,
            })
        );

        let mut both = Socks5Handshake::new();
        let replies = both.push(&[
            0x05, 0x01, 0x00, 0x05, 0x01, 0x00, 0x03, 0x0b, b'e', b'x', b'a', b'm', b'p', b'l',
            b'e', b'.', b'c', b'o', b'm', 0x01, 0xbb,
        ]);
        assert_eq!(replies, vec![vec![0x05, 0x00]]);
        assert_eq!(
            both.target(),
            Some(&SocksTarget {
                host: "example.com".into(),
                port: 443,
            })
        );

        let mut ipv6 = Socks5Handshake::new();
        let mut packet = vec![0x05, 0x01, 0x00, 0x05, 0x01, 0x00, 0x04];
        packet.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        packet.extend_from_slice(&22u16.to_be_bytes());
        ipv6.push(&packet);
        assert_eq!(
            ipv6.target(),
            Some(&SocksTarget {
                host: "0:0:0:0:0:0:0:1".into(),
                port: 22,
            })
        );

        let mut bind_cmd = Socks5Handshake::new();
        bind_cmd.push(&[0x05, 0x01, 0x00, 0x05, 0x02, 0x00, 0x01]);
        assert!(bind_cmd.is_done());
        assert!(bind_cmd.target().is_none());
        assert_eq!(SOCKS5_SUCCESS_REPLY[1], 0x00);
        assert_eq!(SOCKS5_FAILURE_REPLY[1], 0x01);
    }

    #[test]
    fn dynamic_forward_does_not_require_a_remote_target() {
        let mut manager = PortForwardManager::new();
        let forward = manager
            .add(true, "s", PortForwardKind::Dynamic, 0, None, None, |_| Ok(1080))
            .unwrap();
        assert_eq!(forward.kind.as_str(), "dynamic");
        assert_eq!(forward.remote_host, None);
        assert!(manager.remove("missing") == false);
        assert!(manager.remove(&forward.id));
    }
}
