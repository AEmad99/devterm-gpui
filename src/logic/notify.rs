//! Idle notify webhook / Telegram.
//!
//! Port of `src/main/ipc/notify.ts` and the settings split: the Telegram bot
//! token is never written into settings JSON. `strip_notify_secrets` removes
//! it (and any connection-style secret fields) before a settings document is
//! persisted. Webhook URL, bot token, and chat id live in the separate
//! notify-secrets document.

pub const TELEGRAM_TEXT_LIMIT: usize = 3500;

/// Connection / export secret fields (`settings-io.ts` SECRET_FIELDS) plus
/// the notify bot token, which the settings form never persists.
const SECRET_KEYS: &[&str] = &[
    "password",
    "passphrase",
    "privateKeyPath",
    "telegramBotToken",
    "jumpPassword",
    "jumpPassphrase",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotifySecrets {
    pub webhook_url: Option<String>,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleNotifyEvent {
    pub session_id: String,
    pub reason: String,
    pub title: String,
    pub body: Option<String>,
}

/// Drop secret fields from a flat settings / connection object.
/// The Telegram bot token is always removed so it cannot land in settings JSON.
pub fn strip_notify_secrets(fields: &[(String, String)]) -> Vec<(String, String)> {
    fields
        .iter()
        .filter(|(k, _)| !SECRET_KEYS.iter().any(|s| s == k))
        .cloned()
        .collect()
}

/// True when a settings JSON blob still contains a bot token assignment.
pub fn settings_json_contains_bot_token(json: &str) -> bool {
    json.contains("telegramBotToken")
}

/// Build the secrets document (not the settings document). Empty strings are
/// omitted. This is what `notify-secrets.json` stores.
pub fn secrets_document(secrets: &NotifySecrets) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(v) = secrets
        .webhook_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push(("webhookUrl".into(), v.to_string()));
    }
    if let Some(v) = secrets
        .telegram_bot_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push(("telegramBotToken".into(), v.to_string()));
    }
    if let Some(v) = secrets
        .telegram_chat_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push(("telegramChatId".into(), v.to_string()));
    }
    out
}

/// Merge a partial secrets update the way the IPC handler does: a non-empty
/// trimmed incoming value wins, otherwise the current value is kept.
pub fn merge_secrets(current: &NotifySecrets, incoming: &NotifySecrets) -> NotifySecrets {
    let pick = |new: &Option<String>, old: &Option<String>| {
        let trimmed = new.as_deref().map(str::trim).unwrap_or("");
        if trimmed.is_empty() {
            old.clone()
        } else {
            Some(trimmed.to_string())
        }
    };
    NotifySecrets {
        webhook_url: pick(&incoming.webhook_url, &current.webhook_url),
        telegram_bot_token: pick(&incoming.telegram_bot_token, &current.telegram_bot_token),
        telegram_chat_id: pick(&incoming.telegram_chat_id, &current.telegram_chat_id),
    }
}

pub fn webhook_payload(event: &IdleNotifyEvent) -> String {
    format!(
        "{{\"sessionId\":{},\"reason\":{},\"title\":{},\"body\":{}}}",
        json_string(&event.session_id),
        json_string(&event.reason),
        json_string(&event.title),
        match &event.body {
            Some(b) => json_string(b),
            None => "null".to_string(),
        }
    )
}

pub fn telegram_send_url(bot_token: &str) -> String {
    format!("https://api.telegram.org/bot{bot_token}/sendMessage")
}

/// `[title, body].filter(Boolean).join('\n')` then sliced to 3500 chars.
pub fn telegram_text(event: &IdleNotifyEvent) -> String {
    let mut parts = Vec::new();
    if !event.title.is_empty() {
        parts.push(event.title.as_str());
    }
    if let Some(body) = event.body.as_deref() {
        if !body.is_empty() {
            parts.push(body);
        }
    }
    let text = parts.join("\n");
    text.chars().take(TELEGRAM_TEXT_LIMIT).collect()
}

pub fn notify_http_error(status: u16) -> String {
    format!("notify {status}")
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_token_is_stripped_from_settings_json_fields() {
        let fields = vec![
            ("enabled".into(), "true".into()),
            ("webhookUrl".into(), "https://example.com/hook".into()),
            ("telegramChatId".into(), "42".into()),
            ("telegramBotToken".into(), "123:ABC".into()),
            ("password".into(), "hunter2".into()),
        ];
        let stripped = strip_notify_secrets(&fields);
        let json = format!("{stripped:?}");
        assert!(!settings_json_contains_bot_token(&json));
        assert!(stripped.iter().all(|(k, _)| k != "telegramBotToken"));
        assert!(stripped.iter().all(|(k, _)| k != "password"));
        assert!(stripped.iter().any(|(k, _)| k == "webhookUrl"));
    }

    #[test]
    fn secrets_document_holds_the_token_outside_settings() {
        let doc = secrets_document(&NotifySecrets {
            webhook_url: Some(" https://example.com/hook ".into()),
            telegram_bot_token: Some("123:ABC".into()),
            telegram_chat_id: Some("".into()),
        });
        assert!(doc
            .iter()
            .any(|(k, v)| k == "telegramBotToken" && v == "123:ABC"));
        assert!(doc
            .iter()
            .any(|(k, v)| k == "webhookUrl" && v == "https://example.com/hook"));
        assert!(doc.iter().all(|(k, _)| k != "telegramChatId"));
    }

    #[test]
    fn webhook_and_telegram_payloads() {
        let event = IdleNotifyEvent {
            session_id: "s1".into(),
            reason: "idle".into(),
            title: "DevTerm".into(),
            body: Some("waiting".into()),
        };
        let body = webhook_payload(&event);
        assert!(body.contains("\"sessionId\":\"s1\""));
        assert!(body.contains("\"reason\":\"idle\""));
        assert!(body.contains("\"title\":\"DevTerm\""));
        assert!(body.contains("\"body\":\"waiting\""));
        assert_eq!(
            telegram_send_url("123:ABC"),
            "https://api.telegram.org/bot123:ABC/sendMessage"
        );
        assert_eq!(telegram_text(&event), "DevTerm\nwaiting");
        let long = "x".repeat(4000);
        let clipped = telegram_text(&IdleNotifyEvent {
            session_id: "s".into(),
            reason: "approval".into(),
            title: long,
            body: None,
        });
        assert_eq!(clipped.chars().count(), TELEGRAM_TEXT_LIMIT);
        assert_eq!(notify_http_error(500), "notify 500");
    }

    #[test]
    fn merge_keeps_current_when_incoming_is_blank() {
        let cur = NotifySecrets {
            webhook_url: Some("https://old".into()),
            telegram_bot_token: Some("tok".into()),
            telegram_chat_id: Some("1".into()),
        };
        let next = merge_secrets(
            &cur,
            &NotifySecrets {
                webhook_url: Some("  ".into()),
                telegram_bot_token: Some(" new ".into()),
                telegram_chat_id: None,
            },
        );
        assert_eq!(next.webhook_url.as_deref(), Some("https://old"));
        assert_eq!(next.telegram_bot_token.as_deref(), Some("new"));
        assert_eq!(next.telegram_chat_id.as_deref(), Some("1"));
    }
}
