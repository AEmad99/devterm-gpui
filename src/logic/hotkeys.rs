//! Application keyboard shortcuts.
//!
//! A terminal key handler returns the chord to the app (and does not write it
//! to the shell) when `match_hotkey` returns `Some`.
//!
//! `mod` means Ctrl on Windows/Linux and Cmd (meta) on macOS. Combos avoid bare
//! Ctrl+letter (which collides with readline) except the command palette,
//! which is intentionally Ctrl/Cmd+K. Save file is Ctrl/Cmd+S in the registry.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};

static HOTKEY_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_hotkey_capture_active(active: bool) {
    HOTKEY_CAPTURE_ACTIVE.store(active, Ordering::SeqCst);
}

pub fn is_hotkey_capture_active() -> bool {
    HOTKEY_CAPTURE_ACTIVE.load(Ordering::SeqCst)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyId {
    GlobalSearch,
    Palette,
    PaletteAlt,
    NewTerminal,
    NewGrid,
    CloseTerminal,
    DuplicateTerminal,
    ToggleSidebar,
    Find,
    ClearTerminal,
    ZoomIn,
    ZoomInAlt,
    ZoomInAlt2,
    ZoomOut,
    ZoomReset,
    Settings,
    NextTerminal,
    PrevTerminal,
    NextTab,
    PrevTab,
    ToggleFocus,
    ToggleZenMode,
    TmuxSessions,
    Shortcuts,
    SaveEditor,
    PreviewMarkdown,
    Dictate,
    NewGroup,
    NextGroup,
    PrevGroup,
    SplitRight,
    SplitDown,
    Agents,
    ToggleGit,
}

impl HotkeyId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GlobalSearch => "globalSearch",
            Self::Palette => "palette",
            Self::PaletteAlt => "paletteAlt",
            Self::NewTerminal => "newTerminal",
            Self::NewGrid => "newGrid",
            Self::CloseTerminal => "closeTerminal",
            Self::DuplicateTerminal => "duplicateTerminal",
            Self::ToggleSidebar => "toggleSidebar",
            Self::Find => "find",
            Self::ClearTerminal => "clearTerminal",
            Self::ZoomIn => "zoomIn",
            Self::ZoomInAlt => "zoomInAlt",
            Self::ZoomInAlt2 => "zoomInAlt2",
            Self::ZoomOut => "zoomOut",
            Self::ZoomReset => "zoomReset",
            Self::Settings => "settings",
            Self::NextTerminal => "nextTerminal",
            Self::PrevTerminal => "prevTerminal",
            Self::NextTab => "nextTab",
            Self::PrevTab => "prevTab",
            Self::ToggleFocus => "toggleFocus",
            Self::ToggleZenMode => "toggleZenMode",
            Self::TmuxSessions => "tmuxSessions",
            Self::Shortcuts => "shortcuts",
            Self::SaveEditor => "saveEditor",
            Self::PreviewMarkdown => "previewMarkdown",
            Self::Dictate => "dictate",
            Self::NewGroup => "newGroup",
            Self::NextGroup => "nextGroup",
            Self::PrevGroup => "prevGroup",
            Self::SplitRight => "splitRight",
            Self::SplitDown => "splitDown",
            Self::Agents => "agents",
            Self::ToggleGit => "toggleGit",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "globalSearch" => Self::GlobalSearch,
            "palette" => Self::Palette,
            "paletteAlt" => Self::PaletteAlt,
            "newTerminal" => Self::NewTerminal,
            "newGrid" => Self::NewGrid,
            "closeTerminal" => Self::CloseTerminal,
            "duplicateTerminal" => Self::DuplicateTerminal,
            "toggleSidebar" => Self::ToggleSidebar,
            "find" => Self::Find,
            "clearTerminal" => Self::ClearTerminal,
            "zoomIn" => Self::ZoomIn,
            "zoomInAlt" => Self::ZoomInAlt,
            "zoomInAlt2" => Self::ZoomInAlt2,
            "zoomOut" => Self::ZoomOut,
            "zoomReset" => Self::ZoomReset,
            "settings" => Self::Settings,
            "nextTerminal" => Self::NextTerminal,
            "prevTerminal" => Self::PrevTerminal,
            "nextTab" => Self::NextTab,
            "prevTab" => Self::PrevTab,
            "toggleFocus" => Self::ToggleFocus,
            "toggleZenMode" => Self::ToggleZenMode,
            "tmuxSessions" => Self::TmuxSessions,
            "shortcuts" => Self::Shortcuts,
            "saveEditor" => Self::SaveEditor,
            "previewMarkdown" => Self::PreviewMarkdown,
            "dictate" => Self::Dictate,
            "newGroup" => Self::NewGroup,
            "nextGroup" => Self::NextGroup,
            "prevGroup" => Self::PrevGroup,
            "splitRight" => Self::SplitRight,
            "splitDown" => Self::SplitDown,
            "agents" => Self::Agents,
            "toggleGit" => Self::ToggleGit,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub id: HotkeyId,
    pub mod_key: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
    pub label: String,
    pub alias: bool,
}

