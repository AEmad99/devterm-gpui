//! Workspace capture, launch bookkeeping, rename, duplicate, and auto-launch.
//!
//! Port of the pure helpers in `src/renderer/lib/workspace.ts` and the store
//! mutations in `src/main/ipc/workspaces.ts`. Ad-hoc SSH sessions (remote with
//! no `connectionId`) are skipped on capture. Groups remember
//! `launchedFromWorkspaceId`. Save keeps launch stats the renderer did not
//! send back.

pub const DEFAULT_GROUP: &str = "default";

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub group_id: Option<String>,
    pub closed: bool,
    pub kind: String,
    pub connection_id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceItem {
    pub id: String,
    pub kind: String,
    pub connection_id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Leaf {
        tabs: Vec<String>,
        active: String,
    },
    Split {
        dir: String,
        sizes: Vec<f64>,
        children: Vec<LayoutNode>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub items: Vec<WorkspaceItem>,
    pub layout: Option<LayoutNode>,
    /// Legacy pre-1.0.1 field. Migrated into `items` on load.
    pub connection_ids: Option<Vec<String>>,
    pub last_launched_at: Option<i64>,
    pub launch_count: Option<u64>,
    pub auto_launch: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupFlag {
    pub launched_from_workspace_id: String,
}

pub fn capturable_sessions(sessions: &[Session], group_id: &str) -> Vec<Session> {
    sessions
        .iter()
        .filter(|s| {
            s.group_id.as_deref().unwrap_or(DEFAULT_GROUP) == group_id
                && !s.closed
                && !s.id.starts_with("pending-")
                && (s.kind == "local" || (s.kind == "remote" && s.connection_id.is_some()))
        })
        .cloned()
        .collect()
}

pub fn capture_workspace(
    sessions: &[Session],
    group_id: &str,
    layout_root: Option<&LayoutNode>,
) -> (Vec<WorkspaceItem>, Option<LayoutNode>) {
    let capturable = capturable_sessions(sessions, group_id);
    let mut sid_to_item = Vec::new();
    let items: Vec<WorkspaceItem> = capturable
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let id = format!("wi-{i}-{}", s.id);
            sid_to_item.push((s.id.clone(), id.clone()));
            WorkspaceItem {
                id,
                kind: if s.kind == "remote" {
                    "remote"
                } else {
                    "local"
                }
                .into(),
                connection_id: if s.kind == "remote" {
                    s.connection_id.clone()
                } else {
                    None
                },
                cwd: s.cwd.clone(),
                title: s.title.clone(),
            }
        })
        .collect();
    let layout = layout_root.and_then(|n| snapshot_node(n, &sid_to_item));
    (items, layout)
}

fn lookup<'a>(map: &'a [(String, String)], key: &str) -> Option<&'a str> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn snapshot_node(n: &LayoutNode, items: &[(String, String)]) -> Option<LayoutNode> {
    match n {
        LayoutNode::Leaf { tabs, active } => {
            let mut kept = Vec::new();
            for sid in tabs {
                if let Some(iid) = lookup(items, sid) {
                    if !kept.iter().any(|t| t == iid) {
                        kept.push(iid.to_string());
                    }
                }
            }
            if kept.is_empty() {
                return None;
            }
            let active = active
                .as_str()
                .split(' ')
                .next()
                .and_then(|a| lookup(items, a))
                .map(|s| s.to_string())
                .unwrap_or_else(|| kept.last().cloned().unwrap_or_default());
            let active = if kept.iter().any(|t| t == &active) {
                active
            } else {
                kept.last().cloned().unwrap_or_default()
            };
            Some(LayoutNode::Leaf { tabs: kept, active })
        }
        LayoutNode::Split {
            dir,
            sizes,
            children,
        } => {
            let mut kept = Vec::new();
            let mut kept_sizes = Vec::new();
            for (i, c) in children.iter().enumerate() {
                if let Some(r) = snapshot_node(c, items) {
                    kept.push(r);
                    kept_sizes.push(*sizes.get(i).unwrap_or(&1.0));
                }
            }
            if kept.is_empty() {
                return None;
            }
            if kept.len() == 1 {
                return Some(kept.remove(0));
            }
            let total: f64 = kept_sizes.iter().sum::<f64>();
            let total = if total == 0.0 {
                kept.len() as f64
            } else {
                total
            };
            Some(LayoutNode::Split {
                dir: dir.clone(),
                sizes: kept_sizes.into_iter().map(|s| s / total).collect(),
                children: kept,
            })
        }
    }
}

