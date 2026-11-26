//! Desktop preview mode.
//!
//! Opens a winit window, builds a wgpu surface on it, and runs the same panel
//! render pipeline the VR path uses. The overlay texture is rendered
//! offscreen into an [`OverlayTarget`] via egui-wgpu and then blitted onto the
//! window via a fullscreen-triangle pass. This matches the VR flow one-to-one;
//! only the final presentation step differs.
//!
//! Window events are forwarded to `egui-winit` so the UI inside the overlay
//! responds to the cursor exactly the way VR laser-pointer input will
//! eventually drive it.

use anyhow::{Context, Result};
use crate::mirror_source::{fetch_icons, list_sources, GpuMirror};
use rsoverlay_capture::{enumerate_monitors, DxgiDuplication, Frame};
use rsoverlay_input::{InputInjector, MonitorGeom, MouseButton};
use rsoverlay_core::RuntimeMode;
use rsoverlay_notify::NotifyChannel;
use rsoverlay_render::{mirror::MirrorTexture, GpuCtx, OverlayTarget};
use rsoverlay_ui::{
    views::{CaptureSource, DevView, MirrorHandle},
    EguiPanel,
};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct CaptureCfg {
    pub enabled: bool,
    pub monitor_index: u32,
    /// Mirror a single window instead of a monitor: case-insensitive
    /// title substring. Falls back to `monitor_index` if no match.
    pub window: Option<String>,
    /// When true, skip the GPU-to-GPU shared-handle path entirely and
    /// use the CPU readback path. Useful for A/B testing and for working
    /// around driver bugs we haven't shaken out yet.
    pub force_cpu: bool,
}

/// Automation affordances for running the preview non-interactively —
/// an LLM or CI driver can launch with `--screenshot path.png
/// --exit-after 120` and come back to a single PNG without needing to
/// drive the window.
#[derive(Clone, Debug, Default)]
pub struct TestHooks {
    pub screenshot: Option<std::path::PathBuf>,
    pub exit_after: Option<u64>,
    pub overlay_pos: Option<(f32, f32, f32)>,
    pub overlay_size: Option<(f32, f32)>,
    pub overlay_anchor: Option<String>,
    pub show_keyboard: bool,
    pub show_history: bool,
    pub demo_batteries: bool,
    pub switch_to_window: Option<String>,
    pub page: Option<String>,
    pub dock: bool,
    pub wrist: bool,
    pub wrist_edit: bool,
    pub widgets: Option<String>,
    pub vr_keyboard: bool,
    pub search: Option<String>,
    pub scene: Option<String>,
}

/// Apply CLI overrides to a default Placement. Any field not overridden
/// keeps its default value. Kept public so external callers (tests,
/// future tooling) can reuse the CLI-override logic independently of
/// the broader `seed_dev_view` flow.
#[allow(dead_code)]
pub fn placement_with_overrides(test: &TestHooks) -> rsoverlay_ui::views::Placement {
    let mut p = rsoverlay_ui::views::Placement::default();
    if let Some((x, y, z)) = test.overlay_pos {
        p.offset_x = x;
        p.offset_y = y;
        p.offset_z = z;
    }
    if let Some((w, h)) = test.overlay_size {
        p.size_w = w;
        p.size_h = h;
    }
    if let Some(a) = test.overlay_anchor.as_deref() {
        p.anchor = match a.to_ascii_lowercase().as_str() {
            "wrist" => rsoverlay_ui::views::Anchor::Wrist,
            "hmd" | "head" => rsoverlay_ui::views::Anchor::Hmd,
            "world" => rsoverlay_ui::views::Anchor::World,
            _ => p.anchor,
        };
    }
    p
}

