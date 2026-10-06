//! Minimal OpenSSH config parser from `src/main/ssh/ssh-config-parse.ts`.
//!
//! Concrete `Host` entries only. `Host *` (and other wildcard-only blocks)
//! supply defaults and are not imported. Duplicate aliases are skipped.
//! Passwords are never read.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedSshHost {
    pub alias: String,
    pub host: String,
    pub port: i64,
    pub username: Option<String>,
    pub private_key_path: Option<String>,
    pub jump: Option<ParsedJumpValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedJump {
    pub host: String,
    pub port: i64,
    pub username: Option<String>,
}

/// One hop stays an object; two hops stay an array. Never more than two.
#[derive(Clone, Debug, PartialEq)]
pub enum ParsedJumpValue {
    One(ParsedJump),
    Chain(Vec<ParsedJump>),
}

struct HostBlock {
    patterns: Vec<String>,
    opts: HashMap<String, String>,
}

fn strip_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (index, ch) in line.char_indices() {
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single {
            in_double = !in_double;
        } else if ch == '#' && !in_single && !in_double {
            return &line[..index];
        }
    }
    line
}

fn strip_one_wrapping_quote(value: &str) -> String {
    let mut chars: Vec<char> = value.chars().collect();
    if chars
        .first()
        .copied()
        .is_some_and(|c| c == '"' || c == '\'')
    {
        chars.remove(0);
    }
    if chars.last().copied().is_some_and(|c| c == '"' || c == '\'') {
        chars.pop();
    }
    chars.into_iter().collect()
}

fn split_kw(line: &str) -> Option<(String, String)> {
    let trimmed = strip_comment(line).trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    if !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let mut index = 1;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' || bytes[index] == b'-')
    {
        index += 1;
    }
    if index >= bytes.len() {
        return None;
    }
    let key = trimmed[..index].to_ascii_lowercase();
    let rest = &trimmed[index..];
    let value = if rest.as_bytes().first() == Some(&b'=') {
        rest[1..].trim_start()
    } else if rest.starts_with(|c: char| c.is_ascii_whitespace()) {
        let rest_bytes = rest.as_bytes();
        let mut j = 0;
        while j < rest_bytes.len() && rest_bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let after = &rest[j..];
        if after.starts_with('=') {
            after[1..].trim_start()
        } else {
            after
        }
    } else {
        return None;
    };
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some((key, strip_one_wrapping_quote(value)))
}

fn is_wildcard_pattern(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('!')
}

fn js_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    trimmed.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn positive_port(raw: &str) -> Option<i64> {
    let number = js_number(raw)?;
    if number > 0.0 && number.fract() == 0.0 && number <= i64::MAX as f64 {
        Some(number as i64)
    } else {
        None
    }
}

/// Parse one ProxyJump token: `[user@]host[:port]` or `[user@][ipv6][:port]`.
fn parse_one_jump(raw: &str) -> Option<ParsedJump> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut user_end = 0;
    while user_end < bytes.len()
        && bytes[user_end] != b'@'
        && bytes[user_end] != b'['
        && bytes[user_end] != b']'
    {
        user_end += 1;
    }
    let (username, rest) = if user_end > 0 && user_end < bytes.len() && bytes[user_end] == b'@' {
        (Some(text[..user_end].to_string()), &text[user_end + 1..])
    } else {
        (None, text)
    };
    if rest.is_empty() {
        return None;
    }
    let rest_bytes = rest.as_bytes();
    let (host_token, after_host) = if rest_bytes[0] == b'[' {
        let end = rest.find(']')?;
        (&rest[..=end], &rest[end + 1..])
    } else {
        match rest.find(':') {
            Some(colon) if colon == 0 => return None,
            Some(colon) => (&rest[..colon], &rest[colon..]),
            None => (rest, ""),
        }
    };
    if host_token.is_empty() {
        return None;
    }
    let port = if after_host.is_empty() {
        22
    } else if let Some(digits) = after_host.strip_prefix(':') {
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        positive_port(digits)?
    } else {
        return None;
    };
    let host = strip_jump_brackets(host_token);
    if host.is_empty() {
        return None;
    }
    Some(ParsedJump {
        host,
        port,
        username: username.filter(|name| !name.is_empty()),
    })
}

