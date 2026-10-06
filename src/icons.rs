//! Stroke icons from the Electron app.
//!
//! Chrome glyphs match `src/renderer/components/common/Icons.tsx`. File rows
//! use the same type split as `FileTypeIcon.tsx`: folders, code, JSON, text,
//! images, media, archives, shell scripts, config, keys, and a generic file.
//! GPUI tints the SVG alpha mask, so every stroke is solid black.

use std::borrow::Cow;

use gpui::prelude::*;
use gpui::{px, svg, AssetSource, Rgba, SharedString, Svg};

use crate::theme;

pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(svg_source(path).map(|text| Cow::Borrowed(text.as_bytes())))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Folder,
    File,
    FileCode,
    FileJson,
    FileText,
    FileImage,
    FileMedia,
    FileArchive,
    FileTerminal,
    FileConfig,
    FileKey,
    FileLock,
    FileSheet,
    Remote,
    Group,
    Keyboard,
    Settings,
    Mic,
    Branch,
    ChevronDown,
    ChevronUp,
    ArrowUp,
    Plus,
    Close,
    Save,
    Split,
    Agent,
    Activity,
    Transfer,
    Search,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Folder,
    Up,
    Code,
    Json,
    Text,
    Image,
    Media,
    Archive,
    Terminal,
    Config,
    Key,
    Lock,
    Sheet,
    Generic,
}

impl Icon {
    pub fn asset_path(self) -> &'static str {
        match self {
            Self::Folder => "icons/folder",
            Self::File => "icons/file",
            Self::FileCode => "icons/file-code",
            Self::FileJson => "icons/file-json",
            Self::FileText => "icons/file-text",
            Self::FileImage => "icons/file-image",
            Self::FileMedia => "icons/file-media",
            Self::FileArchive => "icons/file-archive",
            Self::FileTerminal => "icons/file-terminal",
            Self::FileConfig => "icons/file-config",
            Self::FileKey => "icons/file-key",
            Self::FileLock => "icons/file-lock",
            Self::FileSheet => "icons/file-sheet",
            Self::Remote => "icons/remote",
            Self::Group => "icons/group",
            Self::Keyboard => "icons/keyboard",
            Self::Settings => "icons/settings",
            Self::Mic => "icons/mic",
            Self::Branch => "icons/branch",
            Self::ChevronDown => "icons/chevron-down",
            Self::ChevronUp => "icons/chevron-up",
            Self::ArrowUp => "icons/arrow-up",
            Self::Plus => "icons/plus",
            Self::Close => "icons/close",
            Self::Save => "icons/save",
            Self::Split => "icons/split",
            Self::Agent => "icons/agent",
            Self::Activity => "icons/activity",
            Self::Transfer => "icons/transfer",
            Self::Search => "icons/search",
        }
    }
}

impl FileKind {
    pub fn icon(self) -> Icon {
        match self {
            Self::Folder => Icon::Folder,
            Self::Up => Icon::ArrowUp,
            Self::Code => Icon::FileCode,
            Self::Json => Icon::FileJson,
            Self::Text => Icon::FileText,
            Self::Image => Icon::FileImage,
            Self::Media => Icon::FileMedia,
            Self::Archive => Icon::FileArchive,
            Self::Terminal => Icon::FileTerminal,
            Self::Config => Icon::FileConfig,
            Self::Key => Icon::FileKey,
            Self::Lock => Icon::FileLock,
            Self::Sheet => Icon::FileSheet,
            Self::Generic => Icon::File,
        }
    }

    /// Same hue roles as the Electron tree: folders and code take the accent,
    /// data and config take the warning tone, keys take danger.
    pub fn color(self) -> Rgba {
        match self {
            Self::Folder | Self::Code | Self::Terminal => theme::accent(),
            Self::Json | Self::Config | Self::Sheet | Self::Media => theme::warn(),
            Self::Image => theme::ok(),
            Self::Key | Self::Lock => theme::danger(),
            Self::Text => theme::fg(),
            Self::Archive | Self::Generic | Self::Up => theme::muted(),
        }
    }
}