/// Seed a fresh [`DevView`] with values from the persisted config,
/// then layer any CLI test-hook overrides on top. Keeps the "load →
/// override → use" ordering in one place so every entry mode
/// (preview, preview_vr, vr) picks the same initial state.
pub fn seed_dev_view(
    dev: &mut rsoverlay_ui::views::DevView,
    cfg: &rsoverlay_core::Config,
    test: &TestHooks,
) {
    use rsoverlay_core::PlacementAnchor;
    use rsoverlay_ui::views::Anchor as UiAnchor;

    // Appearance.
    if let Some(c) = parse_hex_color(&cfg.appearance.accent_hex) {
        dev.accent = c;
    }
    dev.show_mirror = cfg.appearance.show_mirror;
    dev.show_media = cfg.appearance.show_media;
    dev.show_chat = cfg.appearance.show_chat;
    dev.show_keyboard = cfg.appearance.show_keyboard;
    dev.show_history = cfg.appearance.show_history;
    dev.forward_input = cfg.appearance.forward_input;
    dev.language = parse_lang(&cfg.appearance.language);
    dev.keyboard.layout = parse_layout(&cfg.appearance.keyboard_layout);
    dev.now_playing.enabled = cfg.appearance.chat_now_playing;
    dev.show_battery = cfg.appearance.show_battery;
    dev.show_macros = cfg.appearance.show_macros;
    dev.show_volume = cfg.appearance.show_volume;
    dev.show_system = cfg.appearance.show_system;
    dev.vr_left_handed = cfg.vr.left_handed;
    dev.vr_clip_angle = cfg.vr.clip_angle.clamp(30.0, 90.0);
    dev.vr_wrist_opacity = cfg.vr.wrist_opacity.clamp(0.2, 1.0);
    dev.run_as_admin = cfg.vr.run_as_admin;
    dev.vr_ui_scale = cfg.vr.ui_scale.clamp(0.8, 1.6);
    dev.vr_show_lasers = cfg.vr.show_lasers;
    dev.vr_laser_angle = cfg.vr.laser_angle.clamp(-30.0, 30.0);
    dev.cursor_smoothing = cfg.vr.cursor_smoothing.clamp(0.0, 1.0);
    dev.scroll_speed = cfg.vr.scroll_speed.clamp(0.25, 3.0);
    dev.wrist_buttons = if cfg.vr.wrist_buttons & 0x7F == 0 { 0x7F } else { cfg.vr.wrist_buttons };
    dev.media_theme = cfg.vr.media_theme;
    dev.wrist_expanded = cfg.vr.wrist_expanded;
    dev.wrist_compact = cfg.vr.wrist_compact;
    dev.kb_prefs.opacity = cfg.vr.keyboard_opacity.clamp(0.2, 1.0);
    dev.kb_prefs.sounds = cfg.vr.typing_sounds;
    dev.kb_prefs.volume = cfg.vr.typing_volume.clamp(0.0, 1.0);
    dev.kb_prefs.pack = cfg.vr.sound_pack.clone();
    dev.pointer.long_press = cfg.vr.long_press;
    dev.pointer.long_press_s = cfg.vr.long_press_s.clamp(0.3, 1.5);
    dev.pointer.double_click_s = cfg.vr.double_click_s.clamp(0.0, 0.8);
    dev.restore_session = cfg.vr.restore_session;
    dev.vr_hide_wrist = cfg.vr.hide_wrist;
    dev.vr_hide_with_steamvr = cfg.vr.hide_with_steamvr_dashboard;
    dev.vr_windows_follow_dash = cfg.vr.windows_follow_dashboard;
    dev.vr_grab_tilt = cfg.vr.grab_tilt;
    dev.vr_stick_axis_lock = cfg.vr.stick_axis_lock;
    dev.vr_pointer_size = cfg.vr.pointer_size.clamp(0.5, 2.5);
    dev.vr_render_scale = cfg.vr.render_scale.clamp(1.0, 2.0);
    let nc = &cfg.notifications;
    dev.notif.size_m = nc.size_m.clamp(0.15, 0.6);
    dev.notif.offset_y = nc.offset_y.clamp(-0.4, 0.2);
    dev.notif.distance_m = nc.distance_m.clamp(0.3, 1.5);
    dev.notif.volume = nc.volume.clamp(0.0, 1.0);
    dev.notif.sounds = [nc.sound_notification.clone(), nc.sound_warning.clone(), nc.sound_error.clone(), nc.sound_low_battery.clone()];
    dev.notif.files = rsoverlay_audio::alerts::list_sound_files(&rsoverlay_core::config::sounds_dir());
    dev.vr_state = cfg.vr_state.clone();
    dev.kb_prefs.packs = rsoverlay_audio::typing::list_packs(&rsoverlay_core::config::sounds_dir());
    dev.elevated = rsoverlay_input::elevation::current_process_elevated();
    dev.battery_osc.enabled = cfg.osc.battery_params;
    dev.battery_osc.prefix = cfg.osc.battery_prefix.trim_matches('/').to_string();
    dev.reduce_motion = cfg.appearance.reduce_motion;
    dev.macros.entries = cfg
        .macros
        .0
        .iter()
        .map(|m| rsoverlay_ui::macros::MacroEntry {
            label: m.label.clone(),
            keys: m.keys.clone(),
        })
        .collect();
    dev.battery_warn_pct = cfg.appearance.battery_warn_pct.clamp(5, 50);
    dev.battery_sound = cfg.appearance.battery_sound;
    dev.discord_presence = cfg.appearance.discord_presence;
    dev.steamvr_icons = cfg.appearance.steamvr_icons;
    dev.discord = cfg.discord.clone();
    if test.show_keyboard {
        dev.show_keyboard = true;
    }
    if test.show_history {
        dev.show_history = true;
    }
    if test.dock {
        dev.dock_layout = true;
    }
    if let Some(q) = &test.search {
        dev.search.query = q.clone();
    }
    if let Some(p) = test.page.as_deref().and_then(rsoverlay_ui::shell::Page::from_slug) {
        dev.page = p;
    }
    dev.wrist_editing = test.wrist_edit;
    if test.demo_batteries {
        use rsoverlay_ui::battery::BatteryReading;
        let mut icons = crate::device_icons::DeviceIcons::default();
        let use_icons = dev.steamvr_icons;
        let mut r = |id, label: &str, fraction, charging| BatteryReading {
            id,
            label: label.into(),
            fraction,
            charging,
            icon: if use_icons { icons.for_label(label) } else { None },
        };
        dev.set_batteries(vec![
            r(0, "HMD", 0.82, false),
            r(1, "Left hand", 0.14, false),
            r(2, "Right hand", 0.47, true),
            r(3, "Tracker · VIVE Tracker 3.0", 0.63, false),
        ]);
    }

    // Placement.
    dev.placement.anchor = match cfg.placement.anchor {
        PlacementAnchor::Wrist => UiAnchor::Wrist,
        PlacementAnchor::Hmd => UiAnchor::Hmd,
        PlacementAnchor::World => UiAnchor::World,
    };
    dev.placement.offset_x = cfg.placement.offset_x;
    dev.placement.offset_y = cfg.placement.offset_y;
    dev.placement.offset_z = cfg.placement.offset_z;
    dev.placement.size_w = cfg.placement.size_w;
    dev.placement.size_h = cfg.placement.size_h;
    dev.placement.curvature = cfg.placement.curvature;
    dev.placement.opacity = cfg.placement.opacity.clamp(0.2, 1.0);
    {
        use rsoverlay_ui::nightlight::NightMode;
        let n = &cfg.night_light;
        dev.night_light.mode = NightMode::from_slug(&n.mode);
        dev.night_light.strength = n.strength.clamp(0.0, 1.0);
        dev.night_light.start_hour = n.start_hour.min(23);
        dev.night_light.end_hour = n.end_hour.min(23);
    }
    if let Some((x, y, z)) = test.overlay_pos {
        dev.placement.offset_x = x;
        dev.placement.offset_y = y;
        dev.placement.offset_z = z;
    }
    if let Some((w, h)) = test.overlay_size {
        dev.placement.size_w = w;
        dev.placement.size_h = h;
    }
    if let Some(a) = test.overlay_anchor.as_deref() {
        dev.placement.anchor = match a.to_ascii_lowercase().as_str() {
            "wrist" => UiAnchor::Wrist,
            "hmd" | "head" => UiAnchor::Hmd,
            "world" => UiAnchor::World,
            _ => dev.placement.anchor,
        };
    }
}

/// Extract the current DevView state into a fresh Config body, ready
/// to persist. Preserves fields we don't touch (notify, render, etc.)
/// from the given template.
/// Play any alert sounds the dashboard queued, through the user's slot
/// choices (shared by the desktop window and the VR shell).
pub fn play_alert_sounds(dev: &mut rsoverlay_ui::views::DevView) {
    // Same per-frame hook both shells call: keep Discord in step with
    // the Settings toggle (an atomic store, nothing more).
    rsoverlay_discord::set_enabled(dev.discord_presence);
    if dev.discord_presence {
        rsoverlay_discord::set_details(dev.discord_details());
    }
    let due = dev.take_sounds();
    if due.is_empty() {
        return;
    }
    let dir = rsoverlay_core::config::sounds_dir();
    // Several at once (a burst of toasts): one sound is enough.
    if let Some((kind, v)) = due.last() {
        rsoverlay_audio::alerts::play_alert(*kind, dev.notif.sound_for(*kind), &dir, dev.notif.volume * v);
    }
}

