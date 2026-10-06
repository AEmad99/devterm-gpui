//! Theme registry. One theme drives the terminal ANSI palette and the chrome tokens.
//! Glass and Aurora are translucent treatments of the same tokens (a `glass` flag
//! and alpha colors), not a second widget set.

#![allow(dead_code)]

#[derive(Clone, Debug, PartialEq)]
pub struct AnsiPalette {
    pub background: &'static str,
    pub foreground: &'static str,
    pub cursor: &'static str,
    pub cursor_accent: &'static str,
    pub selection: &'static str,
    pub black: &'static str,
    pub red: &'static str,
    pub green: &'static str,
    pub yellow: &'static str,
    pub blue: &'static str,
    pub magenta: &'static str,
    pub cyan: &'static str,
    pub white: &'static str,
    pub bright_black: &'static str,
    pub bright_red: &'static str,
    pub bright_green: &'static str,
    pub bright_yellow: &'static str,
    pub bright_blue: &'static str,
    pub bright_magenta: &'static str,
    pub bright_cyan: &'static str,
    pub bright_white: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChromeColors {
    pub bg: &'static str,
    pub panel: &'static str,
    pub panel2: &'static str,
    pub border: &'static str,
    pub fg: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
    pub accent2: Option<&'static str>,
    pub accent_fg: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub dark: bool,
    pub glass: bool,
    pub terminal: AnsiPalette,
    pub chrome: ChromeColors,
}

fn pal(
    background: &'static str,
    foreground: &'static str,
    cursor: &'static str,
    cursor_accent: &'static str,
    selection: &'static str,
    black: &'static str,
    red: &'static str,
    green: &'static str,
    yellow: &'static str,
    blue: &'static str,
    magenta: &'static str,
    cyan: &'static str,
    white: &'static str,
    bright_black: &'static str,
    bright_red: &'static str,
    bright_green: &'static str,
    bright_yellow: &'static str,
    bright_blue: &'static str,
    bright_magenta: &'static str,
    bright_cyan: &'static str,
    bright_white: &'static str,
) -> AnsiPalette {
    AnsiPalette {
        background,
        foreground,
        cursor,
        cursor_accent,
        selection,
        black,
        red,
        green,
        yellow,
        blue,
        magenta,
        cyan,
        white,
        bright_black,
        bright_red,
        bright_green,
        bright_yellow,
        bright_blue,
        bright_magenta,
        bright_cyan,
        bright_white,
    }
}

fn chrome(
    bg: &'static str,
    panel: &'static str,
    panel2: &'static str,
    border: &'static str,
    fg: &'static str,
    muted: &'static str,
    accent: &'static str,
    accent2: Option<&'static str>,
    accent_fg: Option<&'static str>,
) -> ChromeColors {
    ChromeColors {
        bg,
        panel,
        panel2,
        border,
        fg,
        muted,
        accent,
        accent2,
        accent_fg,
    }
}

pub const THEME_IDS: [&str; 10] = [
    "tokyo-night",
    "dracula",
    "catppuccin-mocha",
    "nord",
    "gruvbox",
    "one-dark",
    "solarized-dark",
    "ayu-mirage",
    "glass",
    "aurora",
];

pub fn themes() -> Vec<Theme> {
    vec![
        theme(
            "tokyo-night",
            "Tokyo Night",
            "Dark",
            true,
            false,
            pal(
                "#1a1b26", "#c0caf5", "#c0caf5", "#1a1b26", "rgba(122,162,247,0.30)", "#15161e",
                "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
                "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff",
                "#c0caf5",
            ),
            chrome(
                "#16161e", "#1a1b26", "#20212e", "#2a2c3d", "#c0caf5", "#565f89", "#7aa2f7",
                Some("#bb9af7"), None,
            ),
        ),
        theme(
            "dracula",
            "Dracula",
            "Dark",
            true,
            false,
            pal(
                "#282a36", "#f8f8f2", "#f8f8f2", "#282a36", "rgba(189,147,249,0.35)", "#21222c",
                "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
                "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff",
                "#ffffff",
            ),
            chrome(
                "#21222c", "#282a36", "#343746", "#3a3c4e", "#f8f8f2", "#6272a4", "#bd93f9",
                Some("#ff79c6"), None,
            ),
        ),
        theme(
            "catppuccin-mocha",
            "Catppuccin Mocha",
            "Dark",
            true,
            false,
            pal(
                "#1e1e2e", "#cdd6f4", "#f5e0dc", "#1e1e2e", "rgba(203,166,247,0.30)", "#45475a",
                "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#bac2de",
                "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5",
                "#a6adc8",
            ),
            chrome(
                "#181825", "#1e1e2e", "#313244", "#45475a", "#cdd6f4", "#7f849c", "#cba6f7",
                Some("#f5c2e7"), None,
            ),
        ),
        theme(
            "nord",
            "Nord",
            "Dark",
            true,
            false,
            pal(
                "#2e3440", "#d8dee9", "#d8dee9", "#2e3440", "rgba(136,192,208,0.30)", "#3b4252",
                "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
                "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb",
                "#eceff4",
            ),
            chrome(
                "#2e3440", "#3b4252", "#434c5e", "#4c566a", "#eceff4", "#7b88a1", "#88c0d0",
                Some("#81a1c1"), None,
            ),
        ),
        theme(
            "gruvbox",
            "Gruvbox Dark",
            "Dark",
            true,
            false,
            pal(
                "#282828", "#ebdbb2", "#ebdbb2", "#282828", "rgba(254,128,25,0.28)", "#3c3836",
                "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
                "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c",
                "#ebdbb2",
            ),
            chrome(
                "#1d2021", "#282828", "#3c3836", "#504945", "#ebdbb2", "#928374", "#fe8019",
                Some("#d3869b"), Some("#1d2021"),
            ),
        ),
        theme(
            "one-dark",
            "One Dark",
            "Dark",
            true,
            false,
            pal(
                "#282c34", "#abb2bf", "#528bff", "#282c34", "rgba(97,175,239,0.28)", "#282c34",
                "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
                "#5c6370", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2",
                "#ffffff",
            ),
            chrome(
                "#21252b", "#282c34", "#2c313a", "#3a3f4b", "#abb2bf", "#5c6370", "#61afef",
                Some("#c678dd"), None,
            ),
        ),
        theme(
            "solarized-dark",
            "Solarized Dark",
            "Dark",
            true,
            false,
            pal(
                "#002b36", "#839496", "#93a1a1", "#002b36", "rgba(38,139,210,0.30)", "#073642",
                "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
                "#586e75", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1",
                "#fdf6e3",
            ),
            chrome(
                "#002b36", "#073642", "#0a4250", "#0e4b5a", "#eee8d5", "#93a1a1", "#268bd2",
                Some("#2aa198"), None,
            ),
        ),
        theme(
            "ayu-mirage",
            "Ayu Mirage",
            "Dark",
            true,
            false,
            pal(
                "#1f2430", "#cbccc6", "#ffcc66", "#1f2430", "rgba(255,204,102,0.25)", "#191e2a",
                "#ed8274", "#87d96c", "#ffd173", "#6dcbfa", "#dabafa", "#5ccfe6", "#c7c7c7",
                "#686868", "#f28779", "#a6cc70", "#ffd580", "#73d0ff", "#dfbfff", "#95e6cb",
                "#ffffff",
            ),
            chrome(
                "#171b24", "#1f2430", "#232834", "#2a3140", "#cbccc6", "#707a8c", "#ffcc66",
                Some("#5ccfe6"), Some("#171b24"),
            ),
        ),
        theme(
            "glass",
            "Glass",
            "Glass",
            true,
            true,
            pal(
                "rgba(18,20,28,0.42)",
                "#e8ebf5",
                "#9bbcff",
                "#12141c",
                "rgba(138,180,255,0.30)",
                "#1b1e29",
                "#ff7a8a",
                "#a6e3a1",
                "#f4d58d",
                "#8ab4ff",
                "#cba6f7",
                "#86e1fc",
                "#d7dbe8",
                "#5b6478",
                "#ff8f9d",
                "#b8efb3",
                "#ffe1a3",
                "#a6c8ff",
                "#d9bcff",
                "#a3ecff",
                "#ffffff",
            ),
            chrome(
                "rgba(16,18,26,0.55)",
                "rgba(28,32,44,0.45)",
                "rgba(44,49,66,0.50)",
                "rgba(255,255,255,0.10)",
                "#e8ebf5",
                "#aab2c6",
                "#8ab4ff",
                Some("#cba6f7"),
                None,
            ),
        ),
        theme(
            "aurora",
            "Aurora",
            "Glass",
            true,
            true,
            pal(
                "rgba(9, 18, 22, 0.42)",
                "#e6f7f3",
                "#5eead4",
                "#091216",
                "rgba(94, 234, 212, 0.28)",
                "#0a141a",
                "#ff8a9a",
                "#7ee7c7",
                "#ffd98a",
                "#7dd3fc",
                "#c9a6ff",
                "#5eead4",
                "#d7eae6",
                "#4d6b6b",
                "#ff9fb0",
                "#9bf0d6",
                "#ffe6ab",
                "#a5e0ff",
                "#dcc2ff",
                "#8ff3e4",
                "#ffffff",
            ),
            chrome(
                "rgba(7, 15, 19, 0.55)",
                "rgba(13, 26, 30, 0.45)",
                "rgba(22, 41, 46, 0.50)",
                "rgba(255, 255, 255, 0.10)",
                "#e6f7f3",
                "#8fb3ad",
                "#5eead4",
                Some("#c084fc"),
                Some("#06121a"),
            ),
        ),
    ]
}

fn theme(
    id: &'static str,
    name: &'static str,
    group: &'static str,
    dark: bool,
    glass: bool,
    terminal: AnsiPalette,
    chrome: ChromeColors,
) -> Theme {
    Theme {
        id,
        name,
        group,
        dark,
        glass,
        terminal,
        chrome,
    }
}

pub const DEFAULT_THEME_ID: &str = "tokyo-night";

pub fn theme_by_id(id: &str) -> Option<Theme> {
    themes().into_iter().find(|t| t.id == id)
}

pub fn get_theme(id: Option<&str>) -> Theme {
    id.and_then(theme_by_id)
        .unwrap_or_else(|| themes().into_iter().next().expect("themes"))
}

/// xterm palette for a theme. An image clears the background; an explicit color overrides it.
pub fn xterm_background(theme: &Theme, image: bool, color: Option<&str>) -> String {
    if image {
        "rgba(0,0,0,0)".to_string()
    } else if let Some(color) = color {
        if !color.is_empty() {
            return color.to_string();
        }
        theme.terminal.background.to_string()
    } else {
        theme.terminal.background.to_string()
    }
}

pub fn terminal_host_color(theme: &Theme) -> &'static str {
    if theme.glass {
        "transparent"
    } else {
        theme.terminal.background
    }
}