pub fn to_live_snapshot(n: &LayoutNode, map: &[(String, String)]) -> Option<LayoutNode> {
    snapshot_node(n, map)
}

pub fn migrate(mut w: Workspace) -> Workspace {
    if w.connection_ids.is_some() && w.items.is_empty() {
        let ids = w.connection_ids.take().unwrap_or_default();
        w.items = ids
            .into_iter()
            .map(|cid| WorkspaceItem {
                id: cid.clone(),
                kind: "remote".into(),
                connection_id: Some(cid),
                cwd: None,
                title: None,
            })
            .collect();
    }
    w
}

#[derive(Clone, Debug, Default)]
pub struct WorkspaceStore {
    pub workspaces: Vec<Workspace>,
    pub group_flags: Vec<(String, GroupFlag)>,
    next_id: u64,
}

impl WorkspaceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> &[Workspace] {
        &self.workspaces
    }

    pub fn auto_launch(&self) -> Vec<Workspace> {
        self.workspaces
            .iter()
            .filter(|w| w.auto_launch)
            .cloned()
            .collect()
    }

    /// Insert or replace. Launch stats the caller omitted are kept from the
    /// existing row (save-back from the capture flow).
    pub fn save(&mut self, mut ws: Workspace) -> &[Workspace] {
        if ws.id.is_empty() {
            self.next_id += 1;
            ws.id = format!("ws-{}", self.next_id);
        }
        ws = migrate(ws);
        if let Some(existing) = self.workspaces.iter().find(|w| w.id == ws.id) {
            if ws.last_launched_at.is_none() {
                ws.last_launched_at = existing.last_launched_at;
            }
            if ws.launch_count.is_none() {
                ws.launch_count = existing.launch_count;
            }
        }
        if let Some(slot) = self.workspaces.iter_mut().find(|w| w.id == ws.id) {
            *slot = ws;
        } else {
            self.workspaces.push(ws);
        }
        &self.workspaces
    }

    pub fn rename(&mut self, id: &str, name: &str) -> &[Workspace] {
        if let Some(ws) = self.workspaces.iter_mut().find(|w| w.id == id) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                ws.name = trimmed.to_string();
            }
        }
        &self.workspaces
    }

    pub fn duplicate(&mut self, id: &str) -> Option<&Workspace> {
        let src = self.workspaces.iter().find(|w| w.id == id)?.clone();
        self.next_id += 1;
        let new_id = format!("ws-copy-{}", self.next_id);
        let mut id_map = Vec::new();
        let new_items: Vec<WorkspaceItem> = src
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                self.next_id += 1;
                let nid = format!("wi-{}", self.next_id + i as u64);
                id_map.push((it.id.clone(), nid.clone()));
                WorkspaceItem {
                    id: nid,
                    ..it.clone()
                }
            })
            .collect();
        let layout = src.layout.as_ref().and_then(|n| remap_layout(n, &id_map));
        let copy = Workspace {
            id: new_id,
            name: format!("{} (copy)", src.name),
            description: src.description.clone(),
            items: new_items,
            layout,
            connection_ids: None,
            last_launched_at: None,
            launch_count: None,
            auto_launch: src.auto_launch,
        };
        self.workspaces.push(copy);
        self.workspaces.last()
    }

    pub fn record_launch(&mut self, id: &str, now: i64) -> Option<&Workspace> {
        let ws = self.workspaces.iter_mut().find(|w| w.id == id)?;
        ws.launch_count = Some(ws.launch_count.unwrap_or(0) + 1);
        ws.last_launched_at = Some(now);
        Some(ws)
    }

    pub fn set_auto_launch(&mut self, id: &str, value: bool) {
        if let Some(ws) = self.workspaces.iter_mut().find(|w| w.id == id) {
            ws.auto_launch = value;
        }
    }

    /// Open a workspace into a fresh group. `record` is false for boot
    /// auto-launch. The group remembers which workspace it came from.
    pub fn launch_into_group(&mut self, id: &str, now: i64, record: bool) -> Option<LaunchResult> {
        let ws = self.workspaces.iter().find(|w| w.id == id)?.clone();
        let group_id = format!("ws-{}-{now}", ws.id);
        self.group_flags.push((
            group_id.clone(),
            GroupFlag {
                launched_from_workspace_id: ws.id.clone(),
            },
        ));
        let mut session_map = Vec::new();
        let mut failures = Vec::new();
        for (i, it) in ws.items.iter().enumerate() {
            if it.kind == "local" {
                session_map.push((it.id.clone(), format!("local-{i}")));
            } else if it.connection_id.is_some() {
                session_map.push((it.id.clone(), format!("pending-remote-{i}")));
            } else {
                failures.push(format!(
                    "{}: saved connection not found",
                    it.title.as_deref().unwrap_or("Remote host")
                ));
            }
        }
        if record {
            self.record_launch(id, now);
        }
        Some(LaunchResult {
            group_id,
            session_map,
            launched_from_workspace_id: ws.id,
            failures,
            recorded: record,
        })
    }

    pub fn launched_from(&self, group_id: &str) -> Option<&str> {
        self.group_flags
            .iter()
            .find(|(id, _)| id == group_id)
            .map(|(_, f)| f.launched_from_workspace_id.as_str())
    }
}