pub fn config_from_dev_view(
    dev: &rsoverlay_ui::views::DevView,
    base: &rsoverlay_core::Config,
) -> rsoverlay_core::Config {
    use rsoverlay_core::PlacementAnchor;
    use rsoverlay_ui::views::Anchor as UiAnchor;
    let mut out = base.clone();
    out.appearance.accent_hex = format!(
        "#{:02X}{:02X}{:02X}",
        dev.accent.r(),
        dev.accent.g(),
        dev.accent.b()
    );
    out.appearance.show_mirror = dev.show_mirror;
    out.appearance.show_media = dev.show_media;
    out.appearance.show_chat = dev.show_chat;
    out.appearance.show_keyboard = dev.show_keyboard;
    out.appearance.show_history = dev.show_history;
    out.appearance.forward_input = dev.forward_input;
    out.appearance.language = dev.language.bcp47().to_string();
    out.appearance.keyboard_layout = dev.keyboard.layout.slug().to_string();
    out.appearance.chat_now_playing = dev.now_playing.enabled;
    out.appearance.show_battery = dev.show_battery;
    out.appearance.show_macros = dev.show_macros;
    out.appearance.show_volume = dev.show_volume;
    out.appearance.show_system = dev.show_system;
    out.osc.battery_params = dev.battery_osc.enabled;
    out.vr = rsoverlay_core::config::VrCfg {
        left_handed: dev.vr_left_handed,
        clip_angle: dev.vr_clip_angle,
        wrist_opacity: dev.vr_wrist_opacity,
        run_as_admin: dev.run_as_admin,
        ui_scale: dev.vr_ui_scale,
        show_lasers: dev.vr_show_lasers,
        laser_angle: dev.vr_laser_angle,
        cursor_smoothing: dev.cursor_smoothing,
        scroll_speed: dev.scroll_speed,
        wrist_buttons: dev.wrist_buttons,
        media_theme: dev.media_theme,
        wrist_expanded: dev.wrist_expanded,
        wrist_compact: dev.wrist_compact,
        keyboard_opacity: dev.kb_prefs.opacity,
        typing_sounds: dev.kb_prefs.sounds,
        typing_volume: dev.kb_prefs.volume,
        sound_pack: dev.kb_prefs.pack.clone(),
        long_press: dev.pointer.long_press,
        long_press_s: dev.pointer.long_press_s,
        double_click_s: dev.pointer.double_click_s,
        restore_session: dev.restore_session,
        hide_wrist: dev.vr_hide_wrist,
        hide_with_steamvr_dashboard: dev.vr_hide_with_steamvr,
        windows_follow_dashboard: dev.vr_windows_follow_dash,
        grab_tilt: dev.vr_grab_tilt,
        stick_axis_lock: dev.vr_stick_axis_lock,
        pointer_size: dev.vr_pointer_size,
        render_scale: dev.vr_render_scale,
    };
    out.vr_state = dev.vr_state.clone();
    out.discord = dev.discord.clone();
    let [n, w, e, b] = dev.notif.sounds.clone();
    out.notifications = rsoverlay_core::config::NotificationsCfg {
        size_m: dev.notif.size_m,
        offset_y: dev.notif.offset_y,
        distance_m: dev.notif.distance_m,
        volume: dev.notif.volume,
        sound_notification: n,
        sound_warning: w,
        sound_error: e,
        sound_low_battery: b,
    };
    out.appearance.reduce_motion = dev.reduce_motion;
    out.macros = rsoverlay_core::config::Macros(
        dev.macros
            .entries
            .iter()
            .map(|e| rsoverlay_core::config::Macro {
                label: e.label.clone(),
                keys: e.keys.clone(),
            })
            .collect(),
    );
    out.appearance.battery_warn_pct = dev.battery_warn_pct;
    out.appearance.battery_sound = dev.battery_sound;
    out.appearance.discord_presence = dev.discord_presence;
    out.appearance.steamvr_icons = dev.steamvr_icons;
    out.placement.anchor = match dev.placement.anchor {
        UiAnchor::Wrist => PlacementAnchor::Wrist,
        UiAnchor::Hmd => PlacementAnchor::Hmd,
        UiAnchor::World => PlacementAnchor::World,
    };
    out.placement.offset_x = dev.placement.offset_x;
    out.placement.offset_y = dev.placement.offset_y;
    out.placement.offset_z = dev.placement.offset_z;
    out.placement.size_w = dev.placement.size_w;
    out.placement.size_h = dev.placement.size_h;
    out.placement.curvature = dev.placement.curvature;
    out.placement.opacity = dev.placement.opacity;
    out.night_light = rsoverlay_core::config::NightLightCfg {
        mode: dev.night_light.mode.slug().to_string(),
        strength: dev.night_light.strength,
        start_hour: dev.night_light.start_hour,
        end_hour: dev.night_light.end_hour,
    };
    out
}

fn parse_lang(tag: &str) -> rsoverlay_ui::i18n::Lang {
    rsoverlay_ui::i18n::Lang::from_bcp47(tag).unwrap_or_default()
}

fn parse_layout(slug: &str) -> rsoverlay_ui::keyboard_layout::LayoutId {
    rsoverlay_ui::keyboard_layout::LayoutId::from_slug(slug).unwrap_or_default()
}

fn parse_hex_color(hex: &str) -> Option<egui::Color32> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(egui::Color32::from_rgb(r, g, b))
}

/// Debounced "save when something changed" helper. Owns the last
/// saved Config and compares what's live each tick. Writes at most
/// once per `SAVE_DEBOUNCE_MS` even if the user is dragging a slider.
///
/// Also watches the backing file for external edits — the user can
/// hand-edit `config.toml` in a text editor and the running overlay
/// picks up the changes within ~1 second without restart.
pub struct ConfigPersister {
    path: std::path::PathBuf,
    last_saved: rsoverlay_core::Config,
    pending_since: Option<std::time::Instant>,
    last_known_mtime: Option<std::time::SystemTime>,
    last_watch_check: std::time::Instant,
}

impl ConfigPersister {
    const SAVE_DEBOUNCE_MS: u64 = 400;
    const WATCH_INTERVAL_MS: u64 = 1000;

    pub fn new(path: std::path::PathBuf, initial: rsoverlay_core::Config) -> Self {
        let last_known_mtime = Self::read_mtime(&path);
        Self {
            path,
            last_saved: initial,
            pending_since: None,
            last_known_mtime,
            last_watch_check: std::time::Instant::now(),
        }
    }