fn strip_jump_brackets(host: &str) -> String {
    let mut out = host.to_string();
    if out.starts_with('[') {
        out.remove(0);
    }
    if out.ends_with(']') {
        out.pop();
    }
    out
}

/// Parse a ProxyJump value. Only the first comma-separated hop is used.
pub fn parse_proxy_jump(raw: &str) -> Option<ParsedJump> {
    parse_one_jump(raw.split(',').next().unwrap_or(""))
}

/// Parse a comma-separated ProxyJump list (capped at two extra hops).
pub fn parse_proxy_jump_list(raw: &str) -> Vec<ParsedJump> {
    raw.split(',').filter_map(parse_one_jump).take(2).collect()
}

/// Parse OpenSSH config text into concrete Host entries.
///
/// `Host *` blocks supply defaults and are not imported. Multi-pattern `Host`
/// lines produce one entry per concrete pattern, using the pattern as the
/// alias. The first value of a keyword wins inside a block and across
/// wildcard blocks. Passwords are not imported.
pub fn parse_ssh_config(text: &str) -> Vec<ParsedSshHost> {
    let mut blocks: Vec<HostBlock> = Vec::new();
    let mut current: Option<HostBlock> = None;
    for raw_line in text.split('\n') {
        let raw_line = raw_line.trim_end_matches('\r');
        let Some((key, value)) = split_kw(raw_line) else {
            continue;
        };
        if key == "host" {
            if let Some(block) = current.take() {
                blocks.push(block);
            }
            current = Some(HostBlock {
                patterns: value.split_whitespace().map(|s| s.to_string()).collect(),
                opts: HashMap::new(),
            });
            continue;
        }
        if let Some(block) = current.as_mut() {
            block.opts.entry(key).or_insert(value);
        }
    }
    if let Some(block) = current {
        blocks.push(block);
    }

    let mut globals: HashMap<String, String> = HashMap::new();
    for block in &blocks {
        if !block.patterns.is_empty() && block.patterns.iter().all(|p| is_wildcard_pattern(p)) {
            for (key, value) in &block.opts {
                globals.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }

    let mut out = Vec::new();
    let mut seen_alias = HashSet::new();
    for block in &blocks {
        let concrete: Vec<&String> = block
            .patterns
            .iter()
            .filter(|pattern| !is_wildcard_pattern(pattern))
            .collect();
        if concrete.is_empty() {
            continue;
        }
        let mut opts = globals.clone();
        for (key, value) in &block.opts {
            opts.insert(key.clone(), value.clone());
        }
        for alias in concrete {
            if !seen_alias.insert(alias.clone()) {
                continue;
            }
            let host = opt_nonempty(opts.get("hostname")).unwrap_or_else(|| alias.clone());
            let port = match opts.get("port") {
                Some(raw) if !raw.is_empty() => match positive_port(raw) {
                    Some(port) => port,
                    None => continue,
                },
                _ => 22,
            };
            let username =
                opt_nonempty(opts.get("user")).or_else(|| opt_nonempty(opts.get("username")));
            let private_key_path = opt_nonempty(opts.get("identityfile"));
            let jump = if let Some(proxy) = opt_nonempty(opts.get("proxyjump")) {
                let hops = parse_proxy_jump_list(&proxy);
                match hops.len() {
                    0 => None,
                    1 => Some(ParsedJumpValue::One(hops.into_iter().next().unwrap())),
                    _ => Some(ParsedJumpValue::Chain(hops)),
                }
            } else {
                None
            };
            out.push(ParsedSshHost {
                alias: alias.clone(),
                host,
                port,
                username,
                private_key_path,
                jump,
            });
        }
    }
    out
}

fn opt_nonempty(value: Option<&String>) -> Option<String> {
    value.and_then(|text| {
        if text.is_empty() {
            None
        } else {
            Some(text.clone())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_at_host_port() {
        assert_eq!(
            parse_proxy_jump("jump@bastion.example:2222"),
            Some(ParsedJump {
                host: "bastion.example".into(),
                port: 2222,
                username: Some("jump".into()),
            })
        );
    }

    #[test]
    fn defaults_port_to_22() {
        assert_eq!(
            parse_proxy_jump("bastion"),
            Some(ParsedJump {
                host: "bastion".into(),
                port: 22,
                username: None,
            })
        );
    }

    #[test]
    fn uses_the_first_hop_of_a_comma_list() {
        let jump = parse_proxy_jump("a@h1:22,b@h2:22").unwrap();
        assert_eq!(jump.host, "h1");
    }

    #[test]
    fn parses_a_two_hop_proxy_jump_list() {
        let hops = parse_proxy_jump_list("a@h1:22,b@h2:2222");
        assert_eq!(hops.len(), 2);
        assert_eq!(hops[1].host, "h2");
        assert_eq!(hops[1].port, 2222);
    }

    #[test]
    fn parses_bracketed_ipv6_and_rejects_a_zero_port() {
        let jump = parse_proxy_jump("[::1]:2222").unwrap();
        assert_eq!(jump.host, "::1");
        assert_eq!(jump.port, 2222);
        assert!(parse_proxy_jump("a@b:0").is_none());
        assert!(parse_proxy_jump("host:").is_none());
        let capped = parse_proxy_jump_list("a@h1,b@h2,c@h3");
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[1].host, "h2");
    }

    #[test]
    fn imports_a_concrete_host_with_hostname_user_port_identity_file() {
        let text = "
Host prod
  HostName 10.0.0.5
  User deploy
  Port 2222
  IdentityFile ~/.ssh/prod_ed25519
";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts.len(), 1);
        assert_eq!(
            hosts[0],
            ParsedSshHost {
                alias: "prod".into(),
                host: "10.0.0.5".into(),
                port: 2222,
                username: Some("deploy".into()),
                private_key_path: Some("~/.ssh/prod_ed25519".into()),
                jump: None,
            }
        );
    }

    #[test]
    fn applies_host_star_defaults_without_importing_the_wildcard() {
        let text = "
Host *
  User ubuntu
  IdentityFile ~/.ssh/id_ed25519

Host web
  HostName web.internal
";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "web");
        assert_eq!(hosts[0].username.as_deref(), Some("ubuntu"));
        assert_eq!(
            hosts[0].private_key_path.as_deref(),
            Some("~/.ssh/id_ed25519")
        );
    }

    #[test]
    fn skips_host_patterns_that_are_only_wildcards() {
        let text = "
Host *.example.com
  User root
Host *
  Port 22
";
        assert_eq!(parse_ssh_config(text).len(), 0);
    }

    #[test]
    fn parses_proxy_jump() {
        let text = "
Host app
  HostName 10.1.2.3
  User app
  ProxyJump jump@bastion:22
";
        let hosts = parse_ssh_config(text);
        match &hosts[0].jump {
            Some(ParsedJumpValue::One(jump)) => {
                assert_eq!(jump.host, "bastion");
                assert_eq!(jump.username.as_deref(), Some("jump"));
            }
            other => panic!("expected a single jump, got {other:?}"),
        }
    }

    #[test]
    fn parses_a_proxy_jump_chain() {
        let text = "
Host app
  HostName 10.1.2.3
  ProxyJump jump@bastion:22,inner@mid:22
";
        let hosts = parse_ssh_config(text);
        match &hosts[0].jump {
            Some(ParsedJumpValue::Chain(hops)) => {
                assert_eq!(hops.len(), 2);
                assert_eq!(hops[1].host, "mid");
            }
            other => panic!("expected a jump chain, got {other:?}"),
        }
    }

    #[test]
    fn uses_host_alias_as_hostname_when_hostname_is_omitted() {
        let text = "Host github.com\n  User git\n";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts[0].host, "github.com");
        assert_eq!(hosts[0].username.as_deref(), Some("git"));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let text = "
# production
Host p1
  # HostName is below
  HostName p1.example # trailing
";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts[0].host, "p1.example");
    }

    #[test]
    fn first_value_wins_and_duplicate_aliases_are_skipped() {
        let text = "
Host web
  User first
  User second
Host web
  User later
  HostName other
";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].username.as_deref(), Some("first"));
        assert_eq!(hosts[0].host, "web");
    }

    #[test]
    fn does_not_import_passwords_or_proxy_command() {
        let text = "
Host box
  HostName box.example
  ProxyCommand ssh -W %h:%p jumphost
";
        let hosts = parse_ssh_config(text);
        assert!(hosts[0].jump.is_none());
        assert_eq!(hosts[0].host, "box.example");
    }
}
