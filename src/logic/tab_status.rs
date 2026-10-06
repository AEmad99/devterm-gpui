//! Per-session tab status. The dot only needs a tone; the reason is the tooltip.
//!
//! Precedence: error > pending approval > needs attention > reconnecting/bridge
//! > running > unread > idle. A closed session stays idle.
//!
//! Tone colors follow the theme CSS variables set in `themes.ts`:
//! warn and pending use terminal yellow, error uses red, attention uses green,
//! running uses blue, unread uses magenta.

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStatusTone {
    Idle,
    Warn,
    Error,
    Pending,
    Attention,
    Running,
    Unread,
}

impl TabStatusTone {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Pending => "pending",
            Self::Attention => "attention",
            Self::Running => "running",
            Self::Unread => "unread",
        }
    }
}

/// CSS custom property for the tone, when the dot is colored.
pub fn tab_status_css_var(tone: TabStatusTone) -> Option<&'static str> {
    match tone {
        TabStatusTone::Warn => Some("--tab-status-warn"),
        TabStatusTone::Error => Some("--tab-status-error"),
        TabStatusTone::Pending => Some("--tab-status-pending"),
        TabStatusTone::Attention => Some("--tab-status-attention"),
        TabStatusTone::Running => Some("--tab-status-running"),
        TabStatusTone::Unread => Some("--tab-status-unread"),
        TabStatusTone::Idle => None,
    }
}

