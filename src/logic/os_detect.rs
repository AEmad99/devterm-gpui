//! Remote OS probe from `src/main/ssh/osDetect.ts`.
//!
//! `uname -a` classifies Unix. A Windows host is recognized only when
//! `cmd /c ver` returns a Windows version banner. A POSIX "command not found"
//! stays `unknown`. An aborted transport raises
//! `SSH transport closed during startup`. A timed-out probe returns stderr
//! `timeout` and, if the channel opens late, closes it so it cannot overlap
//! the interactive shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const SSH_TRANSPORT_CLOSED: &str = "SSH transport closed during startup";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostContext {
    pub kind: &'static str,
    pub os: String,
    pub detail: String,
    pub hostname: String,
}

#[derive(Clone, Debug)]
pub struct AbortFlag {
    aborted: Arc<AtomicBool>,
}

impl AbortFlag {
    pub fn new() -> Self {
        Self {
            aborted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }
}

impl Default for AbortFlag {
    fn default() -> Self {
        Self::new()
    }
}

enum Ev {
    Ready(Arc<RemoteChannel>),
    Fail(String),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Close(Option<i32>),
    StreamErr(String),
}

struct Gate {
    settled: bool,
}

/// Channel handed to a scripted SSH exec, matching ssh2's `ClientChannel`.
pub struct RemoteChannel {
    closed: AtomicBool,
    detached: AtomicBool,
    tx: Sender<Ev>,
}

impl RemoteChannel {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub fn emit_data(&self, bytes: &[u8]) {
        if self.detached.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(Ev::Stdout(bytes.to_vec()));
    }

    pub fn emit_stderr(&self, bytes: &[u8]) {
        if self.detached.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(Ev::Stderr(bytes.to_vec()));
    }

    pub fn emit_close(&self, code: Option<i32>) {
        if self.detached.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(Ev::Close(code));
    }

    pub fn emit_error(&self, message: &str) {
        if self.detached.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(Ev::StreamErr(message.to_string()));
    }
}

/// Callback token for `client.exec(command, callback)`.
pub struct ExecDone {
    tx: Sender<Ev>,
    gate: Arc<Mutex<Gate>>,
}

impl ExecDone {
    pub fn open_channel(&self) -> Arc<RemoteChannel> {
        Arc::new(RemoteChannel {
            closed: AtomicBool::new(false),
            detached: AtomicBool::new(false),
            tx: self.tx.clone(),
        })
    }

    /// `callback(err, channel)` from the TypeScript exec helper.
    pub fn callback(&self, result: Result<Arc<RemoteChannel>, String>) {
        let gate = self.gate.lock().expect("os detect gate");
        if gate.settled {
            if let Ok(channel) = result {
                channel.close();
            }
            return;
        }
        match result {
            Ok(channel) => {
                let _ = self.tx.send(Ev::Ready(channel));
            }
            Err(message) => {
                let _ = self.tx.send(Ev::Fail(message));
            }
        }
    }
}

pub trait SshExec {
    fn exec(&mut self, command: &str, done: ExecDone);
}

pub fn looks_like_windows_banner(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("microsoft windows") || lower.contains("windows [version")
}

pub fn classify_uname(uname: &str) -> &'static str {
    let lower = uname.to_ascii_lowercase();
    if lower.contains("linux") {
        "linux"
    } else if lower.contains("darwin") {
        "mac"
    } else if lower.contains("mingw")
        || lower.contains("msys")
        || lower.contains("cygwin")
        || lower.contains("windows")
    {
        "windows"
    } else {
        "unknown"
    }
}

