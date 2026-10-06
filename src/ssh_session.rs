//! One primary SSH session per DevTerm session.
//!
//! Direct hops set TCP no-delay. The first host key is stored (TOFU) in
//! `known_hosts.json` mode 0o600. A mismatch is rejected. The system agent is
//! the default when no key or password is set. Windows looks at
//! `\\.\pipe\openssh-ssh-agent`.

use std::sync::Arc;

use russh::client::{self, Handler};
use russh::ChannelMsg;
use tokio::net::TcpStream;

use crate::logic::known_hosts::{HostKeyVerdict, KnownHosts};
use crate::logic::shell_integration::{
    build_posix_shell_integration_setup, consume_shell_integration_ready,
    SHELL_INTEGRATION_IDLE_MS, STTY_DISABLE_ECHO, STTY_ENABLE_ECHO,
};
use crate::logic::ssh_auth::{system_agent_path, WINDOWS_SSH_AGENT_PIPE};
use crate::persist;
use crate::term_view::{host_platform, PtyMsg};

pub struct RemoteLink {
    pub input: std::sync::mpsc::Sender<Vec<u8>>,
    pub output: std::sync::mpsc::Receiver<PtyMsg>,
}

pub fn connect(host: &str, user: &str, port: u16) -> RemoteLink {
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (output_tx, output_rx) = std::sync::mpsc::channel::<PtyMsg>();
    let host = host.to_string();
    let user = user.to_string();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = output_tx.send(PtyMsg::Data(format!("SSH: {err}\r\n").into_bytes()));
                let _ = output_tx.send(PtyMsg::Exit(None));
                return;
            }
        };
        if let Err(err) =
            runtime.block_on(run_session(host, user, port, input_rx, output_tx.clone()))
        {
            let _ = output_tx.send(PtyMsg::Data(format!("SSH: {err}\r\n").into_bytes()));
            let _ = output_tx.send(PtyMsg::Exit(None));
        }
    });
    RemoteLink {
        input: input_tx,
        output: output_rx,
    }
}

struct KeyHandler {
    known: KnownHosts,
    host_id: String,
    rejected: Arc<std::sync::Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl Handler for KeyHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let bytes = server_public_key.to_bytes().unwrap_or_default();
        match self.known.verify(&self.host_id, &bytes) {
            HostKeyVerdict::Mismatch {
                fingerprint,
                expected,
            } => {
                if let Ok(mut slot) = self.rejected.lock() {
                    *slot = Some(format!(
                        "Host key mismatch for {} (got {fingerprint}, expected {expected})",
                        self.host_id
                    ));
                }
                Ok(false)
            }
            HostKeyVerdict::Trusted {
                first_use,
                fingerprint,
            } => {
                if first_use {
                    self.known.trust(&self.host_id, &fingerprint)?;
                }
                Ok(true)
            }
        }
    }
}

async fn run_session(
    host: String,
    user: String,
    port: u16,
    input: std::sync::mpsc::Receiver<Vec<u8>>,
    output: std::sync::mpsc::Sender<PtyMsg>,
) -> anyhow::Result<()> {
    let host_id = format!("{host}:{port}");
    let rejected = Arc::new(std::sync::Mutex::new(None));
    let handler = KeyHandler {
        known: KnownHosts::open(persist::user_data_dir().join("known_hosts.json")),
        host_id: host_id.clone(),
        rejected: rejected.clone(),
    };
    let stream = TcpStream::connect((host.as_str(), port)).await?;
    stream.set_nodelay(true)?;
    let config = Arc::new(client::Config::default());
    let mut session = client::connect_stream(config, stream, handler).await?;
    if let Some(reason) = rejected.lock().ok().and_then(|slot| slot.clone()) {
        anyhow::bail!(reason);
    }
    if !authenticate(&mut session, &user).await? {
        anyhow::bail!("authentication failed");
    }
    let mut channel = session.channel_open_session().await?;
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await?;
    channel.request_shell(true).await?;
    let _ = output.send(PtyMsg::Data(
        format!("Connected to {user}@{host_id}\r\n").into_bytes(),
    ));
    inject_shell_integration(&mut channel, &output).await;
    let input = Arc::new(std::sync::Mutex::new(input));
    loop {
        let input_wait = input.clone();
        tokio::select! {
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        if output.send(PtyMsg::Data(data.to_vec())).is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if output.send(PtyMsg::Data(data.to_vec())).is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        let _ = output.send(PtyMsg::Exit(Some(exit_status as i32)));
                        break;
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        let _ = output.send(PtyMsg::Exit(None));
                        break;
                    }
                    _ => {}
                }
            }
            incoming = tokio::task::spawn_blocking(move || input_wait.lock().ok().and_then(|guard| guard.recv().ok())) => {
                match incoming {
                    Ok(Some(bytes)) => {
                        channel.data(&bytes[..]).await?;
                    }
                    _ => break,
                }
            }
        }
    }
    Ok(())
}

async fn authenticate(
    session: &mut client::Handle<KeyHandler>,
    user: &str,
) -> anyhow::Result<bool> {
    let platform = host_platform();
    let sock = std::env::var("SSH_AUTH_SOCK").ok();
    let path = system_agent_path(sock.as_deref(), platform);
    if platform == "win32" {
        #[cfg(windows)]
        if let Ok(mut agent) =
            russh_keys::agent::client::AgentClient::connect_named_pipe(WINDOWS_SSH_AGENT_PIPE).await
        {
            if try_agent(session, user, &mut agent).await? {
                return Ok(true);
            }
        }
        let _ = WINDOWS_SSH_AGENT_PIPE;
    } else if let Some(path) = path {
        if let Ok(mut agent) = russh_keys::agent::client::AgentClient::connect_uds(path).await {
            if try_agent(session, user, &mut agent).await? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn try_agent<S>(
    session: &mut client::Handle<KeyHandler>,
    user: &str,
    agent: &mut russh_keys::agent::client::AgentClient<S>,
) -> anyhow::Result<bool>
where
    S: russh_keys::agent::client::AgentStream + Unpin + Send + 'static,
{
    let keys = agent.request_identities().await.unwrap_or_default();
    for key in keys {
        if session
            .authenticate_publickey_with(user, key, agent)
            .await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn inject_shell_integration(
    channel: &mut russh::Channel<client::Msg>,
    output: &std::sync::mpsc::Sender<PtyMsg>,
) {
    tokio::time::sleep(std::time::Duration::from_millis(SHELL_INTEGRATION_IDLE_MS)).await;
    let setup = build_posix_shell_integration_setup(None);
    let script = format!("{STTY_DISABLE_ECHO}{setup}\n{STTY_ENABLE_ECHO}");
    let _ = channel.data(script.as_bytes()).await;
    let mut carry = String::new();
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_millis(8_000) {
        let wait =
            tokio::time::timeout(std::time::Duration::from_millis(400), channel.wait()).await;
        let Ok(Some(ChannelMsg::Data { data })) = wait else {
            continue;
        };
        let text = String::from_utf8_lossy(&data);
        let ready = consume_shell_integration_ready(&carry, &text);
        carry = ready.carry;
        if !ready.text.is_empty() {
            let _ = output.send(PtyMsg::Data(ready.text.into_bytes()));
        }
        if ready.ready {
            let _ = channel.data(STTY_ENABLE_ECHO.as_bytes()).await;
            break;
        }
    }
}