/// Type of a file-tree row. `..` is the parent directory, not a folder glyph.
pub fn file_kind(name: &str, dir: bool) -> FileKind {
    if name == ".." {
        return FileKind::Up;
    }
    if dir {
        return FileKind::Folder;
    }
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        ".gitignore"
            | ".gitattributes"
            | ".gitmodules"
            | ".env"
            | ".envrc"
            | ".editorconfig"
            | ".prettierrc"
            | ".eslintrc"
            | ".dockerignore"
            | "dockerfile"
    ) {
        return FileKind::Config;
    }
    if matches!(lower.as_str(), ".bashrc" | ".zshrc" | ".vimrc" | ".nanorc") {
        return FileKind::Terminal;
    }
    match extension(&lower).as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "svg" | "tiff" => FileKind::Image,
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "mp3" | "wav" | "ogg" | "flac"
        | "aac" | "m4a" | "wma" => FileKind::Media,
        "zip" | "tar" | "gz" | "bz2" | "7z" | "rar" | "xz" | "tgz" => FileKind::Archive,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "py" | "java" | "c" | "cpp" | "h" | "hpp"
        | "cs" | "go" | "rs" | "rb" | "php" | "swift" | "kt" | "scala" | "html" | "htm" | "css"
        | "scss" | "sass" | "less" | "xml" | "vue" | "svelte" => FileKind::Code,
        "json" => FileKind::Json,
        "sh" | "bash" | "zsh" | "fish" | "ps1" => FileKind::Terminal,
        "txt" | "md" | "rtf" | "log" | "pdf" | "doc" | "docx" | "odt" => FileKind::Text,
        "csv" | "xls" | "xlsx" | "ods" => FileKind::Sheet,
        "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "env" | "properties" => FileKind::Config,
        "key" | "pem" | "crt" | "pub" | "pgp" | "gpg" | "asc" => FileKind::Key,
        "lock" => FileKind::Lock,
        "exe" | "dll" | "so" | "dylib" | "bin" | "app" => FileKind::Config,
        _ => FileKind::Generic,
    }
}

fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(index) if index > 0 => name[index + 1..].to_string(),
        _ => String::new(),
    }
}

pub fn glyph(icon: Icon, size: f32, color: Rgba) -> Svg {
    svg()
        .path(icon.asset_path())
        .w(px(size))
        .h(px(size))
        .flex_none()
        .text_color(color)
}

fn svg_source(path: &str) -> Option<&'static str> {
    Some(match path {
        "icons/folder" => folder_svg(),
        "icons/file" => FILE_PLAIN,
        "icons/file-code" => FILE_CODE,
        "icons/file-json" => FILE_JSON,
        "icons/file-text" => FILE_TEXT,
        "icons/file-image" => FILE_IMAGE,
        "icons/file-media" => FILE_MEDIA,
        "icons/file-archive" => FILE_ARCHIVE,
        "icons/file-terminal" => FILE_TERMINAL,
        "icons/file-config" => FILE_CONFIG,
        "icons/file-key" => FILE_KEY,
        "icons/file-lock" => FILE_LOCK,
        "icons/file-sheet" => FILE_SHEET,
        "icons/remote" => SVG_REMOTE,
        "icons/group" => SVG_GROUP,
        "icons/keyboard" => SVG_KEYBOARD,
        "icons/settings" => SVG_SETTINGS,
        "icons/mic" => SVG_MIC,
        "icons/branch" => SVG_BRANCH,
        "icons/chevron-down" => SVG_CHEVRON_DOWN,
        "icons/chevron-up" => SVG_CHEVRON_UP,
        "icons/arrow-up" => SVG_ARROW_UP,
        "icons/plus" => SVG_PLUS,
        "icons/close" => SVG_CLOSE,
        "icons/save" => SVG_SAVE,
        "icons/split" => SVG_SPLIT,
        "icons/agent" => SVG_AGENT,
        "icons/activity" => SVG_ACTIVITY,
        "icons/transfer" => SVG_TRANSFER,
        "icons/search" => SVG_SEARCH,
        _ => return None,
    })
}

