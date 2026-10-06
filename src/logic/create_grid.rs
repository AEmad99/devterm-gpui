//! Terminal grid creation. Maximum size is 4×4 (16 cells).
//!
//! `build_grid_snapshot` / `pack_ids_as_grid` produce the same layout snapshot
//! shape a normal group restore consumes. Session allocation stays with the host.

#![allow(dead_code)]

pub const GRID_MAX_ROWS: i32 = 4;
pub const GRID_MAX_COLS: i32 = 4;
pub const GRID_MAX_CELLS: i32 = 16;
pub const GRID_MIN_DIM: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub rows: f64,
    pub cols: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridCellKind {
    Local,
    Remote,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDir {
    Row,
    Col,
}

pub fn clamp_grid_spec(spec: GridSpec) -> (i32, i32) {
    let rows = (spec.rows.floor() as i32).clamp(GRID_MIN_DIM, GRID_MAX_ROWS);
    let cols = (spec.cols.floor() as i32).clamp(GRID_MIN_DIM, GRID_MAX_COLS);
    (rows, cols)
}

pub fn grid_cell_count(spec: GridSpec) -> i32 {
    let (rows, cols) = clamp_grid_spec(spec);
    rows * cols
}

/// Validate before create. Returns an error message, or `None` when the spec is usable.
pub fn validate_grid_spec(spec: GridSpec) -> Option<String> {
    if !spec.rows.is_finite() || !spec.cols.is_finite() {
        return Some("Invalid dimensions".to_string());
    }
    if spec.rows < GRID_MIN_DIM as f64 || spec.cols < GRID_MIN_DIM as f64 {
        return Some("Minimum is 1×1".to_string());
    }
    if spec.rows > GRID_MAX_ROWS as f64 || spec.cols > GRID_MAX_COLS as f64 {
        return Some(format!("Maximum is {GRID_MAX_ROWS}×{GRID_MAX_COLS}"));
    }
    if spec.rows * spec.cols > GRID_MAX_CELLS as f64 {
        return Some(format!("Maximum {GRID_MAX_CELLS} terminals"));
    }
    None
}

fn equal_sizes(n: usize) -> Vec<f64> {
    let frac = 1.0 / n as f64;
    vec![frac; n]
}

fn leaf_of(id: &str) -> LayoutSnapshot {
    LayoutSnapshot::Leaf {
        tabs: vec![id.to_string()],
        active: Some(id.to_string()),
    }
}

pub fn build_grid_snapshot(ids: &[String], rows: i32, cols: i32) -> Result<LayoutSnapshot, String> {
    let expected = rows * cols;
    if ids.len() as i32 != expected {
        return Err(format!(
            "buildGridSnapshot: expected {expected} ids (rows={rows} cols={cols}), got {}",
            ids.len()
        ));
    }
    if rows == 1 && cols == 1 {
        return Ok(leaf_of(&ids[0]));
    }
    if rows == 1 {
        return Ok(LayoutSnapshot::Split {
            dir: SplitDir::Row,
            sizes: equal_sizes(cols as usize),
            children: ids.iter().map(|id| leaf_of(id)).collect(),
        });
    }
    if cols == 1 {
        return Ok(LayoutSnapshot::Split {
            dir: SplitDir::Col,
            sizes: equal_sizes(rows as usize),
            children: ids.iter().map(|id| leaf_of(id)).collect(),
        });
    }
    let mut row_nodes = Vec::new();
    for r in 0..rows {
        let start = (r * cols) as usize;
        let slice = &ids[start..start + cols as usize];
        row_nodes.push(LayoutSnapshot::Split {
            dir: SplitDir::Row,
            sizes: equal_sizes(cols as usize),
            children: slice.iter().map(|id| leaf_of(id)).collect(),
        });
    }
    Ok(LayoutSnapshot::Split {
        dir: SplitDir::Col,
        sizes: equal_sizes(rows as usize),
        children: row_nodes,
    })
}

pub fn pack_ids_as_grid(ids: &[String], preferred_cols: i32) -> Option<LayoutSnapshot> {
    if ids.is_empty() {
        return None;
    }
    let cols = preferred_cols.min(GRID_MAX_COLS).min(ids.len() as i32);
    let rows = (ids.len() as i32 + cols - 1) / cols;
    let mut row_nodes = Vec::new();
    let mut i = 0usize;
    for _ in 0..rows {
        let take = cols.min((ids.len() - i) as i32) as usize;
        let slice = &ids[i..i + take];
        i += take;
        if slice.len() == 1 {
            row_nodes.push(leaf_of(&slice[0]));
        } else {
            row_nodes.push(LayoutSnapshot::Split {
                dir: SplitDir::Row,
                sizes: equal_sizes(slice.len()),
                children: slice.iter().map(|id| leaf_of(id)).collect(),
            });
        }
    }
    if row_nodes.len() == 1 {
        return Some(row_nodes.remove(0));
    }
    Some(LayoutSnapshot::Split {
        dir: SplitDir::Col,
        sizes: equal_sizes(row_nodes.len()),
        children: row_nodes,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridPlan {
    pub rows: i32,
    pub cols: i32,
    pub count: i32,
    pub name: String,
    pub kind: GridCellKind,
    pub connection_id: Option<String>,
}

pub fn grid_group_name(rows: i32, cols: i32, custom: Option<&str>) -> String {
    custom
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{rows}×{cols}"))
}

/// Validate a create-grid request and return the clamped plan.
/// Remote grids require a saved connection id. Local grids reject one.
pub fn plan_terminal_grid(
    spec: GridSpec,
    kind: GridCellKind,
    connection_id: Option<&str>,
    group_name: Option<&str>,
) -> Result<GridPlan, String> {
    if let Some(err) = validate_grid_spec(spec) {
        return Err(err);
    }
    if kind == GridCellKind::Remote && connection_id.map(|s| s.is_empty()).unwrap_or(true) {
        return Err("Remote grids require a saved connectionId".to_string());
    }
    if kind == GridCellKind::Local && connection_id.is_some() {
        return Err("connectionId is only used for remote grids".to_string());
    }
    let (rows, cols) = clamp_grid_spec(spec);
    let count = rows * cols;
    Ok(GridPlan {
        rows,
        cols,
        count,
        name: grid_group_name(rows, cols, group_name),
        kind,
        connection_id: connection_id.map(|s| s.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_is_four_by_four() {
        assert!(validate_grid_spec(GridSpec { rows: 4.0, cols: 4.0 }).is_none());
        assert_eq!(grid_cell_count(GridSpec { rows: 4.0, cols: 4.0 }), 16);
        assert_eq!(
            validate_grid_spec(GridSpec { rows: 5.0, cols: 1.0 }).as_deref(),
            Some("Maximum is 4×4")
        );
        assert_eq!(
            validate_grid_spec(GridSpec { rows: 1.0, cols: 5.0 }).as_deref(),
            Some("Maximum is 4×4")
        );
        assert_eq!(
            validate_grid_spec(GridSpec { rows: 0.0, cols: 1.0 }).as_deref(),
            Some("Minimum is 1×1")
        );
        assert_eq!(
            validate_grid_spec(GridSpec {
                rows: f64::NAN,
                cols: 2.0
            })
            .as_deref(),
            Some("Invalid dimensions")
        );
        assert_eq!(GRID_MAX_ROWS, 4);
        assert_eq!(GRID_MAX_COLS, 4);
        assert_eq!(GRID_MAX_CELLS, 16);
    }

    #[test]
    fn plan_rejects_the_wrong_connection_id() {
        let err = plan_terminal_grid(GridSpec { rows: 2.0, cols: 2.0 }, GridCellKind::Remote, None, None);
        assert_eq!(err.unwrap_err(), "Remote grids require a saved connectionId");
        let err = plan_terminal_grid(
            GridSpec { rows: 1.0, cols: 1.0 },
            GridCellKind::Local,
            Some("c1"),
            None,
        );
        assert_eq!(err.unwrap_err(), "connectionId is only used for remote grids");
        let plan = plan_terminal_grid(
            GridSpec { rows: 2.0, cols: 3.0 },
            GridCellKind::Local,
            None,
            None,
        )
        .unwrap();
        assert_eq!(plan.name, "2×3");
        assert_eq!(plan.count, 6);
        let named = plan_terminal_grid(
            GridSpec { rows: 4.0, cols: 4.0 },
            GridCellKind::Remote,
            Some("ssh-1"),
            Some("lab"),
        )
        .unwrap();
        assert_eq!(named.name, "lab");
        assert_eq!(named.count, 16);
    }

    #[test]
    fn build_and_pack_snapshots() {
        let ids: Vec<String> = (0..4).map(|i| format!("s{i}")).collect();
        let snap = build_grid_snapshot(&ids, 2, 2).unwrap();
        match snap {
            LayoutSnapshot::Split { dir, children, sizes } => {
                assert_eq!(dir, SplitDir::Col);
                assert_eq!(children.len(), 2);
                assert_eq!(sizes.len(), 2);
                match &children[0] {
                    LayoutSnapshot::Split { dir, children, .. } => {
                        assert_eq!(*dir, SplitDir::Row);
                        assert_eq!(children.len(), 2);
                    }
                    _ => panic!("row"),
                }
            }
            _ => panic!("split"),
        }
        let one = build_grid_snapshot(&["only".into()], 1, 1).unwrap();
        match one {
            LayoutSnapshot::Leaf { tabs, active } => {
                assert_eq!(tabs, vec!["only".to_string()]);
                assert_eq!(active.as_deref(), Some("only"));
            }
            _ => panic!("leaf"),
        }
        let err = build_grid_snapshot(&ids, 2, 3).unwrap_err();
        assert!(err.contains("buildGridSnapshot: expected 6 ids"));
        assert!(pack_ids_as_grid(&[], 2).is_none());
        let packed = pack_ids_as_grid(&["a".into(), "b".into(), "c".into()], 2).unwrap();
        match packed {
            LayoutSnapshot::Split { dir, children, .. } => {
                assert_eq!(dir, SplitDir::Col);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("packed"),
        }
    }
}
