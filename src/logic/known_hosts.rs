//! Trust-on-first-use known-hosts store from `src/main/ssh/knownHosts.ts`.
//!
//! A first-seen key is reported and not written until [`KnownHosts::trust`].
//! A different fingerprint is rejected. The JSON file is mode `0o600`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownHost {
    pub host_id: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyVerdict {
    Trusted {
        first_use: bool,
        fingerprint: String,
    },
    Mismatch {
        fingerprint: String,
        expected: String,
    },
}

pub struct KnownHosts {
    path: PathBuf,
    cache: Option<Vec<(String, String)>>,
}

impl KnownHosts {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `SHA256:` plus unpadded standard base64 of the SHA-256 digest.
    pub fn fingerprint_of(key: &[u8]) -> String {
        let digest = sha256(key);
        let mut encoded = base64_encode(&digest);
        while encoded.ends_with('=') {
            encoded.pop();
        }
        format!("SHA256:{encoded}")
    }

    /// Check a host key without recording it.
    pub fn verify(&mut self, host_id: &str, key: &[u8]) -> HostKeyVerdict {
        let fingerprint = Self::fingerprint_of(key);
        let store = self.load();
        match store.iter().find(|(id, _)| id == host_id) {
            None => HostKeyVerdict::Trusted {
                first_use: true,
                fingerprint,
            },
            Some((_, expected)) if expected == &fingerprint => HostKeyVerdict::Trusted {
                first_use: false,
                fingerprint,
            },
            Some((_, expected)) => HostKeyVerdict::Mismatch {
                fingerprint,
                expected: expected.clone(),
            },
        }
    }

    /// Record a fingerprint after the operator accepted it.
    pub fn trust(&mut self, host_id: &str, fingerprint: &str) -> io::Result<()> {
        let store = self.load();
        if let Some((_, existing)) = store.iter_mut().find(|(id, _)| id == host_id) {
            *existing = fingerprint.to_string();
        } else {
            store.push((host_id.to_string(), fingerprint.to_string()));
        }
        self.persist()
    }

    /// Trusted hosts, sorted by `host_id`.
    pub fn list(&mut self) -> Vec<KnownHost> {
        let mut hosts: Vec<KnownHost> = self
            .load()
            .iter()
            .map(|(host_id, fingerprint)| KnownHost {
                host_id: host_id.clone(),
                fingerprint: fingerprint.clone(),
            })
            .collect();
        hosts.sort_by(|a, b| a.host_id.cmp(&b.host_id));
        hosts
    }

    /// Forget a trusted host. No-op when the host is absent (the file is left untouched).
    pub fn remove(&mut self, host_id: &str) -> io::Result<bool> {
        let store = self.load();
        let before = store.len();
        store.retain(|(id, _)| id != host_id);
        if store.len() == before {
            return Ok(false);
        }
        self.persist()?;
        Ok(true)
    }

    fn load(&mut self) -> &mut Vec<(String, String)> {
        if self.cache.is_none() {
            self.cache = Some(read_store(&self.path));
        }
        self.cache.as_mut().unwrap()
    }

    fn persist(&self) -> io::Result<()> {
        let entries = self.cache.as_deref().unwrap_or(&[]);
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let body = stringify_store(entries);
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.path)?;
        file.write_all(body.as_bytes())?;
        let _ = fs::set_permissions(&self.path, mode_600());
        Ok(())
    }
}

fn mode_600() -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    fs::Permissions::from_mode(0o600)
}

fn read_store(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_store(&text).unwrap_or_default()
}