fn folder_svg() -> &'static str {
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h3.4a1.5 1.5 0 0 1 1.1.5l1.2 1.3a1.5 1.5 0 0 0 1.1.5H19a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 18H5a1.5 1.5 0 0 1-1.5-1.5v-10Z"/></svg>"#
}

const FILE_PLAIN: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    "</svg>"
);
const FILE_CODE: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<path d="m10 13-2 2 2 2M14 17l2-2-2-2"/>"#,
    "</svg>"
);
const FILE_JSON: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<path d="M10 12.5c-.8 0-1.2.5-1.2 1.2S9.2 15 10 15M14 12.5c.8 0 1.2.5 1.2 1.2S14.8 15 14 15"/>"#,
    "</svg>"
);
const FILE_TEXT: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<path d="M8 12h8M8 15.5h6"/>"#,
    "</svg>"
);
const FILE_IMAGE: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<circle cx="9.5" cy="12" r="1.1"/><path d="m8 17 2.2-2.2 1.4 1.3L14 13.5 16.5 17"/>"#,
    "</svg>"
);
const FILE_MEDIA: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<path d="m11 12 4 2.4-4 2.4z"/>"#,
    "</svg>"
);
const FILE_ARCHIVE: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<path d="M12 10v8M10 12h4M10 15h4"/>"#,
    "</svg>"
);
const FILE_TERMINAL: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<path d="m9 12 2 2-2 2M13 16h3"/>"#,
    "</svg>"
);
const FILE_CONFIG: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<circle cx="12" cy="14" r="1.6"/><path d="M12 11.2v1.2M12 15.6v1.2M14.6 14h-1.1M10.5 14H9.4"/>"#,
    "</svg>"
);
const FILE_KEY: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<circle cx="10" cy="14" r="1.6"/><path d="M11.5 14H16v1.6"/>"#,
    "</svg>"
);
const FILE_LOCK: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<rect x="9" y="13" width="6" height="4.5" rx="1"/><path d="M10.2 13v-1.2a1.8 1.8 0 0 1 3.6 0V13"/>"#,
    "</svg>"
);
const FILE_SHEET: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h7L18 8v11.5A1 1 0 0 1 17 20.5H6.5a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1Z"/><path d="M13 3.5V8h4.5"/>"#,
    r#"<path d="M8 12h8M8 15h8M12 11v6"/>"#,
    "</svg>"
);

const SVG_REMOTE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.6 2.4 4 5.5 4 9s-1.4 6.6-4 9c-2.6-2.4-4-5.5-4-9s1.4-6.6 4-9Z"/></svg>"#;
const SVG_GROUP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 3 7.5l9 4.5 9-4.5L12 3Z"/><path d="m3 12 9 4.5L21 12M3 16.5 12 21l9-4.5"/></svg>"#;
const SVG_KEYBOARD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="5.5" width="19" height="13" rx="2.5"/><path d="M6 9h.01M9.5 9h.01M13 9h.01M16.5 9h.01M6 12.5h.01M9.5 12.5h.01M13 12.5h.01M16.5 12.5h.01M8 15.5h8"/></svg>"#;
const SVG_SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3.1"/><path d="M12 2.8v2.4M12 18.8v2.4M21.2 12h-2.4M5.2 12H2.8M18.5 5.5l-1.7 1.7M7.2 16.8l-1.7 1.7M18.5 18.5l-1.7-1.7M7.2 7.2 5.5 5.5"/></svg>"#;
const SVG_MIC: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="2.5" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M8.5 21h7"/></svg>"#;
const SVG_BRANCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="6.5" cy="5.5" r="2.2"/><circle cx="6.5" cy="18.5" r="2.2"/><circle cx="17.5" cy="9.5" r="2.2"/><path d="M6.5 7.7v8.6"/><path d="M6.5 11.7h6.5a3 3 0 0 0 3-3V11.7"/></svg>"#;
const SVG_CHEVRON_DOWN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
const SVG_CHEVRON_UP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m6 15 6-6 6 6"/></svg>"#;
const SVG_ARROW_UP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V6M6 11l6-6 6 6"/></svg>"#;
const SVG_PLUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>"#;
const SVG_CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6 6l12 12M18 6 6 18"/></svg>"#;
const SVG_SAVE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3.5h11l3 3V19a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 19V5a1.5 1.5 0 0 1 1-1.5Z"/><path d="M8 3.5v5h6v-5M8 20.5v-6h8v6"/></svg>"#;
const SVG_SPLIT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="4" width="17" height="16" rx="2"/><path d="M12 4v16"/></svg>"#;
const SVG_AGENT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m12 2.8 1.55 4.35L18 8.7l-4.45 1.55L12 14.6l-1.55-4.35L6 8.7l4.45-1.55L12 2.8Z"/><path d="m18.2 14.2.75 2.1 2.1.75-2.1.75-.75 2.1-.75-2.1-2.1-.75 2.1-.75.75-2.1Z"/></svg>"#;
const SVG_ACTIVITY: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 12h3.2l2.1-6.5 3.4 13 2.3-6.5H20.5"/></svg>"#;
const SVG_TRANSFER: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M7 4v16M4 7l3-3 3 3M17 20V4M14 17l3 3 3-3"/></svg>"#;
const SVG_SEARCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m20 20-4.5-4.5"/></svg>"#;

