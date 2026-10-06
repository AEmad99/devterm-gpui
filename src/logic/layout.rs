//! N-ary split tree for terminal panes.
//!
//! A leaf holds tabs. A split divides row or column with fractional sizes.
//! The home group id is "default" and its display name is "Group 1". The home
//! group tab stays when it is the only group. Hidden groups stay in the model.
//! Focus mode and zen mode are flags; they do not scale terminal text.
//! Closing a group closes only that group's sessions. Grid restore uses
//! `restore_group`, the same path as a normal group.

#![allow(dead_code)]

use std::collections::HashMap;

pub const DEFAULT_GROUP: &str = "default";
pub const HOME_GROUP_NAME: &str = "Group 1";

/// Focus mode magnifies one pane. It does not scale terminal text.
pub fn focus_mode_scales_text() -> bool {
    false
}

/// Zen mode hides chrome. It does not scale terminal text.
pub fn zen_mode_scales_text() -> bool {
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDir {
    Row,
    Col,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeafNode {
    pub id: String,
    pub tabs: Vec<String>,
    pub active: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitNode {
    pub id: String,
    pub dir: SplitDir,
    pub children: Vec<LayoutNode>,
    pub sizes: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Leaf(LeafNode),
    Split(SplitNode),
}

impl LayoutNode {
    pub fn as_leaf(&self) -> Option<&LeafNode> {
        match self {
            LayoutNode::Leaf(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_split(&self) -> Option<&SplitNode> {
        match self {
            LayoutNode::Split(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutSnapshot {
    Leaf {
        tabs: Vec<String>,
        active: Option<String>,
    },
    Split {
        dir: SplitDir,
        sizes: Vec<f64>,
        children: Vec<LayoutSnapshot>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeafRect {
    pub leaf: LeafNode,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitHandle {
    pub split_id: String,
    pub index: usize,
    pub dir: SplitDir,
    pub rect: Rect,
    pub span: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutGeom {
    pub leaves: Vec<LeafRect>,
    pub handles: Vec<SplitHandle>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub root: Option<LayoutNode>,
    pub active_leaf: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupFlag {
    pub launched_from_workspace_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRef {
    pub id: String,
    pub group_id: Option<String>,
}

struct IdGen {
    n: u64,
}

impl IdGen {
    fn fresh(&mut self, prefix: &str) -> String {
        self.n += 1;
        format!("{prefix}-{}", self.n)
    }
}

#[derive(Clone, Debug)]
struct RootState {
    root: Option<LayoutNode>,
    active_leaf: Option<String>,
}

pub struct LayoutStore {
    pub groups: Vec<Group>,
    pub active_group_id: String,
    pub focused_id: Option<String>,
    pub group_flags: HashMap<String, GroupFlag>,
    pub zen_mode: bool,
    ids: IdGen,
}

impl LayoutStore {
    pub fn new() -> Self {
        Self {
            groups: vec![Group {
                id: DEFAULT_GROUP.to_string(),
                name: HOME_GROUP_NAME.to_string(),
                root: None,
                active_leaf: None,
            }],
            active_group_id: DEFAULT_GROUP.to_string(),
            focused_id: None,
            group_flags: HashMap::new(),
            zen_mode: false,
            ids: IdGen { n: 1000 },
        }
    }

    pub fn set_state(
        &mut self,
        groups: Vec<Group>,
        active_group_id: &str,
        focused_id: Option<String>,
        group_flags: HashMap<String, GroupFlag>,
    ) {
        self.groups = groups;
        self.active_group_id = active_group_id.to_string();
        self.focused_id = focused_id;
        self.group_flags = group_flags;
    }

    /// Focus and zen never scale terminal text.
    pub fn text_scale(&self) -> f64 {
        let _ = (self.focused_id.as_ref(), self.zen_mode);
        1.0
    }

    pub fn set_focus(&mut self, id: Option<String>) {
        self.focused_id = id;
    }

    pub fn toggle_focus(&mut self, sid: Option<&str>) {
        self.focused_id = match sid {
            Some(sid) if self.focused_id.as_deref() != Some(sid) => Some(sid.to_string()),
            _ => None,
        };
    }

    pub fn set_zen_mode(&mut self, on: bool) {
        self.zen_mode = on;
    }

    pub fn sync(&mut self, sessions: &[SessionRef]) {
        let mut ids_by_group: Vec<(String, Vec<String>)> = Vec::new();
        for session in sessions {
            let gid = session
                .group_id
                .clone()
                .filter(|g| !g.is_empty())
                .unwrap_or_else(|| DEFAULT_GROUP.to_string());
            if let Some((_, ids)) = ids_by_group.iter_mut().find(|(id, _)| id == &gid) {
                ids.push(session.id.clone());
            } else {
                ids_by_group.push((gid, vec![session.id.clone()]));
            }
        }
        let mut known: HashMap<String, Group> = self
            .groups
            .iter()
            .cloned()
            .map(|g| (g.id.clone(), g))
            .collect();
        let mut order: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        for (gid, _) in &ids_by_group {
            if !known.contains_key(gid) {
                known.insert(
                    gid.clone(),
                    Group {
                        id: gid.clone(),
                        name: gid.clone(),
                        root: None,
                        active_leaf: None,
                    },
                );
                order.push(gid.clone());
            }
        }
        let mut groups = Vec::new();
        for gid in &order {
            let prev = known.get(gid).cloned().unwrap();
            let ids = ids_by_group
                .iter()
                .find(|(id, _)| id == gid)
                .map(|(_, ids)| ids.clone())
                .unwrap_or_default();
            if ids.is_empty() && gid != DEFAULT_GROUP {
                continue;
            }
            let rec = reconcile(
                RootState {
                    root: prev.root.clone(),
                    active_leaf: prev.active_leaf.clone(),
                },
                &ids,
                &mut self.ids,
            );
            if rec.root == prev.root && rec.active_leaf == prev.active_leaf {
                groups.push(prev);
            } else {
                groups.push(Group {
                    root: rec.root,
                    active_leaf: rec.active_leaf,
                    ..prev
                });
            }
        }
        if !groups.iter().any(|g| g.id == DEFAULT_GROUP) {
            let prev = known.get(DEFAULT_GROUP);
            groups.insert(
                0,
                Group {
                    id: DEFAULT_GROUP.to_string(),
                    name: prev
                        .map(|g| g.name.clone())
                        .unwrap_or_else(|| HOME_GROUP_NAME.to_string()),
                    root: None,
                    active_leaf: None,
                },
            );
        }
        let mut active_group_id = if groups.iter().any(|g| g.id == self.active_group_id) {
            self.active_group_id.clone()
        } else {
            DEFAULT_GROUP.to_string()
        };
        if let Some(active) = groups.iter().find(|g| g.id == active_group_id) {
            if active.root.is_none() {
                if let Some(populated) = groups.iter().find(|g| g.root.is_some()) {
                    active_group_id = populated.id.clone();
                }
            }
        }
        let live: std::collections::HashSet<&str> =
            sessions.iter().map(|s| s.id.as_str()).collect();
        let focused_id = self
            .focused_id
            .as_ref()
            .filter(|id| live.contains(id.as_str()))
            .cloned();
        let live_groups: std::collections::HashSet<&str> =
            groups.iter().map(|g| g.id.as_str()).collect();
        let mut pruned = false;
        for gid in self.group_flags.keys() {
            if !live_groups.contains(gid.as_str()) {
                pruned = true;
            }
        }
        if pruned {
            self.group_flags
                .retain(|gid, _| live_groups.contains(gid.as_str()));
        }
        self.groups = groups;
        self.active_group_id = active_group_id;
        self.focused_id = focused_id;
    }

    pub fn ensure_group(&mut self, id: &str, name: &str) {
        if self.groups.iter().any(|g| g.id == id) {
            return;
        }
        self.groups.push(Group {
            id: id.to_string(),
            name: name.to_string(),
            root: None,
            active_leaf: None,
        });
    }

    pub fn create_group(&mut self, name: Option<&str>) -> String {
        let id = self.ids.fresh("grp");
        let used: std::collections::HashSet<String> =
            self.groups.iter().map(|g| g.name.clone()).collect();
        let mut n = 1;
        while used.contains(&format!("Group {n}")) {
            n += 1;
        }
        let group = Group {
            id: id.clone(),
            name: name.unwrap_or(&format!("Group {n}")).to_string(),
            root: None,
            active_leaf: None,
        };
        self.groups.push(group);
        self.active_group_id = id.clone();
        self.focused_id = None;
        id
    }

    pub fn set_active_group(&mut self, id: &str) {
        if !self.groups.iter().any(|g| g.id == id) {
            return;
        }
        self.active_group_id = id.to_string();
        self.focused_id = None;
    }

    pub fn replace_session_id(&mut self, old_id: &str, new_id: &str) {
        if old_id == new_id {
            return;
        }
        let mut changed = false;
        for group in &mut self.groups {
            let Some(root) = &group.root else {
                continue;
            };
            if !all_leaves(root)
                .iter()
                .any(|l| l.tabs.iter().any(|t| t == old_id))
            {
                continue;
            }
            changed = true;
            group.root = Some(replace_tab_id(root.clone(), old_id, new_id));
        }
        if changed && self.focused_id.as_deref() == Some(old_id) {
            self.focused_id = Some(new_id.to_string());
        }
    }

    pub fn set_active_tab(&mut self, leaf_id: &str, sid: &str) {
        let Some(idx) = self.active_index() else {
            return;
        };
        if let Some(root) = self.groups[idx].root.clone() {
            self.groups[idx].root = Some(update_leaf(root, leaf_id, &|l| {
                if l.tabs.iter().any(|t| t == sid) {
                    LeafNode {
                        active: Some(sid.to_string()),
                        ..l
                    }
                } else {
                    l
                }
            }));
        }
        self.groups[idx].active_leaf = Some(leaf_id.to_string());
    }

    pub fn focus_leaf(&mut self, leaf_id: &str) {
        let idx = self.active_index();
        let Some(idx) = idx else {
            return;
        };
        let Some(root) = self.groups[idx].root.clone() else {
            return;
        };
        if find_leaf(&root, leaf_id).is_none() {
            return;
        }
        self.groups[idx].active_leaf = Some(leaf_id.to_string());
    }

    pub fn reorder_tab(&mut self, sid: &str, target_leaf_id: &str, index: usize) {
        self.edit_active(|root, _active, _ids| {
            let src = leaf_of(&root, sid)?;
            let root = if src.id != target_leaf_id {
                remove_tab(Some(root), sid)?
            } else {
                update_leaf(root, target_leaf_id, &|l| LeafNode {
                    tabs: l.tabs.into_iter().filter(|t| t != sid).collect(),
                    ..l
                })
            };
            if find_leaf(&root, target_leaf_id).is_none() {
                let active = first_leaf(&root).id;
                return Some((root, Some(active)));
            }
            let root = update_leaf(root, target_leaf_id, &|l| {
                let mut tabs = l.tabs;
                let at = index.min(tabs.len());
                tabs.insert(at, sid.to_string());
                LeafNode {
                    tabs,
                    active: Some(sid.to_string()),
                    ..l
                }
            });
            Some((root, Some(target_leaf_id.to_string())))
        });
    }

    pub fn drop(&mut self, sid: &str, target_leaf_id: &str, zone: DropZone) {
        let new_leaf_id_holder: Option<String> = None;
        let _ = new_leaf_id_holder;
        let idx = match self.active_index() {
            Some(i) => i,
            None => return,
        };
        let Some(root0) = self.groups[idx].root.clone() else {
            return;
        };
        let Some(src) = leaf_of(&root0, sid) else {
            return;
        };
        if zone == DropZone::Center {
            let root = if src.id != target_leaf_id {
                remove_tab(Some(root0), sid)
            } else {
                Some(update_leaf(root0, target_leaf_id, &|l| LeafNode {
                    tabs: l.tabs.into_iter().filter(|t| t != sid).collect(),
                    ..l
                }))
            };
            let Some(root) = root else {
                self.groups[idx].root = None;
                self.groups[idx].active_leaf = None;
                return;
            };
            if find_leaf(&root, target_leaf_id).is_none() {
                let active = first_leaf(&root).id;
                self.groups[idx].root = Some(root);
                self.groups[idx].active_leaf = Some(active);
                return;
            }
            let root = update_leaf(root, target_leaf_id, &|l| {
                let mut tabs = l.tabs;
                tabs.push(sid.to_string());
                LeafNode {
                    tabs,
                    active: Some(sid.to_string()),
                    ..l
                }
            });
            self.groups[idx].root = Some(root);
            self.groups[idx].active_leaf = Some(target_leaf_id.to_string());
            return;
        }
        if src.id == target_leaf_id && src.tabs.len() == 1 {
            return;
        }
        let Some(root0) = remove_tab(Some(root0), sid) else {
            self.groups[idx].root = None;
            self.groups[idx].active_leaf = None;
            return;
        };
        if find_leaf(&root0, target_leaf_id).is_none() {
            let active = first_leaf(&root0).id;
            self.groups[idx].root = Some(root0);
            self.groups[idx].active_leaf = Some(active);
            return;
        }
        let nl = mk_leaf(&mut self.ids, vec![sid.to_string()]);
        let nl_id = nl.id.clone();
        let split_id = self.ids.fresh("split");
        let root = replace_leaf(root0, target_leaf_id, &|t| {
            make_split(split_id.clone(), zone, nl.clone(), LayoutNode::Leaf(t))
        });
        self.groups[idx].root = Some(root);
        self.groups[idx].active_leaf = Some(nl_id);
    }

    pub fn split_right(&mut self, anchor_sid: &str, new_sid: &str) {
        self.split_beside(anchor_sid, new_sid, DropZone::Right);
    }

    pub fn split_down(&mut self, anchor_sid: &str, new_sid: &str) {
        self.split_beside(anchor_sid, new_sid, DropZone::Bottom);
    }

    pub fn split_beside(&mut self, anchor_sid: &str, new_sid: &str, zone: DropZone) {
        if anchor_sid == new_sid {
            return;
        }
        let Some(group_idx) = self.groups.iter().position(|g| {
            g.root.as_ref().is_some_and(|root| {
                leaf_of(root, anchor_sid).is_some() || leaf_of(root, new_sid).is_some()
            })
        }) else {
            return;
        };
        let Some(root) = self.groups[group_idx].root.clone() else {
            return;
        };
        let Some(target) = leaf_of(&root, anchor_sid) else {
            return;
        };
        let Some(src) = leaf_of(&root, new_sid) else {
            return;
        };
        if src.id == target.id && src.tabs.len() == 1 {
            return;
        }
        let root = if src.id == target.id {
            update_leaf(root, &target.id, &|l| {
                let rest: Vec<String> = l.tabs.into_iter().filter(|t| t != new_sid).collect();
                let active = if l.active.as_deref() == Some(new_sid) {
                    rest.last().cloned()
                } else {
                    l.active
                };
                LeafNode {
                    tabs: rest,
                    active,
                    id: l.id,
                }
            })
        } else {
            match remove_tab(Some(root), new_sid) {
                Some(root) => root,
                None => return,
            }
        };
        if find_leaf(&root, &target.id).is_none() {
            return;
        }
        let nl = mk_leaf(&mut self.ids, vec![new_sid.to_string()]);
        let nl_id = nl.id.clone();
        let split_id = self.ids.fresh("split");
        let target_id = target.id.clone();
        let root = replace_leaf(root, &target_id, &|t| {
            make_split(split_id.clone(), zone, nl.clone(), LayoutNode::Leaf(t))
        });
        self.groups[group_idx].root = Some(root);
        self.groups[group_idx].active_leaf = Some(nl_id);
        self.focused_id = None;
    }

    pub fn split_new_beside(&mut self, anchor_sid: &str, new_sid: &str, zone: DropZone) -> bool {
        if anchor_sid.is_empty() || anchor_sid == new_sid {
            return false;
        }
        let Some(group_idx) = self.groups.iter().position(|g| {
            g.root
                .as_ref()
                .is_some_and(|root| leaf_of(root, anchor_sid).is_some())
        }) else {
            return false;
        };
        let Some(mut root) = self.groups[group_idx].root.clone() else {
            return false;
        };
        if leaf_of(&root, new_sid).is_some() {
            root = match detach_tab(root, new_sid) {
                Some(root) => root,
                None => return false,
            };
        }
        let Some(target) = leaf_of(&root, anchor_sid) else {
            return false;
        };
        let nl = mk_leaf(&mut self.ids, vec![new_sid.to_string()]);
        let split_id = self.ids.fresh("split");
        let target_id = target.id.clone();
        root = replace_leaf(root, &target_id, &|t| {
            make_split(split_id.clone(), zone, nl.clone(), LayoutNode::Leaf(t))
        });
        self.groups[group_idx].root = Some(root);
        true
    }

    pub fn set_leaf_active_tab(&mut self, leaf_id: &str, sid: &str) {
        for group in &mut self.groups {
            let Some(root) = group.root.clone() else {
                continue;
            };
            let Some(leaf) = find_leaf(&root, leaf_id) else {
                continue;
            };
            if !leaf.tabs.iter().any(|t| t == sid) || leaf.active.as_deref() == Some(sid) {
                continue;
            }
            group.root = Some(update_leaf(root, leaf_id, &|l| LeafNode {
                active: Some(sid.to_string()),
                ..l
            }));
        }
    }

    pub fn add_tab_to_session_leaf(&mut self, anchor_sid: &str, new_sid: &str) -> bool {
        if anchor_sid == new_sid {
            return false;
        }
        let Some(group_idx) = self.groups.iter().position(|g| {
            g.root
                .as_ref()
                .is_some_and(|root| leaf_of(root, anchor_sid).is_some())
        }) else {
            return false;
        };
        let Some(root) = self.groups[group_idx].root.clone() else {
            return false;
        };
        let Some(target) = leaf_of(&root, anchor_sid) else {
            return false;
        };
        if let Some(existing_group) = self.groups.iter().position(|g| {
            g.root
                .as_ref()
                .is_some_and(|root| leaf_of(root, new_sid).is_some())
        }) {
            if existing_group != group_idx {
                return false;
            }
        }
        if let Some(existing) = leaf_of(&root, new_sid) {
            if existing.id == target.id {
                self.active_group_id = self.groups[group_idx].id.clone();
                self.groups[group_idx].root = Some(update_leaf(root, &target.id, &|l| LeafNode {
                    active: Some(new_sid.to_string()),
                    ..l
                }));
                self.groups[group_idx].active_leaf = Some(target.id);
                self.focused_id = None;
                return true;
            }
        }
        let Some(root) = remove_tab(Some(root), new_sid) else {
            return false;
        };
        let Some(next_target) = leaf_of(&root, anchor_sid) else {
            return false;
        };
        let next_id = next_target.id.clone();
        let root = update_leaf(root, &next_id, &|l| {
            let mut tabs = l.tabs;
            tabs.push(new_sid.to_string());
            LeafNode {
                tabs,
                active: Some(new_sid.to_string()),
                ..l
            }
        });
        self.active_group_id = self.groups[group_idx].id.clone();
        self.groups[group_idx].root = Some(root);
        self.groups[group_idx].active_leaf = Some(next_id);
        self.focused_id = None;
        true
    }

    pub fn merge_leaf(&mut self, leaf_id: &str) {
        self.edit_active(|root, _active, _ids| {
            let leaves = all_leaves(&root);
            if leaves.len() < 2 {
                return None;
            }
            let src = leaves.iter().find(|l| l.id == leaf_id)?.clone();
            let dest = leaves.iter().find(|l| l.id != leaf_id)?.clone();
            if src.tabs.is_empty() {
                return None;
            }
            let moving = src.tabs.clone();
            let mut root = root;
            for sid in &moving {
                root = remove_tab(Some(root), sid)?;
            }
            if find_leaf(&root, &dest.id).is_none() {
                return None;
            }
            let last = moving.last().cloned();
            let root = update_leaf(root, &dest.id, &|l| {
                let mut tabs = l.tabs;
                tabs.extend(moving.iter().cloned());
                LeafNode {
                    tabs,
                    active: last.clone(),
                    ..l
                }
            });
            Some((root, Some(dest.id)))
        });
    }

    pub fn equalize(&mut self) {
        self.edit_active(|root, active, _ids| Some((equalize_node(root), active)));
    }

    pub fn resize(&mut self, split_id: &str, index: usize, delta: f64) {
        self.edit_active(|root, active, _ids| {
            let root = update_split(root, split_id, &|sp| {
                if index + 1 >= sp.sizes.len() {
                    return sp;
                }
                let min = 0.18;
                let mut sizes = sp.sizes.clone();
                let mut a = sizes[index] + delta;
                let mut b = sizes[index + 1] - delta;
                for _ in 0..2 {
                    if a < min {
                        let d = min - a;
                        a = min;
                        b -= d;
                    }
                    if b < min {
                        let d = min - b;
                        b = min;
                        a -= d;
                    }
                }
                sizes[index] = a;
                sizes[index + 1] = b;
                SplitNode { sizes, ..sp }
            });
            Some((root, active))
        });
    }

    /// Restore a group from a snapshot. Grids and ordinary groups share this path.
    pub fn restore_group(
        &mut self,
        id: &str,
        name: &str,
        snap: Option<&LayoutSnapshot>,
        activate: bool,
    ) {
        let built = build_snapshot(snap, &mut self.ids);
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            group.name = name.to_string();
            group.root = built.root;
            group.active_leaf = built.active_leaf;
        } else {
            self.groups.push(Group {
                id: id.to_string(),
                name: name.to_string(),
                root: built.root,
                active_leaf: built.active_leaf,
            });
        }
        if activate {
            self.active_group_id = id.to_string();
        }
        self.focused_id = None;
    }

    pub fn flag_group_launched(&mut self, group_id: &str, workspace_id: &str) {
        self.group_flags.insert(
            group_id.to_string(),
            GroupFlag {
                launched_from_workspace_id: Some(workspace_id.to_string()),
            },
        );
    }

    pub fn clear_group_launched(&mut self, group_id: &str) {
        self.group_flags.remove(group_id);
    }

    pub fn close_tab(&mut self, sid: &str) {
        let sessions = self
            .sessions_in_model()
            .into_iter()
            .filter(|s| s.id != sid)
            .collect::<Vec<_>>();
        self.sync(&sessions);
    }

    /// Close only the sessions that live in `group_id`. Other groups stay.
    pub fn close_group(&mut self, group_id: &str) -> Vec<String> {
        let closed = self
            .sessions_in_model()
            .into_iter()
            .filter(|s| s.group_id.as_deref().unwrap_or(DEFAULT_GROUP) == group_id)
            .map(|s| s.id)
            .collect::<Vec<_>>();
        let remaining = self
            .sessions_in_model()
            .into_iter()
            .filter(|s| s.group_id.as_deref().unwrap_or(DEFAULT_GROUP) != group_id)
            .collect::<Vec<_>>();
        self.sync(&remaining);
        closed
    }

    pub fn sessions_in_model(&self) -> Vec<SessionRef> {
        let mut out = Vec::new();
        for group in &self.groups {
            let Some(root) = &group.root else {
                continue;
            };
            for leaf in all_leaves(root) {
                for tab in leaf.tabs {
                    out.push(SessionRef {
                        id: tab,
                        group_id: Some(group.id.clone()),
                    });
                }
            }
        }
        out
    }

    fn active_index(&self) -> Option<usize> {
        self.groups
            .iter()
            .position(|g| g.id == self.active_group_id)
    }

    fn edit_active(
        &mut self,
        f: impl FnOnce(LayoutNode, Option<String>, &mut IdGen) -> Option<(LayoutNode, Option<String>)>,
    ) {
        let Some(idx) = self.active_index() else {
            return;
        };
        let Some(root) = self.groups[idx].root.clone() else {
            return;
        };
        let active = self.groups[idx].active_leaf.clone();
        if let Some((root, active_leaf)) = f(root, active, &mut self.ids) {
            self.groups[idx].root = Some(root);
            self.groups[idx].active_leaf = active_leaf;
        }
    }
}

fn mk_leaf(ids: &mut IdGen, tabs: Vec<String>) -> LeafNode {
    let active = tabs.last().cloned();
    LeafNode {
        id: ids.fresh("leaf"),
        tabs,
        active,
    }
}

pub fn all_leaves(node: &LayoutNode) -> Vec<LeafNode> {
    let mut out = Vec::new();
    fn walk(node: &LayoutNode, out: &mut Vec<LeafNode>) {
        match node {
            LayoutNode::Leaf(l) => out.push(l.clone()),
            LayoutNode::Split(s) => {
                for child in &s.children {
                    walk(child, out);
                }
            }
        }
    }
    walk(node, &mut out);
    out
}

fn find_leaf(node: &LayoutNode, id: &str) -> Option<LeafNode> {
    match node {
        LayoutNode::Leaf(l) if l.id == id => Some(l.clone()),
        LayoutNode::Leaf(_) => None,
        LayoutNode::Split(s) => s.children.iter().find_map(|c| find_leaf(c, id)),
    }
}

fn leaf_of(node: &LayoutNode, sid: &str) -> Option<LeafNode> {
    match node {
        LayoutNode::Leaf(l) if l.tabs.iter().any(|t| t == sid) => Some(l.clone()),
        LayoutNode::Leaf(_) => None,
        LayoutNode::Split(s) => s.children.iter().find_map(|c| leaf_of(c, sid)),
    }
}

fn first_leaf(node: &LayoutNode) -> LeafNode {
    match node {
        LayoutNode::Leaf(l) => l.clone(),
        LayoutNode::Split(s) => first_leaf(&s.children[0]),
    }
}

fn update_leaf(node: LayoutNode, id: &str, f: &impl Fn(LeafNode) -> LeafNode) -> LayoutNode {
    match node {
        LayoutNode::Leaf(l) if l.id == id => LayoutNode::Leaf(f(l)),
        LayoutNode::Leaf(l) => LayoutNode::Leaf(l),
        LayoutNode::Split(s) => LayoutNode::Split(SplitNode {
            children: s
                .children
                .into_iter()
                .map(|c| update_leaf(c, id, f))
                .collect(),
            ..s
        }),
    }
}

fn update_split(node: LayoutNode, id: &str, f: &impl Fn(SplitNode) -> SplitNode) -> LayoutNode {
    match node {
        LayoutNode::Leaf(l) => LayoutNode::Leaf(l),
        LayoutNode::Split(s) => {
            let next = SplitNode {
                children: s
                    .children
                    .into_iter()
                    .map(|c| update_split(c, id, f))
                    .collect(),
                ..s
            };
            if next.id == id {
                LayoutNode::Split(f(next))
            } else {
                LayoutNode::Split(next)
            }
        }
    }
}

fn replace_leaf(node: LayoutNode, id: &str, repl: &impl Fn(LeafNode) -> LayoutNode) -> LayoutNode {
    match node {
        LayoutNode::Leaf(l) if l.id == id => repl(l),
        LayoutNode::Leaf(l) => LayoutNode::Leaf(l),
        LayoutNode::Split(s) => LayoutNode::Split(SplitNode {
            children: s
                .children
                .into_iter()
                .map(|c| replace_leaf(c, id, repl))
                .collect(),
            ..s
        }),
    }
}

fn equal_sizes(n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    vec![1.0 / n as f64; n]
}

fn prune(node: LayoutNode) -> Option<LayoutNode> {
    match node {
        LayoutNode::Leaf(l) => {
            if l.tabs.is_empty() {
                None
            } else {
                Some(LayoutNode::Leaf(l))
            }
        }
        LayoutNode::Split(s) => {
            let orig_len = s.children.len();
            let orig_sizes = s.sizes.clone();
            let kids: Vec<LayoutNode> = s.children.into_iter().filter_map(prune).collect();
            if kids.is_empty() {
                return None;
            }
            if kids.len() == 1 {
                return Some(kids.into_iter().next().unwrap());
            }
            let sizes = if kids.len() == orig_len {
                orig_sizes
            } else {
                equal_sizes(kids.len())
            };
            Some(LayoutNode::Split(SplitNode {
                children: kids,
                sizes,
                ..s
            }))
        }
    }
}

fn remove_tab(root: Option<LayoutNode>, sid: &str) -> Option<LayoutNode> {
    let Some(root) = root else {
        return None;
    };
    let Some(owner) = leaf_of(&root, sid) else {
        return Some(root);
    };
    let updated = update_leaf(root, &owner.id, &|l| {
        let tabs: Vec<String> = l.tabs.into_iter().filter(|t| t != sid).collect();
        let active = if l.active.as_deref() == Some(sid) {
            tabs.last().cloned()
        } else {
            l.active
        };
        LeafNode {
            tabs,
            active,
            id: l.id,
        }
    });
    prune(updated)
}

fn detach_tab(root: LayoutNode, sid: &str) -> Option<LayoutNode> {
    let Some(owner) = leaf_of(&root, sid) else {
        return Some(root);
    };
    let tabs: Vec<String> = owner
        .tabs
        .iter()
        .filter(|t| t.as_str() != sid)
        .cloned()
        .collect();
    let active = if owner
        .active
        .as_ref()
        .is_some_and(|a| a != sid && tabs.iter().any(|t| t == a))
    {
        owner.active.clone()
    } else {
        tabs.last().cloned()
    };
    let tabs_for_leaf = tabs.clone();
    let active_for_leaf = active.clone();
    prune(update_leaf(root, &owner.id, &|l| LeafNode {
        tabs: tabs_for_leaf.clone(),
        active: active_for_leaf.clone(),
        id: l.id,
    }))
}

fn replace_tab_id(node: LayoutNode, old_id: &str, new_id: &str) -> LayoutNode {
    match node {
        LayoutNode::Leaf(l) => {
            if !l.tabs.iter().any(|t| t == old_id) {
                return LayoutNode::Leaf(l);
            }
            LayoutNode::Leaf(LeafNode {
                tabs: l
                    .tabs
                    .into_iter()
                    .map(|id| if id == old_id { new_id.to_string() } else { id })
                    .collect(),
                active: if l.active.as_deref() == Some(old_id) {
                    Some(new_id.to_string())
                } else {
                    l.active
                },
                id: l.id,
            })
        }
        LayoutNode::Split(s) => LayoutNode::Split(SplitNode {
            children: s
                .children
                .into_iter()
                .map(|c| replace_tab_id(c, old_id, new_id))
                .collect(),
            ..s
        }),
    }
}

fn make_split(id: String, zone: DropZone, new_leaf: LeafNode, target: LayoutNode) -> LayoutNode {
    let dir = match zone {
        DropZone::Left | DropZone::Right => SplitDir::Row,
        _ => SplitDir::Col,
    };
    let before = matches!(zone, DropZone::Left | DropZone::Top);
    let children = if before {
        vec![LayoutNode::Leaf(new_leaf), target]
    } else {
        vec![target, LayoutNode::Leaf(new_leaf)]
    };
    LayoutNode::Split(SplitNode {
        id,
        dir,
        children,
        sizes: vec![0.5, 0.5],
    })
}

fn equalize_node(node: LayoutNode) -> LayoutNode {
    match node {
        LayoutNode::Leaf(l) => LayoutNode::Leaf(l),
        LayoutNode::Split(s) => {
            let n = s.children.len();
            LayoutNode::Split(SplitNode {
                sizes: equal_sizes(n),
                children: s.children.into_iter().map(equalize_node).collect(),
                ..s
            })
        }
    }
}

fn reconcile(prev: RootState, ids: &[String], gen: &mut IdGen) -> RootState {
    let present = prev
        .root
        .as_ref()
        .map(all_leaves)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|l| l.tabs)
        .collect::<Vec<_>>();
    let removed: Vec<String> = present
        .iter()
        .filter(|id| !ids.iter().any(|x| x == *id))
        .cloned()
        .collect();
    let added: Vec<String> = ids
        .iter()
        .filter(|id| !present.iter().any(|x| x == *id))
        .cloned()
        .collect();
    let mut root = prev.root;
    let mut active_leaf = prev.active_leaf;
    let pending_removed: Vec<String> = removed
        .iter()
        .filter(|id| id.starts_with("pending-"))
        .cloned()
        .collect();
    if !pending_removed.is_empty() && pending_removed.len() == added.len() {
        let owners: Vec<Option<LeafNode>> = pending_removed
            .iter()
            .map(|id| root.as_ref().and_then(|r| leaf_of(r, id)))
            .collect();
        if owners.iter().all(|o| o.is_some()) {
            for i in 0..pending_removed.len() {
                let old_id = &pending_removed[i];
                let new_id = &added[i];
                let owner = owners[i].as_ref().unwrap();
                root = Some(update_leaf(root.unwrap(), &owner.id, &|l| LeafNode {
                    tabs: l
                        .tabs
                        .into_iter()
                        .map(|t| if t == *old_id { new_id.clone() } else { t })
                        .collect(),
                    active: if l.active.as_ref() == Some(old_id) {
                        Some(new_id.clone())
                    } else {
                        l.active
                    },
                    id: l.id,
                }));
                if active_leaf.is_none() {
                    active_leaf = Some(owner.id.clone());
                }
            }
            return RootState { root, active_leaf };
        }
    }
    for id in &removed {
        root = remove_tab(root, id);
    }
    for id in &added {
        if root.is_none() {
            let leaf = mk_leaf(gen, vec![id.clone()]);
            active_leaf = Some(leaf.id.clone());
            root = Some(LayoutNode::Leaf(leaf));
        } else {
            let current = root.unwrap();
            let target = active_leaf
                .as_ref()
                .and_then(|id| find_leaf(&current, id))
                .unwrap_or_else(|| first_leaf(&current));
            let target_id = target.id.clone();
            root = Some(update_leaf(current, &target_id, &|l| {
                let mut tabs = l.tabs;
                tabs.push(id.clone());
                LeafNode {
                    tabs,
                    active: Some(id.clone()),
                    id: l.id,
                }
            }));
            active_leaf = Some(target_id);
        }
    }
    if root.is_none() {
        active_leaf = None;
    } else if active_leaf
        .as_ref()
        .map(|id| find_leaf(root.as_ref().unwrap(), id).is_none())
        .unwrap_or(true)
    {
        active_leaf = Some(first_leaf(root.as_ref().unwrap()).id);
    }
    RootState { root, active_leaf }
}

fn build_snapshot(snap: Option<&LayoutSnapshot>, gen: &mut IdGen) -> RootState {
    let Some(snap) = snap else {
        return RootState {
            root: None,
            active_leaf: None,
        };
    };
    let mut first_leaf_id = None;
    fn build(
        n: &LayoutSnapshot,
        gen: &mut IdGen,
        first_leaf_id: &mut Option<String>,
    ) -> LayoutNode {
        match n {
            LayoutSnapshot::Leaf { tabs, active } => {
                let id = gen.fresh("leaf");
                if first_leaf_id.is_none() {
                    *first_leaf_id = Some(id.clone());
                }
                let active = active.clone().or_else(|| tabs.last().cloned());
                LayoutNode::Leaf(LeafNode {
                    id,
                    tabs: tabs.clone(),
                    active,
                })
            }
            LayoutSnapshot::Split {
                dir,
                sizes,
                children,
            } => LayoutNode::Split(SplitNode {
                id: gen.fresh("split"),
                dir: *dir,
                sizes: sizes.clone(),
                children: children
                    .iter()
                    .map(|c| build(c, gen, first_leaf_id))
                    .collect(),
            }),
        }
    }
    let root = build(snap, gen, &mut first_leaf_id);
    RootState {
        root: Some(root),
        active_leaf: first_leaf_id,
    }
}

pub fn group_active_session(group: Option<&Group>) -> Option<String> {
    let group = group?;
    let root = group.root.as_ref()?;
    let leaves = all_leaves(root);
    if let Some(id) = &group.active_leaf {
        if let Some(leaf) = leaves.iter().find(|l| &l.id == id) {
            return leaf.active.clone();
        }
    }
    leaves.first().and_then(|l| l.active.clone())
}

pub fn compute_layout(root: Option<&LayoutNode>) -> LayoutGeom {
    let mut leaves = Vec::new();
    let mut handles = Vec::new();
    fn walk(n: &LayoutNode, r: Rect, leaves: &mut Vec<LeafRect>, handles: &mut Vec<SplitHandle>) {
        match n {
            LayoutNode::Leaf(l) => {
                if l.tabs.is_empty() {
                    return;
                }
                leaves.push(LeafRect {
                    leaf: l.clone(),
                    rect: r,
                });
            }
            LayoutNode::Split(s) => {
                let mut sizes = if s.sizes.len() == s.children.len() {
                    s.sizes.clone()
                } else {
                    equal_sizes(s.children.len())
                };
                let mut total: f64 = sizes.iter().sum();
                if total < 0.001 {
                    sizes = equal_sizes(s.children.len());
                    total = sizes.iter().sum::<f64>().max(1.0);
                    if total == 0.0 {
                        total = 1.0;
                    }
                }
                let mut off = if s.dir == SplitDir::Row { r.x } else { r.y };
                for (i, child) in s.children.iter().enumerate() {
                    let frac = sizes[i] / total;
                    let cr = if s.dir == SplitDir::Row {
                        Rect {
                            x: off,
                            y: r.y,
                            w: r.w * frac,
                            h: r.h,
                        }
                    } else {
                        Rect {
                            x: r.x,
                            y: off,
                            w: r.w,
                            h: r.h * frac,
                        }
                    };
                    walk(child, cr, leaves, handles);
                    off += (if s.dir == SplitDir::Row { r.w } else { r.h }) * frac;
                    if i + 1 < s.children.len() {
                        handles.push(SplitHandle {
                            split_id: s.id.clone(),
                            index: i,
                            dir: s.dir,
                            span: if s.dir == SplitDir::Row { r.w } else { r.h },
                            rect: if s.dir == SplitDir::Row {
                                Rect {
                                    x: off,
                                    y: r.y,
                                    w: 0.0,
                                    h: r.h,
                                }
                            } else {
                                Rect {
                                    x: r.x,
                                    y: off,
                                    w: r.w,
                                    h: 0.0,
                                }
                            },
                        });
                    }
                }
            }
        }
    }
    if let Some(root) = root {
        walk(
            root,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            &mut leaves,
            &mut handles,
        );
    }
    LayoutGeom { leaves, handles }
}

/// Drop zone from a cursor position inside a pane. Edges split; the middle stacks.
pub fn zone_at(px: f64, py: f64, w: f64, h: f64) -> DropZone {
    let fx = px / w;
    let fy = py / h;
    let left = fx;
    let right = 1.0 - fx;
    let top = fy;
    let bottom = 1.0 - fy;
    let min = left.min(right).min(top).min(bottom);
    if min > 0.28 {
        return DropZone::Center;
    }
    if min == left {
        return DropZone::Left;
    }
    if min == right {
        return DropZone::Right;
    }
    if min == top {
        return DropZone::Top;
    }
    DropZone::Bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str, tabs: &[&str], active: &str) -> LayoutNode {
        LayoutNode::Leaf(LeafNode {
            id: id.into(),
            tabs: tabs.iter().map(|t| (*t).to_string()).collect(),
            active: Some(active.into()),
        })
    }

    fn home() -> LayoutStore {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![Group {
                id: DEFAULT_GROUP.into(),
                name: HOME_GROUP_NAME.into(),
                root: None,
                active_leaf: None,
            }],
            DEFAULT_GROUP,
            None,
            HashMap::new(),
        );
        store
    }

    #[test]
    fn keeps_a_fresh_terminal_inside_the_single_home_group() {
        let mut store = home();
        store.sync(&[SessionRef {
            id: "local-1".into(),
            group_id: None,
        }]);
        assert_eq!(store.groups.len(), 1);
        assert_eq!(store.active_group_id, DEFAULT_GROUP);
        assert_eq!(store.groups[0].name, HOME_GROUP_NAME);
        let root = store.groups[0].root.as_ref().unwrap();
        assert_eq!(root.as_leaf().unwrap().tabs, vec!["local-1".to_string()]);
    }

    #[test]
    fn names_the_next_group_after_the_home_group() {
        let mut store = home();
        let id = store.create_group(None);
        let created = store.groups.iter().find(|g| g.id == id).unwrap();
        assert_eq!(created.name, "Group 2");
        assert_eq!(store.active_group_id, id);
        assert_eq!(store.groups[0].id, DEFAULT_GROUP);
    }

    #[test]
    fn split_beside_pulls_a_sibling_into_a_right_hand_split() {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![Group {
                id: DEFAULT_GROUP.into(),
                name: "Terminals".into(),
                root: Some(leaf("leaf-1", &["local-1"], "local-1")),
                active_leaf: Some("leaf-1".into()),
            }],
            DEFAULT_GROUP,
            Some("local-1".into()),
            HashMap::new(),
        );
        store.sync(&[
            SessionRef {
                id: "local-1".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
            SessionRef {
                id: "browser-1".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
        ]);
        store.split_beside("local-1", "browser-1", DropZone::Right);
        let root = store.groups[0].root.as_ref().unwrap().as_split().unwrap();
        assert_eq!(root.dir, SplitDir::Row);
        assert_eq!(root.children.len(), 2);
        assert_eq!(
            root.children[0].as_leaf().unwrap().tabs,
            vec!["local-1".to_string()]
        );
        assert_eq!(
            root.children[1].as_leaf().unwrap().tabs,
            vec!["browser-1".to_string()]
        );
        assert!(store.focused_id.is_none());
    }

    #[test]
    fn places_a_prescribed_new_id_on_the_source_session_leaf() {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![Group {
                id: DEFAULT_GROUP.into(),
                name: "Terminals".into(),
                root: Some(LayoutNode::Split(SplitNode {
                    id: "split-1".into(),
                    dir: SplitDir::Row,
                    sizes: vec![0.5, 0.5],
                    children: vec![
                        leaf("leaf-source", &["source"], "source"),
                        leaf("leaf-other", &["other"], "other"),
                    ],
                })),
                active_leaf: Some("leaf-other".into()),
            }],
            DEFAULT_GROUP,
            None,
            HashMap::new(),
        );
        store.sync(&[
            SessionRef {
                id: "source".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
            SessionRef {
                id: "other".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
            SessionRef {
                id: "delegated".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
        ]);
        assert!(store.add_tab_to_session_leaf("source", "delegated"));
        let root = store.groups[0].root.as_ref().unwrap().as_split().unwrap();
        assert_eq!(
            root.children[0].as_leaf().unwrap().tabs,
            vec!["source".to_string(), "delegated".to_string()]
        );
        assert_eq!(
            root.children[1].as_leaf().unwrap().tabs,
            vec!["other".to_string()]
        );
    }

    #[test]
    fn renames_several_pending_ssh_tabs_in_place() {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![Group {
                id: DEFAULT_GROUP.into(),
                name: "Terminals".into(),
                root: Some(leaf(
                    "leaf-pending",
                    &["pending-a", "pending-b"],
                    "pending-b",
                )),
                active_leaf: Some("leaf-pending".into()),
            }],
            DEFAULT_GROUP,
            None,
            HashMap::new(),
        );
        store.sync(&[
            SessionRef {
                id: "pending-a".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
            SessionRef {
                id: "pending-b".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
        ]);
        store.sync(&[
            SessionRef {
                id: "ssh-a".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
            SessionRef {
                id: "ssh-b".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
        ]);
        let root = store.groups[0].root.as_ref().unwrap().as_leaf().unwrap();
        assert_eq!(root.tabs, vec!["ssh-a".to_string(), "ssh-b".to_string()]);
        assert_eq!(root.active.as_deref(), Some("ssh-b"));
    }

    #[test]
    fn split_new_beside_keeps_the_operator_terminal() {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![Group {
                id: DEFAULT_GROUP.into(),
                name: "Group 1".into(),
                root: Some(leaf("leaf-1", &["term-a", "term-b", "term-c"], "term-b")),
                active_leaf: Some("leaf-1".into()),
            }],
            DEFAULT_GROUP,
            Some("term-b".into()),
            HashMap::new(),
        );
        assert!(store.split_new_beside("term-b", "browser-1", DropZone::Right));
        let root = store.groups[0].root.as_ref().unwrap().as_split().unwrap();
        let left = root.children[0].as_leaf().unwrap();
        let right = root.children[1].as_leaf().unwrap();
        assert_eq!(
            left.tabs,
            vec![
                "term-a".to_string(),
                "term-b".to_string(),
                "term-c".to_string()
            ]
        );
        assert_eq!(left.active.as_deref(), Some("term-b"));
        assert_eq!(right.tabs, vec!["browser-1".to_string()]);
        assert_eq!(store.groups[0].active_leaf.as_deref(), Some("leaf-1"));
        assert_eq!(store.focused_id.as_deref(), Some("term-b"));
        assert_eq!(store.active_group_id, DEFAULT_GROUP);
    }

    #[test]
    fn split_new_beside_pulls_a_parked_browser_and_keeps_the_strip() {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![Group {
                id: DEFAULT_GROUP.into(),
                name: "Group 1".into(),
                root: Some(leaf(
                    "leaf-1",
                    &["term-a", "term-b", "term-c", "browser-1"],
                    "browser-1",
                )),
                active_leaf: Some("leaf-1".into()),
            }],
            DEFAULT_GROUP,
            None,
            HashMap::new(),
        );
        store.set_leaf_active_tab("leaf-1", "term-b");
        assert!(store.split_new_beside("term-b", "browser-1", DropZone::Right));
        let root = store.groups[0].root.as_ref().unwrap().as_split().unwrap();
        let terminal = root.children[0].as_leaf().unwrap();
        assert_eq!(
            terminal.tabs,
            vec![
                "term-a".to_string(),
                "term-b".to_string(),
                "term-c".to_string()
            ]
        );
        assert_eq!(terminal.active.as_deref(), Some("term-b"));
        assert_eq!(store.groups[0].active_leaf.as_deref(), Some("leaf-1"));
    }

    #[test]
    fn split_right_and_split_down_drop_zones_merge_equalize_and_layout() {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![Group {
                id: DEFAULT_GROUP.into(),
                name: HOME_GROUP_NAME.into(),
                root: Some(leaf("leaf-1", &["a", "b"], "a")),
                active_leaf: Some("leaf-1".into()),
            }],
            DEFAULT_GROUP,
            None,
            HashMap::new(),
        );
        store.split_right("a", "b");
        let left_id = store.groups[0]
            .root
            .as_ref()
            .unwrap()
            .as_split()
            .unwrap()
            .children[0]
            .as_leaf()
            .unwrap()
            .id
            .clone();
        {
            let split = store.groups[0].root.as_ref().unwrap().as_split().unwrap();
            assert_eq!(split.dir, SplitDir::Row);
            assert_eq!(
                split.children[0].as_leaf().unwrap().tabs,
                vec!["a".to_string()]
            );
            assert_eq!(
                split.children[1].as_leaf().unwrap().tabs,
                vec!["b".to_string()]
            );
        }
        let geom = compute_layout(store.groups[0].root.as_ref());
        assert_eq!(geom.leaves.len(), 2);
        assert!((geom.leaves[0].rect.w - 0.5).abs() < 1e-9);
        assert!((geom.leaves[1].rect.x - 0.5).abs() < 1e-9);
        assert_eq!(geom.handles.len(), 1);
        assert_eq!(zone_at(0.5, 0.5, 1.0, 1.0), DropZone::Center);
        assert_eq!(zone_at(0.02, 0.5, 1.0, 1.0), DropZone::Left);

        store.drop("b", &left_id, DropZone::Center);
        let root = store.groups[0].root.as_ref().unwrap().as_leaf().unwrap();
        assert_eq!(root.tabs, vec!["a".to_string(), "b".to_string()]);

        store.split_down("a", "b");
        let split = store.groups[0].root.as_ref().unwrap().as_split().unwrap();
        assert_eq!(split.dir, SplitDir::Col);
        let split_id = split.id.clone();
        store.resize(&split_id, 0, 0.1);
        let sizes = store.groups[0]
            .root
            .as_ref()
            .unwrap()
            .as_split()
            .unwrap()
            .sizes
            .clone();
        assert!((sizes[0] - 0.6).abs() < 1e-9);
        store.equalize();
        let sizes = store.groups[0]
            .root
            .as_ref()
            .unwrap()
            .as_split()
            .unwrap()
            .sizes
            .clone();
        assert!((sizes[0] - 0.5).abs() < 1e-9);
        let left_id = store.groups[0]
            .root
            .as_ref()
            .unwrap()
            .as_split()
            .unwrap()
            .children[0]
            .as_leaf()
            .unwrap()
            .id
            .clone();
        store.merge_leaf(&left_id);
        assert!(store.groups[0].root.as_ref().unwrap().as_leaf().is_some());
    }

    #[test]
    fn close_group_only_drops_that_groups_sessions_and_hidden_groups_remain() {
        let mut store = LayoutStore::new();
        store.set_state(
            vec![
                Group {
                    id: DEFAULT_GROUP.into(),
                    name: HOME_GROUP_NAME.into(),
                    root: Some(leaf("leaf-home", &["local-1"], "local-1")),
                    active_leaf: Some("leaf-home".into()),
                },
                Group {
                    id: "other".into(),
                    name: "Group 2".into(),
                    root: Some(leaf("leaf-other", &["ssh-1"], "ssh-1")),
                    active_leaf: Some("leaf-other".into()),
                },
            ],
            DEFAULT_GROUP,
            None,
            HashMap::new(),
        );
        store.sync(&[
            SessionRef {
                id: "local-1".into(),
                group_id: Some(DEFAULT_GROUP.into()),
            },
            SessionRef {
                id: "ssh-1".into(),
                group_id: Some("other".into()),
            },
        ]);
        assert_eq!(store.groups.len(), 2);
        let closed = store.close_group("other");
        assert_eq!(closed, vec!["ssh-1".to_string()]);
        assert!(store.groups.iter().any(|g| g.id == DEFAULT_GROUP));
        assert!(store.groups.iter().all(|g| g.id != "other"));
        assert_eq!(
            store.groups[0]
                .root
                .as_ref()
                .unwrap()
                .as_leaf()
                .unwrap()
                .tabs,
            vec!["local-1".to_string()]
        );
        store.close_tab("local-1");
        assert_eq!(store.groups[0].id, DEFAULT_GROUP);
        assert!(store.groups[0].root.is_none());
    }

    #[test]
    fn focus_and_zen_do_not_scale_text() {
        let mut store = home();
        store.sync(&[SessionRef {
            id: "local-1".into(),
            group_id: None,
        }]);
        store.set_focus(Some("local-1".into()));
        store.set_zen_mode(true);
        assert_eq!(store.focused_id.as_deref(), Some("local-1"));
        assert!(store.zen_mode);
        assert!(!focus_mode_scales_text());
        assert!(!zen_mode_scales_text());
        assert_eq!(store.text_scale(), 1.0);
        store.toggle_focus(Some("local-1"));
        assert!(store.focused_id.is_none());
        assert_eq!(store.text_scale(), 1.0);
    }

    #[test]
    fn grid_restore_uses_the_same_path_as_a_normal_group() {
        let mut store = home();
        let snap = LayoutSnapshot::Split {
            dir: SplitDir::Col,
            sizes: vec![0.5, 0.5],
            children: vec![
                LayoutSnapshot::Split {
                    dir: SplitDir::Row,
                    sizes: vec![0.5, 0.5],
                    children: vec![
                        LayoutSnapshot::Leaf {
                            tabs: vec!["a".into()],
                            active: Some("a".into()),
                        },
                        LayoutSnapshot::Leaf {
                            tabs: vec!["b".into()],
                            active: Some("b".into()),
                        },
                    ],
                },
                LayoutSnapshot::Split {
                    dir: SplitDir::Row,
                    sizes: vec![0.5, 0.5],
                    children: vec![
                        LayoutSnapshot::Leaf {
                            tabs: vec!["c".into()],
                            active: Some("c".into()),
                        },
                        LayoutSnapshot::Leaf {
                            tabs: vec!["d".into()],
                            active: Some("d".into()),
                        },
                    ],
                },
            ],
        };
        store.restore_group("grid", "2×2", Some(&snap), true);
        assert_eq!(store.active_group_id, "grid");
        let root = store
            .groups
            .iter()
            .find(|g| g.id == "grid")
            .unwrap()
            .root
            .as_ref()
            .unwrap()
            .as_split()
            .unwrap();
        assert_eq!(root.dir, SplitDir::Col);
        assert_eq!(root.children.len(), 2);
        store.restore_group(DEFAULT_GROUP, HOME_GROUP_NAME, Some(&snap), false);
        assert_eq!(store.active_group_id, "grid");
        assert_eq!(
            store
                .groups
                .iter()
                .find(|g| g.id == DEFAULT_GROUP)
                .unwrap()
                .root
                .as_ref()
                .unwrap()
                .as_split()
                .unwrap()
                .dir,
            SplitDir::Col
        );
    }

    #[test]
    fn single_tab_edge_drop_is_a_no_op_and_home_group_stays() {
        let mut store = home();
        store.sync(&[SessionRef {
            id: "local-1".into(),
            group_id: None,
        }]);
        let before = store.groups[0].root.clone();
        let leaf_id = before.as_ref().unwrap().as_leaf().unwrap().id.clone();
        store.drop("local-1", &leaf_id, DropZone::Right);
        assert_eq!(store.groups[0].root, before);
        store.sync(&[]);
        assert_eq!(store.groups.len(), 1);
        assert_eq!(store.groups[0].id, DEFAULT_GROUP);
        assert_eq!(store.groups[0].name, HOME_GROUP_NAME);
    }
}
