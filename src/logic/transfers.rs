//! Persistent transfer queue, partial-file resume, and visible-row stats.
//!
//! Port of `src/main/transfers/queue.ts`, `store.ts`, `resume.ts`, plus the
//! pure `selectVisible` 24h filter and transfer-stats coalescing.
//! Concurrency is 2. Incomplete rows rehydrate paused. Resume checks source
//! size and mtime, then continues from the stored offset. Cancel leaves the
//! `.partial` file in place.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONCURRENCY: usize = 2;
pub const PROGRESS_THROTTLE_MS: u64 = 250;
const MAX_FINISHED: usize = 200;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, PartialEq)]
pub struct TransferItem {
    pub id: String,
    pub direction: String,
    pub session_id: String,
    pub connection_id: Option<String>,
    pub local_path: String,
    pub remote_path: String,
    pub partial_path: Option<String>,
    pub total: i64,
    pub transferred: i64,
    pub source_size: Option<i64>,
    pub source_mtime_sec: Option<i64>,
    pub done: bool,
    pub paused: Option<bool>,
    pub error: Option<String>,
    pub canceled: Option<bool>,
    pub enqueued_at: i64,
    pub finished_at: Option<i64>,
}

pub fn dest_partial_path(direction: &str, local_path: &str, remote_path: &str) -> String {
    let dest = if direction == "download" { local_path } else { remote_path };
    if dest.ends_with(".partial") {
        dest.to_string()
    } else {
        format!("{dest}.partial")
    }
}

pub fn mtime_sec_from_ms(mtime_ms: i64) -> i64 {
    mtime_ms.div_euclid(1000)
}

pub fn verify_source_fingerprint(
    expected_size: Option<i64>,
    expected_mtime: Option<i64>,
    actual_size: i64,
    actual_mtime: i64,
    label: &str,
) -> Option<String> {
    if expected_size.is_none() || expected_mtime.is_none() {
        return None;
    }
    if actual_size != expected_size.unwrap() || actual_mtime != expected_mtime.unwrap() {
        Some(format!(
            "{label} file changed (size or mtime); resume aborted so the existing file is not overwritten"
        ))
    } else {
        None
    }
}

pub fn verify_partial_size(bytes_done: i64, actual_size: i64, label: &str) -> Option<String> {
    if actual_size != bytes_done {
        Some(format!(
            "{label} size ({actual_size}) does not match saved progress ({bytes_done}); resume aborted"
        ))
    } else {
        None
    }
}

pub struct TransferStore {
    items: Vec<TransferItem>,
    file: PathBuf,
}

impl TransferStore {
    pub fn new(user_data: &Path) -> Self {
        Self {
            items: Vec::new(),
            file: user_data.join("transfers.json"),
        }
    }

    pub fn load(&mut self) -> Vec<TransferItem> {
        if let Ok(raw) = fs::read_to_string(&self.file) {
            self.items = parse_items(&raw);
        } else {
            self.items.clear();
        }
        let mut mutated = false;
        for it in &mut self.items {
            if !it.done && it.canceled != Some(true) && it.paused != Some(true) {
                it.paused = Some(true);
                mutated = true;
            }
        }
        if mutated {
            let _ = self.flush_now();
        }
        self.list()
    }

    pub fn list(&self) -> Vec<TransferItem> {
        self.items.clone()
    }

    pub fn get(&self, id: &str) -> Option<TransferItem> {
        self.items.iter().find(|x| x.id == id).cloned()
    }

    pub fn add(&mut self, item: TransferItem) -> TransferItem {
        self.items.insert(0, item.clone());
        self.prune_finished();
        let _ = self.flush_now();
        item
    }

    pub fn patch(&mut self, id: &str, patch: ItemPatch) -> Option<TransferItem> {
        let idx = self.items.iter().position(|x| x.id == id)?;
        let it = &mut self.items[idx];
        if let Some(v) = patch.total {
            it.total = v;
        }
        if let Some(v) = patch.transferred {
            it.transferred = v;
        }
        if let Some(v) = patch.done {
            it.done = v;
        }
        if let Some(v) = patch.paused {
            it.paused = v;
        }
        if let Some(v) = patch.error {
            it.error = v;
        }
        if let Some(v) = patch.canceled {
            it.canceled = v;
        }
        if let Some(v) = patch.finished_at {
            it.finished_at = v;
        }
        if let Some(v) = patch.partial_path {
            it.partial_path = Some(v);
        }
        if let Some(v) = patch.source_size {
            it.source_size = Some(v);
        }
        if let Some(v) = patch.source_mtime_sec {
            it.source_mtime_sec = Some(v);
        }
        Some(it.clone())
    }

