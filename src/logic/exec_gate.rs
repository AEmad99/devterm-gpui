//! Exec-channel gate from `src/main/ssh/exec-gate.ts`, plus the timeout
//! decision from `SSHManager.execOnClient`.
//!
//! A timeout resolves with `timed_out: true` and whatever stdout/stderr has
//! already arrived. It closes only the exec stream. It does not drop the SSH
//! connection.

use std::sync::{Arc, Condvar, Mutex};
use std::thread;

/// Parallel execs kept below a typical MaxSessions budget (shell + SFTP + git).
pub const POSIX_EXEC_SLOTS: usize = 4;

/// Caps how many SSH exec channels a session may hold open at once.
///
/// OpenSSH's default MaxSessions is 10, and that budget is shared with the
/// interactive shell, SFTP, and port forwards. Extra calls wait here instead
/// of opening another channel. `limit` below 1 is raised to 1.
pub fn create_exec_gate(limit: usize) -> ExecGate {
    let slots = limit.max(1);
    ExecGate {
        inner: Arc::new(Inner {
            state: Mutex::new(State {
                active: 0,
                slots,
                queued: 0,
            }),
            cv: Condvar::new(),
        }),
    }
}

#[derive(Clone)]
pub struct ExecGate {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    cv: Condvar,
}

struct State {
    active: usize,
    slots: usize,
    queued: usize,
}

/// Releases one slot when dropped. Explicit [`ExecPermit::release`] does the same.
pub struct ExecPermit {
    gate: ExecGate,
    alive: bool,
}

impl ExecGate {
    /// Block until a slot is free, then occupy it.
    pub fn acquire(&self) -> ExecPermit {
        let mut state = self.inner.state.lock().expect("exec gate lock");
        if state.active >= state.slots {
            state.queued += 1;
            while state.active >= state.slots {
                state = self.inner.cv.wait(state).expect("exec gate wait");
            }
            state.queued -= 1;
        }
        state.active += 1;
        ExecPermit {
            gate: self.clone(),
            alive: true,
        }
    }

    /// How many callers are blocked inside [`ExecGate::acquire`].
    pub fn queued(&self) -> usize {
        self.inner.state.lock().expect("exec gate lock").queued
    }

    fn release_slot(&self) {
        let mut state = self.inner.state.lock().expect("exec gate lock");
        state.active = state.active.saturating_sub(1);
        self.inner.cv.notify_one();
    }
}

impl ExecPermit {
    pub fn release(mut self) {
        self.alive = false;
        self.gate.release_slot();
    }
}

impl Drop for ExecPermit {
    fn drop(&mut self) {
        if self.alive {
            self.gate.release_slot();
        }
    }
}

/// Result of one exec channel. `drop_connection` is always false: a timeout
/// or a stream error must not be reported as a dead SSH session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub drop_connection: bool,
}

/// Pure model of `execOnClient`: bytes are decoded once at the end, a timeout
/// keeps the partial snapshot, and a later close cannot replace that result.
#[derive(Debug)]
pub struct ExecAttempt {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: Option<i32>,
    saw_exit: bool,
    settled: bool,
}

impl ExecAttempt {
    pub fn new() -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: None,
            saw_exit: false,
            settled: false,
        }
    }

    pub fn push_stdout(&mut self, data: &[u8]) {
        if !self.settled {
            self.stdout.extend_from_slice(data);
        }
    }

    pub fn push_stderr(&mut self, data: &[u8]) {
        if !self.settled {
            self.stderr.extend_from_slice(data);
        }
    }

    /// `exit` event. A non-number code clears the stored exit code, matching
    /// `exitCode = typeof code === 'number' ? code : null`.
    pub fn on_exit(&mut self, code: Option<i32>) {
        if self.settled {
            return;
        }
        self.saw_exit = true;
        self.exit_code = code;
    }

    /// Timeout resolves. Partial output is kept. The SSH connection stays up.
    pub fn on_timeout(&mut self) -> ExecOutput {
        if self.settled {
            return self.snapshot(true);
        }
        self.settled = true;
        ExecOutput {
            stdout: decode(&self.stdout),
            stderr: decode(&self.stderr),
            code: None,
            timed_out: true,
            drop_connection: false,
        }
    }

    /// Channel close. Exit code wins over the close code when both exist.
    /// Returns `None` once the attempt has already settled (timeout or close).
    pub fn on_close(&mut self, close_code: Option<i32>) -> Option<ExecOutput> {
        if self.settled {
            return None;
        }
        self.settled = true;
        let code = if self.saw_exit {
            self.exit_code
        } else {
            close_code
        };
        Some(ExecOutput {
            stdout: decode(&self.stdout),
            stderr: decode(&self.stderr),
            code,
            timed_out: false,
            drop_connection: false,
        })
    }

    /// Stream error rejects the exec promise and does not drop the connection.
    pub fn on_stream_error(&mut self, message: &str) -> Result<ExecOutput, String> {
        if self.settled {
            return Ok(self.snapshot(false));
        }
        self.settled = true;
        Err(message.to_string())
    }

    fn snapshot(&self, timed_out: bool) -> ExecOutput {
        ExecOutput {
            stdout: decode(&self.stdout),
            stderr: decode(&self.stderr),
            code: if timed_out { None } else { self.exit_code },
            timed_out,
            drop_connection: false,
        }
    }
}

