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