    /// Call once per frame. If `now` differs from the last saved
    /// snapshot, start / restart the debounce. Once the debounce
    /// window elapses without further changes, write to disk.
    pub fn tick(&mut self, now: &rsoverlay_core::Config) {
        use std::time::Instant;
        let changed = !configs_equivalent(&self.last_saved, now);
        if changed {
            self.pending_since = Some(Instant::now());
            self.last_saved = now.clone();
        }
        if let Some(since) = self.pending_since {
            if since.elapsed().as_millis() as u64 >= Self::SAVE_DEBOUNCE_MS {
                if let Err(e) = self.last_saved.save(&self.path) {
                    tracing::warn!("config save failed: {e:#}");
                }
                // Our own write bumps mtime; track it so the hot-reload
                // check below doesn't fire on our own writes.
                self.last_known_mtime = Self::read_mtime(&self.path);
                self.pending_since = None;
            }
        }
    }

    /// Check whether the backing file was modified externally since
    /// our last tick. Returns the reloaded config if so.
    ///
    /// Call once per frame alongside `tick`. The poll itself runs at
    /// most once per [`WATCH_INTERVAL_MS`] to keep the stat() rate
    /// trivial.
    pub fn poll_external_change(&mut self) -> Option<rsoverlay_core::Config> {
        use std::time::Instant;
        if self.last_watch_check.elapsed().as_millis() as u64 <= Self::WATCH_INTERVAL_MS {
            return None;
        }
        self.last_watch_check = Instant::now();
        let current_mtime = Self::read_mtime(&self.path);
        if current_mtime == self.last_known_mtime {
            return None;
        }
        // File changed externally. Reload and adopt as our new
        // "last_saved" so tick() doesn't immediately re-save.
        match rsoverlay_core::Config::load(&self.path) {
            Ok(cfg) => {
                tracing::info!("config.toml changed externally — reloading");
                self.last_saved = cfg.clone();
                self.last_known_mtime = current_mtime;
                self.pending_since = None;
                Some(cfg)
            }
            Err(e) => {
                tracing::warn!("external config.toml reload failed: {e:#}");
                None
            }
        }
    }

    fn read_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
    }

    /// Final flush — write unconditionally. Call on clean shutdown.
    #[allow(dead_code)]
    pub fn flush(&mut self) {
        if let Err(e) = self.last_saved.save(&self.path) {
            tracing::warn!("config final flush failed: {e:#}");
        }
        self.pending_since = None;
    }
}

/// Field-by-field compare of the two Config variants that matter for
/// debouncing. Ignores Render / Notify / Preview defaults the user
/// can't change from inside the UI yet.
/// Structural: every config struct derives `PartialEq`, so a new field
/// can never be silently left out of change detection (the hand-written
/// field list this replaced had to be kept in sync by hand).
fn configs_equivalent(a: &rsoverlay_core::Config, b: &rsoverlay_core::Config) -> bool {
    a == b
}

/// Save the given surface frame to a PNG. BGRA → RGBA swizzle, then
/// `png` crate for a minimal encoder footprint. Returns the number of
/// bytes written so the caller can log it. Called from the frame-tick
/// handlers of both preview modes once they've reached their capture
/// frame.
pub fn save_surface_as_png(
    gpu: &rsoverlay_render::GpuCtx,
    surface_texture: &wgpu::Texture,
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> anyhow::Result<usize> {
    use anyhow::Context;

    // wgpu requires row-pitch to be a multiple of 256 bytes for texture
    // → buffer copies. BGRA8 at 4 bytes/px → row pitch = width * 4
    // rounded up.
    let bpp = 4u32;
    let unpadded = width * bpp;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer_size = (padded as u64) * height as u64;

    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot-staging"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("screenshot-encoder"),
        });
    enc.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: surface_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(Some(enc.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .context("map_async channel")?
        .context("map_async")?;

    let data = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        let row_slice = &data[start..start + unpadded as usize];
        for px in row_slice.as_chunks::<4>().0 {
            // wgpu swapchain is BGRA on Windows → emit as RGBA.
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    drop(data);
    buffer.unmap();

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).context("creating output dir")?;
        }
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("creating PNG at {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("PNG header")?;
    writer.write_image_data(&rgba).context("PNG data")?;
    Ok(rgba.len())
}
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::WindowBuilder,
};

/// Overlay texture resolution.
///
/// Sized to stay crisp at ~1.5-2× texels-per-display-pixel on current retail
/// HMDs for a 1 m-wide overlay at 2 m. The worst case we size for today is
/// Pimax Crystal / Varjo Aero (~1200 display pixels across the quad). 2048
/// gives a ~1.7× sampling ratio and ~16 MB of GPU memory — safe for every
/// shipping headset through late-2026. Power users on Crystal Super / XR-4
/// can override via `[render] overlay_texture_size` in config.toml.
const OVERLAY_TEX_SIZE: u32 = 2048;
// Preferred size: every Home card fits without scrolling. Screenshot runs
// use exactly this (deterministic captures); interactive runs shrink it to
// fit the monitor (see `fit_to_monitor`).
const INITIAL_WIN_W: u32 = 1280;
const INITIAL_WIN_H: u32 = 1600;

/// Window frame allowance (title bar + borders), logical px.
const CHROME: (f64, f64) = (16.0, 40.0);
/// Gap kept between the window and the work-area edges, logical px.
const EDGE_GAP: f64 = 16.0;

/// Size (≤ `want`, inner) and outer position that fit the window, title
/// bar included, inside `work` = (x, y, w, h) and centre it there. Pure.
fn fit_in(want: (f64, f64), work: (f64, f64, f64, f64)) -> ((f64, f64), (f64, f64)) {
    let (x, y, w, h) = work;
    let iw = want.0.min(w - CHROME.0 - 2.0 * EDGE_GAP).max(400.0);
    let ih = want.1.min(h - CHROME.1 - 2.0 * EDGE_GAP).max(300.0);
    let px = x + ((w - iw - CHROME.0) / 2.0).max(0.0);
    let py = y + ((h - ih - CHROME.1) / 2.0).max(0.0);
    ((iw, ih), (px, py))
}

/// Primary monitor's work area (screen minus taskbar), logical px.
fn work_area<T>(target: &winit::event_loop::EventLoopWindowTarget<T>) -> Option<(f64, f64, f64, f64)> {
    let m = target.primary_monitor().or_else(|| target.available_monitors().next())?;
    let scale = m.scale_factor();
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
        let mut r = RECT::default();
        // SAFETY: SPI_GETWORKAREA writes one RECT into the pointer we pass,
        // which points at a live, correctly sized RECT on our stack.
        let ok = unsafe {
            SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut r as *mut RECT as *mut std::ffi::c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
        };
        if ok.is_ok() && r.right > r.left && r.bottom > r.top {
            // Physical px (the process is per-monitor DPI aware).
            return Some((r.left as f64 / scale, r.top as f64 / scale, (r.right - r.left) as f64 / scale, (r.bottom - r.top) as f64 / scale));
        }
    }
    let p = m.position().to_logical::<f64>(scale);
    let s = m.size().to_logical::<f64>(scale);
    // No work-area API: assume a ~6 % taskbar along the bottom.
    Some((p.x, p.y, s.width, s.height * 0.94))
}