pub fn detect_remote_context(
    client: &mut dyn SshExec,
    exec_timeout: Duration,
    abort: &AbortFlag,
) -> Result<HostContext, String> {
    let uname = run_exec(client, "uname -a", exec_timeout, abort);
    if abort.is_aborted() {
        return Err(SSH_TRANSPORT_CLOSED.to_string());
    }
    if uname.code == Some(0) && !uname.stdout.trim().is_empty() {
        let detail = uname.stdout.trim().to_string();
        let host = run_exec(client, "hostname", exec_timeout, abort);
        if abort.is_aborted() {
            return Err(SSH_TRANSPORT_CLOSED.to_string());
        }
        let hostname = {
            let trimmed = host.stdout.trim();
            if !trimmed.is_empty() {
                trimmed.to_string()
            } else {
                hostname_fallback(&detail)
            }
        };
        return Ok(HostContext {
            kind: "remote",
            os: classify_uname(&detail).to_string(),
            detail,
            hostname,
        });
    }

    let ver = run_exec(client, "cmd /c \"ver & hostname\"", exec_timeout, abort);
    if abort.is_aborted() {
        return Err(SSH_TRANSPORT_CLOSED.to_string());
    }
    let out = format!("{}\n{}", ver.stdout, ver.stderr);
    let out = out.trim().to_string();
    if ver.code == Some(0) && looks_like_windows_banner(&out) {
        let lines: Vec<String> = split_crlf(&out)
            .into_iter()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();
        return Ok(HostContext {
            kind: "remote",
            os: "windows".into(),
            detail: lines.first().cloned().unwrap_or_else(|| "Windows".into()),
            hostname: lines.last().cloned().unwrap_or_else(|| "remote".into()),
        });
    }
    let detail = {
        let uname_err = uname.stderr.trim();
        if !uname_err.is_empty() {
            uname_err.to_string()
        } else if !out.is_empty() {
            out
        } else {
            "remote OS probe inconclusive".into()
        }
    };
    Ok(HostContext {
        kind: "remote",
        os: "unknown".into(),
        detail,
        hostname: "remote".into(),
    })
}

fn hostname_fallback(detail: &str) -> String {
    let mut parts = detail.split(' ');
    let _ = parts.next();
    match parts.next() {
        Some(part) if !part.is_empty() => part.to_string(),
        _ => "remote".into(),
    }
}

fn split_crlf(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            out.push(&text[start..end]);
            index += 1;
            start = index;
        } else {
            index += 1;
        }
    }
    out.push(&text[start..]);
    out
}

fn run_exec(
    client: &mut dyn SshExec,
    command: &str,
    timeout: Duration,
    abort: &AbortFlag,
) -> ExecOutput {
    if abort.is_aborted() {
        return abort_output();
    }
    let (tx, rx) = mpsc::channel();
    let gate = Arc::new(Mutex::new(Gate { settled: false }));
    let done = ExecDone {
        tx,
        gate: Arc::clone(&gate),
    };
    client.exec(command, done);
    wait_exec(rx, gate, timeout, abort)
}

fn abort_output() -> ExecOutput {
    ExecOutput {
        stdout: String::new(),
        stderr: SSH_TRANSPORT_CLOSED.to_string(),
        code: None,
    }
}

fn timeout_output() -> ExecOutput {
    ExecOutput {
        stdout: String::new(),
        stderr: "timeout".into(),
        code: None,
    }
}

