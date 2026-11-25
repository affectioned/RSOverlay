//! Finding SteamVR's own device icons on disk.
//!
//! RSOverlay ships none of Valve's or hardware makers' artwork: it reads
//! the icons from the user's SteamVR install at runtime, the same files
//! SteamVR shows for each device.
//!
//! - The install: `%LOCALAPPDATA%\openvr\openvrpaths.vrpath` lists the
//!   runtime folder and any external driver folders.
//! - A device's icon (OpenVR `Prop_NamedIconPathDeviceReady_String`) looks
//!   like `{indexcontroller}/icons/left_controller_status_ready.png`:
//!   `{driver}` is that driver's `resources` folder. A path without braces
//!   is relative to the device's own resource root.
//!
//! Driver-supplied strings are only ever joined below a driver's
//! `resources` folder, and rejected if they contain `..`, a drive or a
//! root, so they can't point anywhere else on disk.

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SteamVrDirs {
    /// SteamVR install (contains `drivers/`).
    pub runtime: Option<PathBuf>,
    /// Third-party driver folders registered with OpenVR.
    pub external_drivers: Vec<PathBuf>,
}

impl SteamVrDirs {
    /// Read the OpenVR path registry. `None` when SteamVR isn't installed.
    pub fn locate() -> Option<Self> {
        let base = std::env::var_os("LOCALAPPDATA")?;
        let text = std::fs::read_to_string(Path::new(&base).join("openvr").join("openvrpaths.vrpath")).ok()?;
        Self::parse(&text)
    }

    /// Parse `openvrpaths.vrpath` (JSON). Pure, tested.
    pub fn parse(json: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let list = |key: &str| -> Vec<PathBuf> {
            v.get(key)
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str()).map(PathBuf::from).collect())
                .unwrap_or_default()
        };
        let runtime = list("runtime").into_iter().next();
        let external_drivers = list("external_drivers");
        (runtime.is_some() || !external_drivers.is_empty()).then_some(Self { runtime, external_drivers })
    }

    /// `resources` folders a driver name can mean: the built-in driver
    /// folder, or an external driver whose folder name matches.
    fn resource_dirs(&self, driver: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(rt) = &self.runtime {
            out.push(rt.join("drivers").join(driver).join("resources"));
        }
        for ext in &self.external_drivers {
            if ext.file_name().is_some_and(|n| n.eq_ignore_ascii_case(driver)) {
                out.push(ext.join("resources"));
            }
        }
        out
    }

    /// Resolve a driver icon path to an existing PNG, preferring the
    /// sharper `_2x` variant. `resource_root` is the device's own driver
    /// (OpenVR `Prop_ResourceRoot_String`), used for paths without `{…}`.
    pub fn resolve_icon(&self, icon: &str, resource_root: &str) -> Option<PathBuf> {
        let (driver, rest) = split_icon_path(icon, resource_root)?;
        let rel = safe_relative(rest)?;
        for dir in self.resource_dirs(driver) {
            let p = dir.join(&rel);
            let hi = with_2x(&p);
            if hi.as_ref().is_some_and(|h| h.is_file()) {
                return hi;
            }
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }

    /// Representative SteamVR icon for a device *label* (the desktop
    /// preview's demo devices, which have no OpenVR properties).
    pub fn demo_icon(&self, label: &str) -> Option<PathBuf> {
        let (driver, file) = match label {
            "HMD" => ("indexhmd", "icons/headset_status_ready.png"),
            "Left hand" => ("indexcontroller", "icons/left_controller_status_ready.png"),
            "Right hand" => ("indexcontroller", "icons/right_controller_status_ready.png"),
            l if l.starts_with("Tracker") => ("htc", "icons/tracker_status_ready.png"),
            _ => return None,
        };
        self.resolve_icon(&format!("{{{driver}}}/{file}"), driver)
    }
}

/// `{driver}/rest` → (driver, rest); `rest` alone → (resource_root, rest).
fn split_icon_path<'a>(icon: &'a str, resource_root: &'a str) -> Option<(&'a str, &'a str)> {
    let icon = icon.trim();
    let (driver, rest) = match icon.strip_prefix('{') {
        Some(s) => {
            let (d, r) = s.split_once('}')?;
            (d, r.trim_start_matches(['/', '\\']))
        }
        None => (resource_root, icon),
    };
    let ok_name = !driver.is_empty() && driver.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') && driver != "." && driver != "..";
    (ok_name && !rest.is_empty()).then_some((driver, rest))
}

/// A relative path with only normal components (no `..`, root or drive).
fn safe_relative(rest: &str) -> Option<PathBuf> {
    let p = PathBuf::from(rest.replace('\\', "/"));
    p.components().all(|c| matches!(c, Component::Normal(_))).then_some(p)
}

/// `foo.png` → `foo_2x.png`.
fn with_2x(p: &Path) -> Option<PathBuf> {
    let stem = p.file_stem()?.to_str()?;
    let ext = p.extension()?.to_str()?;
    Some(p.with_file_name(format!("{stem}_2x.{ext}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_vrpath_registry() {
        let d = SteamVrDirs::parse(r#"{"runtime":["C:\\Steam\\steamapps\\common\\SteamVR"],"external_drivers":["D:\\Drv\\spacesync"],"version":1}"#).unwrap();
        assert_eq!(d.runtime, Some(PathBuf::from(r"C:\Steam\steamapps\common\SteamVR")));
        assert_eq!(d.external_drivers, vec![PathBuf::from(r"D:\Drv\spacesync")]);
        assert!(SteamVrDirs::parse("not json").is_none());
        assert!(SteamVrDirs::parse("{}").is_none());
    }

    #[test]
    fn icon_paths_split_into_driver_and_file() {
        assert_eq!(split_icon_path("{indexhmd}/icons/headset.png", "x"), Some(("indexhmd", "icons/headset.png")));
        assert_eq!(split_icon_path("icons/a.png", "lighthouse"), Some(("lighthouse", "icons/a.png")));
        assert_eq!(split_icon_path("{}/a.png", "x"), None);
        assert_eq!(split_icon_path("{..}/a.png", "x"), None);
        assert_eq!(split_icon_path("{a/b}/x.png", "x"), None);
    }

    #[test]
    fn driver_paths_cannot_escape_the_resources_folder() {
        assert!(safe_relative("icons/a.png").is_some());
        for bad in ["../../secret.png", "icons/../../x.png", "/etc/x.png", r"C:\x.png", r"\\host\share\x.png"] {
            assert!(safe_relative(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn resolves_files_and_prefers_2x() {
        let root = std::env::temp_dir().join(format!("rso-steamvr-{}", std::process::id()));
        let icons = root.join("drivers").join("indexhmd").join("resources").join("icons");
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::write(icons.join("headset_status_ready.png"), b"x").unwrap();
        let d = SteamVrDirs { runtime: Some(root.clone()), external_drivers: vec![] };
        assert_eq!(d.demo_icon("HMD"), Some(icons.join("headset_status_ready.png")));
        std::fs::write(icons.join("headset_status_ready_2x.png"), b"x").unwrap();
        assert_eq!(d.demo_icon("HMD"), Some(icons.join("headset_status_ready_2x.png")));
        assert_eq!(d.resolve_icon("{indexhmd}/icons/missing.png", "indexhmd"), None);
        assert_eq!(d.resolve_icon("{indexhmd}/../../../x.png", "indexhmd"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
