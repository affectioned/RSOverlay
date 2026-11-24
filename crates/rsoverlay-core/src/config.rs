//! TOML config with hot-reload hooks.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub render: Render,
    pub notify: Notify,
    pub preview: Preview,
    pub appearance: Appearance,
    pub placement: Placement,
    pub night_light: NightLightCfg,
    /// Virtual StreamDeck buttons. Serialized as `[[macros]]` tables.
    /// Must stay the last field: TOML arrays of tables have to follow
    /// every plain table in the document.
    pub macros: Macros,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NightLightCfg {
    /// off / on / scheduled
    pub mode: String,
    /// 0.0 neutral ..= 1.0 warmest.
    pub strength: f32,
    pub start_hour: u8,
    pub end_hour: u8,
}

impl Default for NightLightCfg {
    fn default() -> Self {
        Self { mode: "off".into(), strength: 0.5, start_hour: 21, end_hour: 7 }
    }
}

/// One macro button: a label and a shortcut string such as
/// `ctrl+shift+m` (grammar in `rsoverlay_input::shortcut`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Macro {
    pub label: String,
    pub keys: String,
}

/// Newtype so `Config::default()` ships starter macros while a file
/// with `macros = []` still loads as empty (user deleted them all).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Macros(pub Vec<Macro>);

impl Default for Macros {
    fn default() -> Self {
        // All global hotkeys — they fire regardless of which window has
        // focus, which is the whole point of a VR StreamDeck.
        let m = |label: &str, keys: &str| Macro { label: label.into(), keys: keys.into() };
        Self(vec![
            m("Discord mute", "ctrl+shift+m"),
            m("Clip last 30s", "alt+f10"),
            m("Snip", "win+shift+s"),
            m("Task view", "win+tab"),
            m("Play / pause", "media_play_pause"),
        ])
    }
}

/// User-tweakable dashboard appearance. Persisted so your accent choice
/// and card visibility toggles survive a restart.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Accent color stored as `#rrggbb` hex. Falls back to the muted
    /// teal default if missing or malformed.
    pub accent_hex: String,
    pub show_mirror: bool,
    pub show_media: bool,
    pub show_chat: bool,
    pub show_keyboard: bool,
    pub show_history: bool,
    pub forward_input: bool,
    /// BCP-47 UI language tag — en-US, de-DE, ja-JP, zh-CN, etc.
    /// Defaults to en-US; parse errors fall back to that too.
    pub language: String,
    /// Virtual keyboard physical layout — qwerty / qwertz / azerty /
    /// jcuken-ru / abnt2. Parse errors fall back to qwerty.
    pub keyboard_layout: String,
    /// Mirror the SMTC now-playing track into the VRChat chatbox as
    /// `♪ Title — Artist`. Off by default — it speaks for the user.
    pub chat_now_playing: bool,
    pub show_battery: bool,
    pub show_macros: bool,
    pub show_volume: bool,
    pub show_system: bool,
    /// Accessibility: disable UI transitions.
    pub reduce_motion: bool,
    /// Low-battery toast threshold, percent. Clamped to 5..=50 in UI.
    pub battery_warn_pct: u8,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            accent_hex: "#7DD3C0".into(),
            show_mirror: true,
            show_media: true,
            show_chat: true,
            show_keyboard: false,
            show_history: false,
            forward_input: false,
            language: "en-US".into(),
            keyboard_layout: "qwerty".into(),
            chat_now_playing: false,
            show_battery: true,
            show_macros: true,
            show_volume: true,
            show_system: true,
            reduce_motion: false,
            battery_warn_pct: 20,
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlacementAnchor {
    #[default]
    Wrist,
    Hmd,
    World,
}

/// Spatial placement of the main overlay. Persisted so your position
/// tweaks survive a restart.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Placement {
    pub anchor: PlacementAnchor,
    pub offset_x: f32,
    pub offset_y: f32,
    pub offset_z: f32,
    pub size_w: f32,
    pub size_h: f32,
    /// 0.0 flat → 1.0 fully wrapped around a cylinder. Curves the
    /// overlay toward the viewer for immersive feel at wider sizes.
    /// Matches XSO's curved overlay option.
    pub curvature: f32,
    /// Overlay opacity in the compositor, 0.2..=1.0.
    pub opacity: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            anchor: PlacementAnchor::Wrist,
            offset_x: -0.22,
            offset_y: 1.30,
            offset_z: -0.40,
            size_w: 0.36,
            size_h: 0.225,
            curvature: 0.0,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Render {
    /// VSync in preview mode. In VR mode we align to the compositor.
    pub preview_vsync: bool,
    /// Preview window logical size. The overlay texture itself is independent.
    pub preview_window_width: u32,
    pub preview_window_height: u32,
    /// Side length of the square overlay texture, in pixels.
    ///
    /// Sizing rule of thumb: for a 1 m-wide overlay at 2 m from the HMD,
    /// the Pimax Crystal / Varjo Aero class of headsets will cover the
    /// overlay with ≈1200 display pixels. A ratio of texels-to-display-
    /// pixels of 1.5-2.0 is where SteamVR overlay sampling stays crisp.
    ///
    /// - 1024 — fine for Index / Quest 2 and mild oversampling
    /// - **2048** (default) — safe across all current retail HMDs
    /// - 2560 — Bigscreen Beyond, Pimax Crystal power users
    /// - 3072 — Crystal Super, XR-4, Varjo XR-4 class
    ///
    /// Memory cost is `size × size × 4` bytes (BGRA8). 2048 = 16 MB.
    pub overlay_texture_size: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Notify {
    pub enabled: bool,
    pub bind: String,
    /// Token required for any non-localhost origin. Localhost is unauthenticated by default.
    pub token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    /// When true, launch into desktop preview even if SteamVR is running.
    pub force_preview: bool,
}

impl Default for Render {
    fn default() -> Self {
        Self {
            preview_vsync: true,
            preview_window_width: 1200,
            preview_window_height: 1200,
            overlay_texture_size: 2048,
        }
    }
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: "127.0.0.1:42069".into(),
            token: None,
        }
    }
}


impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        let cfg: Self = toml::from_str(&text)
            .with_context(|| format!("parsing config at {}", path.display()))?;
        Ok(cfg)
    }

    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("RSOverlay").join("config.toml")
    }

    /// Serialize to TOML and write atomically (write to `.tmp`, then
    /// rename). Keeps the file consistent even if the process dies
    /// mid-write.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serialize config to TOML")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)
            .with_context(|| format!("writing config at {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("committing config at {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macros_roundtrip_as_array_of_tables() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("[[macros]]"));
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.macros, cfg.macros);
    }

    #[test]
    fn missing_macros_gets_starters_but_empty_stays_empty() {
        let missing: Config = toml::from_str("").unwrap();
        assert_eq!(missing.macros, Macros::default());
        let empty: Config = toml::from_str("macros = []").unwrap();
        assert!(empty.macros.0.is_empty());
    }
}