impl Default for ExecAttempt {
    fn default() -> Self {
        Self::new()
    }
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn runs_up_to_the_slot_limit_and_queues_the_rest() {
        let gate = create_exec_gate(2);
        let order = Arc::new(Mutex::new(Vec::new()));
        let release_a = gate.acquire();
        let release_b = gate.acquire();
        let started = Arc::new(AtomicBool::new(false));
        let gate_c = gate.clone();
        let order_c = Arc::clone(&order);
        let started_c = Arc::clone(&started);
        let pending = thread::spawn(move || {
            let release = gate_c.acquire();
            started_c.store(true, Ordering::SeqCst);
            order_c.lock().unwrap().push("c");
            release.release();
        });
        while gate.queued() == 0 {
            thread::yield_now();
        }
        assert!(!started.load(Ordering::SeqCst));
        order.lock().unwrap().push("a");
        release_a.release();
        pending.join().unwrap();
        assert_eq!(order.lock().unwrap().as_slice(), ["a", "c"]);
        release_b.release();
    }

    #[test]
    fn slot_limit_below_one_still_allows_a_single_exec() {
        let gate = create_exec_gate(0);
        let permit = gate.acquire();
        let started = Arc::new(AtomicBool::new(false));
        let gate_c = gate.clone();
        let started_c = Arc::clone(&started);
        let pending = thread::spawn(move || {
            let release = gate_c.acquire();
            started_c.store(true, Ordering::SeqCst);
            release.release();
        });
        while gate.queued() == 0 {
            thread::yield_now();
        }
        assert!(!started.load(Ordering::SeqCst));
        permit.release();
        pending.join().unwrap();
        assert!(started.load(Ordering::SeqCst));
    }

    #[test]
    fn timeout_returns_timed_out_partial_output_and_does_not_drop_connection() {
        let mut attempt = ExecAttempt::new();
        attempt.push_stdout(&[0xc3]);
        attempt.push_stdout(&[0xa9, b'h', b'i']);
        attempt.push_stderr(b"err");
        let out = attempt.on_timeout();
        assert!(out.timed_out);
        assert_eq!(out.stdout, "\u{00e9}hi");
        assert_eq!(out.stderr, "err");
        assert_eq!(out.code, None);
        assert!(!out.drop_connection);
        attempt.push_stdout(b"late");
        assert!(attempt.on_close(Some(0)).is_none());
        let again = attempt.on_timeout();
        assert_eq!(again.stdout, "\u{00e9}hi");
        assert!(again.timed_out);
        assert!(!again.drop_connection);
    }

    #[test]
    fn close_prefers_the_exit_code_and_keeps_the_connection() {
        let mut attempt = ExecAttempt::new();
        attempt.push_stdout(b"ok");
        attempt.on_exit(Some(0));
        let out = attempt.on_close(Some(1)).unwrap();
        assert!(!out.timed_out);
        assert!(!out.drop_connection);
        assert_eq!(out.code, Some(0));
        assert_eq!(out.stdout, "ok");
    }

    #[test]
    fn close_without_exit_uses_the_close_code() {
        let mut attempt = ExecAttempt::new();
        let out = attempt.on_close(Some(127)).unwrap();
        assert_eq!(out.code, Some(127));
        assert!(!out.timed_out);
        assert!(!out.drop_connection);
    }

    #[test]
    fn stream_error_rejects_without_dropping_the_connection() {
        let mut attempt = ExecAttempt::new();
        attempt.push_stdout(b"partial");
        let err = attempt.on_stream_error("channel reset").unwrap_err();
        assert_eq!(err, "channel reset");
        assert!(attempt.on_close(Some(1)).is_none());
    }

    #[test]
    fn posix_slot_budget_is_four() {
        assert_eq!(POSIX_EXEC_SLOTS, 4);
    }
}
