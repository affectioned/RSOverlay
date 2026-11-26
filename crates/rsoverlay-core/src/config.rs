//! TOML config with hot-reload hooks.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub render: Render,
    pub notify: Notify,
    pub preview: Preview,
    pub appearance: Appearance,
    pub placement: Placement,
    pub night_light: NightLightCfg,
    pub osc: OscCfg,
    pub vr: VrCfg,
    /// Where things were left in VR, restored next session.
    pub vr_state: VrState,
    /// VR toast placement + alert sounds.
    pub notifications: NotificationsCfg,
    /// What Discord Rich Presence shares (on/off is `appearance.discord_presence`).
    pub discord: DiscordCfg,
    /// Update notifier (GitHub builds only; Steam updates itself).
    pub updates: UpdatesCfg,
    /// Gamepad (XInput) shortcuts.
    pub gamepad: GamepadCfg,
    /// Virtual StreamDeck buttons. Serialized as `[[macros]]` tables.
    /// Must stay the last field: TOML arrays of tables have to follow
    /// every plain table in the document.
    pub macros: Macros,
}

/// VR presentation behaviour (XSO-equivalent knobs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VrCfg {
    /// The wrist goes on the *other* hand; the laser hand is this one.
    pub left_handed: bool,
    /// Panels fade out when viewed more than this many degrees off-axis.
    pub clip_angle: f32,
    pub wrist_opacity: f32,
    /// Relaunch elevated at startup so laser/keyboard input can reach
    /// "Run as administrator" windows (Windows UIPI blocks it otherwise).
    pub run_as_admin: bool,
    /// Accessibility: VR text/UI size multiplier (0.8–1.6).
    pub ui_scale: f32,
    /// Accessibility: draw laser beams from the controllers.
    pub show_lasers: bool,
    /// Fine-tune the laser's pitch on top of SteamVR's tip pose, degrees
    /// (negative = aim lower).
    pub laser_angle: f32,
    /// 0 = raw cursor, 1 = heavy smoothing (velocity-adaptive).
    pub cursor_smoothing: f32,
    /// Stick scroll speed multiplier.
    pub scroll_speed: f32,
    /// Wrist toolbar buttons shown (bit mask over the 8 actions).
    pub wrist_buttons: u8,
    /// Revision of the wrist action set the mask was saved with. Rev 1
    /// added Focus (bit 7): older masks get it switched on once.
    /// Missing from a saved file = 0 (an old config), not the default 1.
    #[serde(default)]
    pub wrist_actions_rev: u8,
    /// Accent follows album art while music plays.
    pub media_theme: bool,
    /// Wrist shows its full set (layouts, devices, views), not the
    /// compact one.
    pub wrist_expanded: bool,
    /// What the *compact* wrist shows: bits 0..7 = actions (same order
    /// as `wrist_buttons`), then clock, media, batteries, layouts,
    /// views (see `rsoverlay_ui::wrist::compact`). Default: Add,
    /// Recenter, Keyboard, Dashboard + clock, media, batteries.
    pub wrist_compact: u32,
    /// VR keyboard opacity (0.2–1.0).
    pub keyboard_opacity: f32,
    /// Click sounds while typing on the VR keyboard.
    pub typing_sounds: bool,
    /// Typing sound volume (0–1).
    pub typing_volume: f32,
    /// Sound pack folder under `sounds/`; empty = built-in clicks.
    pub sound_pack: String,
    /// Laser: hold the trigger still for a right click (opt-in).
    pub long_press: bool,
    /// Long-press time, seconds (0.3–1.5).
    pub long_press_s: f32,
    /// Laser double-click assist window, seconds (0 = off).
    pub double_click_s: f32,
    /// Controller vibration strength (0 = off, 1 = default, up to 1.5).
    pub haptics: f32,
    /// Grab smoothing 0..1 (0 = raw hand motion).
    pub grab_smoothing: f32,
    /// The real mouse follows the laser over windows (off: only on click
    /// / drag).
    pub mouse_follows_laser: bool,
    /// Windows get touch input instead of left clicks.
    pub window_touch: bool,
    /// Reopen the windows / widgets that were open when VR last closed
    /// (a starred startup layout takes precedence).
    pub restore_session: bool,
    /// Don't show the wrist toolbar (the dashboard is still summoned with
    /// the controller's menu double-press).
    pub hide_wrist: bool,
    /// Fade every RSOverlay panel out while SteamVR's own dashboard is
    /// open (it sits behind ours otherwise).
    pub hide_with_steamvr_dashboard: bool,
    /// XSO layout mode: windows that aren't pinned show only while the
    /// RSOverlay dashboard is open; pinned ones always stay.
    pub windows_follow_dashboard: bool,
    /// While grabbing, the panel tilts/rolls with the controller instead
    /// of staying upright and facing you.
    pub grab_tilt: bool,
    /// While grabbing, the stick does one thing at a time: the axis
    /// pushed first (push/pull or scale) wins until the stick recentres.
    pub stick_axis_lock: bool,
    /// Laser cursor + beam size multiplier (0.5–2.5).
    pub pointer_size: f32,
    /// Panel render resolution multiplier (1, 1.5 or 2): sharper text on
    /// high-resolution headsets, more VRAM.
    pub render_scale: f32,
}