fn remap_layout(n: &LayoutNode, map: &[(String, String)]) -> Option<LayoutNode> {
    match n {
        LayoutNode::Leaf { tabs, active } => {
            let tabs: Vec<String> = tabs
                .iter()
                .filter_map(|t| lookup(map, t).map(|s| s.to_string()))
                .collect();
            if tabs.is_empty() {
                return None;
            }
            let active = lookup(map, active)
                .map(|s| s.to_string())
                .filter(|a| tabs.contains(a))
                .unwrap_or_else(|| tabs.last().cloned().unwrap_or_default());
            Some(LayoutNode::Leaf { tabs, active })
        }
        LayoutNode::Split {
            dir,
            sizes,
            children,
        } => {
            let children: Vec<LayoutNode> = children
                .iter()
                .filter_map(|c| remap_layout(c, map))
                .collect();
            if children.is_empty() {
                return None;
            }
            Some(LayoutNode::Split {
                dir: dir.clone(),
                sizes: sizes.clone(),
                children,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchResult {
    pub group_id: String,
    pub session_map: Vec<(String, String)>,
    pub launched_from_workspace_id: String,
    pub failures: Vec<String>,
    pub recorded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(id: &str, group: &str) -> Session {
        Session {
            id: id.into(),
            group_id: Some(group.into()),
            closed: false,
            kind: "local".into(),
            connection_id: None,
            cwd: Some("/work".into()),
            title: Some("shell".into()),
        }
    }

    #[test]
    fn capture_skips_adhoc_ssh_without_connection_id() {
        let sessions = vec![
            local("l1", "g1"),
            Session {
                id: "r-saved".into(),
                group_id: Some("g1".into()),
                closed: false,
                kind: "remote".into(),
                connection_id: Some("conn-1".into()),
                cwd: Some("/home".into()),
                title: Some("prod".into()),
            },
            Session {
                id: "r-adhoc".into(),
                group_id: Some("g1".into()),
                closed: false,
                kind: "remote".into(),
                connection_id: None,
                cwd: Some("/tmp".into()),
                title: Some("adhoc".into()),
            },
            Session {
                id: "pending-1".into(),
                group_id: Some("g1".into()),
                closed: false,
                kind: "local".into(),
                connection_id: None,
                cwd: None,
                title: None,
            },
            Session {
                id: "b1".into(),
                group_id: Some("g1".into()),
                closed: false,
                kind: "browser".into(),
                connection_id: None,
                cwd: None,
                title: Some("web".into()),
            },
        ];
        let (items, _) = capture_workspace(&sessions, "g1", None);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.kind == "local"));
        assert!(items
            .iter()
            .any(|i| i.connection_id.as_deref() == Some("conn-1")));
        assert!(items.iter().all(|i| i.title.as_deref() != Some("adhoc")));
    }

    #[test]
    fn save_back_keeps_launch_stats_rename_duplicate_and_auto_launch() {
        let mut store = WorkspaceStore::new();
        store.save(Workspace {
            id: "w1".into(),
            name: "Ops".into(),
            description: None,
            items: vec![WorkspaceItem {
                id: "i1".into(),
                kind: "local".into(),
                connection_id: None,
                cwd: Some("/work".into()),
                title: Some("sh".into()),
            }],
            layout: Some(LayoutNode::Leaf {
                tabs: vec!["i1".into()],
                active: "i1".into(),
            }),
            connection_ids: None,
            last_launched_at: None,
            launch_count: None,
            auto_launch: false,
        });
        store.record_launch("w1", 50);
        store.save(Workspace {
            id: "w1".into(),
            name: "Ops".into(),
            description: None,
            items: vec![WorkspaceItem {
                id: "i1".into(),
                kind: "local".into(),
                connection_id: None,
                cwd: Some("/work".into()),
                title: Some("sh".into()),
            }],
            layout: Some(LayoutNode::Leaf {
                tabs: vec!["i1".into()],
                active: "i1".into(),
            }),
            connection_ids: None,
            last_launched_at: None,
            launch_count: None,
            auto_launch: false,
        });
        let saved = store.list().iter().find(|w| w.id == "w1").unwrap();
        assert_eq!(saved.launch_count, Some(1));
        assert_eq!(saved.last_launched_at, Some(50));
        store.rename("w1", "  Fleet  ");
        assert_eq!(store.list()[0].name, "Fleet");
        store.rename("w1", "   ");
        assert_eq!(store.list()[0].name, "Fleet");
        store.set_auto_launch("w1", true);
        assert_eq!(store.auto_launch().len(), 1);
        let copy = store.duplicate("w1").unwrap().clone();
        assert_eq!(copy.name, "Fleet (copy)");
        assert!(copy.launch_count.is_none());
        assert!(copy.last_launched_at.is_none());
        assert_ne!(copy.id, "w1");
        assert_ne!(copy.items[0].id, "i1");
        match copy.layout.unwrap() {
            LayoutNode::Leaf { tabs, active } => {
                assert_eq!(tabs, vec![copy.items[0].id.clone()]);
                assert_eq!(active, copy.items[0].id);
            }
            _ => panic!("leaf"),
        }
    }

    #[test]
    fn launch_remembers_workspace_and_auto_launch_does_not_count() {
        let mut store = WorkspaceStore::new();
        store.save(Workspace {
            id: "w1".into(),
            name: "Ops".into(),
            description: None,
            items: vec![
                WorkspaceItem {
                    id: "i1".into(),
                    kind: "local".into(),
                    connection_id: None,
                    cwd: Some("/work".into()),
                    title: None,
                },
                WorkspaceItem {
                    id: "i2".into(),
                    kind: "remote".into(),
                    connection_id: None,
                    cwd: None,
                    title: Some("gone".into()),
                },
            ],
            layout: None,
            connection_ids: None,
            last_launched_at: None,
            launch_count: None,
            auto_launch: true,
        });
        let boot = store.launch_into_group("w1", 10, false).unwrap();
        assert!(!boot.recorded);
        assert_eq!(store.launched_from(&boot.group_id), Some("w1"));
        assert_eq!(store.list()[0].launch_count, None);
        assert!(boot
            .failures
            .iter()
            .any(|f| f.contains("saved connection not found")));
        let manual = store.launch_into_group("w1", 11, true).unwrap();
        assert!(manual.recorded);
        assert_eq!(store.list()[0].launch_count, Some(1));
        assert_eq!(store.list()[0].last_launched_at, Some(11));
        assert_eq!(store.launched_from(&manual.group_id), Some("w1"));
    }

    #[test]
    fn migrates_legacy_connection_ids() {
        let mut store = WorkspaceStore::new();
        store.save(Workspace {
            id: "old".into(),
            name: "Legacy".into(),
            description: None,
            items: vec![],
            layout: None,
            connection_ids: Some(vec!["c1".into(), "c2".into()]),
            last_launched_at: None,
            launch_count: None,
            auto_launch: false,
        });
        let w = &store.list()[0];
        assert_eq!(w.items.len(), 2);
        assert_eq!(w.items[0].id, "c1");
        assert_eq!(w.items[0].connection_id.as_deref(), Some("c1"));
        assert_eq!(w.items[0].kind, "remote");
    }
}