fn stringify_store(entries: &[(String, String)]) -> String {
    if entries.is_empty() {
        return "{}".to_string();
    }
    let mut out = String::from("{\n");
    for (index, (key, value)) in entries.iter().enumerate() {
        out.push_str("  ");
        out.push_str(&json_string(key));
        out.push_str(": ");
        out.push_str(&json_string(value));
        if index + 1 != entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push('}');
    out
}

fn parse_store(text: &str) -> Option<Vec<(String, String)>> {
    let value = parse_json(text)?;
    let Json::Object(pairs) = value else {
        return Some(Vec::new());
    };
    let mut out = Vec::new();
    for (key, value) in pairs {
        if let Json::String(fingerprint) = value {
            out.push((key, fingerprint));
        }
    }
    Some(out)
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

fn parse_json(text: &str) -> Option<Json> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        index: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.index != parser.bytes.len() {
        return None;
    }
    Some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.index += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.index += 1;
        Some(b)
    }

    fn parse_value(&mut self) -> Option<Json> {
        self.skip_ws();
        match self.peek()? {
            b'n' => self.consume(b"null").then_some(Json::Null),
            b't' => self.consume(b"true").then_some(Json::Bool(true)),
            b'f' => self.consume(b"false").then_some(Json::Bool(false)),
            b'"' => self.parse_string().map(Json::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => None,
        }
    }

    fn consume(&mut self, literal: &[u8]) -> bool {
        if self.bytes[self.index..].starts_with(literal) {
            self.index += literal.len();
            true
        } else {
            false
        }
    }

    fn parse_object(&mut self) -> Option<Json> {
        self.bump()?;
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.bump();
                break;
            }
            if !pairs.is_empty() {
                if self.bump()? != b',' {
                    return None;
                }
                self.skip_ws();
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bump()? != b':' {
                return None;
            }
            let value = self.parse_value()?;
            pairs.push((key, value));
        }
        Some(Json::Object(pairs))
    }

    fn parse_array(&mut self) -> Option<Json> {
        self.bump()?;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.bump();
                break;
            }
            if !items.is_empty() {
                if self.bump()? != b',' {
                    return None;
                }
            }
            items.push(self.parse_value()?);
        }
        Some(Json::Array(items))
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.bump()? != b'"' {
            return None;
        }
        let mut out = String::new();
        loop {
            match self.bump()? {
                b'"' => return Some(out),
                b'\\' => {
                    let esc = self.bump()?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut hex = [0u8; 4];
                            for slot in &mut hex {
                                *slot = self.bump()?;
                            }
                            let text = std::str::from_utf8(&hex).ok()?;
                            let code = u32::from_str_radix(text, 16).ok()?;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return None,
                    }
                }
                byte => {
                    if byte < 0x80 {
                        out.push(byte as char);
                    } else {
                        self.index -= 1;
                        let rest = &self.bytes[self.index..];
                        let text = std::str::from_utf8(rest).ok()?;
                        let ch = text.chars().next()?;
                        out.push(ch);
                        self.index += ch.len_utf8();
                    }
                }
            }
        }
    }

    fn parse_number(&mut self) -> Option<Json> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.index += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index]).ok()?;
        let number = text.parse::<f64>().ok()?;
        Some(Json::Number(number))
    }
}

fn sha256(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hash: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (message.len() as u64).saturating_mul(8);
    let mut data = message.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }
    let mut out = [0u8; 32];
    for (i, word) in hash.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "devterm-kh-{}-{}-{}",
            std::process::id(),
            nanos,
            name
        ))
    }

    #[test]
    fn fingerprint_matches_openssh_sha256_base64() {
        assert_eq!(
            KnownHosts::fingerprint_of(b"hello"),
            "SHA256:LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ"
        );
    }

    #[test]
    fn first_use_is_not_persisted_until_trust() {
        let path = temp_path("tofu.json");
        let mut store = KnownHosts::open(&path);
        let verdict = store.verify("example.test:22", b"key-a");
        match verdict {
            HostKeyVerdict::Trusted {
                first_use,
                fingerprint,
            } => {
                assert!(first_use);
                assert!(fingerprint.starts_with("SHA256:"));
            }
            other => panic!("expected first use, got {other:?}"),
        }
        assert!(!path.exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn trust_writes_mode_600_json_and_mismatch_is_rejected() {
        let path = temp_path("trust.json");
        let mut store = KnownHosts::open(&path);
        let HostKeyVerdict::Trusted { fingerprint, .. } = store.verify("b.example:22", b"key-b")
        else {
            panic!("first use");
        };
        store.trust("b.example:22", &fingerprint).unwrap();
        store.trust("a.example:22", "SHA256:other").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(
            body,
            format!(
                "{{\n  \"b.example:22\": \"{fingerprint}\",\n  \"a.example:22\": \"SHA256:other\"\n}}"
            )
        );
        assert!(body.contains("\"b.example:22\""));
        assert!(body.starts_with("{\n  \"b.example:22\": "));
        match store.verify("b.example:22", b"key-b") {
            HostKeyVerdict::Trusted { first_use, .. } => assert!(!first_use),
            other => panic!("{other:?}"),
        }
        match store.verify("b.example:22", b"different") {
            HostKeyVerdict::Mismatch {
                fingerprint: seen,
                expected,
            } => {
                assert_ne!(seen, expected);
                assert_eq!(expected, fingerprint);
            }
            other => panic!("expected mismatch, got {other:?}"),
        }
        let listed = store.list();
        assert_eq!(listed[0].host_id, "a.example:22");
        assert_eq!(listed[1].host_id, "b.example:22");
        assert!(!store.remove("missing").unwrap());
        assert!(store.remove("b.example:22").unwrap());
        match store.verify("b.example:22", b"key-b") {
            HostKeyVerdict::Trusted { first_use, .. } => assert!(first_use),
            other => panic!("{other:?}"),
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn corrupt_json_loads_as_an_empty_store() {
        let path = temp_path("bad.json");
        fs::write(&path, "not json").unwrap();
        let mut store = KnownHosts::open(&path);
        match store.verify("h:22", b"k") {
            HostKeyVerdict::Trusted { first_use, .. } => assert!(first_use),
            other => panic!("{other:?}"),
        }
        let _ = fs::remove_file(&path);
    }
}