fn channel(c: f64) -> f64 {
    let s = c / 255.0;
    if s <= 0.03928 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(hex: &str) -> f64 {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return 0.5;
    }
    let Ok(n) = u32::from_str_radix(hex, 16) else {
        return 0.5;
    };
    let r = ((n >> 16) & 255) as f64;
    let g = ((n >> 8) & 255) as f64;
    let b = (n & 255) as f64;
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

pub fn readable_on(bg: &str) -> &'static str {
    if luminance(bg) > 0.5 {
        "#0b0e14"
    } else {
        "#ffffff"
    }
}

/// CSS variables themes write for tab status dots.
pub fn tab_status_vars(theme: &Theme) -> [(&'static str, &'static str); 6] {
    let t = &theme.terminal;
    [
        ("--tab-status-warn", t.yellow),
        ("--tab-status-error", t.red),
        ("--tab-status-pending", t.yellow),
        ("--tab-status-attention", t.green),
        ("--tab-status-running", t.blue),
        ("--tab-status-unread", t.magenta),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ten_exist_and_tokyo_night_background_matches_the_file() {
        let all = themes();
        assert_eq!(all.len(), 10);
        for id in THEME_IDS {
            let theme = theme_by_id(id).unwrap_or_else(|| panic!("missing {id}"));
            assert_eq!(theme.id, id);
        }
        let tokyo = theme_by_id("tokyo-night").unwrap();
        assert_eq!(tokyo.chrome.bg, "#16161e");
        assert_eq!(tokyo.terminal.background, "#1a1b26");
        assert_eq!(tokyo.name, "Tokyo Night");
        assert!(!tokyo.glass);
    }

    #[test]
    fn glass_and_aurora_are_translucent_treatments() {
        let glass = theme_by_id("glass").unwrap();
        let aurora = theme_by_id("aurora").unwrap();
        assert!(glass.glass);
        assert!(aurora.glass);
        assert_eq!(glass.group, "Glass");
        assert_eq!(aurora.group, "Glass");
        assert!(glass.terminal.background.contains("rgba"));
        assert!(aurora.terminal.background.contains("rgba"));
        assert!(glass.chrome.bg.contains("rgba"));
        assert!(aurora.chrome.bg.contains("rgba"));
        assert_eq!(terminal_host_color(&glass), "transparent");
        assert_eq!(terminal_host_color(&theme_by_id("dracula").unwrap()), "#282a36");
        let vars = tab_status_vars(&theme_by_id("tokyo-night").unwrap());
        assert_eq!(vars[0], ("--tab-status-warn", "#e0af68"));
        assert_eq!(vars[1], ("--tab-status-error", "#f7768e"));
        assert_eq!(vars[5], ("--tab-status-unread", "#bb9af7"));
    }
}
