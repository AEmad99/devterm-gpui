//! Settings fields, normalizers, and export/import.
//!
//! Persist shape matches the renderer snapshot. Export strips connection secrets
//! (`password`, `passphrase`, `privateKeyPath`, including a nested jump hop) and
//! never carries webhook URLs, API keys, or tokens. Import merges through the
//! same normalizers. `sessionRestore` defaults on. The getting-started row is
//! `welcomeHintSeen`; importing settings keeps that flag, so a dismissed
//! "Getting started" hint is not shown again. Scrollback defaults to 10000 and
//! is clamped to 100–100000. Zen mode is a chrome flag and does not scale text.

#![allow(dead_code)]

use std::collections::BTreeMap;

pub const SCROLLBACK_DEFAULT: u32 = 10_000;
pub const SCROLLBACK_MIN: u32 = 100;
pub const SCROLLBACK_MAX: u32 = 100_000;
pub const DEFAULT_FONT_FAMILY: &str = "Cascadia Code, Consolas, \"Courier New\", monospace";
pub const DEFAULT_THEME_ID: &str = "tokyo-night";

/// Connection secret fields stripped on export. Matches settings-io.ts.
pub const CONNECTION_SECRET_FIELDS: &[&str] = &["password", "passphrase", "privateKeyPath"];

/// Extra secret keys that never travel in a settings export.
pub const EXPORT_SECRET_FIELDS: &[&str] = &[
    "password",
    "passphrase",
    "privateKeyPath",
    "webhookUrl",
    "telegramBotToken",
    "apiKey",
    "token",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn object(pairs: Vec<(&str, Json)>) -> Self {
        Json::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }
}

fn is_secret_key(key: &str, fields: &[&str]) -> bool {
    fields.iter().any(|f| *f == key)
}

pub fn strip_named_secrets(value: Json, fields: &[&str]) -> Json {
    match value {
        Json::Array(items) => Json::Array(
            items
                .into_iter()
                .map(|v| strip_named_secrets(v, fields))
                .collect(),
        ),
        Json::Object(pairs) => {
            let mut out = Vec::new();
            for (k, v) in pairs {
                if is_secret_key(&k, fields) {
                    continue;
                }
                out.push((k, strip_named_secrets(v, fields)));
            }
            Json::Object(out)
        }
        other => other,
    }
}

pub fn strip_export_secrets(value: Json) -> Json {
    strip_named_secrets(value, EXPORT_SECRET_FIELDS)
}

/// Strip `password`, `passphrase`, and `privateKeyPath` from a connection and
/// from each nested jump hop. Non-SSH protocols are dropped. `protocol` and
/// `domain` are removed from the portable record.
pub fn sanitize_connection(value: Json) -> Option<Json> {
    let Json::Object(pairs) = value else {
        return None;
    };
    if let Some(Json::String(protocol)) =
        pairs.iter().find(|(k, _)| k == "protocol").map(|(_, v)| v)
    {
        if protocol != "ssh" {
            return None;
        }
    }
    let mut obj = Json::Object(pairs);
    obj = strip_named_secrets(obj, CONNECTION_SECRET_FIELDS);
    let Json::Object(mut pairs) = obj else {
        return None;
    };
    pairs.retain(|(k, _)| k != "protocol" && k != "domain");
    if let Some(pos) = pairs.iter().position(|(k, _)| k == "jump") {
        let (_, jump) = pairs.remove(pos);
        let cleaned = match jump {
            Json::Array(hops) => Json::Array(
                hops.into_iter()
                    .map(|hop| strip_named_secrets(hop, CONNECTION_SECRET_FIELDS))
                    .collect(),
            ),
            other => strip_named_secrets(other, CONNECTION_SECRET_FIELDS),
        };
        pairs.push(("jump".into(), cleaned));
    }
    Some(Json::Object(pairs))
}