#[cfg(test)]
mod tests {
    use super::{file_kind, svg_source, FileKind, Icon};

    #[test]
    fn file_types_match_the_electron_map() {
        assert_eq!(file_kind("..", true), FileKind::Up);
        assert_eq!(file_kind("src", true), FileKind::Folder);
        assert_eq!(file_kind("main.rs", false), FileKind::Code);
        assert_eq!(file_kind("App.tsx", false), FileKind::Code);
        assert_eq!(file_kind("theme.json", false), FileKind::Json);
        assert_eq!(file_kind("README.md", false), FileKind::Text);
        assert_eq!(file_kind("shot.png", false), FileKind::Image);
        assert_eq!(file_kind("clip.mp4", false), FileKind::Media);
        assert_eq!(file_kind("pack.zip", false), FileKind::Archive);
        assert_eq!(file_kind("run.sh", false), FileKind::Terminal);
        assert_eq!(file_kind("Cargo.toml", false), FileKind::Config);
        assert_eq!(file_kind(".gitignore", false), FileKind::Config);
        assert_eq!(file_kind("id_rsa.pub", false), FileKind::Key);
        assert_eq!(file_kind("Cargo.lock", false), FileKind::Lock);
        assert_eq!(file_kind("sheet.csv", false), FileKind::Sheet);
        assert_eq!(file_kind("notes.binlog", false), FileKind::Generic);
    }

    #[test]
    fn every_icon_has_a_stroke_svg() {
        let icons = [
            Icon::Folder,
            Icon::File,
            Icon::FileCode,
            Icon::FileJson,
            Icon::FileText,
            Icon::FileImage,
            Icon::FileMedia,
            Icon::FileArchive,
            Icon::FileTerminal,
            Icon::FileConfig,
            Icon::FileKey,
            Icon::FileLock,
            Icon::FileSheet,
            Icon::Remote,
            Icon::Group,
            Icon::Keyboard,
            Icon::Settings,
            Icon::Mic,
            Icon::Branch,
            Icon::ChevronDown,
            Icon::ChevronUp,
            Icon::ArrowUp,
            Icon::Plus,
            Icon::Close,
            Icon::Save,
            Icon::Split,
            Icon::Agent,
            Icon::Activity,
            Icon::Transfer,
            Icon::Search,
        ];
        for icon in icons {
            let svg = svg_source(icon.asset_path()).unwrap();
            assert!(svg.contains("<svg"), "{}", icon.asset_path());
            assert!(
                svg.contains("<path") || svg.contains("<circle") || svg.contains("<rect"),
                "{}",
                icon.asset_path()
            );
        }
    }
}