/// Inner size + outer position so the window fits the primary monitor's
/// work area (never under the taskbar), centred. `None` position = let
/// the OS place it (monitor unknown).
pub fn fit_to_monitor<T>(
    target: &winit::event_loop::EventLoopWindowTarget<T>,
    want: (f64, f64),
) -> (winit::dpi::LogicalSize<f64>, Option<winit::dpi::LogicalPosition<f64>>) {
    match work_area(target) {
        Some(work) => {
            let ((w, h), (x, y)) = fit_in(want, work);
            (winit::dpi::LogicalSize::new(w, h), Some(winit::dpi::LogicalPosition::new(x, y)))
        }
        None => (winit::dpi::LogicalSize::new(want.0, want.1), None),
    }
}

pub fn run(
    mut notify: Option<NotifyChannel>,
    capture_cfg: CaptureCfg,
    osc: Option<rsoverlay_osc::OscClient>,
    test: TestHooks,
    app_config: rsoverlay_core::Config,
    config_path: std::path::PathBuf,
) -> Result<()> {
    let event_loop = EventLoop::new().context("creating winit event loop")?;
    let want = (f64::from(INITIAL_WIN_W), f64::from(INITIAL_WIN_H));
    let (size, pos) = if test.screenshot.is_some() { (winit::dpi::LogicalSize::new(want.0, want.1), None) } else { fit_to_monitor(&event_loop, want) };
    let mut builder = WindowBuilder::new();
    if let Some(p) = pos {
        builder = builder.with_position(p);
    }
    let window = Arc::new(
        builder
            .with_title("RSOverlay")
            .with_inner_size(size)
            .with_min_inner_size(winit::dpi::LogicalSize::new(900u32, 600u32))
            .build(&event_loop)
            .context("creating preview window")?,
    );

    let gpu = pollster::block_on(GpuCtx::new(RuntimeMode::Preview))?;

    // Surface on the winit window.
    let surface = gpu
        .instance
        .create_surface(window.clone())
        .context("creating wgpu surface for preview window")?;
    let size = window.inner_size();
    let caps = surface.get_capabilities(&gpu.adapter);
    let surface_format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        format: surface_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&gpu.device, &config);

    // In preview mode we size the overlay texture to match the window
    // surface so blit is 1:1 — this means mouse clicks and egui's
    // rendered widgets agree in coordinates (no non-uniform stretch).
    // In VR mode a separate code path pins the overlay to the
    // HMD-tuned OVERLAY_TEX_SIZE; the preview just needs
    // input/render consistency.
    let mut overlay_target =
        OverlayTarget::new(&gpu, size.width.max(1), size.height.max(1));
    let _ = OVERLAY_TEX_SIZE;

    // Blit pipeline: fullscreen triangle samples the overlay texture.
    let mut blit = BlitPipeline::new(&gpu.device, &overlay_target, surface_format);

    // egui panel drawing into the overlay target.
    let mut egui_panel = EguiPanel::new(
        &gpu.device,
        overlay_target.format,
        overlay_target.width,
        overlay_target.height,
    );
    // native_pixels_per_point = None so egui-winit reads from
    // ctx.pixels_per_point() each frame. Combined with overlay_target
    // tracking window size 1:1 this keeps mouse hit-testing aligned
    // with egui's rendered widget positions.
    let mut egui_winit = egui_winit::State::new(
        egui_panel.ctx.clone(),
        egui::ViewportId::ROOT,
        &*window,
        None,
        None,
    );
    let mut dev_view = DevView::default();
    seed_dev_view(&mut dev_view, &app_config, &test);
    // Show saved VR layouts (read-only here; saving/loading happens in VR).
    dev_view.layouts = rsoverlay_core::layouts::Layouts::load(&rsoverlay_core::layouts::Layouts::path());
    let mut persister = ConfigPersister::new(config_path.clone(), app_config.clone());
    let mut thumbs = crate::mirror_source::ThumbWorker::spawn();
    let config_base = app_config.clone();
    let mut fps_counter = FpsCounter::new();
    let mut frame_counter: u64 = 0;
    let screenshot_path = test.screenshot.clone();
    let exit_after = test.exit_after;

    // Capture pipeline. We try the GPU-to-GPU shared-handle path first
    // (see `rsoverlay-capture::shared` + `rsoverlay-render::shared`). If
    // anything along that path fails — or the user forced CPU mode —
    // we fall back to the original CPU readback through an mpsc channel.
    let (capture_tx, capture_rx) = mpsc::sync_channel::<Frame>(2);
    // `None` once the CPU path is abandoned (runtime source switch);
    // dropping the receiver makes the CPU capture thread exit.
    let mut capture_rx = Some(capture_rx);
    let mut mirror_texture: Option<MirrorTexture> = None;
    let mut mirror_handle: Option<MirrorHandle> = None;
    let mut mirror_geom: Option<MonitorGeom> = None;
    let mut gpu_mirror: Option<GpuMirror> = None;

    if capture_cfg.enabled {
        if let Ok(mons) = enumerate_monitors() {
            if let Some(m) = mons.into_iter().find(|m| m.index == capture_cfg.monitor_index) {
                mirror_geom = Some(MonitorGeom {
                    origin_x: m.origin_x,
                    origin_y: m.origin_y,
                    width: m.width,
                    height: m.height,
                });
            }
        }

        let window_source = capture_cfg.window.as_deref().and_then(|needle| {
            match rsoverlay_capture::find_window(needle) {
                Ok(Some(w)) => Some(CaptureSource::Window { hwnd: w.hwnd, title: w.title }),
                Ok(None) => {
                    warn!("--window {needle:?}: no matching window; mirroring monitor instead");
                    None
                }
                Err(e) => {
                    warn!("window enumeration failed: {e:#}");
                    None
                }
            }
        });
        let initial = window_source.unwrap_or(CaptureSource::Monitor(capture_cfg.monitor_index));

        if !capture_cfg.force_cpu {
            match GpuMirror::start(&gpu, &mut egui_panel, initial) {
                Ok(m) => {
                    dev_view.sources.current = Some(m.source.clone());
                    gpu_mirror = Some(m);
                }
                Err(e) => warn!("GPU-to-GPU capture unavailable, falling back to CPU: {e:#}"),
            }
        }

        if gpu_mirror.is_none() {
            let tx = capture_tx;
            let idx = capture_cfg.monitor_index;
            dev_view.sources.current = Some(CaptureSource::Monitor(idx));
            std::thread::Builder::new()
                .name("rsoverlay-capture".into())
                .spawn(move || capture_loop(idx, tx))
                .context("spawning CPU capture thread")?;
        }
    }
    let injector = InputInjector::new().context("initializing input injector")?;
    let mut vr_keyboard = rsoverlay_ui::vrkeyboard::VrKeyboard::default();
    // Every VR surface, switchable live (dev bar), at real VR sizes.
    let mut harness = crate::preview_dev::DevHarness::new(&test);

    info!("preview window up — close window or press Esc to exit");

    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { event, window_id } if window_id == window.id() => {
                // Forward to egui first so the UI gets input.
                let _ = egui_winit.on_window_event(&window, &event);
                match event {
                    WindowEvent::CloseRequested => elwt.exit(),
                    WindowEvent::KeyboardInput { event, .. } => {
                        if event.state == winit::event::ElementState::Pressed
                            && event.logical_key == Key::Named(NamedKey::Escape)
                        {
                            elwt.exit();
                        }
                    }
                    WindowEvent::Resized(new_size) => {
                        config.width = new_size.width.max(1);
                        config.height = new_size.height.max(1);
                        surface.configure(&gpu.device, &config);
                        // Rebuild the overlay target + blit pipeline so
                        // the overlay texture stays 1:1 with the window
                        // surface — mouse coordinates flow straight
                        // through without a non-uniform stretch.
                        overlay_target =
                            OverlayTarget::new(&gpu, config.width, config.height);
                        blit = BlitPipeline::new(
                            &gpu.device,
                            &overlay_target,
                            surface_format,
                        );
                        egui_panel.resize(config.width, config.height);
                    }
                    WindowEvent::RedrawRequested => {
                        let frame_start = Instant::now();
                        // 0a. Drain any newly-arrived notifications.
                        if let Some(n) = notify.as_mut() {
                            while let Ok(notif) = n.rx.try_recv() {
                                dev_view.push_notification(notif);
                            }
                        }
                        play_alert_sounds(&mut dev_view);

                        // 0b. CPU path only: drain the latest captured frame
                        // (coalesce — only the newest matters) and upload it.
                        // On the GPU path the shared texture is already
                        // kept current by the capture thread; nothing to do
                        // here except acquire a read lease below.
                        if frame_counter == 30 {
                            if let Some(needle) = test.switch_to_window.as_deref() {
                                match rsoverlay_capture::find_window(needle) {
                                    Ok(Some(w)) => {
                                        dev_view.sources.request =
                                            Some(CaptureSource::Window { hwnd: w.hwnd, title: w.title });
                                    }
                                    _ => warn!("--switch-to-window {needle:?}: no match"),
                                }
                            }
                        }

                        // 0a'. Source picker: refresh choices while the
                        // dropdown is open, swap pipelines on request.
                        if std::mem::take(&mut dev_view.sources.wants_refresh) {
                            dev_view.sources.choices = list_sources();
                            fetch_icons(&dev_view.sources.choices, &mut dev_view.sources.icons);
                            thumbs.request(&dev_view.sources.choices);
                        }
                        thumbs.collect(&mut dev_view.sources);
                        if let Some(req) = dev_view.sources.request.take() {
                            // Tear down first: DXGI allows one duplication
                            // per output per process.
                            let previous = gpu_mirror.take().map(|m| {
                                let src = m.source.clone();
                                m.shutdown(&mut egui_panel);
                                src
                            });
                            match GpuMirror::start(&gpu, &mut egui_panel, req) {
                                Ok(m) => {
                                    capture_rx = None;
                                    mirror_texture = None;
                                    dev_view.sources.current = Some(m.source.clone());
                                    gpu_mirror = Some(m);
                                }
                                Err(e) => {
                                    warn!("switching mirror source failed: {e:#}");
                                    gpu_mirror = previous
                                        .and_then(|src| GpuMirror::start(&gpu, &mut egui_panel, src).ok());
                                }
                            }
                        }

                        if let Some(rx) = capture_rx.as_ref().filter(|_| gpu_mirror.is_none()) {
                            let mut latest: Option<Frame> = None;
                            while let Ok(f) = rx.try_recv() {
                                latest = Some(f);
                            }
                            if let Some(frame) = latest {
                                let (mt, needs_register) = match mirror_texture.as_ref() {
                                    Some(mt) if mt.width == frame.width && mt.height == frame.height => {
                                        (mt, false)
                                    }
                                    _ => {
                                        let mt = MirrorTexture::new(&gpu, frame.width, frame.height);
                                        mirror_texture = Some(mt);
                                        (mirror_texture.as_ref().unwrap(), true)
                                    }
                                };
                                if let Err(e) = mt.upload(&gpu, &frame) {
                                    warn!("mirror upload: {e:#}");
                                }
                                if needs_register {
                                    let id = egui_panel.renderer.register_native_texture(
                                        &gpu.device,
                                        &mt.view,
                                        wgpu::FilterMode::Linear,
                                    );
                                    mirror_handle =
                                        Some(MirrorHandle::full(id, frame.width, frame.height));
                                }
                            }
                        }
                        dev_view.mirror = match &gpu_mirror {
                            Some(m) => Some(m.handle()),
                            None => mirror_handle,
                        };

                        // Acquire the keyed-mutex read lease on the GPU
                        // path for the duration of this frame. Dropped at
                        // the end of RedrawRequested, handing write
                        // ownership back to the capture thread.
                        let _read_lease = gpu_mirror.as_ref().and_then(|m| m.acquire_read(8));

                        // 0c. Forward any pending mirror hit to the real
                        // desktop. Move always; click only if a click was
                        // actually recorded this frame.
                        let geom = match &gpu_mirror {
                            Some(m) => m.geom(),
                            None => mirror_geom,
                        };
                        if let (Some(hit), Some(geom)) = (dev_view.take_mirror_hit(), geom) {
                            if hit.clicked {
                                // Raise a mirrored window first so the click
                                // lands on it, not whatever overlaps it.
                                if let Some(hwnd) = gpu_mirror.as_ref().and_then(|m| m.window_hwnd()) {
                                    rsoverlay_capture::window::focus_window(hwnd);
                                }
                                if let Err(e) = injector.click_at(&geom, hit.x, hit.y, MouseButton::Left) {
                                    warn!("input inject click: {e:#}");
                                }
                            } else if let Err(e) = injector.move_to(&geom, hit.x, hit.y) {
                                warn!("input inject move: {e:#}");
                            }
                        }

                        // 0d'. Avatar parameters (battery -> VRChat).
                        for (name, value) in dev_view.take_pending_osc() {
                            if let Some(client) = &osc {
                                if let Err(e) = client.send_param(name, value) {
                                    warn!("osc: {e:#}");
                                }
                            }
                        }

                        // 0d. Forward any pending chatbox send.
                        if let Some(msg) = dev_view.take_pending_chat() {
                            match &osc {
                                Some(client) => {
                                    if let Err(e) = client.send_chat(msg) {
                                        warn!("osc: {e:#}");
                                    }
                                }
                                None => warn!("chatbox send ignored: OSC disabled via --no-osc"),
                            }
                        }

                        // 1. Run egui into the overlay target.
                        dev_view.frame = dev_view.frame.wrapping_add(1);
                        dev_view.last_fps = fps_counter.tick();
                        let raw_input = egui_winit.take_egui_input(&window);
                        egui_panel.render(
                            &gpu.device,
                            &gpu.queue,
                            &overlay_target.view,
                            raw_input,
                            |ctx| harness.show(ctx, &mut dev_view, &mut vr_keyboard, &injector),
                        );

                        // 2. Blit the overlay target into the window.
                        let frame = match surface.get_current_texture() {
                            Ok(f) => f,
                            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                                surface.configure(&gpu.device, &config);
                                return;
                            }
                            Err(e) => {
                                warn!("surface error: {e:?}");
                                return;
                            }
                        };
                        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("preview-blit-encoder"),
                        });
                        blit.set_tint(&gpu.queue, dev_view.overlay_tint());
                        blit.record(&mut enc, &view);
                        gpu.queue.submit(Some(enc.finish()));

                        // Snap the OverlayTarget (what VR would receive)
                        // before presenting, so the PNG matches what a
                        // headset would actually render.
                        frame_counter = frame_counter.wrapping_add(1);
                        if let Some(trigger) = exit_after {
                            if frame_counter == trigger {
                                if let Some(path) = &screenshot_path {
                                    match save_surface_as_png(
                                        &gpu,
                                        &overlay_target.texture,
                                        overlay_target.width,
                                        overlay_target.height,
                                        path,
                                    ) {
                                        Ok(n) => info!(
                                            "wrote screenshot {} ({} bytes)",
                                            path.display(),
                                            n
                                        ),
                                        Err(e) => warn!("screenshot failed: {e:#}"),
                                    }
                                }
                                info!("--exit-after {trigger} reached; clean exit");
                                elwt.exit();
                            }
                        }

                        frame.present();
                        dev_view.record_frame_ms(frame_start.elapsed().as_secs_f32() * 1000.0);

                        // "Reset defaults" clicked: snap DevView back to
                        // pristine state. Persistence tick on the next
                        // line notices the drift and writes that out.
                        if dev_view.reset_requested {
                            let base = rsoverlay_core::Config::default();
                            seed_dev_view(&mut dev_view, &base, &TestHooks::default());
                            dev_view.reset_requested = false;
                        }
                        if std::mem::take(&mut dev_view.restart_elevated_requested) {
                            persister.tick(&config_from_dev_view(&dev_view, &config_base));
                            persister.flush();
                            let args: Vec<String> = std::env::args().skip(1).collect();
                            if rsoverlay_input::elevation::relaunch_elevated(&args) {
                                elwt.exit();
                            } else {
                                warn!("elevated relaunch cancelled or failed");
                            }
                        }

                        // Pick up external edits (user opened
                        // config.toml in a text editor and saved).
                        if let Some(cfg) = persister.poll_external_change() {
                            seed_dev_view(&mut dev_view, &cfg, &TestHooks::default());
                        }

                        // Persist state changes (debounced inside the
                        // persister). Slider/toggle writes land on disk
                        // ~400ms after the last change.
                        let current = config_from_dev_view(&dev_view, &config_base);
                        persister.tick(&current);
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            // Closing within the save debounce must not lose the change.
            Event::LoopExiting => {
                persister.tick(&config_from_dev_view(&dev_view, &config_base));
                persister.flush();
            }
            _ => {}
        }
    })?;
    Ok(())
}

