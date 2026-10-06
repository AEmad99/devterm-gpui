//! Chrome colors from the active DevTerm theme.
//!
//! One theme drives the window and the terminal palette. Glass and Aurora are
//! translucent treatments of the same tokens.

use std::sync::Mutex;

use gpui::{rgb, rgba, Rgba};

use crate::logic::themes::{self, Theme};
use crate::term_view::Palette;

static ACTIVE: Mutex<Option<String>> = Mutex::new(None);

pub fn set_id(id: &str) {
    if let Ok(mut slot) = ACTIVE.lock() {
        *slot = Some(id.to_string());
    }
}

pub fn id() -> String {
    ACTIVE
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(|| themes::DEFAULT_THEME_ID.to_string())
}

fn active() -> Theme {
    themes::get_theme(Some(&id()))
}

pub fn bg() -> Rgba {
    paint(&active().chrome.bg)
}
pub fn term_bg() -> Rgba {
    paint(themes::terminal_host_color(&active()))
}
pub fn panel() -> Rgba {
    paint(&active().chrome.panel)
}
pub fn panel2() -> Rgba {
    paint(&active().chrome.panel2)
}
pub fn border() -> Rgba {
    paint(&active().chrome.border)
}
pub fn fg() -> Rgba {
    paint(&active().chrome.fg)
}
pub fn muted() -> Rgba {
    paint(&active().chrome.muted)
}
pub fn accent() -> Rgba {
    paint(&active().chrome.accent)
}
pub fn danger() -> Rgba {
    paint(active().terminal.red)
}
pub fn ok() -> Rgba {
    paint(active().terminal.green)
}
pub fn warn() -> Rgba {
    paint(active().terminal.yellow)
}
pub fn accent_quiet() -> Rgba {
    mix(active().chrome.panel, active().chrome.accent, 0.22)
}
pub fn hover() -> Rgba {
    mix(active().chrome.panel, active().chrome.fg, 0.08)
}
pub fn group_active() -> Rgba {
    mix(active().chrome.panel, active().chrome.fg, 0.14)
}

pub fn terminal_palette() -> Palette {
    let theme = active();
    let ansi = &theme.terminal;
    let parse = |value: &str| css_u32(value).unwrap_or(0);
    Palette {
        foreground: parse(ansi.foreground),
        background: parse(ansi.background),
        cursor: parse(ansi.cursor),
        selection: parse(ansi.selection),
        ansi: [
            parse(ansi.black),
            parse(ansi.red),
            parse(ansi.green),
            parse(ansi.yellow),
            parse(ansi.blue),
            parse(ansi.magenta),
            parse(ansi.cyan),
            parse(ansi.white),
            parse(ansi.bright_black),
            parse(ansi.bright_red),
            parse(ansi.bright_green),
            parse(ansi.bright_yellow),
            parse(ansi.bright_blue),
            parse(ansi.bright_magenta),
            parse(ansi.bright_cyan),
            parse(ansi.bright_white),
        ],
    }
}

fn paint(value: &str) -> Rgba {
    if value == "transparent" {
        return rgba(0x00000000);
    }
    css_rgba(value).unwrap_or_else(|| rgb(0x16161e))
}

fn mix(base: &str, tint: &str, amount: f32) -> Rgba {
    let base = paint(base);
    let tint = paint(tint);
    let t = amount.clamp(0.0, 1.0);
    Rgba {
        r: base.r + (tint.r - base.r) * t,
        g: base.g + (tint.g - base.g) * t,
        b: base.b + (tint.b - base.b) * t,
        a: base.a + (tint.a - base.a) * t,
    }
}

fn css_u32(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 {
            return u32::from_str_radix(hex, 16).ok();
        }
    }
    None
}

fn css_rgba(value: &str) -> Option<Rgba> {
    let value = value.trim();
    if let Some(n) = css_u32(value) {
        return Some(rgb(n));
    }
    let start = value.find('(')?;
    let end = value.find(')')?;
    let parts: Vec<&str> = value[start + 1..end].split(',').map(str::trim).collect();
    if parts.len() < 3 {
        return None;
    }
    let r = parts[0].parse::<f32>().ok()? / 255.0;
    let g = parts[1].parse::<f32>().ok()? / 255.0;
    let b = parts[2].parse::<f32>().ok()? / 255.0;
    let a = parts
        .get(3)
        .and_then(|p| p.parse::<f32>().ok())
        .unwrap_or(1.0);
    Some(Rgba { r, g, b, a })
}