/// Terminal palette slot the theme copies into the status variable.
pub fn tab_status_palette_slot(tone: TabStatusTone) -> Option<&'static str> {
    match tone {
        TabStatusTone::Warn | TabStatusTone::Pending => Some("yellow"),
        TabStatusTone::Error => Some("red"),
        TabStatusTone::Attention => Some("green"),
        TabStatusTone::Running => Some("blue"),
        TabStatusTone::Unread => Some("magenta"),
        TabStatusTone::Idle => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentBridgeState {
    Starting,
    Listening,
    Connected,
    Disconnected,
    Stopped,
    Error,
}

impl AgentBridgeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Listening => "listening",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Stopped => "stopped",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStatus {
    pub tone: TabStatusTone,
    pub reason: Option<String>,
    pub bridge_state: Option<AgentBridgeState>,
    pub pending_approval: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionStatusInput {
    pub status: Option<String>,
    pub closed: bool,
    pub agent_bridge_state: Option<AgentBridgeState>,
    pub agent_pending_approval: bool,
    pub needs_attention: bool,
    pub has_unread_output: bool,
    pub process_running: bool,
    pub exit_code: Option<i32>,
}

fn is_error_status(status: Option<&str>) -> bool {
    let Some(status) = status else {
        return false;
    };
    let s = status.to_lowercase();
    s.starts_with("error:")
        || s.starts_with("failed:")
        || s.contains("host key mismatch")
        || s.contains("reconnect failed")
}

fn is_reconnecting_status(status: Option<&str>) -> bool {
    status
        .map(|s| s.to_lowercase().starts_with("reconnecting"))
        .unwrap_or(false)
}

pub fn derive_tab_status(s: &SessionStatusInput) -> TabStatus {
    if is_error_status(s.status.as_deref()) || matches!(s.exit_code, Some(code) if code != 0) {
        let reason = if matches!(s.exit_code, Some(code) if code != 0) {
            format!("exited with code {}", s.exit_code.unwrap())
        } else {
            s.status.clone().unwrap_or_else(|| "error".to_string())
        };
        return TabStatus {
            tone: TabStatusTone::Error,
            reason: Some(reason),
            bridge_state: s.agent_bridge_state,
            pending_approval: Some(s.agent_pending_approval),
        };
    }
    if s.agent_pending_approval {
        return TabStatus {
            tone: TabStatusTone::Warn,
            reason: Some("Agent awaiting approval".to_string()),
            bridge_state: s.agent_bridge_state,
            pending_approval: Some(true),
        };
    }
    if s.needs_attention {
        return TabStatus {
            tone: TabStatusTone::Attention,
            reason: Some("Needs your attention".to_string()),
            bridge_state: s.agent_bridge_state,
            pending_approval: None,
        };
    }
    if !s.closed && is_reconnecting_status(s.status.as_deref()) {
        return TabStatus {
            tone: TabStatusTone::Pending,
            reason: s.status.clone(),
            bridge_state: s.agent_bridge_state,
            pending_approval: None,
        };
    }
    if !s.closed && s.agent_bridge_state == Some(AgentBridgeState::Starting) {
        return TabStatus {
            tone: TabStatusTone::Pending,
            reason: Some("Starting agent bridge".to_string()),
            bridge_state: s.agent_bridge_state,
            pending_approval: None,
        };
    }
    if !s.closed && s.agent_bridge_state == Some(AgentBridgeState::Error) {
        return TabStatus {
            tone: TabStatusTone::Error,
            reason: Some("Agent bridge error".to_string()),
            bridge_state: s.agent_bridge_state,
            pending_approval: None,
        };
    }
    if s.process_running {
        return TabStatus {
            tone: TabStatusTone::Running,
            reason: Some("Process running".to_string()),
            bridge_state: s.agent_bridge_state,
            pending_approval: None,
        };
    }
    if s.has_unread_output {
        return TabStatus {
            tone: TabStatusTone::Unread,
            reason: Some("Unread output".to_string()),
            bridge_state: s.agent_bridge_state,
            pending_approval: None,
        };
    }
    TabStatus {
        tone: TabStatusTone::Idle,
        reason: None,
        bridge_state: s.agent_bridge_state,
        pending_approval: Some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> SessionStatusInput {
        SessionStatusInput::default()
    }

    #[test]
    fn idle_when_nothing_is_set() {
        assert_eq!(derive_tab_status(&input()).tone, TabStatusTone::Idle);
    }

    #[test]
    fn error_wins_over_pending_approval() {
        let mut s = input();
        s.agent_pending_approval = true;
        s.status = Some("error: handshake failed".into());
        assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Error);
    }

    #[test]
    fn pending_approval_wins_over_needs_attention() {
        let mut s = input();
        s.agent_pending_approval = true;
        s.needs_attention = true;
        let got = derive_tab_status(&s);
        assert_eq!(got.tone, TabStatusTone::Warn);
        assert_eq!(got.pending_approval, Some(true));
        assert_eq!(got.reason.as_deref(), Some("Agent awaiting approval"));
    }

    #[test]
    fn needs_attention_wins_over_running() {
        let mut s = input();
        s.needs_attention = true;
        s.process_running = true;
        assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Attention);
        assert_eq!(
            derive_tab_status(&s).reason.as_deref(),
            Some("Needs your attention")
        );
    }

    #[test]
    fn reconnecting_shows_pending_when_not_closed() {
        let mut s = input();
        s.status = Some("reconnecting".into());
        assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Pending);
    }

    #[test]
    fn reconnecting_on_a_closed_session_is_idle() {
        let mut s = input();
        s.status = Some("reconnecting".into());
        s.closed = true;
        assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Idle);
    }

    #[test]
    fn bridge_error_shows_error_tone() {
        let mut s = input();
        s.agent_bridge_state = Some(AgentBridgeState::Error);
        let got = derive_tab_status(&s);
        assert_eq!(got.tone, TabStatusTone::Error);
        assert!(got.reason.unwrap().contains("Agent bridge error"));
    }

    #[test]
    fn bridge_starting_shows_pending_tone() {
        let mut s = input();
        s.agent_bridge_state = Some(AgentBridgeState::Starting);
        let got = derive_tab_status(&s);
        assert_eq!(got.tone, TabStatusTone::Pending);
        assert!(got.reason.unwrap().contains("Starting agent bridge"));
    }

    #[test]
    fn running_beats_unread() {
        let mut s = input();
        s.process_running = true;
        s.has_unread_output = true;
        assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Running);
    }

    #[test]
    fn unread_is_shown_when_nothing_else_is_set() {
        let mut s = input();
        s.has_unread_output = true;
        assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Unread);
        assert_eq!(
            derive_tab_status(&s).reason.as_deref(),
            Some("Unread output")
        );
    }

    #[test]
    fn non_zero_exit_code_is_an_error() {
        let mut s = input();
        s.exit_code = Some(1);
        let got = derive_tab_status(&s);
        assert_eq!(got.tone, TabStatusTone::Error);
        assert!(got.reason.unwrap().contains("exited with code 1"));
    }

    #[test]
    fn zero_exit_code_is_not_an_error() {
        let mut s = input();
        s.exit_code = Some(0);
        assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Idle);
    }

    #[test]
    fn status_string_classification() {
        for status in [
            "error: handshake failed",
            "failed: cannot connect",
            "host key mismatch",
            "reconnect failed: timed out",
        ] {
            let mut s = input();
            s.status = Some(status.into());
            assert_eq!(derive_tab_status(&s).tone, TabStatusTone::Error, "{status}");
        }
        for status in ["reconnecting", "reconnecting (attempt 2/5)"] {
            let mut s = input();
            s.status = Some(status.into());
            assert_eq!(
                derive_tab_status(&s).tone,
                TabStatusTone::Pending,
                "{status}"
            );
        }
        let mut closed = input();
        closed.status = Some("closed".into());
        assert_eq!(derive_tab_status(&closed).tone, TabStatusTone::Idle);
        let mut reconnected = input();
        reconnected.status = Some("reconnected".into());
        assert_eq!(derive_tab_status(&reconnected).tone, TabStatusTone::Idle);
    }

    #[test]
    fn tone_colors_match_the_theme_slots() {
        assert_eq!(tab_status_palette_slot(TabStatusTone::Warn), Some("yellow"));
        assert_eq!(tab_status_palette_slot(TabStatusTone::Pending), Some("yellow"));
        assert_eq!(tab_status_palette_slot(TabStatusTone::Error), Some("red"));
        assert_eq!(tab_status_palette_slot(TabStatusTone::Attention), Some("green"));
        assert_eq!(tab_status_palette_slot(TabStatusTone::Running), Some("blue"));
        assert_eq!(tab_status_palette_slot(TabStatusTone::Unread), Some("magenta"));
        assert_eq!(tab_status_css_var(TabStatusTone::Error), Some("--tab-status-error"));
        assert_eq!(tab_status_css_var(TabStatusTone::Unread), Some("--tab-status-unread"));
        assert_eq!(tab_status_css_var(TabStatusTone::Idle), None);
    }
}