/// DXGI duplication pump. Runs on its own thread, publishes the newest frame
/// every ~16 ms. On disconnection of the channel it exits cleanly. On any
/// recoverable duplication error (e.g. output switch) it retries after a
/// brief sleep.
fn capture_loop(monitor_index: u32, tx: mpsc::SyncSender<Frame>) {
    loop {
        let mut dup = match DxgiDuplication::new(monitor_index) {
            Ok(d) => d,
            Err(e) => {
                error!("desktop capture setup failed for monitor {monitor_index}: {e:#}");
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
        };
        info!("desktop capture started on monitor {monitor_index}");
        loop {
            match dup.next_frame(16) {
                Ok(Some(frame)) => {
                    // try_send: drop the frame if UI is behind, keep latency low.
                    let _ = tx.try_send(frame);
                }
                Ok(None) => {} // timeout; no new frame this tick
                Err(e) => {
                    warn!("capture lost ({e:#}); retrying");
                    break;
                }
            }
        }
    }
}

/// Rolling FPS counter — one sample per second.
pub(crate) struct FpsCounter {
    last_sample: Instant,
    frames: u32,
    cached: f32,
}

impl FpsCounter {
    pub(crate) fn new() -> Self {
        Self { last_sample: Instant::now(), frames: 0, cached: 0.0 }
    }
    pub(crate) fn tick(&mut self) -> f32 {
        self.frames += 1;
        let elapsed = self.last_sample.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.cached = self.frames as f32 / elapsed;
            self.frames = 0;
            self.last_sample = Instant::now();
        }
        self.cached
    }
}