fn wait_exec(
    rx: Receiver<Ev>,
    gate: Arc<Mutex<Gate>>,
    timeout: Duration,
    abort: &AbortFlag,
) -> ExecOutput {
    let deadline = Instant::now() + timeout;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stream: Option<Arc<RemoteChannel>> = None;
    loop {
        if abort.is_aborted() {
            settle(&gate, &mut stream, &rx);
            return abort_output();
        }
        let now = Instant::now();
        if now >= deadline {
            settle(&gate, &mut stream, &rx);
            return timeout_output();
        }
        let slice = (deadline - now).min(Duration::from_millis(5));
        match rx.recv_timeout(slice) {
            Ok(Ev::Ready(channel)) => {
                stream = Some(channel);
            }
            Ok(Ev::Fail(message)) => {
                settle(&gate, &mut stream, &rx);
                return ExecOutput {
                    stdout: String::new(),
                    stderr: message,
                    code: None,
                };
            }
            Ok(Ev::Stdout(bytes)) => stdout.extend(bytes),
            Ok(Ev::Stderr(bytes)) => stderr.extend(bytes),
            Ok(Ev::Close(code)) => {
                settle(&gate, &mut stream, &rx);
                return ExecOutput {
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    code,
                };
            }
            Ok(Ev::StreamErr(message)) => {
                settle(&gate, &mut stream, &rx);
                return ExecOutput {
                    stdout: String::new(),
                    stderr: message,
                    code: None,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                thread::sleep(slice);
            }
        }
    }
}

fn settle(gate: &Mutex<Gate>, stream: &mut Option<Arc<RemoteChannel>>, rx: &Receiver<Ev>) {
    {
        let mut guard = gate.lock().expect("os detect gate");
        guard.settled = true;
    }
    if let Some(channel) = stream.take() {
        channel.detached.store(true, Ordering::SeqCst);
        channel.close();
    }
    while let Ok(event) = rx.try_recv() {
        if let Ev::Ready(channel) = event {
            channel.detached.store(true, Ordering::SeqCst);
            channel.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NopClient;

    impl SshExec for NopClient {
        fn exec(&mut self, _command: &str, _done: ExecDone) {}
    }

    struct LateClient {
        channels: Arc<Mutex<Vec<Arc<RemoteChannel>>>>,
    }

    impl SshExec for LateClient {
        fn exec(&mut self, _command: &str, done: ExecDone) {
            let channel = done.open_channel();
            self.channels.lock().unwrap().push(Arc::clone(&channel));
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                done.callback(Ok(channel));
            });
        }
    }

    struct ScriptedClient {
        respond: fn(&str, &Arc<RemoteChannel>),
    }

    impl SshExec for ScriptedClient {
        fn exec(&mut self, command: &str, done: ExecDone) {
            let channel = done.open_channel();
            done.callback(Ok(Arc::clone(&channel)));
            (self.respond)(command, &channel);
        }
    }

    #[test]
    fn aborts_context_detection_when_the_transport_closes() {
        let abort = AbortFlag::new();
        let flag = abort.clone();
        let handle = thread::spawn(move || {
            let mut client = NopClient;
            detect_remote_context(&mut client, Duration::from_secs(3), &flag)
        });
        thread::sleep(Duration::from_millis(30));
        abort.abort();
        let err = handle.join().unwrap().unwrap_err();
        assert!(err.to_ascii_lowercase().contains("transport closed during startup"));
    }

    #[test]
    fn closes_exec_channels_whose_callbacks_arrive_after_timeout() {
        let channels = Arc::new(Mutex::new(Vec::new()));
        let mut client = LateClient {
            channels: Arc::clone(&channels),
        };
        let context = detect_remote_context(&mut client, Duration::from_millis(2), &AbortFlag::new())
            .unwrap();
        assert_eq!(context.os, "unknown");
        thread::sleep(Duration::from_millis(80));
        let channels = channels.lock().unwrap();
        assert_eq!(channels.len(), 2);
        assert!(channels.iter().all(|channel| channel.is_closed()));
    }

    #[test]
    fn does_not_treat_a_posix_command_not_found_as_windows() {
        let mut client = ScriptedClient {
            respond: |command, channel| {
                if command.starts_with("uname") {
                    channel.emit_stderr(b"bash: uname: command not found\n");
                    channel.emit_close(Some(127));
                } else {
                    channel.emit_stderr(b"bash: cmd: command not found\n");
                    channel.emit_close(Some(127));
                }
            },
        };
        let context = detect_remote_context(&mut client, Duration::from_millis(1000), &AbortFlag::new())
            .unwrap();
        assert_eq!(context.os, "unknown");
        assert!(!looks_like_windows_banner(
            "bash: powershell.exe: command not found"
        ));
        assert!(looks_like_windows_banner(
            "Microsoft Windows [Version 10.0.22631.3447]\nBASTION"
        ));
    }

    #[test]
    fn detects_a_windows_banner_from_cmd_ver() {
        let mut client = ScriptedClient {
            respond: |command, channel| {
                if command.starts_with("uname") {
                    channel.emit_close(Some(1));
                } else {
                    channel.emit_data(
                        b"Microsoft Windows [Version 10.0.22631.3447]\r\nBASTION\r\n",
                    );
                    channel.emit_close(Some(0));
                }
            },
        };
        let context = detect_remote_context(&mut client, Duration::from_millis(1000), &AbortFlag::new())
            .unwrap();
        assert_eq!(context.os, "windows");
        assert_eq!(context.hostname, "BASTION");
        assert_eq!(
            context.detail,
            "Microsoft Windows [Version 10.0.22631.3447]"
        );
    }

    #[test]
    fn classifies_linux_and_falls_back_to_the_uname_hostname_token() {
        let mut client = ScriptedClient {
            respond: |command, channel| {
                if command.starts_with("uname") {
                    channel.emit_data(b"Linux box 5.15.0\n");
                    channel.emit_close(Some(0));
                } else {
                    channel.emit_close(Some(0));
                }
            },
        };
        let context = detect_remote_context(&mut client, Duration::from_millis(1000), &AbortFlag::new())
            .unwrap();
        assert_eq!(context.os, "linux");
        assert_eq!(context.hostname, "box");
        assert_eq!(context.kind, "remote");
        assert_eq!(classify_uname("Darwin host 23.0.0"), "mac");
    }
}