/// Where things were left in VR — not choices made in Settings, but
/// hands-on adjustments restored so the next session starts the same.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VrState {
    /// Wrist pose relative to its controller (row-major 3×4) after the
    /// user grabbed and moved it. `None` = default mount.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrist_pose: Option<[[f32; 4]; 3]>,
    /// Wrist width, metres, after a grab-scale. `None` = default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrist_width: Option<f32>,
    /// VR keyboard toolbar toggles.
    pub kb_pinned: bool,
    pub kb_auto_repeat: bool,
    pub kb_full: bool,
    pub kb_vrchat: bool,
}

/// VR toast placement and the sound each alert plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsCfg {
    /// Toast panel width, metres (0.15–0.6).
    pub size_m: f32,
    /// Height relative to eye level, metres (−0.4–0.2).
    pub offset_y: f32,
    /// Distance in front of the eyes, metres (0.3–1.5).
    pub distance_m: f32,
    /// Alert sound volume (0–1).
    pub volume: f32,
    /// Per alert: "builtin", "none" or a WAV file name in `sounds/`.
    pub sound_notification: String,
    pub sound_warning: String,
    pub sound_error: String,
    pub sound_low_battery: String,
    /// Apps (their `sourceApp`) whose notifications skip the toast and
    /// sound; they still land in history.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub muted_apps: Vec<String>,
}

impl Default for NotificationsCfg {
    fn default() -> Self {
        Self {
            size_m: 0.30,
            offset_y: -0.12,
            distance_m: 0.65,
            volume: 0.6,
            sound_notification: "builtin".into(),
            sound_warning: "builtin".into(),
            sound_error: "builtin".into(),
            sound_low_battery: "builtin".into(),
            muted_apps: Vec::new(),
        }
    }
}

/// Gamepad shortcuts: XInput button chords (bit masks, 0 = unset).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GamepadCfg {
    /// Off by default: never react to a game's controller unless asked.
    pub enabled: bool,
    pub dashboard: u16,
    pub recenter: u16,
    pub keyboard: u16,
}

impl Default for GamepadCfg {
    fn default() -> Self {
        // Back + LB / Back + RB / Back + Y.
        Self { enabled: false, dashboard: 0x0120, recenter: 0x0220, keyboard: 0x8020 }
    }
}

/// Update notifier. Only ever *tells* you about a new version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatesCfg {
    /// Ask GitHub for new versions at startup and every few hours.
    pub check: bool,
    /// Also offer test (pre-release) builds.
    pub prereleases: bool,
    /// A version the user chose to skip (its tag).
    pub skipped: String,
}

impl Default for UpdatesCfg {
    fn default() -> Self {
        Self { check: true, prereleases: false, skipped: String::new() }
    }
}

/// What Discord Rich Presence shares. Window titles are never shared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordCfg {
    /// "In VR · 3 windows open" (count only).
    pub show_windows: bool,
    /// Headset battery + time left.
    pub show_battery: bool,
    /// Current song + artist (off by default: visible to everyone).
    pub show_music: bool,
    /// "Get RSOverlay" button linking to the project.
    pub show_button: bool,
}