/// Fullscreen-triangle blit of a sampled texture onto the swapchain view.
struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    /// vec4 colour multiplier (night light). Written every frame.
    tint_buf: wgpu::Buffer,
}

impl BlitPipeline {
    fn new(device: &wgpu::Device, source: &OverlayTarget, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let tint_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blit-tint"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: tint_buf.as_entire_binding() },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit-pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        Self { pipeline, bind_group, tint_buf }
    }

    fn set_tint(&self, queue: &wgpu::Queue, tint: [f32; 4]) {
        queue.write_buffer(&self.tint_buf, 0, bytemuck::cast_slice(&tint));
    }

    fn record(&self, enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod window_fit_tests {
    use super::fit_in;

    /// Outer bottom (inner top + height + chrome) must stay inside the work area.
    fn fits(want: (f64, f64), work: (f64, f64, f64, f64)) {
        let ((w, h), (x, y)) = fit_in(want, work);
        assert!(x >= work.0 && y >= work.1, "starts inside: {x},{y}");
        assert!(x + w + super::CHROME.0 <= work.0 + work.2 + 0.01, "right edge");
        assert!(y + h + super::CHROME.1 <= work.1 + work.3 + 0.01, "bottom edge (taskbar)");
    }

    #[test]
    fn fits_1440p_with_bottom_taskbar() {
        // 2560 × 1440 at 100 %, 48 px taskbar: the reported work area.
        fits((1280.0, 1600.0), (0.0, 0.0, 2560.0, 1392.0));
        let ((w, h), (_, y)) = fit_in((1280.0, 1600.0), (0.0, 0.0, 2560.0, 1392.0));
        assert_eq!(w, 1280.0, "width untouched when it fits");
        assert!(h < 1392.0 && y > 0.0);
    }

    #[test]
    fn fits_scaled_small_and_odd_taskbars() {
        fits((1280.0, 1600.0), (0.0, 0.0, 1707.0, 928.0)); // 1440p at 150 %
        fits((1280.0, 1600.0), (0.0, 0.0, 1280.0, 680.0)); // 1080p at 150 %
        fits((1280.0, 1600.0), (0.0, 48.0, 2560.0, 1392.0)); // taskbar on top
        fits((1280.0, 1600.0), (60.0, 0.0, 2500.0, 1440.0)); // taskbar on the left
    }

    #[test]
    fn small_windows_are_centred_not_moved_to_the_corner() {
        let ((w, h), (x, y)) = fit_in((800.0, 600.0), (0.0, 0.0, 2560.0, 1392.0));
        assert_eq!((w, h), (800.0, 600.0));
        assert!((x - (2560.0 - 816.0) / 2.0).abs() < 0.01 && (y - (1392.0 - 640.0) / 2.0).abs() < 0.01);
    }
}

#[cfg(test)]
mod config_roundtrip_tests {
    use super::*;
    use rsoverlay_core::config::{Macro, Macros, VrState};

    /// A config with (nearly) every user-facing value moved off its default.
    fn everything_changed() -> rsoverlay_core::Config {
        let mut c = rsoverlay_core::Config::default();
        let a = &mut c.appearance;
        a.accent_hex = "#12AB34".into();
        a.show_mirror = !a.show_mirror;
        a.show_media = !a.show_media;
        a.show_chat = !a.show_chat;
        a.show_keyboard = !a.show_keyboard;
        a.show_history = !a.show_history;
        a.forward_input = !a.forward_input;
        a.chat_now_playing = !a.chat_now_playing;
        a.show_battery = !a.show_battery;
        a.show_macros = !a.show_macros;
        a.show_volume = !a.show_volume;
        a.show_system = !a.show_system;
        a.reduce_motion = !a.reduce_motion;
        a.battery_warn_pct = 33;
        a.battery_sound = false;
        a.discord_presence = false;
        a.steamvr_icons = false;
        c.discord = rsoverlay_core::config::DiscordCfg { show_windows: false, show_battery: false, show_music: true, show_button: false };
        let p = &mut c.placement;
        p.anchor = rsoverlay_core::PlacementAnchor::World;
        p.offset_x = 0.5;
        p.offset_y = 1.1;
        p.offset_z = -0.9;
        p.size_w = 0.5;
        p.size_h = 0.3;
        p.curvature = 0.25;
        p.opacity = 0.6;
        c.night_light.mode = "on".into();
        c.night_light.strength = 0.8;
        c.osc.battery_params = true;
        c.osc.battery_prefix = "My/Prefix".into();
        let v = &mut c.vr;
        v.left_handed = true;
        v.clip_angle = 45.0;
        v.wrist_opacity = 0.5;
        v.run_as_admin = true;
        v.ui_scale = 1.25;
        v.show_lasers = false;
        v.laser_angle = -10.0;
        v.cursor_smoothing = 0.25;
        v.scroll_speed = 2.0;
        v.wrist_buttons = 0x15;
        v.media_theme = true;
        v.wrist_expanded = true;
        v.wrist_compact = 0x1_0105;
        v.keyboard_opacity = 0.7;
        v.typing_sounds = false;
        v.typing_volume = 0.9;
        v.sound_pack = "Thocky".into();
        v.long_press = true;
        v.long_press_s = 0.9;
        v.double_click_s = 0.2;
        v.restore_session = false;
        v.hide_wrist = true;
        v.hide_with_steamvr_dashboard = false;
        v.windows_follow_dashboard = false;
        v.grab_tilt = true;
        v.stick_axis_lock = false;
        v.pointer_size = 1.5;
        v.render_scale = 2.0;
        c.vr_state = VrState {
            wrist_pose: Some([[1.0, 0.0, 0.0, 0.1], [0.0, 1.0, 0.0, 0.2], [0.0, 0.0, 1.0, 0.3]]),
            wrist_width: Some(0.3),
            kb_pinned: true,
            kb_auto_repeat: true,
            kb_full: true,
            kb_vrchat: true,
        };
        c.notifications = rsoverlay_core::config::NotificationsCfg {
            size_m: 0.4,
            offset_y: 0.1,
            distance_m: 1.0,
            volume: 0.3,
            sound_notification: "ping.wav".into(),
            sound_warning: "none".into(),
            sound_error: "buzz.wav".into(),
            sound_low_battery: "none".into(),
        };
        c.macros = Macros(vec![Macro { label: "Mute".into(), keys: "ctrl+m".into() }]);
        c
    }

    /// Every setting must survive config → dashboard state → config.
    /// A setting that isn't wired through DevView would come back as its
    /// default here — and be silently reset by the next autosave.
    #[test]
    fn every_setting_survives_the_dashboard_round_trip() {
        let cfg = everything_changed();
        let mut dev = rsoverlay_ui::views::DevView::default();
        seed_dev_view(&mut dev, &cfg, &TestHooks::default());
        let back = config_from_dev_view(&dev, &cfg);
        assert_eq!(back, cfg);
    }

    #[test]
    fn change_detection_is_structural() {
        let a = everything_changed();
        let mut b = a.clone();
        assert!(configs_equivalent(&a, &b));
        b.vr_state.kb_full = false;
        assert!(!configs_equivalent(&a, &b));
    }

    #[test]
    fn config_written_by_an_older_build_still_loads() {
        let old = "[vr]\nleft_handed = true\n";
        let c: rsoverlay_core::Config = toml::from_str(old).unwrap();
        assert!(c.vr.left_handed);
        assert_eq!(c.vr_state, VrState::default());
        assert!(c.vr.restore_session);
        assert_eq!(c.vr.wrist_compact, rsoverlay_core::config::DEFAULT_WRIST_COMPACT);
    }
}
