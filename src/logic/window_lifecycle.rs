//! Window close vs tray hide, and the exits that actually end the process.
//!
//! Port of `src/main/window-lifecycle.ts` plus the close/quit comments in
//! `src/main/index.ts`. `keepSessionsInTray` turns the window X into a hide:
//! the window disappears and the process stays up with its sessions. Explicit
//! Quit, OS shutdown, and an installer kill all exit. There is no supervisor
//! that restarts the process after those.

/// Decide whether a user-originated BrowserWindow close should become a tray hide.
/// Explicit quit (`allow_window_close`) and the headless self-test always keep
/// the normal close path.
pub fn should_hide_to_tray_on_close(input: HideToTrayInput) -> bool {
    input.keep_sessions_in_tray && !input.allow_window_close && !input.self_test
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HideToTrayInput {
    pub keep_sessions_in_tray: bool,
    pub allow_window_close: bool,
    pub self_test: bool,
}

/// Why the process is being asked to go away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// The window's X button (not Quit).
    WindowX,
    /// Menu / tray "Quit DevTerm", which sets `allowWindowClose` in `before-quit`.
    Quit,
    /// The OS is shutting down or logging off. Treated as a real quit.
    OsShutdown,
    /// NSIS / installer is force-closing DevTerm so it can overwrite the install dir.
    InstallerKill,
}

/// What the close should do. `HideProcessStays` does not stop agents or PTYs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    /// Window hides to the tray. The process stays.
    HideProcessStays,
    /// Process exits. Sessions are torn down; nothing supervises a restart.
    Exit,
}

/// Window X hides when tray-resident mode is on. Quit, OS shutdown, and
/// installer kill always exit — DevTerm does not add a supervisor.
pub fn decide_close(reason: CloseReason, input: HideToTrayInput) -> CloseOutcome {
    match reason {
        CloseReason::Quit | CloseReason::OsShutdown | CloseReason::InstallerKill => {
            CloseOutcome::Exit
        }
        CloseReason::WindowX => {
            if should_hide_to_tray_on_close(input) {
                CloseOutcome::HideProcessStays
            } else {
                CloseOutcome::Exit
            }
        }
    }
}

/// True when this reason ends the process. Quit, OS shutdown, and installer
/// kill always do. A tray hide does not.
pub fn process_exits(reason: CloseReason, input: HideToTrayInput) -> bool {
    decide_close(reason, input) == CloseOutcome::Exit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(keep: bool, allow: bool, self_test: bool) -> HideToTrayInput {
        HideToTrayInput {
            keep_sessions_in_tray: keep,
            allow_window_close: allow,
            self_test,
        }
    }

    #[test]
    fn hides_instead_of_closing_when_enabled_for_a_normal_user_close() {
        assert!(should_hide_to_tray_on_close(input(true, false, false)));
        assert_eq!(
            decide_close(CloseReason::WindowX, input(true, false, false)),
            CloseOutcome::HideProcessStays
        );
    }

    #[test]
    fn keeps_explicit_quit_on_the_destructive_close_path() {
        assert!(!should_hide_to_tray_on_close(input(true, true, false)));
        assert_eq!(
            decide_close(CloseReason::Quit, input(true, false, false)),
            CloseOutcome::Exit
        );
    }

    #[test]
    fn never_hides_the_self_test_window() {
        assert!(!should_hide_to_tray_on_close(input(true, false, true)));
    }

    #[test]
    fn os_shutdown_and_installer_kill_exit_without_a_supervisor() {
        let tray = input(true, false, false);
        assert_eq!(
            decide_close(CloseReason::OsShutdown, tray),
            CloseOutcome::Exit
        );
        assert_eq!(
            decide_close(CloseReason::InstallerKill, tray),
            CloseOutcome::Exit
        );
        assert!(process_exits(CloseReason::Quit, tray));
        assert!(process_exits(CloseReason::OsShutdown, tray));
        assert!(process_exits(CloseReason::InstallerKill, tray));
        assert!(!process_exits(CloseReason::WindowX, tray));
    }
}