impl Default for DiscordCfg {
    fn default() -> Self {
        Self { show_windows: true, show_battery: true, show_music: false, show_button: true }
    }
}

/// Log files (`rsoverlay.log`, previous run in `rsoverlay.prev.log`),
/// next to config.toml.
pub fn logs_dir() -> std::path::PathBuf {
    Config::default_path().with_file_name("logs")
}

/// Folder holding user typing-sound packs (`sounds/<pack>/key.wav` …),
/// next to config.toml.
pub fn sounds_dir() -> std::path::PathBuf {
    Config::default_path().with_file_name("sounds")
}

/// Compact wrist default: the four everyday actions + clock (with the
/// session timer), media and device batteries (battery is the
/// most-glanced wrist readout after time).
pub const DEFAULT_WRIST_COMPACT: u32 = 0x0F | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 13);

impl Default for VrCfg {
    fn default() -> Self {
        Self { left_handed: false, clip_angle: 60.0, wrist_opacity: 1.0, run_as_admin: false, ui_scale: 1.0, show_lasers: true, laser_angle: 0.0, cursor_smoothing: 0.5, scroll_speed: 1.0, wrist_buttons: 0xFF, wrist_actions_rev: 1, media_theme: false, wrist_expanded: false, wrist_compact: DEFAULT_WRIST_COMPACT, keyboard_opacity: 1.0, typing_sounds: true, typing_volume: 0.4, sound_pack: String::new(), long_press: false, long_press_s: 0.6, double_click_s: 0.4, haptics: 1.0, grab_smoothing: 0.3, mouse_follows_laser: true, window_touch: false, restore_session: true, hide_wrist: false, hide_with_steamvr_dashboard: true, windows_follow_dashboard: true, grab_tilt: false, stick_axis_lock: true, pointer_size: 1.0, render_scale: 1.0 }
    }
}

/// VRChat OSC output beyond the chatbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OscCfg {
    /// Publish battery levels as avatar parameters. Off by default.
    pub battery_params: bool,
    /// Parameter path prefix under /avatar/parameters/.
    pub battery_prefix: String,
}

impl Default for OscCfg {
    fn default() -> Self {
        Self { battery_params: false, battery_prefix: "RSO/Battery".into() }
    }
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Accent color stored as `#rrggbb` hex. Falls back to the rose
    /// default if missing or malformed.
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
    /// Play a chime with the low-battery toast (VR).
    pub battery_sound: bool,
    /// Clock: "system", "12h" or "24h".
    pub clock_format: String,
    /// Date: "system" (Thu, Nov 26), "day-month" (Thu 26 Nov) or "iso".
    pub date_format: String,
    /// Wrist background: an image file name in `backgrounds/` ("" = none).
    pub wrist_background: String,
    /// Darkening over the wrist background, 0..1.
    pub wrist_background_dim: f32,
    /// Devices (by label) hidden from the battery card, wrist and header;
    /// they don't raise low-battery alerts either.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub battery_hidden: Vec<String>,
    /// Show "Using RSOverlay" on the user's Discord profile.
    pub discord_presence: bool,
    /// Use each device's icon from the user's SteamVR install (never
    /// shipped with RSOverlay); off → RSOverlay's own icons.
    pub steamvr_icons: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            accent_hex: "#E88FA2".into(),
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
            battery_hidden: Vec::new(),
            clock_format: "system".into(),
            date_format: "system".into(),
            wrist_background: String::new(),
            wrist_background_dim: 0.55,
            battery_sound: true,
            discord_presence: true,
            steamvr_icons: true,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notify {
    pub enabled: bool,
    pub bind: String,
    /// Token required for any non-localhost origin. Localhost is unauthenticated by default.
    pub token: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

#[cfg(test)]
mod wrist_rev_tests {
    #[test]
    fn old_config_without_rev_reads_as_rev_0() {
        let old: super::VrCfg = toml::from_str("wrist_buttons = 127").unwrap();
        assert_eq!(old.wrist_actions_rev, 0);
        assert_eq!(super::VrCfg::default().wrist_actions_rev, 1);
    }
}
