//! Local editor. Files over 5 MiB are refused with the original limit text.
//! Save writes the newline the file had when it was opened. Markdown preview
//! is the sanitized HTML; nothing in that string is executed.

use std::fs;
use std::path::{Path, PathBuf};

use crate::logic::markdown_preview::{
    next_markdown_preview_mode, render_markdown_to_safe_html, MarkdownPreviewMode,
};

pub const MAX_EDIT_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct EditorDoc {
    pub path: PathBuf,
    pub text: String,
    pub newline: String,
    pub dirty: bool,
    pub mode: MarkdownPreviewMode,
}

pub fn open_file(path: &Path) -> Result<EditorDoc, String> {
    let meta = fs::metadata(path).map_err(|err| err.to_string())?;
    if meta.len() > MAX_EDIT_BYTES {
        let mb = (meta.len() as f64 / 1024.0 / 1024.0).round() as u64;
        return Err(format!(
            "File is too large to edit ({mb} MB; limit {} MB)",
            MAX_EDIT_BYTES / 1024 / 1024
        ));
    }
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    if bytes.len() as u64 > MAX_EDIT_BYTES {
        return Err(format!(
            "Refusing to write more than {} MB",
            MAX_EDIT_BYTES / 1024 / 1024
        ));
    }
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let newline = if raw.contains("\r\n") {
        "\r\n".to_string()
    } else {
        "\n".to_string()
    };
    Ok(EditorDoc {
        path: path.to_path_buf(),
        text: raw.replace("\r\n", "\n"),
        newline,
        dirty: false,
        mode: MarkdownPreviewMode::Edit,
    })
}

pub fn save_file(doc: &EditorDoc) -> Result<(), String> {
    let body = if doc.newline == "\r\n" {
        doc.text.replace('\n', "\r\n")
    } else {
        doc.text.clone()
    };
    if body.len() as u64 > MAX_EDIT_BYTES {
        return Err(format!(
            "Refusing to write more than {} MB",
            MAX_EDIT_BYTES / 1024 / 1024
        ));
    }
    fs::write(&doc.path, body).map_err(|err| err.to_string())
}

pub fn preview_html(doc: &EditorDoc) -> String {
    render_markdown_to_safe_html(&doc.text)
}

pub fn cycle_preview(doc: &mut EditorDoc) {
    doc.mode = next_markdown_preview_mode(Some(doc.mode));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn refuses_a_file_over_five_mib() {
        let dir = std::env::temp_dir().join(format!("devterm-edit-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("big.txt");
        let mut file = fs::File::create(&path).unwrap();
        let chunk = vec![b'a'; 1024 * 1024];
        for _ in 0..6 {
            file.write_all(&chunk).unwrap();
        }
        drop(file);
        let err = open_file(&path).unwrap_err();
        assert!(err.contains("File is too large to edit"));
        assert!(err.contains("limit 5 MB"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_restores_crlf() {
        let dir = std::env::temp_dir().join(format!("devterm-eol-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("note.txt");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        let mut doc = open_file(&path).unwrap();
        assert_eq!(doc.newline, "\r\n");
        doc.text.push_str("c\n");
        save_file(&doc).unwrap();
        let saved = fs::read(&path).unwrap();
        assert!(saved.windows(2).any(|pair| pair == b"\r\n"));
        assert!(
            !String::from_utf8_lossy(&saved).contains("\n\n")
                || saved.windows(2).filter(|p| *p == b"\r\n").count() >= 2
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
