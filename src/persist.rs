//! JSON settings in the OS config directory.
//!
//! Secrets never land in this file. Export and import go through the same
//! normalizers as the Electron settings store, and a dismissed getting-started
//! row stays dismissed.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::logic::settings::{
    apply_imported, default_settings, AppSettings, SettingsSnapshot, DEFAULT_THEME_ID,
};

pub fn user_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DEVTERM_USER_DATA") {
        return PathBuf::from(dir);
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("devterm")
}

pub fn settings_path() -> PathBuf {
    user_data_dir().join("settings.json")
}

pub fn load_settings() -> AppSettings {
    let path = settings_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return default_settings();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return default_settings();
    };
    let mut settings = apply_imported(&default_settings(), &snapshot_from_json(&value));
    if let Some(seen) = value.get("welcomeHintSeen").and_then(|v| v.as_bool()) {
        settings.welcome_hint_seen = seen;
    }
    if let Some(kind) = value.get("defaultShell").and_then(|v| v.as_str()) {
        settings.default_shell.kind = kind.to_string();
    }
    if let Some(path) = value.get("defaultShellPath").and_then(|v| v.as_str()) {
        settings.default_shell.path = Some(path.to_string());
    }
    if settings.theme_id.is_empty() {
        settings.theme_id = DEFAULT_THEME_ID.to_string();
    }
    settings
}

pub fn save_settings(settings: &AppSettings) -> std::io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body =
        serde_json::to_string_pretty(&settings_json(settings)).unwrap_or_else(|_| "{}".into());
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn settings_json(settings: &AppSettings) -> Value {
    json!({
        "theme": settings.theme_id,
        "scrollback": settings.prefs.scrollback,
        "fontSize": settings.prefs.font_size,
        "sessionRestore": settings.session_restore,
        "zenMode": settings.zen_mode,
        "hibernateAfterMs": settings.hibernate_after_ms,
        "outputRingLines": settings.output_ring_lines,
        "searchIndexLines": settings.search_index_lines,
        "agentKind": settings.agent_kind,
        "showStatusBar": settings.show_status_bar,
        "welcomeHintSeen": settings.welcome_hint_seen,
        "defaultShell": settings.default_shell.kind,
        "defaultShellPath": settings.default_shell.path,
        "keepSessionsInTray": settings.keep_sessions_in_tray,
        "searchPersist": settings.search_persist,
        "remoteDetachedSessions": settings.remote_detached_sessions,
    })
}

pub fn snapshot_from_json(value: &Value) -> SettingsSnapshot {
    SettingsSnapshot {
        theme_id: value
            .get("theme")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        scrollback: value.get("scrollback").and_then(|v| v.as_f64()),
        font_size: value.get("fontSize").and_then(|v| v.as_f64()),
        session_restore: value.get("sessionRestore").and_then(|v| v.as_bool()),
        zen_mode: value.get("zenMode").and_then(|v| v.as_bool()),
        hibernate_after_ms: value.get("hibernateAfterMs").and_then(|v| v.as_f64()),
        output_ring_lines: value.get("outputRingLines").and_then(|v| v.as_f64()),
        search_index_lines: value.get("searchIndexLines").and_then(|v| v.as_f64()),
        agent_kind: value
            .get("agentKind")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        show_status_bar: value.get("showStatusBar").and_then(|v| v.as_bool()),
        density: None,
        remote_connect_mode: None,
    }
}

/// Import merges through the normalizers and does not clear `welcome_hint_seen`.
pub fn import_settings(current: &AppSettings, text: &str) -> Result<AppSettings, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let stripped = crate::logic::settings::strip_export_secrets(json_to_logic(&value));
    let raw = logic_to_json(&stripped);
    let mut next = apply_imported(current, &snapshot_from_json(&raw));
    next.welcome_hint_seen = current.welcome_hint_seen;
    next.first_run = current.first_run.clone();
    Ok(next)
}

pub fn export_settings(settings: &AppSettings) -> String {
    serde_json::to_string_pretty(&settings_json(settings)).unwrap_or_else(|_| "{}".into())
}

fn json_to_logic(value: &Value) -> crate::logic::settings::Json {
    match value {
        Value::Null => crate::logic::settings::Json::Null,
        Value::Bool(v) => crate::logic::settings::Json::Bool(*v),
        Value::Number(v) => crate::logic::settings::Json::Number(v.as_f64().unwrap_or(0.0)),
        Value::String(v) => crate::logic::settings::Json::String(v.clone()),
        Value::Array(items) => {
            crate::logic::settings::Json::Array(items.iter().map(json_to_logic).collect())
        }
        Value::Object(map) => crate::logic::settings::Json::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_logic(v)))
                .collect(),
        ),
    }
}

fn logic_to_json(value: &crate::logic::settings::Json) -> Value {
    match value {
        crate::logic::settings::Json::Null => Value::Null,
        crate::logic::settings::Json::Bool(v) => Value::Bool(*v),
        crate::logic::settings::Json::Number(v) => json!(v),
        crate::logic::settings::Json::String(v) => Value::String(v.clone()),
        crate::logic::settings::Json::Array(items) => {
            Value::Array(items.iter().map(logic_to_json).collect())
        }
        crate::logic::settings::Json::Object(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                obj.insert(k.clone(), logic_to_json(v));
            }
            Value::Object(obj)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::settings::default_settings;

    #[test]
    fn import_keeps_a_dismissed_getting_started_row() {
        let mut current = default_settings();
        current.welcome_hint_seen = true;
        let imported = import_settings(
            &current,
            r#"{"theme":"nord","scrollback":5000,"webhookUrl":"https://secret"}"#,
        )
        .unwrap();
        assert!(imported.welcome_hint_seen);
        assert_eq!(imported.theme_id, "nord");
        assert_eq!(imported.prefs.scrollback, 5000);
        let exported = export_settings(&imported);
        assert!(!exported.contains("webhookUrl"));
        assert!(!exported.contains("secret"));
    }
}

pub fn read_text(path: &Path) -> std::io::Result<String> {
    fs::read_to_string(path)
}

pub fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}
