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
            preview_window_width: 1024,
            preview_window_height: 1024,
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
