//! Main-window layout that survives restarts: window geometry, sidebar,
//! toolbar style, column order and widths. Kept in its own file so the
//! settings dialogs never overwrite it.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const DEFAULT_SIDEBAR: f32 = 142.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub id: String,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default = "default_sidebar")]
    pub sidebar_width: f32,
    #[serde(default = "yes")]
    pub show_sidebar: bool,
    #[serde(default)]
    pub small_toolbar: bool,
    /// What a double click on a row does: open, folder or properties.
    #[serde(default = "default_double_click")]
    pub double_click: String,
    #[serde(default = "default_columns")]
    pub columns: Vec<Column>,
    #[serde(default = "default_expanded")]
    pub expanded: Vec<String>,
}

fn default_sidebar() -> f32 {
    DEFAULT_SIDEBAR
}

fn yes() -> bool {
    true
}

fn default_double_click() -> String {
    "open".into()
}

fn default_expanded() -> Vec<String> {
    vec!["all".into()]
}

pub fn default_columns() -> Vec<Column> {
    [
        ("filename", 151.0),
        ("q", 20.0),
        ("size", 75.0),
        ("status", 65.0),
        ("time_left", 75.0),
        ("transfer_rate", 95.0),
        ("last_try_date", 65.0),
        ("description", 190.0),
        ("category", 80.0),
        ("queue", 80.0),
    ]
    .into_iter()
    .map(|(id, width)| Column { id: id.into(), width })
    .collect()
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            x: None,
            y: None,
            sidebar_width: DEFAULT_SIDEBAR,
            show_sidebar: true,
            small_toolbar: false,
            double_click: default_double_click(),
            columns: default_columns(),
            expanded: default_expanded(),
        }
    }
}

impl Layout {
    /// Drops unknown columns, restores missing ones at the end and clamps widths.
    pub fn normalized(mut self) -> Self {
        let known = default_columns();
        let mut seen = Vec::new();
        self.columns.retain(|c| {
            let ok = known.iter().any(|k| k.id == c.id) && !seen.contains(&c.id);
            if ok {
                seen.push(c.id.clone());
            }
            ok
        });
        for column in &mut self.columns {
            column.width = column.width.clamp(18.0, 1200.0);
        }
        for k in known {
            if !seen.contains(&k.id) {
                self.columns.push(k);
            }
        }
        self.sidebar_width = self.sidebar_width.clamp(90.0, 600.0);
        if !matches!(self.double_click.as_str(), "open" | "folder" | "properties") {
            self.double_click = default_double_click();
        }
        self
    }
}

const FILE: &str = "gui-layout.json";

pub fn load(dir: &Path) -> Layout {
    fs::read(dir.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Layout>(&bytes).ok())
        .unwrap_or_default()
        .normalized()
}

pub fn save(dir: &Path, layout: &Layout) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(layout).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    fs::write(&tmp, json)?;
    fs::rename(&tmp, dir.join(FILE))
}

/// Index a dragged header lands on: `from` moved by `dx` pixels over the
/// visible columns' widths.
pub fn drop_index(columns: &[Column], visible: impl Fn(&str) -> bool, from: usize, dx: f32) -> usize {
    let mut start = 0.0;
    let mut center = 0.0;
    for (i, column) in columns.iter().enumerate() {
        let width = if visible(&column.id) { column.width } else { 0.0 };
        if i == from {
            center = start + width / 2.0 + dx;
        }
        start += width;
    }
    let mut edge = 0.0;
    for (i, column) in columns.iter().enumerate() {
        let width = if visible(&column.id) { column.width } else { 0.0 };
        if center < edge + width {
            return i;
        }
        edge += width;
    }
    columns.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_repairs_columns() {
        let dir = std::env::temp_dir().join(format!("xdm-layout-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut layout = Layout::default();
        layout.width = 1000;
        layout.columns.swap(0, 2);
        layout.columns[1].width = 99.0;
        save(&dir, &layout).unwrap();
        assert_eq!(load(&dir), layout);
        fs::write(dir.join(FILE), r#"{"columns":[{"id":"size","width":5},{"id":"bogus","width":50},{"id":"size","width":60}]}"#).unwrap();
        let repaired = load(&dir);
        assert_eq!(repaired.columns[0], Column { id: "size".into(), width: 18.0 });
        assert_eq!(repaired.columns.len(), default_columns().len());
        assert!(repaired.columns.iter().all(|c| c.id != "bogus"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn drag_target_follows_the_pointer() {
        let cols = default_columns();
        let all = |_: &str| true;
        assert_eq!(drop_index(&cols, all, 0, 0.0), 0);
        assert_eq!(drop_index(&cols, all, 0, 215.0), 3);
        assert_eq!(drop_index(&cols, all, 2, -100.0), 0);
        assert_eq!(drop_index(&cols, all, 0, 5000.0), cols.len() - 1);
    }
}