impl Hotkey {
    fn def(
        id: HotkeyId,
        mod_key: bool,
        shift: bool,
        alt: bool,
        key: &str,
        label: &str,
        alias: bool,
    ) -> Self {
        Self {
            id,
            mod_key,
            shift,
            alt,
            key: key.to_string(),
            label: label.to_string(),
            alias,
        }
    }
}

pub fn default_hotkeys() -> Vec<Hotkey> {
    use HotkeyId::*;
    vec![
        Hotkey::def(Palette, true, false, false, "k", "Command palette", false),
        Hotkey::def(PaletteAlt, true, true, false, "p", "Command palette", true),
        Hotkey::def(NewTerminal, true, true, false, "t", "New terminal", false),
        Hotkey::def(NewGrid, true, true, false, "g", "New terminal grid", false),
        Hotkey::def(
            CloseTerminal,
            true,
            true,
            false,
            "w",
            "Close current terminal",
            false,
        ),
        Hotkey::def(
            DuplicateTerminal,
            true,
            true,
            false,
            "d",
            "Duplicate terminal",
            false,
        ),
        Hotkey::def(
            ToggleSidebar,
            true,
            true,
            false,
            "e",
            "Toggle file explorer",
            false,
        ),
        Hotkey::def(Find, true, true, false, "f", "Find in terminal", false),
        Hotkey::def(ClearTerminal, true, true, false, "l", "Clear terminal", false),
        Hotkey::def(ZoomIn, true, false, false, "=", "Increase font size", false),
        Hotkey::def(ZoomInAlt, true, true, false, "+", "Increase font size", true),
        Hotkey::def(ZoomInAlt2, true, false, false, "+", "Increase font size", true),
        Hotkey::def(ZoomOut, true, false, false, "-", "Decrease font size", false),
        Hotkey::def(ZoomReset, true, false, false, "0", "Reset font size", false),
        Hotkey::def(NextTerminal, true, false, false, "PageDown", "Next terminal", false),
        Hotkey::def(
            PrevTerminal,
            true,
            false,
            false,
            "PageUp",
            "Previous terminal",
            false,
        ),
        Hotkey::def(NextTab, true, false, false, "Tab", "Next tab", false),
        Hotkey::def(PrevTab, true, true, false, "Tab", "Previous tab", false),
        Hotkey::def(
            ToggleFocus,
            true,
            true,
            false,
            "z",
            "Focus (magnify) terminal",
            false,
        ),
        Hotkey::def(ToggleZenMode, true, false, true, "z", "Zen mode", false),
        Hotkey::def(TmuxSessions, true, false, true, "t", "tmux sessions", false),
        Hotkey::def(Settings, true, false, false, ",", "Settings", false),
        Hotkey::def(
            GlobalSearch,
            true,
            false,
            true,
            "f",
            "Search across all terminals",
            false,
        ),
        Hotkey::def(Shortcuts, true, false, false, "/", "Keyboard shortcuts", false),
        Hotkey::def(SaveEditor, true, false, false, "s", "Save file", false),
        Hotkey::def(
            PreviewMarkdown,
            true,
            false,
            true,
            "m",
            "Markdown preview",
            false,
        ),
        Hotkey::def(
            Dictate,
            true,
            true,
            false,
            "m",
            "Toggle voice dictation",
            false,
        ),
        Hotkey::def(NewGroup, true, true, false, "n", "New terminal group", false),
        Hotkey::def(
            NextGroup,
            true,
            false,
            true,
            "PageDown",
            "Next group",
            false,
        ),
        Hotkey::def(
            PrevGroup,
            true,
            false,
            true,
            "PageUp",
            "Previous group",
            false,
        ),
        Hotkey::def(
            SplitRight,
            true,
            false,
            true,
            "r",
            "Split terminal right",
            false,
        ),
        Hotkey::def(SplitDown, true, false, true, "d", "Split terminal down", false),
        Hotkey::def(Agents, true, false, true, "a", "Agent overview", false),
        Hotkey::def(ToggleGit, true, false, true, "g", "Toggle Git panel", false),
    ]
}

