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
}