pub fn sanitize_connections(list: Vec<Json>) -> Vec<Json> {
    list.into_iter().filter_map(sanitize_connection).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalBg {
    pub color: String,
    pub image: Option<String>,
    pub dim: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BellStyle {
    None,
    Visual,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalPrefs {
    pub font_size: f64,
    pub font_family: String,
    pub line_height: f64,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub scrollback: u32,
    pub copy_on_select: bool,
    pub right_click_paste: bool,
    pub scroll_sensitivity: f64,
    pub bell: BellStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoReconnectSettings {
    pub enabled: bool,
    pub max_attempts: u32,
    pub base_delay_ms: u32,
    pub max_delay_ms: u32,
    pub factor: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttentionSettings {
    pub enabled: bool,
    pub sound: bool,
    pub volume: f64,
    pub system: bool,
    pub idle: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IdleNotify {
    pub enabled: bool,
    pub webhook_url: String,
    pub telegram_chat_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentPreferences {
    pub provider: String,
    pub model: String,
    pub fallback_models: Vec<String>,
    pub resume_sessions: bool,
    pub browser_tools: bool,
    pub agent_handoff: bool,
    pub trusted_skills: Vec<TrustedSkill>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrustedSkill {
    pub name: String,
    pub path: String,
    pub sha256: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DefaultShellPref {
    pub kind: String,
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyCombo {
    pub mod_key: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SttSettings {
    pub enabled: bool,
    pub model_id: String,
    pub language: String,
    pub append_space: bool,
    pub show_floating_status: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FirstRun {
    pub local_terminal: bool,
    pub imported_ssh: bool,
    pub opened_agent: bool,
    pub picked_theme: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pinned {
    pub connections: Vec<String>,
    pub snippets: Vec<String>,
    pub workspaces: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteConnectMode {
    Focus,
    Stagger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    Comfortable,
    Compact,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub theme_id: String,
    pub terminal_bg: TerminalBg,
    pub prefs: TerminalPrefs,
    pub auto_reconnect: AutoReconnectSettings,
    pub attention: AttentionSettings,
    pub idle_notify: IdleNotify,
    pub show_status_bar: bool,
    pub agent_activity_collapsed: bool,
    pub inactive_pane_dimming: bool,
    pub sftp_side_pane: bool,
    pub activity_indicators: bool,
    pub zen_mode: bool,
    pub agent_kind: String,
    pub agent_preferences: AgentPreferences,
    pub remote_detached_sessions: bool,
    pub session_restore: bool,
    pub hibernate_enabled: bool,
    pub keep_sessions_in_tray: bool,
    pub hibernate_after_ms: u64,
    pub output_ring_lines: u32,
    pub remote_connect_mode: RemoteConnectMode,
    pub transfers_panel_open: bool,
    pub default_shell: DefaultShellPref,
    pub git_panel_open: bool,
    pub keybindings: BTreeMap<String, KeyCombo>,
    pub stt: SttSettings,
    pub search_persist: bool,
    pub search_index_lines: u32,
    /// Getting started is dismissed when this is true. Import must not clear it.
    pub welcome_hint_seen: bool,
    pub first_run: FirstRun,
    pub density: Density,
    pub pinned: Pinned,
    pub last_connected_at: BTreeMap<String, f64>,
}

pub fn default_settings() -> AppSettings {
    AppSettings {
        theme_id: DEFAULT_THEME_ID.to_string(),
        terminal_bg: TerminalBg {
            color: "#16181d".into(),
            image: None,
            dim: 0.35,
        },
        prefs: TerminalPrefs {
            font_size: 14.0,
            font_family: DEFAULT_FONT_FAMILY.into(),
            line_height: 1.0,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            scrollback: SCROLLBACK_DEFAULT,
            copy_on_select: false,
            right_click_paste: false,
            scroll_sensitivity: 1.0,
            bell: BellStyle::None,
        },
        auto_reconnect: AutoReconnectSettings {
            enabled: true,
            max_attempts: 5,
            base_delay_ms: 1000,
            max_delay_ms: 30000,
            factor: 2.0,
        },
        attention: AttentionSettings {
            enabled: true,
            sound: true,
            volume: 0.5,
            system: true,
            idle: true,
        },
        idle_notify: IdleNotify {
            enabled: false,
            webhook_url: String::new(),
            telegram_chat_id: String::new(),
        },
        show_status_bar: true,
        agent_activity_collapsed: false,
        inactive_pane_dimming: true,
        sftp_side_pane: false,
        activity_indicators: true,
        zen_mode: false,
        agent_kind: "devterm".into(),
        agent_preferences: AgentPreferences {
            provider: String::new(),
            model: String::new(),
            fallback_models: Vec::new(),
            resume_sessions: true,
            browser_tools: true,
            agent_handoff: true,
            trusted_skills: Vec::new(),
        },
        remote_detached_sessions: true,
        session_restore: true,
        hibernate_enabled: true,
        keep_sessions_in_tray: false,
        hibernate_after_ms: 30_000,
        output_ring_lines: 10_000,
        remote_connect_mode: RemoteConnectMode::Focus,
        transfers_panel_open: false,
        default_shell: DefaultShellPref {
            kind: "auto".into(),
            path: None,
        },
        git_panel_open: false,
        keybindings: BTreeMap::new(),
        stt: SttSettings {
            enabled: true,
            model_id: "base".into(),
            language: "auto".into(),
            append_space: true,
            show_floating_status: true,
        },
        search_persist: false,
        search_index_lines: 2000,
        welcome_hint_seen: false,
        first_run: FirstRun {
            local_terminal: false,
            imported_ssh: false,
            opened_agent: false,
            picked_theme: false,
        },
        density: Density::Comfortable,
        pinned: Pinned {
            connections: Vec::new(),
            snippets: Vec::new(),
            workspaces: Vec::new(),
        },
        last_connected_at: BTreeMap::new(),
    }
}

pub fn normalize_scrollback(value: Option<f64>) -> u32 {
    let Some(value) = value else {
        return SCROLLBACK_DEFAULT;
    };
    if !value.is_finite() {
        return SCROLLBACK_DEFAULT;
    }
    let n = value.floor() as i64;
    n.clamp(SCROLLBACK_MIN as i64, SCROLLBACK_MAX as i64) as u32
}

pub fn normalize_hibernate_after_ms(value: Option<f64>) -> u64 {
    let Some(value) = value else {
        return 30_000;
    };
    if !value.is_finite() {
        return 30_000;
    }
    let n = value.floor() as i64;
    n.clamp(1_000, 24 * 60 * 60 * 1000) as u64
}

pub fn normalize_output_ring_lines(value: Option<f64>, fallback: u32) -> u32 {
    let Some(value) = value else {
        return fallback;
    };
    if !value.is_finite() {
        return fallback;
    }
    let n = value.floor() as i64;
    n.clamp(100, 100_000) as u32
}

pub fn normalize_search_index_lines(value: Option<f64>) -> u32 {
    let Some(value) = value else {
        return 2000;
    };
    if !value.is_finite() {
        return 2000;
    }
    let n = value.floor() as i64;
    n.clamp(200, 10_000) as u32
}

pub fn is_agent_kind(value: &str) -> bool {
    matches!(
        value,
        "devterm"
            | "claude"
            | "pi"
            | "opencode"
            | "kimi"
            | "grok"
            | "codex"
            | "antigravity"
            | "muse"
            | "cursor"
    )
}

pub fn normalize_agent_kind(value: Option<&str>, fallback: &str) -> String {
    match value {
        Some(v) if is_agent_kind(v) => v.to_string(),
        _ => fallback.to_string(),
    }
}

pub fn normalize_default_shell(kind: Option<&str>, path: Option<&str>) -> DefaultShellPref {
    match kind {
        Some("auto" | "pwsh" | "powershell" | "cmd") => DefaultShellPref {
            kind: kind.unwrap().to_string(),
            path: None,
        },
        Some("custom") if path.map(|p| !p.is_empty()).unwrap_or(false) => DefaultShellPref {
            kind: "custom".into(),
            path: path.map(|p| p.to_string()),
        },
        _ => DefaultShellPref {
            kind: "auto".into(),
            path: None,
        },
    }
}

const STT_MODELS: &[&str] = &["tiny", "base", "small"];
const STT_LANGUAGES: &[&str] = &[
    "auto", "en", "es", "fr", "de", "it", "pt", "nl", "ru", "zh", "ja", "ko", "ar", "hi",
];

pub fn normalize_stt(incoming: &SttSettings, fallback: &SttSettings) -> SttSettings {
    SttSettings {
        enabled: incoming.enabled,
        model_id: if STT_MODELS.contains(&incoming.model_id.as_str()) {
            incoming.model_id.clone()
        } else {
            fallback.model_id.clone()
        },
        language: if STT_LANGUAGES.contains(&incoming.language.as_str()) {
            incoming.language.clone()
        } else {
            fallback.language.clone()
        },
        append_space: incoming.append_space,
        show_floating_status: incoming.show_floating_status,
    }
}

pub fn normalize_agent_preferences(
    value: &AgentPreferences,
    fallback: &AgentPreferences,
) -> AgentPreferences {
    AgentPreferences {
        provider: value.provider.chars().take(120).collect(),
        model: value.model.chars().take(240).collect(),
        fallback_models: value
            .fallback_models
            .iter()
            .filter(|item| item.contains('/'))
            .map(|item| item.chars().take(240).collect::<String>())
            .take(12)
            .collect(),
        resume_sessions: value.resume_sessions,
        browser_tools: value.browser_tools,
        agent_handoff: value.agent_handoff,
        trusted_skills: value
            .trusted_skills
            .iter()
            .filter(|skill| {
                !skill.name.is_empty()
                    && !skill.path.is_empty()
                    && skill.sha256.len() == 64
                    && skill.sha256.chars().all(|c| c.is_ascii_hexdigit())
            })
            .map(|skill| TrustedSkill {
                name: skill.name.chars().take(120).collect(),
                path: skill.path.clone(),
                sha256: skill.sha256.to_lowercase(),
                enabled: skill.enabled,
            })
            .take(24)
            .collect(),
        ..fallback.clone()
    }
}

fn cap_strings(list: &[String]) -> Vec<String> {
    list.iter().take(200).cloned().collect()
}

pub fn normalize_pinned(pinned: &Pinned) -> Pinned {
    Pinned {
        connections: cap_strings(&pinned.connections),
        snippets: cap_strings(&pinned.snippets),
        workspaces: cap_strings(&pinned.workspaces),
    }
}

pub fn normalize_last_connected(map: &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
    map.iter()
        .filter(|(_, v)| v.is_finite())
        .map(|(k, v)| (k.chars().take(120).collect::<String>(), *v))
        .collect()
}

/// Partial snapshot carried by an export bundle. Getting started
/// (`welcome_hint_seen`), first-run progress, and idle-notify secrets are not
/// in the bundle.
#[derive(Clone, Debug)]
pub struct SettingsSnapshot {
    pub theme_id: Option<String>,
    pub scrollback: Option<f64>,
    pub font_size: Option<f64>,
    pub session_restore: Option<bool>,
    pub zen_mode: Option<bool>,
    pub hibernate_after_ms: Option<f64>,
    pub output_ring_lines: Option<f64>,
    pub search_index_lines: Option<f64>,
    pub agent_kind: Option<String>,
    pub show_status_bar: Option<bool>,
    pub density: Option<Density>,
    pub remote_connect_mode: Option<RemoteConnectMode>,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            theme_id: None,
            scrollback: None,
            font_size: None,
            session_restore: None,
            zen_mode: None,
            hibernate_after_ms: None,
            output_ring_lines: None,
            search_index_lines: None,
            agent_kind: None,
            show_status_bar: None,
            density: None,
            remote_connect_mode: None,
        }
    }
}

pub fn merge_snapshot_with_defaults(raw: &SettingsSnapshot) -> AppSettings {
    let mut out = default_settings();
    if let Some(theme) = &raw.theme_id {
        out.theme_id = theme.clone();
    }
    if raw.scrollback.is_some() {
        out.prefs.scrollback = normalize_scrollback(raw.scrollback);
    }
    if let Some(size) = raw.font_size {
        if size.is_finite() {
            out.prefs.font_size = size;
        }
    }
    if let Some(v) = raw.session_restore {
        out.session_restore = v;
    }
    if let Some(v) = raw.zen_mode {
        out.zen_mode = v;
    }
    if raw.hibernate_after_ms.is_some() {
        out.hibernate_after_ms = normalize_hibernate_after_ms(raw.hibernate_after_ms);
    }
    if raw.output_ring_lines.is_some() {
        out.output_ring_lines =
            normalize_output_ring_lines(raw.output_ring_lines, out.output_ring_lines);
    }
    if raw.search_index_lines.is_some() {
        out.search_index_lines = normalize_search_index_lines(raw.search_index_lines);
    }
    out.agent_kind = normalize_agent_kind(raw.agent_kind.as_deref(), &out.agent_kind);
    if let Some(v) = raw.show_status_bar {
        out.show_status_bar = v;
    }
    if let Some(v) = raw.density {
        out.density = v;
    }
    if let Some(v) = raw.remote_connect_mode {
        out.remote_connect_mode = v;
    }
    out.welcome_hint_seen = false;
    out
}

/// Apply an imported snapshot onto the live settings.
/// `welcome_hint_seen` and `first_run` stay as they are, so dismissing
/// Getting started survives import. Idle-notify secrets stay local.
/// Zen mode does not change the font size unless the bundle sets `font_size`.
pub fn apply_imported(current: &AppSettings, incoming: &SettingsSnapshot) -> AppSettings {
    let mut next = current.clone();
    if let Some(theme) = &incoming.theme_id {
        next.theme_id = theme.clone();
    }
    if incoming.scrollback.is_some() {
        next.prefs.scrollback = normalize_scrollback(incoming.scrollback);
    }
    if let Some(size) = incoming.font_size {
        if size.is_finite() {
            next.prefs.font_size = size;
        }
    }
    if let Some(v) = incoming.session_restore {
        next.session_restore = v;
    }
    if let Some(v) = incoming.zen_mode {
        next.zen_mode = v;
    }
    if incoming.hibernate_after_ms.is_some() {
        next.hibernate_after_ms = normalize_hibernate_after_ms(incoming.hibernate_after_ms);
    }
    if incoming.output_ring_lines.is_some() {
        next.output_ring_lines =
            normalize_output_ring_lines(incoming.output_ring_lines, next.output_ring_lines);
    }
    if incoming.search_index_lines.is_some() {
        next.search_index_lines = normalize_search_index_lines(incoming.search_index_lines);
    }
    if incoming.agent_kind.is_some() {
        next.agent_kind = normalize_agent_kind(incoming.agent_kind.as_deref(), &current.agent_kind);
    }
    if let Some(v) = incoming.show_status_bar {
        next.show_status_bar = v;
    }
    if let Some(v) = incoming.density {
        next.density = v;
    }
    if let Some(v) = incoming.remote_connect_mode {
        next.remote_connect_mode = v;
    }
    next.welcome_hint_seen = current.welcome_hint_seen;
    next.first_run = current.first_run.clone();
    next.idle_notify = current.idle_notify.clone();
    next
}

pub fn set_zen_mode(settings: &mut AppSettings, on: bool) {
    settings.zen_mode = on;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdRecord {
    pub id: String,
}

pub fn merge_by_id(current: Vec<IdRecord>, incoming: Vec<IdRecord>) -> Vec<IdRecord> {
    let mut order: Vec<String> = current.iter().map(|r| r.id.clone()).collect();
    let mut by_id: BTreeMap<String, IdRecord> = BTreeMap::new();
    for r in current {
        by_id.insert(r.id.clone(), r);
    }
    for r in incoming {
        if !by_id.contains_key(&r.id) {
            order.push(r.id.clone());
        }
        by_id.insert(r.id.clone(), r);
    }
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

#[derive(Clone, Debug)]
pub struct ExportBundle {
    pub version: u32,
    pub settings: AppSettings,
    pub connections: Vec<Json>,
}

pub fn export_bundle(settings: &AppSettings, connections: Vec<Json>) -> ExportBundle {
    let mut settings = settings.clone();
    settings.idle_notify.webhook_url.clear();
    settings.idle_notify.telegram_chat_id.clear();
    ExportBundle {
        version: 1,
        settings,
        connections: sanitize_connections(connections)
            .into_iter()
            .map(strip_export_secrets)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_session_restore_and_scrollback() {
        let settings = default_settings();
        assert!(settings.session_restore);
        assert_eq!(settings.prefs.scrollback, 10_000);
        assert!(!settings.welcome_hint_seen);
        assert!(!settings.zen_mode);
        assert_eq!(settings.prefs.font_size, 14.0);
    }

    #[test]
    fn scrollback_clamp() {
        assert_eq!(normalize_scrollback(None), 10_000);
        assert_eq!(normalize_scrollback(Some(f64::NAN)), 10_000);
        assert_eq!(normalize_scrollback(Some(50.0)), 100);
        assert_eq!(normalize_scrollback(Some(100_000_000.0)), 100_000);
        assert_eq!(normalize_scrollback(Some(10_000.0)), 10_000);
        let merged = merge_snapshot_with_defaults(&SettingsSnapshot {
            scrollback: Some(12.0),
            ..SettingsSnapshot::default()
        });
        assert_eq!(merged.prefs.scrollback, 100);
        assert!(merged.session_restore);
    }

    #[test]
    fn strips_connection_secrets_and_export_secrets() {
        let conn = Json::object(vec![
            ("id", Json::String("c1".into())),
            ("name", Json::String("box".into())),
            ("host", Json::String("example".into())),
            ("protocol", Json::String("ssh".into())),
            ("password", Json::String("secret".into())),
            ("passphrase", Json::String("phrase".into())),
            ("privateKeyPath", Json::String("/keys/id".into())),
            ("domain", Json::String("gone".into())),
            (
                "jump",
                Json::object(vec![
                    ("host", Json::String("bastion".into())),
                    ("password", Json::String("hop-secret".into())),
                    ("passphrase", Json::String("hop-phrase".into())),
                    ("privateKeyPath", Json::String("/keys/jump".into())),
                ]),
            ),
        ]);
        let clean = sanitize_connection(conn).unwrap();
        assert!(clean.get("password").is_none());
        assert!(clean.get("passphrase").is_none());
        assert!(clean.get("privateKeyPath").is_none());
        assert!(clean.get("protocol").is_none());
        assert!(clean.get("domain").is_none());
        assert_eq!(clean.get("host").and_then(Json::as_str), Some("example"));
        let jump = clean.get("jump").unwrap();
        assert_eq!(jump.get("host").and_then(Json::as_str), Some("bastion"));
        assert!(jump.get("password").is_none());
        assert!(jump.get("passphrase").is_none());
        assert!(jump.get("privateKeyPath").is_none());

        let dropped = sanitize_connection(Json::object(vec![
            ("protocol", Json::String("telnet".into())),
            ("host", Json::String("nope".into())),
        ]));
        assert!(dropped.is_none());

        let leaked = Json::object(vec![
            (
                "webhookUrl",
                Json::String("https://hooks.example/secret".into()),
            ),
            ("apiKey", Json::String("ak_live".into())),
            ("token", Json::String("tok".into())),
            ("telegramBotToken", Json::String("123:abc".into())),
            ("themeId", Json::String("nord".into())),
        ]);
        let stripped = strip_export_secrets(leaked);
        assert!(stripped.get("webhookUrl").is_none());
        assert!(stripped.get("apiKey").is_none());
        assert!(stripped.get("token").is_none());
        assert!(stripped.get("telegramBotToken").is_none());
        assert_eq!(stripped.get("themeId").and_then(Json::as_str), Some("nord"));
    }

    #[test]
    fn import_keeps_getting_started_dismissed_and_zen_does_not_scale_text() {
        let mut current = default_settings();
        current.welcome_hint_seen = true;
        current.first_run.local_terminal = true;
        current.prefs.font_size = 16.0;
        current.idle_notify.webhook_url = "https://hooks.example/keep".into();
        let incoming = SettingsSnapshot {
            zen_mode: Some(true),
            scrollback: Some(50.0),
            session_restore: Some(false),
            theme_id: Some("nord".into()),
            ..SettingsSnapshot::default()
        };
        let next = apply_imported(&current, &incoming);
        assert!(
            next.welcome_hint_seen,
            "dismissed getting started must survive import"
        );
        assert!(next.first_run.local_terminal);
        assert!(next.zen_mode);
        assert_eq!(next.prefs.font_size, 16.0);
        assert_eq!(next.prefs.scrollback, 100);
        assert!(!next.session_restore);
        assert_eq!(next.theme_id, "nord");
        assert_eq!(next.idle_notify.webhook_url, "https://hooks.example/keep");

        let mut live = next.clone();
        let font = live.prefs.font_size;
        set_zen_mode(&mut live, false);
        assert!(!live.zen_mode);
        assert_eq!(live.prefs.font_size, font);
    }

    #[test]
    fn export_bundle_strips_secrets_and_webhook() {
        let mut settings = default_settings();
        settings.welcome_hint_seen = true;
        settings.idle_notify.webhook_url = "https://hooks.example/secret".into();
        let bundle = export_bundle(
            &settings,
            vec![Json::object(vec![
                ("host", Json::String("h".into())),
                ("password", Json::String("p".into())),
                ("token", Json::String("t".into())),
            ])],
        );
        assert_eq!(bundle.version, 1);
        assert!(bundle.settings.idle_notify.webhook_url.is_empty());
        assert!(bundle.settings.welcome_hint_seen);
        assert!(bundle.connections[0].get("password").is_none());
        assert!(bundle.connections[0].get("token").is_none());
        assert_eq!(
            bundle.connections[0].get("host").and_then(Json::as_str),
            Some("h")
        );
    }
}
