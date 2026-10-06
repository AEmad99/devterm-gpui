//! Pure git status porcelain parsing and commit-graph lane layout.
//!
//! Port of the porcelain parser in `src/main/git/index.ts` (`parsePorcelain`,
//! `parseBranch`, `pickStatus`) and `layoutGraph` from
//! `src/renderer/components/git/gitGraphLayout.ts`. No git binary and no
//! mutating commands (no commit, push, or other write-side tools).

pub const MAX_ENTRIES: usize = 5000;

pub type GitFileStatus = char;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitStatus {
    pub is_repo: bool,
    pub branch: String,
    pub ahead: i64,
    pub behind: i64,
    pub entries: Vec<(String, GitFileStatus)>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitLogEntry {
    pub sha: String,
    pub parents: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneRow {
    pub sha: String,
    pub lane: usize,
    pub parents: Vec<String>,
    pub lanes_before: Vec<usize>,
    pub lanes_after: Vec<usize>,
    pub first_parent_lane: i64,
    pub merge_lanes: Vec<usize>,
    pub is_merge: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphLayout {
    pub rows: Vec<LaneRow>,
    pub max_lane: i64,
}

pub fn not_a_repo() -> GitStatus {
    GitStatus {
        is_repo: false,
        branch: String::new(),
        ahead: -1,
        behind: -1,
        entries: Vec::new(),
        truncated: false,
    }
}

pub fn parse_porcelain(stdout: &str) -> GitStatus {
    let mut branch = String::new();
    let mut ahead = -1i64;
    let mut behind = -1i64;
    let mut entries: Vec<(String, GitFileStatus)> = Vec::new();
    let mut truncated = false;
    for raw in stdout.split('\n') {
        let raw = raw.trim_end_matches('\r');
        if raw.is_empty() {
            continue;
        }
        if raw.starts_with("## ") {
            let h = parse_branch(raw);
            branch = h.0;
            ahead = h.1;
            behind = h.2;
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            continue;
        }
        if let Some((status, path)) = parse_porcelain_line(raw) {
            if let Some(slot) = entries.iter_mut().find(|(p, _)| p == &path) {
                slot.1 = status;
            } else {
                entries.push((path, status));
            }
        }
    }
    GitStatus {
        is_repo: true,
        branch,
        ahead,
        behind,
        entries,
        truncated,
    }
}

fn parse_branch(line: &str) -> (String, i64, i64) {
    if !line.starts_with("## ") {
        return (String::new(), -1, -1);
    }
    let mut body = line[3..].to_string();
    let mut counters = String::new();
    if let Some(bracket) = body.find(" [") {
        if body.ends_with(']') {
            counters = body[bracket + 2..body.len() - 1].to_string();
            body = body[..bracket].to_string();
        }
    }
    let branch = match body.find("...") {
        Some(sep) => body[..sep].to_string(),
        None => body,
    };
    let mut ahead = -1i64;
    let mut behind = -1i64;
    if !counters.is_empty() {
        for piece in counters.split(',') {
            let piece = piece.trim();
            if let Some(rest) = piece.strip_prefix("ahead ") {
                if let Ok(n) = rest.parse::<i64>() {
                    if rest.chars().all(|c| c.is_ascii_digit()) {
                        ahead = n;
                    }
                }
            }
            if let Some(rest) = piece.strip_prefix("behind ") {
                if rest.chars().all(|c| c.is_ascii_digit()) {
                    if let Ok(n) = rest.parse::<i64>() {
                        behind = n;
                    }
                }
            }
        }
    }
    (branch, ahead, behind)
}

fn pick_status(index: char, worktree: char) -> GitFileStatus {
    if worktree == '?' || worktree == '!' {
        return '?';
    }
    let xy = format!("{index}{worktree}");
    if matches!(
        xy.as_str(),
        "UU" | "AA" | "DD" | "AU" | "UA" | "DU" | "UD"
    ) || worktree == 'U'
        || index == 'U'
    {
        return 'U';
    }
    if worktree == 'R' || index == 'R' {
        return 'R';
    }
    if worktree == 'D' || index == 'D' {
        return 'D';
    }
    if index == 'A' || worktree == 'A' {
        return 'A';
    }
    'M'
}

fn parse_porcelain_line(line: &str) -> Option<(GitFileStatus, String)> {
    if line.chars().count() < 4 {
        return None;
    }
    let mut chars = line.chars();
    let index = chars.next().unwrap_or(' ');
    let worktree = chars.next().unwrap_or(' ');
    let mut rel: String = line.chars().skip(3).collect();
    if worktree == 'R' || index == 'R' {
        if let Some(arrow) = rel.find(" -> ") {
            rel = rel[arrow + 4..].to_string();
        }
    }
    if rel.starts_with('"') && rel.ends_with('"') && rel.len() >= 2 {
        rel = unescape_git_path(&rel);
    }
    Some((pick_status(index, worktree), rel))
}

fn unescape_git_path(quoted: &str) -> String {
    let inner = &quoted[1..quoted.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Lane assignment matching `layoutGraph` / GitGraphView.
pub fn layout_graph(entries: &[GitLogEntry]) -> GraphLayout {
    let mut rows = Vec::new();
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut max_lane: i64 = -1;

    let first_free = |lanes: &[Option<String>]| -> usize {
        lanes.iter().position(|l| l.is_none()).unwrap_or(lanes.len())
    };
    let occupied = |lanes: &[Option<String>]| -> Vec<usize> {
        lanes
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.as_ref().map(|_| i))
            .collect()
    };

    for entry in entries {
        let mut lane = lanes.iter().position(|l| l.as_deref() == Some(entry.sha.as_str()));
        let lane = if let Some(l) = lane.take() {
            l
        } else {
            let f = first_free(&lanes);
            if f == lanes.len() {
                lanes.push(None);
            }
            f
        };
        let lanes_before = occupied(&lanes);
        lanes[lane] = None;
        let first_parent = entry.parents.first().cloned();
        if let Some(fp) = first_parent.clone() {
            lanes[lane] = Some(fp);
        }
        let mut merge_lanes = Vec::new();
        for p in entry.parents.iter().skip(1) {
            let new_lane = first_free(&lanes);
            if new_lane == lanes.len() {
                lanes.push(Some(p.clone()));
            } else {
                lanes[new_lane] = Some(p.clone());
            }
            merge_lanes.push(new_lane);
        }
        let lanes_after = occupied(&lanes);
        for l in &lanes_after {
            if *l as i64 > max_lane {
                max_lane = *l as i64;
            }
        }
        if lane as i64 > max_lane {
            max_lane = lane as i64;
        }
        rows.push(LaneRow {
            sha: entry.sha.clone(),
            lane,
            parents: entry.parents.clone(),
            lanes_before,
            lanes_after,
            first_parent_lane: if first_parent.is_some() { lane as i64 } else { -1 },
            merge_lanes,
            is_merge: entry.parents.len() > 1,
        });
    }
    GraphLayout { rows, max_lane }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_ahead_behind_and_staged_unstaged() {
        let stdout = "\
## main...origin/main [ahead 2, behind 1]
M  src/staged.ts
 M src/unstaged.ts
A  src/added.ts
D  src/deleted.ts
?? src/untracked.ts
R  src/old.ts -> src/new.ts
UU src/conflict.ts
";
        let st = parse_porcelain(stdout);
        assert!(st.is_repo);
        assert_eq!(st.branch, "main");
        assert_eq!(st.ahead, 2);
        assert_eq!(st.behind, 1);
        let get = |p: &str| st.entries.iter().find(|(path, _)| path == p).map(|(_, s)| *s);
        assert_eq!(get("src/staged.ts"), Some('M'));
        assert_eq!(get("src/unstaged.ts"), Some('M'));
        assert_eq!(get("src/added.ts"), Some('A'));
        assert_eq!(get("src/deleted.ts"), Some('D'));
        assert_eq!(get("src/untracked.ts"), Some('?'));
        assert_eq!(get("src/new.ts"), Some('R'));
        assert_eq!(get("src/conflict.ts"), Some('U'));
        assert!(!st.truncated);
    }

    #[test]
    fn detached_head_and_no_upstream() {
        let det = parse_porcelain("## HEAD (no branch)\n");
        assert_eq!(det.branch, "HEAD (no branch)");
        assert_eq!(det.ahead, -1);
        assert_eq!(det.behind, -1);
        let plain = parse_porcelain("## main\n");
        assert_eq!(plain.branch, "main");
        assert_eq!(plain.ahead, -1);
    }

    #[test]
    fn not_a_repo_shape() {
        let st = not_a_repo();
        assert!(!st.is_repo);
        assert_eq!(st.branch, "");
        assert_eq!(st.ahead, -1);
        assert_eq!(st.behind, -1);
        assert!(st.entries.is_empty());
    }

    #[test]
    fn graph_lays_out_a_merge() {
        let entries = vec![
            GitLogEntry {
                sha: "c".into(),
                parents: vec!["b".into(), "a".into()],
            },
            GitLogEntry {
                sha: "b".into(),
                parents: vec!["a".into()],
            },
            GitLogEntry {
                sha: "a".into(),
                parents: vec![],
            },
        ];
        let g = layout_graph(&entries);
        assert_eq!(g.rows[0].sha, "c");
        assert_eq!(g.rows[0].lane, 0);
        assert!(g.rows[0].is_merge);
        assert_eq!(g.rows[0].first_parent_lane, 0);
        assert_eq!(g.rows[0].merge_lanes, vec![1]);
        assert!(g.max_lane >= 1);
        assert!(!g.rows[2].is_merge);
        assert_eq!(g.rows[2].first_parent_lane, -1);
    }
}
