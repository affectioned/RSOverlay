//! VR layouts: save slots for the whole floating arrangement — every
//! window / widget overlay (source, pose, size, dock) plus the keyboard —
//! like XSOverlay's layouts. A favourite slot loads at startup.
//!
//! Stored in `%APPDATA%\RSOverlay\layouts.toml`, separate from
//! config.toml so a reset of settings never wipes saved layouts.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SLOTS: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutPanel {
    /// "window" | "monitor" | "widgets"
    pub kind: String,
    /// Window title at save time (matched on load: exact, then prefix).
    pub title: String,
    pub monitor: u32,
    /// Widget slugs for a tab group (`system`, `volume`, …).
    pub widgets: Vec<String>,
    /// "world" | "head" | "left" | "right"
    pub attach: String,
    /// Row-major 3×4 pose: world pose (attach = world) or device-local.
    pub pose: [[f32; 4]; 3],
    pub width: f32,
    /// Edge-docked group id (panels sharing one move together). Only
    /// meaningful within one layout; absent = not docked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub name: String,
    pub saved_unix: u64,
    pub panels: Vec<LayoutPanel>,
    /// Keyboard world pose + width, if it was open.
    pub keyboard: Option<KeyboardPose>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KeyboardPose {
    pub pose: [[f32; 4]; 3],
    pub width: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layouts {
    /// Keyed by slot number ("0".."7"): TOML has no null, so empty
    /// slots are simply absent.
    pub slots: std::collections::BTreeMap<String, Layout>,
    /// Slot loaded automatically when VR starts.
    pub favorite: Option<usize>,
}

impl Layouts {
    pub fn path() -> PathBuf {
        crate::Config::default_path().with_file_name("layouts.toml")
    }

    pub fn load(path: &Path) -> Self {
        let mut l: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default();
        l.slots.retain(|k, _| k.parse::<usize>().is_ok_and(|i| i < SLOTS));
        l
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).context("creating layouts dir")?;
        }
        let text = toml::to_string_pretty(self).context("serialize layouts")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).context("write layouts")?;
        std::fs::rename(&tmp, path).context("commit layouts")?;
        Ok(())
    }

    pub fn slot(&self, i: usize) -> Option<&Layout> {
        self.slots.get(&i.to_string())
    }

    pub fn set(&mut self, i: usize, layout: Option<Layout>) {
        match layout {
            Some(l) => {
                self.slots.insert(i.to_string(), l);
            }
            None => {
                self.slots.remove(&i.to_string());
                if self.favorite == Some(i) {
                    self.favorite = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_empty_slots_and_favorite() {
        let mut l = Layouts { favorite: Some(2), ..Default::default() };
        l.set(2, Some(Layout {
            name: "Gaming".into(),
            saved_unix: 1,
            panels: vec![LayoutPanel {
                kind: "widgets".into(),
                widgets: vec!["system".into(), "volume".into()],
                attach: "left".into(),
                pose: [[1.0, 0.0, 0.0, 0.1], [0.0, 1.0, 0.0, 0.2], [0.0, 0.0, 1.0, 0.3]],
                width: 0.34,
                group: Some(3),
                ..Default::default()
            }],
            keyboard: Some(KeyboardPose { pose: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, -0.5]], width: 0.75 }),
        }));
        let dir = std::env::temp_dir().join(format!("rso-layouts-{}", std::process::id()));
        let p = dir.join("layouts.toml");
        l.save(&p).unwrap();
        let back = Layouts::load(&p);
        assert_eq!(back, l);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_gives_empty_slots_and_clear_drops_favorite() {
        let mut l = Layouts::load(Path::new("Z:/definitely/not/here.toml"));
        assert!((0..SLOTS).all(|i| l.slot(i).is_none()));
        l.set(1, Some(Layout::default()));
        l.favorite = Some(1);
        l.set(1, None);
        assert_eq!(l.favorite, None);
    }
}
