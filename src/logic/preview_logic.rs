//! Local preview static server and annotation comments.
//!
//! Port of `src/main/preview/static-server.ts` and `annotations.ts`.
//! A preview is a local http(s) URL or a folder served on 127.0.0.1.
//! Comments persist as JSON under `userData/annotations/<session>.json`.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const KINDS: &[&str] = &["pin", "rect", "text"];

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewAnnotation {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub body: String,
    pub created_at: f64,
    pub screenshot_ref: Option<String>,
}

pub fn annotation_file_name(session_id: &str) -> String {
    let mut s = session_id.replace("..", "_");
    let mut out = String::new();
    for c in s.drain(..) {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    // The TS replace of `..` runs first, then the charset replace. A run of
    // disallowed chars collapses to a single `_` because of `+` in the regex.
    // Re-do with the regex semantics: replace `..` globally, then runs.
    let collapsed = collapse_unsafe(&session_id.replace("..", "_"));
    let name: String = collapsed.chars().take(180).collect();
    if name.is_empty() {
        "session".into()
    } else {
        name
    }
}

fn collapse_unsafe(s: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            pending = false;
            out.push(c);
        } else if !pending {
            pending = true;
            out.push('_');
        }
    }
    out
}

fn clamp01(n: f64) -> f64 {
    if !n.is_finite() {
        return 0.0;
    }
    n.clamp(0.0, 1.0)
}

pub fn normalize_annotation(raw: Option<&AnnInput>) -> Option<PreviewAnnotation> {
    let o = raw?;
    let id = o.id.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
    let kind = o.kind.as_deref().filter(|k| KINDS.contains(k))?;
    let body = o
        .body
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(4000)
        .collect::<String>();
    let x = clamp01(o.x.unwrap_or(f64::NAN));
    let y = clamp01(o.y.unwrap_or(f64::NAN));
    let w = clamp01(o.w.unwrap_or(f64::NAN));
    let h = clamp01(o.h.unwrap_or(f64::NAN));
    let created_at = match o.created_at {
        Some(n) if n.is_finite() => n,
        _ => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as f64)
            .unwrap_or(0.0),
    };
    Some(PreviewAnnotation {
        id: id.to_string(),
        kind: kind.to_string(),
        x,
        y,
        w: if kind == "pin" { 0.0 } else { w },
        h: if kind == "pin" { 0.0 } else { h },
        body,
        created_at,
        screenshot_ref: o.screenshot_ref.clone(),
    })
}

/// Loose input so tests can omit fields the way the TS objects do.
#[derive(Clone, Debug, Default)]
pub struct AnnInput {
    pub id: Option<String>,
    pub kind: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub body: Option<String>,
    pub created_at: Option<f64>,
    pub screenshot_ref: Option<String>,
}

pub struct AnnotationStore {
    user_data: PathBuf,
}

impl AnnotationStore {
    pub fn new(user_data: impl Into<PathBuf>) -> Self {
        Self {
            user_data: user_data.into(),
        }
    }

    fn dir(&self) -> PathBuf {
        self.user_data.join("annotations")
    }

    fn file(&self, session_id: &str) -> PathBuf {
        self.dir()
            .join(format!("{}.json", annotation_file_name(session_id)))
    }

    pub fn load(&self, session_id: &str) -> Vec<PreviewAnnotation> {
        let raw = match fs::read_to_string(self.file(session_id)) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        parse_annotation_list(&raw)
            .into_iter()
            .filter_map(|a| normalize_annotation(Some(&a)))
            .collect()
    }

    pub fn save(&self, session_id: &str, annotations: &[PreviewAnnotation]) -> std::io::Result<()> {
        fs::create_dir_all(self.dir())?;
        let cleaned: Vec<PreviewAnnotation> = annotations
            .iter()
            .filter_map(|a| {
                normalize_annotation(Some(&AnnInput {
                    id: Some(a.id.clone()),
                    kind: Some(a.kind.clone()),
                    x: Some(a.x),
                    y: Some(a.y),
                    w: Some(a.w),
                    h: Some(a.h),
                    body: Some(a.body.clone()),
                    created_at: Some(a.created_at),
                    screenshot_ref: a.screenshot_ref.clone(),
                }))
            })
            .collect();
        let body = annotations_to_json(&cleaned);
        let mut f = File::create(self.file(session_id))?;
        f.write_all(body.as_bytes())?;
        Ok(())
    }
}