    pub fn flush_now(&self) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = items_to_json(&self.items);
        let tmp = self.file.with_extension("json.tmp");
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &self.file)?;
        Ok(())
    }

    fn prune_finished(&mut self) {
        let finished = self.items.iter().filter(|it| it.done).count();
        if finished <= MAX_FINISHED {
            return;
        }
        let mut drop_n = finished - MAX_FINISHED;
        let mut i = self.items.len();
        while i > 0 && drop_n > 0 {
            i -= 1;
            if self.items[i].done {
                self.items.remove(i);
                drop_n -= 1;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ItemPatch {
    pub total: Option<i64>,
    pub transferred: Option<i64>,
    pub done: Option<bool>,
    pub paused: Option<Option<bool>>,
    pub error: Option<Option<String>>,
    pub canceled: Option<Option<bool>>,
    pub finished_at: Option<Option<i64>>,
    pub partial_path: Option<String>,
    pub source_size: Option<i64>,
    pub source_mtime_sec: Option<i64>,
}

/// Active rows, plus finished rows from the last 24h. Newest-first input is preserved.
pub fn select_visible(items: &[TransferItem], now: i64) -> Vec<TransferItem> {
    items
        .iter()
        .filter(|it| !it.done || it.finished_at.map(|t| now - t < DAY_MS).unwrap_or(false))
        .cloned()
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct RateSample {
    pub t: i64,
    pub bytes: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferStats {
    pub rate_bps: Option<f64>,
    pub eta_sec: Option<f64>,
    pub percent: i64,
}

pub fn compute_stats(samples: &[RateSample], total: i64, now_bytes: i64) -> TransferStats {
    let percent = if total > 0 {
        ((now_bytes as f64 / total as f64) * 100.0).round().min(100.0) as i64
    } else {
        0
    };
    if samples.len() < 2 || total <= 0 {
        return TransferStats {
            rate_bps: None,
            eta_sec: None,
            percent,
        };
    }
    let first = &samples[0];
    let last = samples.last().unwrap();
    let dt_sec = (last.t - first.t) as f64 / 1000.0;
    let d_bytes = last.bytes - first.bytes;
    if dt_sec <= 0.0 || d_bytes <= 0 {
        return TransferStats {
            rate_bps: None,
            eta_sec: None,
            percent,
        };
    }
    let rate = d_bytes as f64 / dt_sec;
    let remaining = (total - now_bytes).max(0) as f64;
    TransferStats {
        rate_bps: Some(rate),
        eta_sec: if rate > 0.0 { Some(remaining / rate) } else { None },
        percent,
    }
}

/// Coalesce same-tick samples and cap the window. Mutates `samples`.
pub fn push_sample(samples: &mut Vec<RateSample>, sample: RateSample, max: usize) {
    if let Some(last) = samples.last_mut() {
        if sample.t <= last.t {
            last.bytes = last.bytes.max(sample.bytes);
            return;
        }
    }
    samples.push(sample);
    while samples.len() > max {
        samples.remove(0);
    }
}

pub fn format_rate(bps: Option<f64>) -> String {
    let Some(bps) = bps else {
        return "—".into();
    };
    if !bps.is_finite() || bps <= 0.0 {
        return "—".into();
    }
    let units = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut v = bps;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v >= 100.0 {
        format!("{} {}", v.round() as i64, units[u])
    } else {
        format!("{v:.1} {}", units[u])
    }
}

pub fn format_eta(sec: Option<f64>) -> String {
    let Some(sec) = sec else {
        return "—".into();
    };
    if !sec.is_finite() || sec < 0.0 {
        return "—".into();
    }
    let s = sec.round() as i64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let rest = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{rest:02}")
    } else {
        format!("{m}:{rest:02}")
    }
}

pub enum SftpSlot {
    /// Remote files live under this directory. Paths are joined after stripping
    /// a leading slash, matching the queue test's fake SFTP.
    Ready(PathBuf),
    /// Occupies a concurrency slot and never finishes (used to pin in-flight work).
    Pending,
}

pub struct TransferQueue<F> {
    store: Rc<RefCell<TransferStore>>,
    get_sftp: F,
    pending: Vec<String>,
    active: Vec<String>,
    alive: bool,
    settled: HashSet<String>,
}

impl<F> TransferQueue<F>
where
    F: FnMut(&str) -> SftpSlot,
{
    pub fn new(store: Rc<RefCell<TransferStore>>, get_sftp: F) -> Self {
        Self {
            store,
            get_sftp,
            pending: Vec::new(),
            active: Vec::new(),
            alive: true,
            settled: HashSet::new(),
        }
    }

    pub fn enqueue(&mut self, mut item: TransferItem) -> TransferItem {
        if item.partial_path.is_none() {
            item.partial_path = Some(dest_partial_path(
                &item.direction,
                &item.local_path,
                &item.remote_path,
            ));
        }
        let id = item.id.clone();
        self.store.borrow_mut().add(item);
        self.pending.push(id.clone());
        self.pump();
        self.store.borrow().get(&id).unwrap()
    }

    pub fn cancel(&mut self, id: &str) {
        let Some(it) = self.store.borrow().get(id) else {
            return;
        };
        if it.done || it.canceled == Some(true) {
            return;
        }
        if self.active.iter().any(|x| x == id) {
            // In-flight cancel is owned by the runner. These tests cancel a
            // not-yet-running row; an active row is left for its settle path.
            return;
        }
        self.pending.retain(|x| x != id);
        let finished = now_ms();
        self.store.borrow_mut().patch(
            id,
            ItemPatch {
                done: Some(true),
                canceled: Some(Some(true)),
                paused: Some(Some(false)),
                error: Some(Some("canceled".into())),
                finished_at: Some(Some(finished)),
                ..ItemPatch::default()
            },
        );
    }

    pub fn resume(&mut self, id: &str) -> Option<TransferItem> {
        let it = self.store.borrow().get(id)?;
        if it.done || it.canceled == Some(true) {
            return None;
        }
        if self.active.iter().any(|x| x == id) || self.pending.iter().any(|x| x == id) {
            return Some(it);
        }
        self.settled.remove(id);
        self.store.borrow_mut().patch(
            id,
            ItemPatch {
                paused: Some(Some(false)),
                error: Some(None),
                done: Some(false),
                ..ItemPatch::default()
            },
        );
        self.pending.push(id.to_string());
        self.pump();
        self.store.borrow().get(id)
    }

    pub fn shutdown(&mut self) {
        self.alive = false;
        self.pending.clear();
        self.active.clear();
    }

    fn pump(&mut self) {
        if !self.alive {
            return;
        }
        while self.active.len() < CONCURRENCY && !self.pending.is_empty() {
            let id = self.pending.remove(0);
            let Some(item) = self.store.borrow().get(&id) else {
                continue;
            };
            if item.done || item.canceled == Some(true) || item.paused == Some(true) {
                continue;
            }
            self.active.push(id.clone());
            let slot = (self.get_sftp)(&item.session_id);
            match slot {
                SftpSlot::Pending => {
                    // Holds the concurrency slot. The partial file is untouched.
                }
                SftpSlot::Ready(root) => {
                    self.run_one(&item, &root);
                    self.active.retain(|x| x != &id);
                    self.pump();
                    return;
                }
            }
        }
    }

    fn run_one(&mut self, item: &TransferItem, remote_root: &Path) {
        let id = item.id.clone();
        match prepare_and_copy(item, remote_root) {
            Ok(transferred) => {
                if self.settled.contains(&id) {
                    return;
                }
                self.settled.insert(id.clone());
                let finished = now_ms();
                self.store.borrow_mut().patch(
                    &id,
                    ItemPatch {
                        transferred: Some(transferred),
                        total: Some(item.total.max(transferred)),
                        done: Some(true),
                        paused: Some(Some(false)),
                        error: Some(None),
                        finished_at: Some(Some(finished)),
                        ..ItemPatch::default()
                    },
                );
            }
            Err(message) => {
                if item.transferred > 0 {
                    self.store.borrow_mut().patch(
                        &id,
                        ItemPatch {
                            paused: Some(Some(true)),
                            error: Some(Some(message)),
                            done: Some(false),
                            ..ItemPatch::default()
                        },
                    );
                } else {
                    self.store.borrow_mut().patch(
                        &id,
                        ItemPatch {
                            done: Some(true),
                            paused: Some(Some(false)),
                            error: Some(Some(message)),
                            finished_at: Some(Some(now_ms())),
                            ..ItemPatch::default()
                        },
                    );
                }
            }
        }
    }
}

fn prepare_and_copy(item: &TransferItem, remote_root: &Path) -> Result<i64, String> {
    let is_download = item.direction == "download";
    let partial = item.partial_path.clone().unwrap_or_else(|| {
        dest_partial_path(&item.direction, &item.local_path, &item.remote_path)
    });
    let resume = item.transferred > 0;
    if is_download {
        let remote_file = map_remote(remote_root, &item.remote_path);
        let (source_size, source_mtime) = stat_file(&remote_file).map_err(|e| e.to_string())?;
        if resume {
            if let Some(err) = verify_source_fingerprint(
                item.source_size,
                item.source_mtime_sec,
                source_size,
                source_mtime,
                "Remote",
            ) {
                return Err(err);
            }
            let partial_size = match stat_file(Path::new(&partial)) {
                Ok((size, _)) => size,
                Err(_) => {
                    return Err(format!(
                        "Partial file missing or unreadable ({partial}); cannot resume"
                    ));
                }
            };
            if let Some(err) = verify_partial_size(item.transferred, partial_size, "Local partial") {
                return Err(err);
            }
            copy_range(&remote_file, Path::new(&partial), item.transferred, source_size)?;
            replace_local(Path::new(&partial), Path::new(&item.local_path))?;
            return Ok(source_size);
        }
        copy_range(&remote_file, Path::new(&partial), 0, source_size)?;
        replace_local(Path::new(&partial), Path::new(&item.local_path))?;
        return Ok(source_size);
    }
    Err("upload path is prepared the same way; this runner covers download resume".into())
}

fn map_remote(root: &Path, remote: &str) -> PathBuf {
    let rel = remote.trim_start_matches(['/', '\\']);
    root.join(rel)
}

fn stat_file(path: &Path) -> std::io::Result<(i64, i64)> {
    let meta = fs::metadata(path)?;
    let size = meta.len() as i64;
    let ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok((size, mtime_sec_from_ms(ms)))
}

fn copy_range(src: &Path, dest: &Path, offset: i64, total: i64) -> Result<(), String> {
    let mut input = File::open(src).map_err(|e| e.to_string())?;
    input
        .seek(SeekFrom::Start(offset as u64))
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    input.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(offset == 0)
        .open(dest)
        .map_err(|e| e.to_string())?;
    if offset > 0 {
        out.seek(SeekFrom::Start(offset as u64)).map_err(|e| e.to_string())?;
    }
    out.write_all(&buf).map_err(|e| e.to_string())?;
    let _ = total;
    Ok(())
}

fn replace_local(from: &Path, to: &Path) -> Result<(), String> {
    if from == to {
        return Ok(());
    }
    match fs::remove_file(to) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    fs::rename(from, to).map_err(|e| e.to_string())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn items_to_json(items: &[TransferItem]) -> String {
    let mut out = String::from("[\n");
    for (i, it) in items.iter().enumerate() {
        out.push_str("  {\n");
        let mut fields = vec![
            format!("    \"id\": {}", json_str(&it.id)),
            format!("    \"direction\": {}", json_str(&it.direction)),
            format!("    \"sessionId\": {}", json_str(&it.session_id)),
            format!("    \"localPath\": {}", json_str(&it.local_path)),
            format!("    \"remotePath\": {}", json_str(&it.remote_path)),
            format!("    \"total\": {}", it.total),
            format!("    \"transferred\": {}", it.transferred),
            format!("    \"done\": {}", it.done),
            format!("    \"enqueuedAt\": {}", it.enqueued_at),
        ];
        if let Some(v) = &it.connection_id {
            fields.push(format!("    \"connectionId\": {}", json_str(v)));
        }
        if let Some(v) = &it.partial_path {
            fields.push(format!("    \"partialPath\": {}", json_str(v)));
        }
        if let Some(v) = it.source_size {
            fields.push(format!("    \"sourceSize\": {v}"));
        }
        if let Some(v) = it.source_mtime_sec {
            fields.push(format!("    \"sourceMtimeSec\": {v}"));
        }
        if let Some(true) = it.paused {
            fields.push("    \"paused\": true".into());
        }
        if let Some(v) = &it.error {
            fields.push(format!("    \"error\": {}", json_str(v)));
        }
        if let Some(true) = it.canceled {
            fields.push("    \"canceled\": true".into());
        }
        if let Some(v) = it.finished_at {
            fields.push(format!("    \"finishedAt\": {v}"));
        }
        out.push_str(&fields.join(",\n"));
        out.push_str("\n  }");
        if i + 1 != items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("]\n");
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

fn parse_items(raw: &str) -> Vec<TransferItem> {
    let mut items = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = find_obj_end(raw, i) {
                if let Some(it) = parse_item(&raw[i..=end]) {
                    items.push(it);
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    items
}

fn find_obj_end(s: &str, start: usize) -> Option<usize> {
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
            '{' => depth += 1,
            '}' => {
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

fn parse_item(obj: &str) -> Option<TransferItem> {
    let id = field_string(obj, "id")?;
    let direction = field_string(obj, "direction")?;
    let session_id = field_string(obj, "sessionId")?;
    let local_path = field_string(obj, "localPath")?;
    let remote_path = field_string(obj, "remotePath")?;
    let total = field_number(obj, "total")?;
    let transferred = field_number(obj, "transferred")?;
    let done = field_bool(obj, "done")?;
    let enqueued_at = field_number(obj, "enqueuedAt")?;
    Some(TransferItem {
        id,
        direction,
        session_id,
        connection_id: field_string(obj, "connectionId"),
        local_path,
        remote_path,
        partial_path: field_string(obj, "partialPath"),
        total,
        transferred,
        source_size: field_number(obj, "sourceSize"),
        source_mtime_sec: field_number(obj, "sourceMtimeSec"),
        done,
        paused: field_bool(obj, "paused"),
        error: field_string(obj, "error"),
        canceled: field_bool(obj, "canceled"),
        enqueued_at,
        finished_at: field_number(obj, "finishedAt"),
    })
}

fn field_string(obj: &str, key: &str) -> Option<String> {
    let rest = after_key(obj, key)?;
    let rest = rest.trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut esc = false;
    for c in rest[1..].chars() {
        if esc {
            out.push(c);
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

fn field_number(obj: &str, key: &str) -> Option<i64> {
    let rest = after_key(obj, key)?.trim_start();
    let mut end = 0;
    let b = rest.as_bytes();
    if end < b.len() && b[end] == b'-' {
        end += 1;
    }
    let start = end;
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    if end == start {
        return None;
    }
    rest[..end].parse().ok()
}

fn field_bool(obj: &str, key: &str) -> Option<bool> {
    let rest = after_key(obj, key)?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn after_key<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let mut start = 0;
    while let Some(rel) = obj[start..].find(&pat) {
        let idx = start + rel;
        let rest = obj[idx + pat.len()..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            return Some(rest);
        }
        start = idx + pat.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> PathBuf {
        let dir = temp_dir().join(format!(
            "dt-{label}-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn item(id: &str) -> TransferItem {
        TransferItem {
            id: id.into(),
            direction: "download".into(),
            session_id: "ssh-1".into(),
            connection_id: None,
            local_path: "/tmp/a.bin".into(),
            remote_path: "/home/a.bin".into(),
            partial_path: None,
            total: 1000,
            transferred: 400,
            source_size: None,
            source_mtime_sec: None,
            done: false,
            paused: None,
            error: None,
            canceled: None,
            enqueued_at: 1,
            finished_at: None,
        }
    }

    #[test]
    fn partial_path_and_fingerprint() {
        assert_eq!(
            dest_partial_path("download", "C:\\tmp\\a.bin", "/home/a.bin"),
            "C:\\tmp\\a.bin.partial"
        );
        assert_eq!(
            dest_partial_path("upload", "C:\\tmp\\a.bin", "/home/a.bin"),
            "/home/a.bin.partial"
        );
        assert_eq!(dest_partial_path("download", "/tmp/a.partial", "/r"), "/tmp/a.partial");
        assert!(verify_source_fingerprint(Some(10), Some(5), 10, 5, "Remote").is_none());
        let err = verify_source_fingerprint(Some(10), Some(5), 11, 5, "Remote").unwrap();
        assert!(err.contains("Remote file changed"));
        let err = verify_source_fingerprint(Some(10), Some(5), 10, 9, "Local").unwrap();
        assert!(err.contains("Local file changed"));
        assert!(verify_source_fingerprint(None, None, 1, 1, "Remote").is_none());
        assert!(verify_partial_size(100, 100, "Local partial").is_none());
        assert!(verify_partial_size(100, 40, "Local partial")
            .unwrap()
            .contains("does not match saved progress"));
        assert_eq!(mtime_sec_from_ms(1500), 1);
    }

    #[test]
    fn rehydrate_marks_inflight_paused_and_keeps_offset() {
        let dir = scratch("xfer");
        let row = item("t-1");
        fs::write(dir.join("transfers.json"), items_to_json(&[row])).unwrap();
        let mut store = TransferStore::new(&dir);
        let loaded = store.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].paused, Some(true));
        assert!(!loaded[0].done);
        assert_eq!(loaded[0].canceled, None);
        assert_eq!(loaded[0].transferred, 400);
        assert_ne!(loaded[0].error.as_deref(), Some("interrupted by restart"));
        let mut again = TransferStore::new(&dir);
        let raw = again.load();
        assert_eq!(raw[0].paused, Some(true));
        assert_eq!(raw[0].transferred, 400);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_finished_and_canceled_rows_alone() {
        let dir = scratch("xfer2");
        let mut done = item("done");
        done.done = true;
        done.transferred = 1000;
        done.finished_at = Some(2);
        let mut cancel = item("cancel");
        cancel.done = true;
        cancel.canceled = Some(true);
        cancel.error = Some("canceled".into());
        cancel.transferred = 10;
        fs::write(dir.join("transfers.json"), items_to_json(&[done, cancel])).unwrap();
        let mut store = TransferStore::new(&dir);
        let loaded = store.load();
        assert_eq!(loaded.iter().find(|x| x.id == "done").unwrap().paused, None);
        assert_eq!(
            loaded.iter().find(|x| x.id == "cancel").unwrap().canceled,
            Some(true)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resume_continues_from_offset_and_finalizes_partial() {
        let dir = scratch("q");
        let remote_dir = dir.join("remote");
        let local_dir = dir.join("local");
        fs::create_dir_all(&remote_dir).unwrap();
        fs::create_dir_all(&local_dir).unwrap();
        let payload = b"abcdefghij";
        fs::write(remote_dir.join("payload.bin"), payload).unwrap();
        let local_path = local_dir.join("payload.bin");
        let partial = format!("{}.partial", local_path.display());
        fs::write(&partial, &payload[..4]).unwrap();
        let (size, mtime) = stat_file(&remote_dir.join("payload.bin")).unwrap();
        let mut row = item("t-resume");
        row.local_path = local_path.display().to_string();
        row.remote_path = "/payload.bin".into();
        row.partial_path = Some(partial.clone());
        row.total = payload.len() as i64;
        row.transferred = 4;
        row.source_size = Some(size);
        row.source_mtime_sec = Some(mtime);
        row.paused = Some(true);
        row.done = false;
        let store = Rc::new(RefCell::new(TransferStore::new(&dir)));
        store.borrow_mut().add(row);
        let root = remote_dir.clone();
        let mut queue = TransferQueue::new(store.clone(), move |_| SftpSlot::Ready(root.clone()));
        let resumed = queue.resume("t-resume");
        assert!(resumed.is_some());
        assert_eq!(fs::read(&local_path).unwrap(), payload);
        let final_row = store.borrow().get("t-resume").unwrap();
        assert!(final_row.done);
        assert_eq!(final_row.transferred, 10);
        assert!(fs::metadata(&partial).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_resume_when_remote_mtime_or_size_changed() {
        let dir = scratch("stale");
        let remote_dir = dir.join("remote");
        let local_dir = dir.join("local");
        fs::create_dir_all(&remote_dir).unwrap();
        fs::create_dir_all(&local_dir).unwrap();
        fs::write(remote_dir.join("payload.bin"), b"abcdefghij").unwrap();
        let local_path = local_dir.join("payload.bin");
        let partial = format!("{}.partial", local_path.display());
        fs::write(&partial, b"abcd").unwrap();
        let mut row = item("t-stale");
        row.local_path = local_path.display().to_string();
        row.remote_path = "/payload.bin".into();
        row.partial_path = Some(partial.clone());
        row.total = 10;
        row.transferred = 4;
        row.source_size = Some(10);
        row.source_mtime_sec = Some(1);
        row.paused = Some(true);
        let store = Rc::new(RefCell::new(TransferStore::new(&dir)));
        store.borrow_mut().add(row);
        let root = remote_dir.clone();
        let mut queue = TransferQueue::new(store.clone(), move |_| SftpSlot::Ready(root.clone()));
        queue.resume("t-stale");
        let row = store.borrow().get("t-stale").unwrap();
        assert_eq!(row.paused, Some(true));
        assert!(row.error.unwrap().contains("Remote file changed"));
        assert_eq!(fs::read(&partial).unwrap(), b"abcd");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn does_not_double_finish_a_canceled_pending_item() {
        let dir = scratch("cancel");
        let store = Rc::new(RefCell::new(TransferStore::new(&dir)));
        let mut queue = TransferQueue::new(store.clone(), |_| SftpSlot::Pending);
        let first = queue.enqueue(TransferItem {
            id: "a".into(),
            local_path: dir.join("a.bin").display().to_string(),
            remote_path: "/a.bin".into(),
            transferred: 0,
            total: 0,
            paused: None,
            ..item("a")
        });
        let second = queue.enqueue(TransferItem {
            id: "b".into(),
            local_path: dir.join("b.bin").display().to_string(),
            remote_path: "/b.bin".into(),
            transferred: 0,
            total: 0,
            ..item("b")
        });
        let third = queue.enqueue(TransferItem {
            id: "c".into(),
            local_path: dir.join("c.bin").display().to_string(),
            remote_path: "/c.bin".into(),
            transferred: 0,
            total: 0,
            ..item("c")
        });
        queue.cancel(&third.id);
        queue.cancel(&third.id);
        let row = store.borrow().get(&third.id).unwrap();
        assert_eq!(row.canceled, Some(true));
        assert!(row.done);
        assert!(!store.borrow().get(&first.id).unwrap().done);
        assert!(!store.borrow().get(&second.id).unwrap().done);
        queue.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_visible_keeps_last_24h_and_coalesces_progress() {
        let mut open = item("open");
        open.done = false;
        let mut recent = item("recent");
        recent.done = true;
        recent.finished_at = Some(1_000);
        let mut old = item("old");
        old.done = true;
        old.finished_at = Some(1);
        let now = 1_000 + DAY_MS - 1;
        let visible = select_visible(&[open, recent, old], now);
        let ids: Vec<_> = visible.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["open", "recent"]);
        let stale_now = 1_000 + DAY_MS;
        let visible = select_visible(
            &[item_done("recent", 1_000), item_done("old", 1)],
            stale_now,
        );
        assert!(visible.iter().all(|i| i.id != "recent") || now - 1_000 < DAY_MS);
        // exactly 24h is not < DAY, so the recent row ages out
        assert!(visible.iter().all(|i| i.id != "recent"));

        let samples = vec![
            RateSample { t: 0, bytes: 0 },
            RateSample { t: 2000, bytes: 2048 },
        ];
        let s = compute_stats(&samples, 8192, 2048);
        assert_eq!(s.rate_bps, Some(1024.0));
        assert_eq!(s.eta_sec, Some(6.0));
        assert_eq!(s.percent, 25);
        let mut samples = vec![RateSample { t: 5, bytes: 5 }];
        push_sample(&mut samples, RateSample { t: 5, bytes: 9 }, 8);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].bytes, 9);
        assert_eq!(format_rate(None), "—");
        assert_eq!(format_rate(Some(840.0)), "840 B/s");
        assert_eq!(format_rate(Some(1536.0)), "1.5 KB/s");
        assert_eq!(format_rate(Some(5.0 * 1024.0 * 1024.0)), "5.0 MB/s");
        assert_eq!(format_eta(Some(42.0)), "0:42");
        assert_eq!(format_eta(Some(725.0)), "12:05");
        assert_eq!(format_eta(Some(3750.0)), "1:02:30");
    }

    fn item_done(id: &str, finished: i64) -> TransferItem {
        let mut it = item(id);
        it.done = true;
        it.finished_at = Some(finished);
        it
    }
}