/// Every id registered in the shortcut sheet, in registry order.
pub fn hotkey_ids() -> Vec<HotkeyId> {
    default_hotkeys().into_iter().map(|h| h.id).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCombo {
    pub mod_key: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct KeyEvent {
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

/// Build the effective hotkey list by layering custom overrides onto defaults.
pub fn resolve_hotkeys(custom: &[(HotkeyId, KeyCombo)]) -> Vec<Hotkey> {
    let mut list = default_hotkeys();
    for (id, combo) in custom {
        if let Some(slot) = list.iter_mut().find(|h| h.id == *id) {
            slot.mod_key = combo.mod_key;
            slot.shift = combo.shift;
            slot.alt = combo.alt;
            slot.key = combo.key.clone();
        }
    }
    list
}

fn event_key(key: &str) -> String {
    if key.chars().count() == 1 {
        key.to_lowercase()
    } else {
        key.to_string()
    }
}

/// Return the id of the hotkey this event matches, or `None`.
pub fn match_hotkey(e: &KeyEvent, hotkeys: Option<&[Hotkey]>) -> Option<HotkeyId> {
    let owned;
    let list: &[Hotkey] = if let Some(h) = hotkeys {
        h
    } else {
        owned = default_hotkeys();
        &owned
    };
    let mod_key = e.ctrl || e.meta;
    let key = event_key(&e.key);
    for h in list {
        if h.mod_key == mod_key && h.shift == e.shift && h.alt == e.alt && h.key == key {
            return Some(h.id);
        }
    }
    None
}

/// xterm's custom key handler returns the chord to the app when it matches a
/// hotkey, so the keystroke is not sent to the shell.
pub fn terminal_yields_chord_to_app(e: &KeyEvent) -> bool {
    match_hotkey(e, None).is_some()
}

/// Convert a raw keydown into a hotkey combo. Ignores bare modifiers.
pub fn capture_combo(e: &KeyEvent) -> Option<KeyCombo> {
    if matches!(
        e.key.as_str(),
        "Control" | "Shift" | "Alt" | "Meta" | "CapsLock"
    ) {
        return None;
    }
    let mod_key = e.ctrl || e.meta;
    let shift = e.shift;
    let alt = e.alt;
    if !mod_key && !shift && !alt {
        return None;
    }
    Some(KeyCombo {
        mod_key,
        shift,
        alt,
        key: event_key(&e.key),
    })
}

fn pretty_key(key: &str) -> String {
    if key == "PageUp" {
        return "PgUp".to_string();
    }
    if key == "PageDown" {
        return "PgDn".to_string();
    }
    if key.chars().count() == 1 {
        return key.to_uppercase();
    }
    key.to_string()
}

/// Render a hotkey as a display string, e.g. "Ctrl+Shift+T" or "⌘⇧T".
/// Ctrl on Windows/Linux, Cmd on macOS.
pub fn combo_label(h: &Hotkey, is_mac: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if h.mod_key {
        parts.push(if is_mac {
            "⌘".to_string()
        } else {
            "Ctrl".to_string()
        });
    }
    if h.shift {
        parts.push(if is_mac {
            "⇧".to_string()
        } else {
            "Shift".to_string()
        });
    }
    if h.alt {
        parts.push(if is_mac {
            "⌥".to_string()
        } else {
            "Alt".to_string()
        });
    }
    parts.push(pretty_key(&h.key));
    parts.join(if is_mac { "" } else { "+" })
}

fn is_ascii_letter(key: &str) -> bool {
    key.len() == 1 && key.chars().all(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ctrl: bool, meta: bool, shift: bool, alt: bool, key: &str) -> KeyEvent {
        KeyEvent {
            ctrl,
            meta,
            shift,
            alt,
            key: key.to_string(),
        }
    }

    #[test]
    fn every_hotkey_id_is_registered() {
        let ids = hotkey_ids();
        let expected = [
            HotkeyId::Palette,
            HotkeyId::PaletteAlt,
            HotkeyId::NewTerminal,
            HotkeyId::NewGrid,
            HotkeyId::CloseTerminal,
            HotkeyId::DuplicateTerminal,
            HotkeyId::ToggleSidebar,
            HotkeyId::Find,
            HotkeyId::ClearTerminal,
            HotkeyId::ZoomIn,
            HotkeyId::ZoomInAlt,
            HotkeyId::ZoomInAlt2,
            HotkeyId::ZoomOut,
            HotkeyId::ZoomReset,
            HotkeyId::NextTerminal,
            HotkeyId::PrevTerminal,
            HotkeyId::NextTab,
            HotkeyId::PrevTab,
            HotkeyId::ToggleFocus,
            HotkeyId::ToggleZenMode,
            HotkeyId::TmuxSessions,
            HotkeyId::Settings,
            HotkeyId::GlobalSearch,
            HotkeyId::Shortcuts,
            HotkeyId::SaveEditor,
            HotkeyId::PreviewMarkdown,
            HotkeyId::Dictate,
            HotkeyId::NewGroup,
            HotkeyId::NextGroup,
            HotkeyId::PrevGroup,
            HotkeyId::SplitRight,
            HotkeyId::SplitDown,
            HotkeyId::Agents,
            HotkeyId::ToggleGit,
        ];
        assert_eq!(ids, expected);
        assert_eq!(ids.len(), 34);
    }

    #[test]
    fn bare_ctrl_letter_stays_out_except_palette_k() {
        let bare: Vec<(String, HotkeyId)> = default_hotkeys()
            .into_iter()
            .filter(|h| h.mod_key && !h.shift && !h.alt && is_ascii_letter(&h.key))
            .map(|h| (h.key, h.id))
            .collect();
        // Documented readline exception is Ctrl/Cmd+K (command palette).
        // The registry also binds Save file as Ctrl/Cmd+S.
        assert_eq!(
            bare,
            vec![
                ("k".to_string(), HotkeyId::Palette),
                ("s".to_string(), HotkeyId::SaveEditor),
            ]
        );
        let palette = default_hotkeys()
            .into_iter()
            .find(|h| h.id == HotkeyId::Palette)
            .unwrap();
        assert!(palette.mod_key && !palette.shift && !palette.alt);
        assert_eq!(palette.key, "k");
        assert_eq!(palette.label, "Command palette");
    }

    #[test]
    fn terminal_handler_yields_the_chord_when_it_matches() {
        let palette = ev(true, false, false, false, "k");
        assert_eq!(match_hotkey(&palette, None), Some(HotkeyId::Palette));
        assert!(terminal_yields_chord_to_app(&palette));

        let cmd_k = ev(false, true, false, false, "K");
        assert_eq!(match_hotkey(&cmd_k, None), Some(HotkeyId::Palette));
        assert!(terminal_yields_chord_to_app(&cmd_k));

        let bare_a = ev(true, false, false, false, "a");
        assert_eq!(match_hotkey(&bare_a, None), None);
        assert!(!terminal_yields_chord_to_app(&bare_a));

        let split = ev(true, false, false, true, "r");
        assert_eq!(match_hotkey(&split, None), Some(HotkeyId::SplitRight));
        assert!(terminal_yields_chord_to_app(&split));
    }

    #[test]
    fn combo_labels_use_ctrl_or_cmd() {
        let keys = default_hotkeys();
        let new_term = keys.iter().find(|h| h.id == HotkeyId::NewTerminal).unwrap();
        assert_eq!(combo_label(new_term, false), "Ctrl+Shift+T");
        assert_eq!(combo_label(new_term, true), "⌘⇧T");
        let palette = keys.iter().find(|h| h.id == HotkeyId::Palette).unwrap();
        assert_eq!(combo_label(palette, false), "Ctrl+K");
        assert_eq!(combo_label(palette, true), "⌘K");
        let split = keys.iter().find(|h| h.id == HotkeyId::SplitDown).unwrap();
        assert_eq!(combo_label(split, false), "Ctrl+Alt+D");
        assert_eq!(combo_label(split, true), "⌘⌥D");
        let next = keys.iter().find(|h| h.id == HotkeyId::NextTerminal).unwrap();
        assert_eq!(combo_label(next, false), "Ctrl+PgDn");
    }

    #[test]
    fn capture_and_resolve() {
        assert!(capture_combo(&ev(false, false, false, false, "k")).is_none());
        assert!(capture_combo(&ev(true, false, false, false, "Control")).is_none());
        let combo = capture_combo(&ev(true, false, true, false, "T")).unwrap();
        assert!(combo.mod_key && combo.shift && !combo.alt);
        assert_eq!(combo.key, "t");
        let resolved = resolve_hotkeys(&[(
            HotkeyId::Palette,
            KeyCombo {
                mod_key: true,
                shift: true,
                alt: false,
                key: "p".into(),
            },
        )]);
        let palette = resolved.iter().find(|h| h.id == HotkeyId::Palette).unwrap();
        assert_eq!(palette.key, "p");
        assert!(palette.shift);
        assert_eq!(palette.label, "Command palette");
        assert_eq!(resolved.len(), default_hotkeys().len());
    }
}