fn annotations_to_json(items: &[PreviewAnnotation]) -> String {
    if items.is_empty() {
        return "[]".into();
    }
    let mut out = String::from("[\n");
    for (i, a) in items.iter().enumerate() {
        out.push_str("  {\n");
        out.push_str(&format!("    \"id\": {},\n", json_str(&a.id)));
        out.push_str(&format!("    \"kind\": {},\n", json_str(&a.kind)));
        out.push_str(&format!("    \"x\": {},\n", json_num(a.x)));
        out.push_str(&format!("    \"y\": {},\n", json_num(a.y)));
        out.push_str(&format!("    \"w\": {},\n", json_num(a.w)));
        out.push_str(&format!("    \"h\": {},\n", json_num(a.h)));
        out.push_str(&format!("    \"body\": {},\n", json_str(&a.body)));
        out.push_str(&format!("    \"createdAt\": {}", json_num(a.created_at)));
        if let Some(ref shot) = a.screenshot_ref {
            out.push_str(",\n");
            out.push_str(&format!("    \"screenshotRef\": {}", json_str(shot)));
        }
        out.push('\n');
        out.push_str("  }");
        if i + 1 != items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push(']');
    out
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn json_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Minimal reader for the arrays this module writes (and a wrapped
/// `{ "annotations": [...] }` envelope).
fn parse_annotation_list(raw: &str) -> Vec<AnnInput> {
    let text = raw.trim();
    let body = if text.starts_with('{') {
        if let Some(idx) = text.find("\"annotations\"") {
            let rest = &text[idx..];
            if let Some(b) = rest.find('[') {
                let slice = &rest[b..];
                if let Some(end) = slice.rfind(']') {
                    &slice[..=end]
                } else {
                    return Vec::new();
                }
            } else {
                return Vec::new();
            }
        } else {
            return Vec::new();
        }
    } else {
        text
    };
    if !body.starts_with('[') {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = find_matching(body, i, '{', '}') {
                out.push(parse_ann_object(&body[i..=end]));
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn find_matching(s: &str, start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut in_str = false;
    let mut esc = false;
    for (idx, c) in s[start..].char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_ann_object(obj: &str) -> AnnInput {
    AnnInput {
        id: json_field_string(obj, "id"),
        kind: json_field_string(obj, "kind"),
        x: json_field_number(obj, "x"),
        y: json_field_number(obj, "y"),
        w: json_field_number(obj, "w"),
        h: json_field_number(obj, "h"),
        body: json_field_string(obj, "body"),
        created_at: json_field_number(obj, "createdAt"),
        screenshot_ref: json_field_string(obj, "screenshotRef"),
    }
}

fn json_field_string(obj: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let idx = obj.find(&pat)?;
    let rest = &obj[idx + pat.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut esc = false;
    for c in rest[1..].chars() {
        if esc {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' {
            break;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn json_field_number(obj: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{key}\"");
    let idx = obj.find(&pat)?;
    let rest = &obj[idx + pat.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let mut end = 0;
    let b = rest.as_bytes();
    if end < b.len() && (b[end] == b'-' || b[end] == b'+') {
        end += 1;
    }
    let start_digits = end;
    while end < b.len()
        && (b[end].is_ascii_digit()
            || b[end] == b'.'
            || b[end] == b'e'
            || b[end] == b'E'
            || b[end] == b'+'
            || b[end] == b'-')
    {
        end += 1;
    }
    if end == start_digits {
        return None;
    }
    rest[..end].parse().ok()
}

/// Reject paths that escape `root`. `None` means traversal.
pub fn resolve_safe_path(root: &Path, url_path: &str) -> Option<PathBuf> {
    let no_query = url_path.split('?').next().unwrap_or("/");
    let decoded = percent_decode(no_query);
    let trimmed = decoded.trim_start_matches('/');
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut abs = root.clone();
    for comp in Path::new(trimmed).components() {
        match comp {
            Component::Normal(p) => abs.push(p),
            Component::CurDir => {}
            Component::ParentDir => return None,
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if abs != root && !abs.starts_with(&root) {
        return None;
    }
    Some(abs)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub struct FolderServe {
    pub id: String,
    pub url: String,
    pub port: u16,
    pub folder_path: PathBuf,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl FolderServe {
    pub fn close(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FolderServe {
    fn drop(&mut self) {
        self.close();
    }
}

/// Serve `folder_path` on 127.0.0.1. Preview never binds a public interface.
pub fn start_folder_server(folder_path: &Path) -> Result<FolderServe, String> {
    let root = folder_path
        .canonicalize()
        .map_err(|_| format!("Folder is not a directory: {}", folder_path.display()))?;
    if !root.is_dir() {
        return Err(format!(
            "Folder is not a directory: {}",
            folder_path.display()
        ));
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| "Failed to bind preview server".to_string())?;
    let port = listener
        .local_addr()
        .map_err(|_| "Failed to bind preview server".to_string())?
        .port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let root_thread = root.clone();
    let join = thread::spawn(move || {
        let _ = listener.set_nonblocking(false);
        loop {
            if stop_thread.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if stop_thread.load(Ordering::SeqCst) {
                        break;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = handle_preview(&mut stream, &root_thread);
                }
                Err(_) => break,
            }
        }
    });
    Ok(FolderServe {
        id: format!("preview-{port}"),
        url: format!("http://127.0.0.1:{port}/"),
        port,
        folder_path: root,
        stop,
        join: Some(join),
    })
}

fn handle_preview(stream: &mut TcpStream, root: &Path) -> std::io::Result<()> {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let Some(mut abs) = resolve_safe_path(root, path) else {
        let body = b"forbidden";
        write!(
            stream,
            "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        stream.write_all(body)?;
        return Ok(());
    };
    if abs.is_dir() {
        abs = abs.join("index.html");
    }
    if !abs.is_file() {
        let body = b"not found";
        write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        stream.write_all(body)?;
        return Ok(());
    }
    let data = fs::read(&abs).unwrap_or_default();
    let mime = mime_of(&abs);
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        data.len()
    )?;
    stream.write_all(&data)?;
    Ok(())
}

fn mime_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// Preview targets: an http(s) URL, or a folder that will be served on loopback.
pub fn preview_is_local_http(url: &str) -> bool {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::io::Read;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn sanitizes_session_ids_used_as_filenames() {
        assert_eq!(annotation_file_name("../evil/id"), "__evil_id");
        assert_eq!(annotation_file_name("preview-abc"), "preview-abc");
    }

    #[test]
    fn drops_malformed_rows_and_clamps_coordinates() {
        let a = normalize_annotation(Some(&AnnInput {
            id: Some("p1".into()),
            kind: Some("rect".into()),
            x: Some(1.5),
            y: Some(-0.2),
            w: Some(0.4),
            h: Some(0.2),
            body: Some("n".into()),
            created_at: Some(1.0),
            screenshot_ref: None,
        }));
        let a = a.unwrap();
        assert_eq!(a.x, 1.0);
        assert_eq!(a.y, 0.0);
        assert!(normalize_annotation(Some(&AnnInput {
            kind: Some("pin".into()),
            ..AnnInput::default()
        }))
        .is_none());
    }

    #[test]
    fn round_trips_json_under_user_data_annotations() {
        let dir = temp_dir().join(format!(
            "dt-ann-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let store = AnnotationStore::new(&dir);
        store
            .save(
                "s1",
                &[PreviewAnnotation {
                    id: "a".into(),
                    kind: "pin".into(),
                    x: 0.2,
                    y: 0.3,
                    w: 0.0,
                    h: 0.0,
                    body: "here".into(),
                    created_at: 9.0,
                    screenshot_ref: None,
                }],
            )
            .unwrap();
        let loaded = store.load("s1");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].body, "here");
        let raw = fs::read_to_string(dir.join("annotations").join("s1.json")).unwrap();
        assert!(raw.contains("\"pin\""));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_path_traversal() {
        let root = temp_dir().join("dt-preview-root");
        assert!(resolve_safe_path(&root, "/../../etc/passwd").is_none());
    }

    #[test]
    fn serves_index_html_from_a_local_folder_on_loopback() {
        let dir = temp_dir().join(format!(
            "dt-preview-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.html"), "<h1>preview-ok</h1>").unwrap();
        let mut serve = start_folder_server(&dir).unwrap();
        assert!(serve.url.starts_with("http://127.0.0.1:"));
        assert!(serve.url.ends_with('/'));
        let mut stream = TcpStream::connect(("127.0.0.1", serve.port)).unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        assert!(buf.starts_with("HTTP/1.1 200"));
        assert!(buf.contains("preview-ok"));
        serve.close();
        let _ = fs::remove_dir_all(&dir);
    }
}
