//! Built-in egui views used by the default panel set.
//!
//! Visual language: Terminal Brutalist chassis + Spatial Glass-Lite cards.
//! Tokens live in `theme::*`; no hardcoded colors in this module.

use crate::battery::{BatteryReading, LowBatteryWatch};
use crate::icons::{self, icon};
use crate::media::{self, MediaCommand, MediaInfo};
use crate::now_playing::NowPlayingBridge;
use crate::theme;
use rsoverlay_notify::Notification;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Handle to a wgpu-backed egui texture plus its intrinsic size.
#[derive(Copy, Clone, Debug)]
pub struct MirrorHandle {
    pub id: egui::TextureId,
    /// Live content size in source pixels (window or monitor). Hit
    /// coordinates are reported in this space.
    pub width: u32,
    pub height: u32,
    /// Sub-rect of the texture holding the content. Window capture
    /// writes into the top-left of an oversized shared texture.
    pub uv: egui::Rect,
    /// OS cursor position within the content (0..1 each axis), for
    /// sources whose capture leaves the cursor out (monitor duplication).
    pub cursor: Option<egui::Vec2>,
    /// A real window whose size we may change (not a monitor).
    pub resizable: bool,
}

impl MirrorHandle {
    pub fn full(id: egui::TextureId, width: u32, height: u32) -> Self {
        Self { id, width, height, uv: egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), cursor: None, resizable: false }
    }
}

/// Draw the OS cursor over a mirrored image occupying `rect`.
pub fn paint_mirror_cursor(ui: &egui::Ui, rect: egui::Rect, cursor: Option<egui::Vec2>) {
    if let Some(c) = cursor {
        let tip = rect.min + egui::vec2(c.x * rect.width(), c.y * rect.height());
        crate::glyphs::draw_pointer(&ui.painter_at(rect), tip, 20.0);
    }
}

/// What the Desktop card mirrors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    Monitor(u32),
    /// `hwnd` is the raw Win32 handle; `title` is for display only.
    Window { hwnd: isize, title: String },
}

impl CaptureSource {
    pub fn label(&self) -> String {
        match self {
            Self::Monitor(i) => format!("Monitor {i}"),
            Self::Window { title, .. } => title.clone(),
        }
    }
}

/// Picker state shared between the Desktop card and the app's capture
/// pipeline. The UI only *requests*; the app owns the pipeline swap.
#[derive(Default)]
pub struct SourcePicker {
    /// Currently mirrored source (set by the app).
    pub current: Option<CaptureSource>,
    /// Choices shown in the dropdown (refreshed by the app on request).
    pub choices: Vec<CaptureSource>,
    /// Set by the UI while the dropdown is open; the app refreshes
    /// `choices` (EnumWindows) and clears it.
    pub wants_refresh: bool,
    /// Set by the UI when the user picks a different source.
    pub request: Option<CaptureSource>,
    /// "All monitors": open every monitor as a wall arced around you.
    pub wall_request: bool,
    /// "Big screen": the main monitor as one large curved screen.
    pub big_screen_request: bool,
    /// Program of each listed window ("Discord.exe"), set by the app.
    pub apps: std::collections::HashMap<isize, String>,
    /// Programs hidden from the picker (persisted).
    pub hidden_apps: Vec<String>,
    /// "Show hidden" toggle on the Desktop page.
    pub show_hidden: bool,
    /// Desktop-page filter text.
    pub filter: String,
    /// App icons per window (raw pixels from the app; textures are made
    /// lazily per egui context).
    pub icons: std::collections::HashMap<isize, egui::ColorImage>,
    icon_tex: std::collections::HashMap<isize, egui::TextureHandle>,
    /// Window thumbnails (generation, image) from the app's worker.
    thumbs: std::collections::HashMap<isize, (u64, egui::ColorImage)>,
    thumb_tex: std::collections::HashMap<isize, (u64, egui::TextureHandle)>,
}

impl SourcePicker {
    /// Replace the thumbnails with a finished batch (`generation` bumps
    /// each batch so textures re-upload only when the picture changed).
    pub fn set_thumbnails(&mut self, generation: u64, thumbs: &std::collections::HashMap<isize, egui::ColorImage>) {
        self.thumbs = thumbs.iter().map(|(h, img)| (*h, (generation, img.clone()))).collect();
        self.thumb_tex.retain(|h, _| thumbs.contains_key(h));
    }

    /// Texture + pixel size of a window's thumbnail, uploaded / updated
    /// lazily per egui context.
    pub fn thumb_texture(&mut self, ctx: &egui::Context, hwnd: isize) -> Option<(egui::TextureId, egui::Vec2)> {
        let (generation, img) = self.thumbs.get(&hwnd)?;
        let size = egui::vec2(img.size[0] as f32, img.size[1] as f32);
        match self.thumb_tex.get_mut(&hwnd) {
            Some((g, t)) if g == generation => return Some((t.id(), size)),
            Some((g, t)) => {
                t.set(img.clone(), egui::TextureOptions::LINEAR);
                *g = *generation;
                return Some((t.id(), size));
            }
            None => {}
        }
        let t = ctx.load_texture(format!("winthumb-{hwnd}"), img.clone(), egui::TextureOptions::LINEAR);
        let id = t.id();
        self.thumb_tex.insert(hwnd, (*generation, t));
        Some((id, size))
    }

    /// Texture for a window's app icon, uploaded on first use.
    pub fn icon_texture(&mut self, ctx: &egui::Context, hwnd: isize) -> Option<egui::TextureId> {
        if let Some(t) = self.icon_tex.get(&hwnd) {
            return Some(t.id());
        }
        let img = self.icons.get(&hwnd)?.clone();
        let t = ctx.load_texture(format!("winicon-{hwnd}"), img, egui::TextureOptions::LINEAR);
        let id = t.id();
        self.icon_tex.insert(hwnd, t);
        Some(id)
    }
}

struct Toast {
    n: Notification,
    expires: Instant,
    /// Stable id for per-toast animation state.
    seq: u64,
    /// egui time the toast arrived (slide-in start).
    born: Option<f64>,
    /// Last measured height, for stacking the next toast below it.
    height: f32,
}

/// Slide distance for toast enter/exit, px.
const TOAST_SLIDE: f32 = 320.0;
/// Toasts start sliding out this long before they expire.
const TOAST_EXIT: Duration = Duration::from_millis(220);

/// One entry in the scrollable notification history. Tracks the arrival
/// instant so we can render a relative "5m ago" timestamp rather than
/// clock time, which is more actionable in VR where you'd otherwise
/// have to subtract.
#[derive(Clone)]
struct HistoryEntry {
    n: Notification,
    received: Instant,
}

/// Mouse interaction reported by the mirror this frame. Coordinates are in
/// the monitor's pixel space (origin top-left of the monitor).
#[derive(Copy, Clone, Debug)]
pub struct MirrorHit {
    pub x: i32,
    pub y: i32,
    pub clicked: bool,
}

/// Where the overlay is anchored in space. XSOverlay-style spatial
/// states, so users migrating from XSO have the same mental model.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Follows the non-dominant hand controller.
    Wrist,
    /// Follows the HMD — always in view.
    Hmd,
    /// Pinned to a fixed world-space position. Doesn't move with
    /// the body or head.
    World,
}

impl Anchor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wrist => "Wrist",
            Self::Hmd => "HMD-locked",
            Self::World => "World-pinned",
        }
    }
}

/// Placement state the VR preview + eventual real-VR submission pipe
/// both read from. Offset coordinates are in metres, local to whatever
/// the current anchor is. This mirrors XSO's "Edit → Position" workflow:
/// pick an anchor, tweak offset, grab-to-undock (future milestone).
#[derive(Copy, Clone, Debug)]
pub struct Placement {
    pub anchor: Anchor,
    pub offset_x: f32,
    pub offset_y: f32,
    pub offset_z: f32,
    pub size_w: f32,
    pub size_h: f32,
    /// 0.0 = flat, 1.0 = fully wrapped around a cylinder of the quad's
    /// width. Bends the panel toward the viewer so edges are closer
    /// than the center — XSO's "curved overlay" option.
    pub curvature: f32,
    /// Compositor opacity, 0.2..=1.0.
    pub opacity: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            anchor: Anchor::Wrist,
            // Natural wrist-glance offset, same numbers the VR preview
            // used to hardcode. Tweakable by the user now.
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

pub struct DevView {
    pub frame: u64,
    pub last_fps: f32,
    /// CPU time to build + render every panel this frame (ms), and a
    /// short history for the sparkline. Set by the app.
    pub frame_ms: f32,
    frame_ms_hist: VecDeque<f32>,
    pub mirror: Option<MirrorHandle>,
    pub sources: SourcePicker,
    /// Sources currently open as floating VR window overlays (set by the
    /// app) — highlighted in the VR Desktop grid.
    pub open_windows: Vec<CaptureSource>,
    sources_refreshed_at: f64,
    pub forward_input: bool,
    pub accent: egui::Color32,
    pub show_mirror: bool,
    pub show_media: bool,
    pub show_chat: bool,
    pub show_keyboard: bool,
    /// VR dashboard docking, shown in its bottom bar like every other
    /// floating panel; the VR shell owns the pose and reports back here.
    pub dash_attach: crate::window_panel::Attach,
    /// Requests from that bar, taken by the VR shell.
    pub dash_dock: Option<crate::window_panel::Attach>,
    pub dash_close: bool,
    /// Wrist shows everything (layouts, devices, views) vs the compact set.
    pub wrist_expanded: bool,
    /// Wrist edit mode (tap items to show / hide them on the compact wrist).
    pub wrist_editing: bool,
    pub show_history: bool,
    pub show_battery: bool,
    pub battery_warn_pct: u8,
    /// Device labels hidden from battery displays and alerts.
    pub battery_hidden: Vec<String>,
    /// Chime with the low-battery toast.
    pub battery_sound: bool,
    /// Alert sounds due, (kind, per-message volume); the app plays them.
    pending_sounds: Vec<(rsoverlay_audio::alerts::AlertKind, f32)>,
    /// VR toast placement + alert sound choices (Settings › Notifications).
    pub notif: NotificationPrefs,
    /// Toasts are drawn on their own panel (VR), not over the dashboard.
    pub toasts_external: bool,
    /// Settings › Discord: show RSOverlay on the Discord profile.
    pub discord_presence: bool,
    /// Wrist background image (Settings › Wrist toolbar).
    pub wrist_bg: crate::backgrounds::WristBackground,
    /// Gamepad shortcuts (Settings › Gamepad).
    pub gamepad: rsoverlay_core::config::GamepadCfg,
    /// Buttons held on any gamepad right now (set by the VR app).
    pub gamepad_held: u16,
    /// "Set" pressed for shortcut i: buttons collected so far.
    gamepad_capture: Option<(usize, u16)>,
    /// Focus mode (wrist): notifications go to history without a toast
    /// or sound, and windows you aren't looking at dim. Session only.
    pub focus_mode: bool,
    /// Windows up without the dashboard (wrist Windows button; the VR
    /// shell resets it when the follow-dashboard setting changes).
    pub windows_up: bool,
    /// The keyboard was open when the Windows button hid everything.
    pub kb_hidden_with_windows: bool,
    /// Screenshots / previews only: battery-life estimates to show (the
    /// real ones need minutes of history).
    pub demo_battery_estimates: Option<Vec<Option<String>>>,
    /// The dashboard is up (VR; lights the wrist Dashboard button).
    pub dashboard_open: bool,
    /// Update notifier (Settings > Updates). The app enables it on
    /// GitHub builds; Steam builds and tests leave it off.
    pub updates: crate::updates::Updates,
    /// SteamVR frame stats (VR shell only; `None` on the desktop).
    pub vr_perf: Option<rsoverlay_core::perf::VrPerf>,
    /// Battery card: device icons from the user's SteamVR install.
    pub steamvr_icons: bool,
    /// What the presence shares.
    pub discord: rsoverlay_core::config::DiscordCfg,
    /// Settings › VR: hide the wrist toolbar.
    pub vr_hide_wrist: bool,
    pub show_macros: bool,
    pub macros: crate::macros::MacroDeck,
    pub show_volume: bool,
    volume: crate::volume::VolumeCard,
    pub show_system: bool,
    system: crate::system::SystemCard,
    /// Current dashboard page (nav rail selection).
    pub page: crate::shell::Page,
    /// Notifications received since the Notifications page was last
    /// open — drives the rail badge and the header bell highlight.
    unread: usize,
    /// Active UI language + keyboard layout. Both persist to config.
    pub language: crate::i18n::Lang,
    /// Set to true when the user clicks "Reset defaults" in the
    /// Appearance card. The main loop consumes it on the next tick.
    pub reset_requested: bool,
    /// Spatial placement of the overlay quad. The VR preview reads
    /// this every frame to position the quad; VR mode will too.
    pub placement: Placement,
    pub keyboard: crate::keyboard::Keyboard,
    /// SMTC → chatbox bridge. `enabled` persists to config.
    pub now_playing: NowPlayingBridge,
    pending_hit: Option<MirrorHit>,
    batteries: Vec<BatteryReading>,
    pub battery_osc: crate::battery::BatteryOsc,
    battery_est: crate::battery::BatteryEstimator,
    charge_sense: crate::battery::ChargeSense,
    started: Instant,
    pending_osc: Vec<(String, rsoverlay_osc::OscValue)>,
    low_battery: LowBatteryWatch,
    toasts: VecDeque<Toast>,
    toast_seq: u64,
    /// Page shown last frame + when the current one appeared, for the
    /// page-enter transition.
    last_page: crate::shell::Page,
    page_since: f64,
    /// Accessibility: make every transition instant. Persists.
    pub reduce_motion: bool,
    /// VR dashboards: SteamVR-style bottom dock instead of the side rail.
    pub dock_layout: bool,
    /// VR: wrist on the right hand, laser from the left.
    pub vr_left_handed: bool,
    /// VR: panels fade out beyond this off-axis view angle (degrees).
    pub vr_clip_angle: f32,
    pub vr_wrist_opacity: f32,
    /// Accessibility: VR UI size multiplier.
    pub vr_ui_scale: f32,
    pub vr_show_lasers: bool,
    pub vr_laser_angle: f32,
    /// Laser cursor + beam size multiplier.
    pub vr_pointer_size: f32,
    /// Clock / date format slugs (see `locale::CLOCK_FORMATS`).
    pub clock_format: String,
    pub date_format: String,
    /// Fade all panels while SteamVR's own dashboard is open.
    pub vr_hide_with_steamvr: bool,
    /// Old XSO layout-mode setting, kept so saved configs round-trip.
    /// No longer used: the dashboard never shows or hides windows (the
    /// wrist Windows button does).
    pub vr_windows_follow_dash: bool,
    /// Grabbed panels tilt/roll with the controller.
    pub vr_grab_tilt: bool,
    /// Grab stick: one axis (push/pull or scale) at a time.
    pub vr_stick_axis_lock: bool,
    /// Panel render resolution multiplier (1, 1.5, 2).
    pub vr_render_scale: f32,
    /// Refresh rate of panels you aren't looking at (fps).
    pub vr_background_fps: f32,
    pub run_as_admin: bool,
    /// Set by the app: is this process elevated right now?
    pub elevated: bool,
    /// "Restart as admin now" clicked; the app relaunches via UAC.
    pub restart_elevated_requested: bool,
    pub search: crate::shell::SearchState,
    /// Saved VR layouts (loaded/saved by the app).
    pub layouts: rsoverlay_core::layouts::Layouts,
    /// Layout slot action requested from the Layouts page.
    pub layout_request: Option<crate::layouts_page::LayoutAction>,
    /// Layout slot selected on the wrist toolbar.
    pub wrist_slot: usize,
    /// Media theme: accent follows the album art while something plays.
    pub media_theme: bool,
    art_accent: Option<egui::Color32>,
    /// Background SMTC poller (spawned on first use).
    media_worker: Option<media::MediaWorker>,
    /// Wrist toolbar: which of the 7 action buttons show (bit i).
    pub wrist_buttons: u16,
    /// Compact wrist contents (see `wrist::compact`).
    pub wrist_compact: u32,
    /// VR keyboard look + typing sounds (Settings › Keyboard).
    pub kb_prefs: KeyboardPrefs,
    /// XSO-style knobs: cursor smoothing 0..1, scroll speed multiplier.
    pub cursor_smoothing: f32,
    pub scroll_speed: f32,
    /// Long-press + double-click assist (Settings › Pointer).
    pub pointer: PointerPrefs,
    /// Reopen last session's windows at VR startup.
    pub restore_session: bool,
    /// Hands-on VR adjustments (wrist pose, keyboard toggles), mirrored
    /// here by the VR shell so they autosave with the config.
    pub vr_state: rsoverlay_core::config::VrState,
    /// Reset button armed at this egui time (two-step confirm — VR has
    /// no dialogs).
    reset_armed_at: Option<f64>,
    pub night_light: crate::nightlight::NightLight,
    /// Overlay colour multiplier (rgb) + opacity (a) for this frame;
    /// the app hands it to the compositor / preview shaders.
    tint: [f32; 4],
    last_frame_time: Option<f64>,
    history: VecDeque<HistoryEntry>,
    notifications_received: u64,
    media: Option<MediaInfo>,
    demo_media: bool,
    chat_draft: String,
    pending_chat: Option<rsoverlay_osc::ChatMessage>,
}

impl Default for DevView {
    fn default() -> Self {
        Self {
            frame: 0,
            last_fps: 0.0,
            frame_ms: 0.0,
            frame_ms_hist: VecDeque::new(),
            mirror: None,
            sources: SourcePicker::default(),
            open_windows: Vec::new(),
            sources_refreshed_at: f64::NEG_INFINITY,
            forward_input: false,
            accent: theme::ACCENT,
            show_mirror: true,
            show_media: true,
            show_chat: true,
            show_keyboard: false,
            dash_attach: crate::window_panel::Attach::World,
            dash_dock: None,
            dash_close: false,
            wrist_expanded: false,
            wrist_editing: false,
            show_history: false,
            show_battery: true,
            battery_warn_pct: 20,
            battery_hidden: Vec::new(),
            battery_sound: true,
            pending_sounds: Vec::new(),
            notif: NotificationPrefs::default(),
            toasts_external: false,
            discord_presence: true,
            vr_perf: None,
            updates: Default::default(),
            focus_mode: false,
            windows_up: false,
            kb_hidden_with_windows: false,
            demo_battery_estimates: None,
            dashboard_open: false,
            gamepad: Default::default(),
            wrist_bg: Default::default(),
            gamepad_held: 0,
            gamepad_capture: None,
            steamvr_icons: true,
            discord: Default::default(),
            vr_hide_wrist: false,
            show_macros: true,
            macros: crate::macros::MacroDeck::default(),
            show_volume: true,
            volume: crate::volume::VolumeCard::default(),
            show_system: true,
            system: crate::system::SystemCard::default(),
            page: crate::shell::Page::default(),
            unread: 0,
            language: crate::i18n::Lang::default(),
            reset_requested: false,
            placement: Placement::default(),
            keyboard: crate::keyboard::Keyboard::default(),
            now_playing: NowPlayingBridge::default(),
            pending_hit: None,
            batteries: Vec::new(),
            battery_osc: crate::battery::BatteryOsc::default(),
            battery_est: crate::battery::BatteryEstimator::default(),
            charge_sense: crate::battery::ChargeSense::default(),
            started: Instant::now(),
            pending_osc: Vec::new(),
            low_battery: LowBatteryWatch::default(),
            toasts: VecDeque::new(),
            toast_seq: 0,
            last_page: crate::shell::Page::default(),
            page_since: f64::NEG_INFINITY,
            reduce_motion: false,
            dock_layout: false,
            vr_left_handed: false,
            vr_clip_angle: 60.0,
            vr_wrist_opacity: 1.0,
            vr_ui_scale: 1.0,
            vr_show_lasers: true,
            vr_laser_angle: 0.0,
            vr_pointer_size: 1.0,
            clock_format: "system".into(),
            date_format: "system".into(),
            vr_hide_with_steamvr: true,
            vr_windows_follow_dash: true,
            vr_grab_tilt: false,
            vr_stick_axis_lock: true,
            vr_render_scale: 1.0,
            vr_background_fps: 5.0,
            run_as_admin: false,
            elevated: false,
            restart_elevated_requested: false,
            reset_armed_at: None,
            search: crate::shell::SearchState::default(),
            layouts: Default::default(),
            layout_request: None,
            wrist_slot: 0,
            media_theme: false,
            art_accent: None,
            media_worker: None,
            wrist_buttons: crate::wrist::compact::DEFAULT_ACTIONS,
            wrist_compact: rsoverlay_core::config::DEFAULT_WRIST_COMPACT,
            kb_prefs: KeyboardPrefs::default(),
            cursor_smoothing: 0.0,
            scroll_speed: 1.0,
            pointer: PointerPrefs::default(),
            restore_session: true,
            vr_state: Default::default(),
            night_light: crate::nightlight::NightLight::default(),
            tint: [1.0; 4],
            last_frame_time: None,
            history: VecDeque::new(),
            notifications_received: 0,
            media: None,
            demo_media: false,
            chat_draft: String::new(),
            pending_chat: None,
        }
    }
}

impl DevView {
    pub fn push_notification(&mut self, n: Notification) {
        const MAX_HISTORY: usize = 50;
        self.notifications_received += 1;
        // Muted app: keep it in history, but no toast, sound or badge.
        let muted = !n.source_app.is_empty() && self.notif.muted.contains(&n.source_app);
        // Focus mode: same, but it still counts as unread for later.
        if self.focus_mode && !muted {
            self.unread += 1;
        }
        if muted || self.focus_mode {
            self.history.push_back(HistoryEntry { n, received: Instant::now() });
            while self.history.len() > MAX_HISTORY {
                self.history.pop_front();
            }
            return;
        }
        self.unread += 1;
        // The sender picks a category; the user's slot decides the file.
        if let Some(kind) = rsoverlay_audio::alerts::AlertKind::from_audio_path(&n.audio_path) {
            self.pending_sounds.push((kind, n.volume.clamp(0.0, 1.0)));
        }
        let timeout = n.timeout.clamp(0.5, 30.0);
        let expires = Instant::now() + Duration::from_secs_f32(timeout);
        self.history.push_back(HistoryEntry {
            n: n.clone(),
            received: Instant::now(),
        });
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
        self.toast_seq += 1;
        self.toasts.push_back(Toast { n, expires, seq: self.toast_seq, born: None, height: 88.0 });
        while self.toasts.len() > 8 {
            self.toasts.pop_front();
        }
    }

    /// Clear every live toast + the full history. XSO's #131 cluster:
    /// one action dismisses everything, including the backlog.
    /// Rich Presence details from the user's sharing choices. Only counts
    /// and the user's own media/battery — never window titles.
    pub fn discord_details(&self) -> rsoverlay_discord::Details {
        let c = &self.discord;
        let mut lines = Vec::new();
        if c.show_battery {
            if let Some(b) = self.batteries.iter().find(|b| b.label == "HMD") {
                let pct = (b.fraction * 100.0).round() as u32;
                let est = self.battery_est.estimate(b).filter(|e| !e.charging).map(|e| crate::battery::format_duration(e.secs));
                lines.push(match (b.charging, est) {
                    (true, _) => format!("⚡ Headset {pct}% · charging"),
                    (false, Some(e)) => format!("🔋 Headset {pct}% · ~{e} left"),
                    (false, None) => format!("🔋 Headset {pct}%"),
                });
            }
        }
        if c.show_music {
            if let Some(m) = self.media.as_ref().filter(|m| !m.title.is_empty()) {
                lines.push(if m.artist.is_empty() { format!("🎵 {}", m.title) } else { format!("🎵 {} — {}", m.title, m.artist) });
            }
        }
        rsoverlay_discord::Details {
            windows: c.show_windows.then_some(self.open_windows.len()),
            lines,
            button: c.show_button,
        }
    }

    /// Alert sounds due since the last call (the app plays them).
    pub fn take_sounds(&mut self) -> Vec<(rsoverlay_audio::alerts::AlertKind, f32)> {
        std::mem::take(&mut self.pending_sounds)
    }

    /// Any toast on screen (VR shows its toast panel only then).
    pub fn has_toasts(&self) -> bool {
        !self.toasts.is_empty()
    }

    /// The VR toast panel: just the toasts, centred, on a transparent
    /// panel. Advances per-frame state like the other surfaces.
    pub fn toasts_ui(&mut self, ctx: &egui::Context) {
        self.tick_state(ctx);
        let accent = self.effective_accent();
        self.paint_toasts(ctx, accent, 8.0, egui::Align2::CENTER_TOP, 0.0);
    }

    pub fn clear_all_notifications(&mut self) {
        self.toasts.clear();
        self.history.clear();
        self.unread = 0;
    }

    /// Replace the battery readings (the app polls OpenVR every few
    /// seconds) and raise a toast for any device that just went low.
    /// Alerts fire even with the card hidden — that's the point of them.
    pub fn set_batteries(&mut self, readings: Vec<BatteryReading>) {
        // Hidden devices don't alert (still tracked for estimates / OSC).
        let shown: Vec<BatteryReading> = readings.iter().filter(|r| !self.battery_hidden.contains(&r.label)).cloned().collect();
        for mut n in self.low_battery.update(&shown, self.battery_warn_pct) {
            if self.battery_sound {
                self.pending_sounds.push((rsoverlay_audio::alerts::AlertKind::LowBattery, 1.0));
            }
            // Its own sound (above), not the generic notification one.
            n.audio_path = "none".into();
            self.push_notification(n);
        }
        let mut readings = readings;
        self.charge_sense.apply(&mut readings);
        self.battery_est.update(self.started.elapsed().as_secs_f64(), &readings);
        let msgs = self.battery_osc.messages(&readings, Instant::now());
        self.pending_osc.extend(msgs);
        self.batteries = readings;
    }

    /// Battery readings minus the devices the user hid.
    fn shown_batteries(&self) -> Vec<BatteryReading> {
        self.batteries.iter().filter(|r| !self.battery_hidden.contains(&r.label)).cloned().collect()
    }

    pub(crate) fn apply_search(&mut self, ctx: &egui::Context, t: crate::search::Target) {
        use crate::search::Target;
        match t {
            Target::Page(p) => self.page = p,
            Target::Card(p, caption) => {
                self.page = p;
                crate::widgets::highlight_card(ctx, caption);
            }
            Target::Window(src) => {
                if !self.dock_layout {
                    self.page = crate::shell::Page::Desktop;
                }
                self.sources.request = Some(src);
            }
            Target::Widget(k) => crate::widgets::request_popout(ctx, k),
        }
    }

    /// Widget popped out from a card's ↗ this frame (VR dashboard).
    pub fn take_widget_request(&mut self, ctx: &egui::Context) -> Option<crate::widgets::WidgetKind> {
        crate::widgets::take_popout_request(ctx)
    }

    /// A floating widget panel: tab strip + the live card + dock bar.
    #[allow(clippy::too_many_arguments)]
    pub fn widget_ui(
        &mut self,
        ctx: &egui::Context,
        tabs: &[crate::widgets::WidgetKind],
        active: usize,
        attach: crate::window_panel::Attach,
        linked: bool,
        pinned: Option<bool>,
        injector: Option<&rsoverlay_input::InputInjector>,
    ) -> crate::widgets::WidgetAction {
        use crate::widgets::WidgetKind as K;
        self.tick_state(ctx);
        crate::widgets::set_popout_enabled(ctx, false);
        let accent = self.effective_accent();
        if ctx.style().visuals.selection.bg_fill != accent {
            ctx.style_mut(|st| st.visuals.selection.bg_fill = accent);
        }
        crate::widgets::frame(ctx, tabs, active, attach, linked, pinned, accent, |ui, k| match k {
            K::NowPlaying => media_card(ui, self.media.as_ref(), accent),
            K::System => self.system.show(ui, self.vr_perf.as_ref(), accent),
            K::Volume => self.volume.show(ui, accent),
            K::Battery => crate::battery::card(ui, &self.batteries, &mut self.battery_hidden, &self.battery_est, &mut self.battery_warn_pct, &mut self.battery_sound, &mut self.steamvr_icons, &mut self.battery_osc, accent),
            K::Macros => self.macros.card(ui, injector, accent),
            K::Chat => {
                if let Some(msg) = chatbox_card(ui, &mut self.chat_draft, &mut self.now_playing.enabled, accent) {
                    self.pending_chat = Some(msg);
                }
            }
            K::Notifications => {
                if history_card(ui, &self.history, &mut self.notif.muted, accent) {
                    self.clear_all_notifications();
                }
            }
        })
    }

    /// The wrist watch-face (VR). Shares all state with the dashboard;
    /// returns what the wearer tapped.
    /// Wrist Windows button. Hiding the windows takes the keyboard with
    /// them (one press clears everything, tester); showing brings the
    /// keyboard back if it was open.
    pub fn toggle_windows(&mut self) {
        self.windows_up = !self.windows_up;
        if self.windows_up {
            if std::mem::take(&mut self.kb_hidden_with_windows) {
                self.show_keyboard = true;
            }
        } else {
            self.kb_hidden_with_windows = self.show_keyboard;
            self.show_keyboard = false;
        }
    }

    pub fn wrist_ui(&mut self, ctx: &egui::Context) -> crate::wrist::WristAction {
        self.tick_state(ctx);
        // Estimates line up with the batteries actually shown.
        let shown_batteries = self.shown_batteries();
        let est: Vec<Option<String>> = match &self.demo_battery_estimates {
            Some(demo) => demo.clone(),
            None => shown_batteries
                .iter()
                .map(|b| self.battery_est.estimate(b).filter(|e| !e.charging).map(|e| crate::battery::format_duration(e.secs)))
                .collect(),
        };
        // Music volume on the wrist: the playing app's mixer session, or
        // the master volume when it can't be matched. (pid, volume)
        let music: Option<(Option<u32>, f32)> = match self.media.as_ref().filter(|m| !m.title.is_empty()).map(|m| m.source.clone()) {
            Some(source) => {
                let snap = self.volume.mixer().snapshot();
                rsoverlay_audio::match_app(&source, &snap.apps)
                    .map(|a| (Some(a.pid), a.level.volume))
                    .or(snap.master.map(|m| (None, m.volume)))
            }
            None => None,
        };
        let media = self.media.as_ref().filter(|m| !m.title.is_empty());
        // System load only when a chip needs it: the sampler is a thread.
        use crate::wrist::compact as wc;
        let wants_sys = self.wrist_editing || self.wrist_expanded || self.wrist_compact & (wc::PERF | wc::RAM | wc::VRAM) != 0;
        let sys = if wants_sys { self.system.snapshot() } else { None };
        let data = crate::wrist::WristData {
            batteries: &shown_batteries,
            estimates: &est,
            title: media.map(|m| m.title.as_str()),
            artist: media.map(|m| m.artist.as_str()).filter(|a| !a.is_empty()),
            playing: media.is_some_and(|m| m.playing),
            session_secs: self.started.elapsed().as_secs(),
            slot: self.wrist_slot,
            slot_filled: self.layouts.slot(self.wrist_slot).is_some(),
            slot_favorite: self.layouts.favorite == Some(self.wrist_slot),
            keyboard_open: self.show_keyboard,
            focus: self.focus_mode,
            music_volume: music.map(|m| m.1),
            windows_up: self.windows_up,
            dashboard_open: self.dashboard_open,
            background: self.wrist_bg.image.as_ref().map(|i| (i, self.wrist_bg.dim)),
            accent: self.effective_accent(),
            buttons: self.wrist_buttons,
            compact: self.wrist_compact,
            expanded: self.wrist_expanded,
            editing: self.wrist_editing,
            system: sys.as_ref(),
            vr_perf: self.vr_perf.as_ref(),
            unread: self.unread,
        };
        let act = crate::wrist::show(ctx, &data);
        if act.toggle_more {
            self.wrist_expanded = !self.wrist_expanded;
        }
        if act.volume_delta != 0.0 {
            // Diagnostics (tester: wrist volume does nothing).
            let source = self.media.as_ref().map(|m| m.source.clone()).unwrap_or_default();
            let apps: Vec<String> = self.volume.mixer().snapshot().apps.iter().map(|a| format!("{}={}", a.name, a.pid)).collect();
            tracing::info!("wrist volume {:+.2}: media app {source:?} → target {:?}; mixer apps {apps:?}", act.volume_delta, music);
            if let Some((pid, v)) = music {
                let v = (v + act.volume_delta).clamp(0.0, 1.0);
                match pid {
                    Some(pid) => self.volume.mixer().set_app_volume(pid, v),
                    None => self.volume.mixer().set_master_volume(v),
                }
            }
        }
        if act.toggle_focus {
            self.focus_mode = !self.focus_mode;
        }
        if act.toggle_windows {
            self.toggle_windows();
        }
        if act.toggle_edit {
            self.wrist_editing = !self.wrist_editing;
            // Done → land on the compact wrist to see the result.
            if !self.wrist_editing {
                self.wrist_expanded = false;
            }
        }
        if let Some(bit) = act.toggle_compact {
            self.wrist_compact ^= bit;
        }
        if act.slot_delta != 0 {
            let n = rsoverlay_core::layouts::SLOTS as i32;
            self.wrist_slot = (self.wrist_slot as i32 + act.slot_delta).rem_euclid(n) as usize;
        }
        act
    }

    /// Accent in effect: the album-art colour while Media theme is on
    /// and something with art is playing, else the user's accent.
    pub fn effective_accent(&self) -> egui::Color32 {
        match (self.media_theme, self.art_accent) {
            (true, Some(c)) if self.media.as_ref().is_some_and(|m| !m.title.is_empty()) => c,
            _ => self.accent,
        }
    }

    /// Queue a VRChat chatbox message (e.g. from the VR keyboard).
    pub fn send_chat(&mut self, text: String) {
        if !text.trim().is_empty() {
            self.pending_chat = Some(rsoverlay_osc::ChatMessage { text, send_immediately: true, play_notification: true });
        }
    }

    /// Record this frame's render cost (ms) for the Performance card.
    pub fn record_frame_ms(&mut self, ms: f32) {
        // Light smoothing so the headline number is readable.
        self.frame_ms = if self.frame_ms == 0.0 { ms } else { self.frame_ms * 0.9 + ms * 0.1 };
        self.frame_ms_hist.push_back(ms);
        while self.frame_ms_hist.len() > 120 {
            self.frame_ms_hist.pop_front();
        }
    }

    /// RGB multiplier + opacity to apply to the whole overlay.
    pub fn overlay_tint(&self) -> [f32; 4] {
        self.tint
    }

    pub fn notifications_received(&self) -> u64 {
        self.notifications_received
    }

    /// Take the pending mirror hit, if any. The caller is expected to drain
    /// this once per frame and forward valid hits to the input injector.
    pub fn take_mirror_hit(&mut self) -> Option<MirrorHit> {
        self.pending_hit.take()
    }

    /// Drain queued avatar-parameter sends (battery -> VRChat).
    pub fn take_pending_osc(&mut self) -> Vec<(String, rsoverlay_osc::OscValue)> {
        std::mem::take(&mut self.pending_osc)
    }

    /// Take the pending VRChat chatbox message, if any.
    pub fn take_pending_chat(&mut self) -> Option<rsoverlay_osc::ChatMessage> {
        self.pending_chat.take()
    }

    /// One Settings card on its own, filling the screen: `index` 0..
    /// walks every card in page order. Returns the card's name, or None
    /// past the last one. Used to render the settings overview image.
    pub fn settings_card_ui(&mut self, ctx: &egui::Context, index: usize) -> Option<&'static str> {
        self.tick_state(ctx);
        let accent = self.effective_accent();
        if ctx.style().visuals.selection.bg_fill != accent {
            ctx.style_mut(|st| {
                st.visuals.selection.bg_fill = accent;
                st.visuals.selection.stroke.color = theme::BG_ROOT;
            });
        }
        let mut name = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG_ROOT).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                name = match index {
                    0 => {
                        let _ = appearance_card(
                            ui,
                            &mut self.accent,
                            &mut self.show_mirror,
                            &mut self.show_media,
                            &mut self.show_chat,
                            &mut self.show_keyboard,
                            &mut self.show_history,
                            &mut self.show_battery,
                            &mut self.show_macros,
                            &mut self.show_volume,
                            &mut self.show_system,
                            &mut self.language,
                            &mut self.keyboard.layout,
                        );
                        Some("appearance")
                    }
                    1 => {
                        vr_card(ui, &mut self.vr_left_handed, &mut self.vr_clip_angle, &mut self.vr_wrist_opacity, &mut self.vr_hide_wrist, accent);
                        Some("vr")
                    }
                    2 => {
                        vr_panels_card(ui, &mut self.vr_hide_with_steamvr, &mut self.vr_grab_tilt, &mut self.vr_stick_axis_lock, &mut self.vr_render_scale, &mut self.vr_background_fps, &mut self.pointer, accent);
                        Some("vr-panels")
                    }
                    3 => {
                        pointer_card(ui, &mut self.scroll_speed, &mut self.pointer, accent);
                        Some("pointer")
                    }
                    4 => {
                        window_input_card(ui, &mut self.pointer, accent);
                        Some("window-input")
                    }
                    5 => {
                        keyboard_card(ui, &mut self.kb_prefs, accent);
                        Some("keyboard")
                    }
                    6 => {
                        accessibility_card(ui, &mut self.cursor_smoothing, &mut self.vr_ui_scale, &mut self.vr_show_lasers, &mut self.vr_laser_angle, &mut self.vr_pointer_size, &mut self.reduce_motion, accent);
                        Some("accessibility")
                    }
                    7 => {
                        wrist_look_card(ui, &mut self.clock_format, &mut self.date_format, &mut self.wrist_bg, &mut self.media_theme, accent);
                        Some("wrist-look")
                    }
                    8 => {
                        wrist_toolbar_card(ui, &mut self.wrist_buttons, &mut self.wrist_compact, accent);
                        Some("wrist-toolbar")
                    }
                    9 => {
                        notifications_card(ui, &mut self.notif, &mut self.pending_sounds, accent);
                        Some("notifications")
                    }
                    10 => {
                        self.night_light.card(ui, minute_of_day(), accent);
                        Some("night-light")
                    }
                    11 => {
                        gamepad_card(ui, &mut self.gamepad, self.gamepad_held, &mut self.gamepad_capture, accent);
                        Some("gamepad")
                    }
                    12 => {
                        self.updates.card(ui, accent);
                        Some("updates")
                    }
                    13 => {
                        discord_card(ui, &mut self.discord_presence, &mut self.discord, accent);
                        Some("discord")
                    }
                    14 => {
                        let snap = self.system.snapshot();
                        performance_card(ui, snap.as_ref(), self.frame_ms, &self.frame_ms_hist, self.last_fps, self.notifications_received, accent);
                        Some("stats")
                    }
                    15 => {
                        placement_card(ui, &mut self.placement, accent);
                        Some("placement")
                    }
                    16 => {
                        let _ = admin_card(ui, &mut self.run_as_admin, self.elevated, accent);
                        Some("administrator")
                    }
                    17 => {
                        let _ = reset_card(ui, &mut self.reset_armed_at);
                        Some("reset")
                    }
                    _ => None,
                };
            });
        name
    }

    /// Drop on-screen toasts (history keeps them).
    pub fn dismiss_toasts(&mut self) {
        self.toasts.clear();
    }

    /// Pin a fixed track instead of reading the system's media session
    /// (VR preview / screenshots: nobody's real listening history).
    pub fn set_demo_media(&mut self, title: &str, artist: &str) {
        self.media = Some(MediaInfo { title: title.into(), artist: artist.into(), source: String::new(), playing: true });
        self.demo_media = true;
    }

    fn maybe_refresh_media(&mut self) {
        if self.demo_media {
            return;
        }
        // Off the UI thread: the media service can stall indefinitely.
        let worker = self.media_worker.get_or_insert_with(media::MediaWorker::spawn);
        (self.media, self.art_accent) = worker.latest(self.media_theme);
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        self.ui_with(ctx, None);
    }

    /// Full UI with optional input injector. When `Some`, keyboard key
    /// presses inside the dashboard fire SendInput events against the
    /// OS. When `None`, the keyboard renders but keys are inert —
    /// useful for VR mode before laser input is wired, or headless
    /// rendering.
    /// Per-frame state that must advance whether or not the dashboard is
    /// drawn (toast expiry, media poll, now-playing bridge, night light).
    /// Called by every surface (dashboard, wrist); idempotent within a
    /// frame.
    fn tick_state(&mut self, ctx: &egui::Context) {
        crate::laser::set_cursor_scale(self.vr_pointer_size);
        crate::locale::set_formats(&self.clock_format, &self.date_format);
        self.wrist_bg.tick();
        if let Some(v) = self.updates.take_updated_notice() {
            self.push_notification(Notification {
                title: format!("Updated to RSOverlay {v}"),
                content: "See what's new in Settings > Updates.".into(),
                source_app: "RSOverlay".into(),
                timeout: 8.0,
                ..Default::default()
            });
        }
        if let Some(r) = self.updates.tick() {
            let kind = if r.prerelease { "Test build" } else { "Update" };
            self.push_notification(Notification {
                title: format!("{kind} available: RSOverlay {}", r.tag),
                content: "Settings > Updates to get it (nothing installs by itself).".into(),
                source_app: "RSOverlay".into(),
                timeout: 8.0,
                ..Default::default()
            });
        }
        let now = Instant::now();
        while matches!(self.toasts.front(), Some(t) if t.expires < now) {
            self.toasts.pop_front();
        }
        self.maybe_refresh_media();
        // Manual messages win the single outbound slot; the bridge only
        // ticks when it's free, so its pacing state never records a
        // send that didn't happen.
        if self.pending_chat.is_none() {
            if let Some(text) = self.now_playing.tick(self.media.as_ref(), now) {
                self.pending_chat = Some(rsoverlay_osc::ChatMessage {
                    text,
                    send_immediately: true,
                    play_notification: false,
                });
            }
        }
        crate::motion::set_reduce_motion(self.reduce_motion);
        let t = ctx.input(|i| i.time);
        let dt = self.last_frame_time.map_or(0.0, |p| (t - p) as f32);
        self.last_frame_time = Some(t);
        let [r, g, b] = self.night_light.tick(minute_of_day(), dt);
        self.tint = [r, g, b, self.placement.opacity.clamp(0.2, 1.0)];
        if (self.tint[2] - crate::nightlight::multiplier(self.night_light.strength)[2]).abs() > 1e-3
            && self.night_light.active_at(minute_of_day())
        {
            ctx.request_repaint();
        }
    }

    pub fn ui_with(
        &mut self,
        ctx: &egui::Context,
        injector: Option<&rsoverlay_input::InputInjector>,
    ) {
        self.tick_state(ctx);
        crate::widgets::set_popout_enabled(ctx, self.dock_layout);
        let accent = self.effective_accent();
        // Slider fills, checkbox ticks and text selection follow the
        // user accent instead of egui's stock blue.
        if ctx.style().visuals.selection.bg_fill != accent {
            ctx.style_mut(|st| {
                st.visuals.selection.bg_fill = accent;
                st.visuals.selection.stroke.color = theme::BG_ROOT;
            });
        }
        if self.page == crate::shell::Page::Notifications {
            self.unread = 0;
        }
        if self.page != self.last_page {
            self.last_page = self.page;
            self.page_since = ctx.input(|i| i.time);
        }
        let enter = crate::motion::progress(ctx, self.page_since, crate::motion::MEDIUM);

        // VR: the dashboard is one rounded black surface with a hairline
        // edge (panels draw transparent on top of it); desktop keeps the
        // edge-to-edge AMOLED black.
        let root = if self.dock_layout {
            // Available, not screen: identical in VR (nothing else on the
            // texture), and leaves the desktop dev bar visible.
            let slab = ctx.available_rect().shrink(6.0);
            let bg = ctx.layer_painter(egui::LayerId::background());
            theme::surface(&bg, slab, 24.0);
            egui::Frame::none()
        } else {
            egui::Frame::none().fill(theme::BG_ROOT)
        };
        // VR: the same bottom bar every floating panel has (dock + close).
        if self.dock_layout {
            egui::TopBottomPanel::bottom("panel-bar")
                .frame(root.inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 6.0, bottom: 14.0 }))
                .show_separator_line(false)
                .show(ctx, |ui| {
                    let top = ui.max_rect().top() - 4.0;
                    ui.painter().line_segment(
                        [egui::pos2(ui.max_rect().left(), top), egui::pos2(ui.max_rect().right(), top)],
                        egui::Stroke::new(1.0_f32, theme::STROKE_CARD),
                    );
                    ui.horizontal(|ui| {
                        ui.set_height(40.0);
                        ui.label(egui::RichText::new(format!("RSOverlay · {}", self.page.label())).color(theme::FG_MUTED).size(theme::SIZE_BODY));
                        // Version in view, so screenshots in bug reports carry it.
                        ui.label(egui::RichText::new(rsoverlay_core::update::CURRENT).color(theme::FG_FAINT).size(theme::SIZE_CAPTION));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (dock, close, _) = crate::window_panel::dock_controls(ui, self.dash_attach, None, accent);
                            self.dash_dock = dock.or(self.dash_dock);
                            self.dash_close |= close;
                        });
                    });
                });
        }
        // One labelled sidebar on both surfaces: in VR as on the desktop,
        // every destination names itself.
        egui::SidePanel::left("nav")
            .exact_width(crate::shell::RAIL_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(root.inner_margin(egui::Margin { left: 14.0, right: 0.0, top: 14.0, bottom: 16.0 }))
            .show(ctx, |ui| {
                let q = crate::shell::nav_rail(ui, &mut self.page, self.unread, self.show_keyboard, self.dock_layout, accent);
                if q.toggle_keyboard {
                    self.show_keyboard = !self.show_keyboard;
                }
                if q.add_window {
                    self.page = crate::shell::Page::Desktop;
                }
            });
        egui::TopBottomPanel::top("header")
            .frame(root.inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 14.0, bottom: 10.0 }))
            .show_separator_line(false)
            .show(ctx, |ui| {
                let entries = crate::search::index(&self.sources.choices, self.dock_layout);
                // Window results need a list; ask for one when searching.
                if !self.search.query.is_empty() && self.sources.choices.is_empty() {
                    self.sources.wants_refresh = true;
                }
                let shown = self.shown_batteries();
                let act = crate::shell::header(ui, &shown, self.unread, &mut self.search, &entries, accent);
                if let Some(t) = act.search_pick {
                    self.apply_search(ui.ctx(), t);
                }
                if act.open_notifications {
                    self.page = crate::shell::Page::Notifications;
                }
            });
        // Desktop keyboard: a real bottom panel, so the page shrinks above
        // it instead of being covered.
        if self.show_keyboard && !self.dock_layout {
            if let Some(inj) = injector {
                egui::TopBottomPanel::bottom("virtual-keyboard")
                    .frame(root.inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 4.0, bottom: 14.0 }))
                    .show_separator_line(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        theme::card().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                card_header(ui, icons::KEYBOARD, "Keyboard", accent);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if crate::window_panel::pill(ui, "✕", false, accent).on_hover_text("Hide keyboard").clicked() {
                                        self.show_keyboard = false;
                                    }
                                });
                            });
                            ui.add_space(6.0);
                            crate::keyboard::show(ui, &mut self.keyboard, inj, accent);
                        });
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(root.inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 8.0, bottom: 16.0 }))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_source(("page", self.page.slug()))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Page enter: content rises 12 px…
                        ui.add_space((1.0 - enter) * 12.0);
                        self.page_body(ui, injector, accent);
                    });
                // …and fades up from black. On a true-black UI a black
                // veil with falling alpha *is* a fade — one quad, no
                // offscreen layer.
                if enter < 1.0 {
                    let a = ((1.0 - enter) * 255.0) as u8;
                    ui.painter().rect_filled(ui.max_rect().expand(4.0), 0.0, egui::Color32::from_black_alpha(a));
                }
            });


        if !self.toasts_external {
            self.paint_toasts(ctx, accent, 84.0, egui::Align2::RIGHT_TOP, -theme::GUTTER);
        }

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl DevView {
    /// Toasts: each in its own Area so it can slide independently.
    /// Stacked from `top`, anchored at `anchor` offset by `x`.
    fn paint_toasts(&mut self, ctx: &egui::Context, accent: egui::Color32, top: f32, anchor: egui::Align2, x: f32) {
        let now = Instant::now();
    // Toasts: each in its own Area so it can slide independently.
    // Enter from the right, exit to the right in the last 220 ms,
    // and survivors glide up into the gap a departing toast leaves.
    // Stacked below the header so they never cover the clock.
    let t_now = ctx.input(|i| i.time);
    let mut y = top;
    for t in self.toasts.iter_mut() {
        let born = *t.born.get_or_insert(t_now);
        let enter = crate::motion::progress(ctx, born, crate::motion::MEDIUM);
        let left = t.expires.saturating_duration_since(now);
        let exit = if left < TOAST_EXIT {
            ctx.request_repaint();
            crate::motion::ease_out(1.0 - left.as_secs_f32() / TOAST_EXIT.as_secs_f32())
        } else {
            0.0
        };
        let exit = if crate::motion::reduced() { 0.0 } else { exit };
        let slide = (1.0 - enter + exit) * TOAST_SLIDE;
        let id = egui::Id::new(("toast", t.seq));
        let ty = crate::motion::value(ctx, id.with("y"), y, crate::motion::MEDIUM);
        let resp = egui::Area::new(id)
            .anchor(anchor, egui::vec2(x + slide, ty))
            .interactable(false)
            .show(ctx, |ui| toast_card(ui, &t.n, accent));
        t.height = resp.response.rect.height();
        y += t.height + 8.0;
    }
    }

    fn page_body(
        &mut self,
        ui: &mut egui::Ui,
        injector: Option<&rsoverlay_input::InputInjector>,
        accent: egui::Color32,
    ) {
        use crate::shell::{page_title, Page};
        page_title(ui, self.page.label(), accent);
        match self.page {
            Page::Home => {
                // Two tile columns; each tile honours its Appearance toggle.
                ui.columns(2, |cols| {
                    let (lc, rc) = cols.split_at_mut(1);
                    let (lp, rp) = (&mut lc[0], &mut rc[0]);
                    // ui.columns() hands out *justified* layouts, and egui
                    // letter-justifies wrapped text in those. Re-plain them
                    // (and hand the used rects back to the parents below,
                    // so the scroll area still knows the page height).
                    let plain = egui::Layout::top_down(egui::Align::Min);
                    let mut l = lp.child_ui(lp.max_rect(), plain);
                    let mut r = rp.child_ui(rp.max_rect(), plain);
                    let (l, r) = (&mut l, &mut r);
                    if self.show_media {
                        media_card(l, self.media.as_ref(), accent);
                        l.add_space(theme::GUTTER);
                    }
                    if self.show_system {
                        self.system.show(l, self.vr_perf.as_ref(), accent);
                        l.add_space(theme::GUTTER);
                    }
                    if self.show_volume {
                        self.volume.show(l, accent);
                        l.add_space(theme::GUTTER);
                    }
                    if self.show_battery {
                        crate::battery::card(r, &self.batteries, &mut self.battery_hidden, &self.battery_est, &mut self.battery_warn_pct, &mut self.battery_sound, &mut self.steamvr_icons, &mut self.battery_osc, accent);
                        r.add_space(theme::GUTTER);
                    }
                    if self.show_macros {
                        self.macros.card(r, injector, accent);
                        r.add_space(theme::GUTTER);
                    }
                    if self.show_chat {
                        if let Some(msg) = chatbox_card(r, &mut self.chat_draft, &mut self.now_playing.enabled, accent) {
                            self.pending_chat = Some(msg);
                        }
                    }
                    lp.allocate_rect(l.min_rect(), egui::Sense::hover());
                    rp.allocate_rect(r.min_rect(), egui::Sense::hover());
                });
            }
            Page::Desktop if self.dock_layout => {
                // VR: pick a window/monitor to pop out as its own overlay.
                // Refresh the list every 2 s while this page is open.
                let t = ui.input(|i| i.time);
                if t - self.sources_refreshed_at > 2.0 {
                    self.sources_refreshed_at = t;
                    self.sources.wants_refresh = true;
                }
                crate::window_panel::source_grid(ui, &mut self.sources, &self.open_windows, accent);
            }
            Page::Desktop => {
                forward_toggle(ui, &mut self.forward_input, accent);
                ui.add_space(theme::GUTTER);
                if let Some(hit) = mirror_card(ui, self.mirror, &mut self.sources, self.forward_input, accent) {
                    self.pending_hit = Some(hit);
                }
            }
            Page::Audio => {
                media_card(ui, self.media.as_ref(), accent);
                ui.add_space(theme::GUTTER);
                self.volume.show(ui, accent);
            }
            Page::Chat => {
                if let Some(msg) = chatbox_card(ui, &mut self.chat_draft, &mut self.now_playing.enabled, accent) {
                    self.pending_chat = Some(msg);
                }
            }
            Page::Macros => self.macros.card(ui, injector, accent),
            Page::Layouts => {
                ui.checkbox(&mut self.restore_session, "Restore last session")
                    .on_hover_text("Reopen the windows and widgets you had open when VR last closed. A ★ startup slot takes precedence.");
                ui.add_space(6.0);
                if let Some(a) = crate::layouts_page::page(ui, &self.layouts, self.dock_layout, accent) {
                    self.layout_request = Some(a);
                }
            }
            Page::Notifications => {
                if history_card(ui, &self.history, &mut self.notif.muted, accent) {
                    self.clear_all_notifications();
                }
            }
            Page::Settings => {
                {
                    // Two columns when there's room; one when the window is
                    // narrow (label + slider + value rows overflowed narrow
                    // columns into their neighbour). Plain top-down layouts:
                    // ui.columns() letter-justifies wrapped text.
                    const TWO_COLUMNS_MIN: f32 = 960.0;
                    let plain = egui::Layout::top_down(egui::Align::Min);
                    let gap = theme::GUTTER;
                    let width = ui.available_width();
                    let narrow = width < TWO_COLUMNS_MIN;
                    let col_w = if narrow { width } else { (width - gap) / 2.0 };
                    let top = ui.cursor().min;
                    let bottom = ui.max_rect().bottom().max(top.y + 1.0);
                    let column = |min: egui::Pos2| egui::Rect::from_min_max(min, egui::pos2(min.x + col_w, bottom.max(min.y + 1.0)));
                    let mut l = ui.child_ui(column(top), plain);
                    let l = &mut l;
                    if appearance_card(
                        l,
                        &mut self.accent,
                        &mut self.show_mirror,
                        &mut self.show_media,
                        &mut self.show_chat,
                        &mut self.show_keyboard,
                        &mut self.show_history,
                        &mut self.show_battery,
                        &mut self.show_macros,
                        &mut self.show_volume,
                        &mut self.show_system,
                        &mut self.language,
                        &mut self.keyboard.layout,
                    ) {
                        self.reset_requested = true;
                    }
                    l.add_space(theme::GUTTER);
                    vr_card(l, &mut self.vr_left_handed, &mut self.vr_clip_angle, &mut self.vr_wrist_opacity, &mut self.vr_hide_wrist, accent);
                    l.add_space(theme::GUTTER);
                    vr_panels_card(l, &mut self.vr_hide_with_steamvr, &mut self.vr_grab_tilt, &mut self.vr_stick_axis_lock, &mut self.vr_render_scale, &mut self.vr_background_fps, &mut self.pointer, accent);
                    l.add_space(theme::GUTTER);
                    pointer_card(l, &mut self.scroll_speed, &mut self.pointer, accent);
                    l.add_space(theme::GUTTER);
                    window_input_card(l, &mut self.pointer, accent);
                    l.add_space(theme::GUTTER);
                    keyboard_card(l, &mut self.kb_prefs, accent);
                    l.add_space(theme::GUTTER);
                    accessibility_card(l, &mut self.cursor_smoothing, &mut self.vr_ui_scale, &mut self.vr_show_lasers, &mut self.vr_laser_angle, &mut self.vr_pointer_size, &mut self.reduce_motion, accent);
                    // Right column: beside the left one, or below it when narrow.
                    let r_min = if narrow { egui::pos2(top.x, l.min_rect().bottom() + gap) } else { egui::pos2(top.x + col_w + gap, top.y) };
                    let mut r = ui.child_ui(column(r_min), plain);
                    let r = &mut r;
                    wrist_look_card(r, &mut self.clock_format, &mut self.date_format, &mut self.wrist_bg, &mut self.media_theme, accent);
                    r.add_space(theme::GUTTER);
                    wrist_toolbar_card(r, &mut self.wrist_buttons, &mut self.wrist_compact, accent);
                    r.add_space(theme::GUTTER);
                    notifications_card(r, &mut self.notif, &mut self.pending_sounds, accent);
                    r.add_space(theme::GUTTER);
                    self.night_light.card(r, minute_of_day(), accent);
                    r.add_space(theme::GUTTER);
                    gamepad_card(r, &mut self.gamepad, self.gamepad_held, &mut self.gamepad_capture, accent);
                    r.add_space(theme::GUTTER);
                    self.updates.card(r, accent);
                    r.add_space(theme::GUTTER);
                    discord_card(r, &mut self.discord_presence, &mut self.discord, accent);
                    r.add_space(theme::GUTTER);
                    let snap = self.system.snapshot();
                    performance_card(r, snap.as_ref(), self.frame_ms, &self.frame_ms_hist, self.last_fps, self.notifications_received, accent);
                    r.add_space(theme::GUTTER);
                    // Rarely needed: tucked away, opened by search when a
                    // result lands in here.
                    let wanted = crate::widgets::highlighted_card(r.ctx()).is_some_and(|c| ["Placement", "Administrator", "Reset"].contains(&c.as_str()));
                    egui::CollapsingHeader::new(theme::body_muted("Advanced"))
                        .id_source("settings-advanced")
                        .open(wanted.then_some(true))
                        .show(r, |r| {
                            placement_card(r, &mut self.placement, accent);
                            r.add_space(theme::GUTTER);
                            if admin_card(r, &mut self.run_as_admin, self.elevated, accent) {
                                self.restart_elevated_requested = true;
                            }
                            r.add_space(theme::GUTTER);
                            if reset_card(r, &mut self.reset_armed_at) {
                                self.reset_requested = true;
                            }
                        });
                    // Hand the used area back so the scroll area knows the height.
                    ui.allocate_rect(l.min_rect().union(r.min_rect()), egui::Sense::hover());
                }
            }
        }
    }
}

/// Returns true when "Restart as admin now" is clicked.
fn admin_card(ui: &mut egui::Ui, run_as_admin: &mut bool, elevated: bool, accent: egui::Color32) -> bool {
    let mut restart = false;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::SETTINGS, "Administrator", accent);
        ui.add_space(6.0);
        ui.checkbox(run_as_admin, "Run as administrator")
            .on_hover_text("Windows blocks VR input to apps running as administrator unless RSOverlay is elevated too. Applies at next launch.");
        ui.add_space(4.0);
        if elevated {
            ui.label(egui::RichText::new("running elevated — input reaches admin windows").color(accent).size(theme::SIZE_CAPTION + 1.0));
        } else {
            ui.label(theme::body_muted("not elevated — admin windows are view-only"));
            if *run_as_admin && ui.button(egui::RichText::new("Restart as admin now").color(accent)).clicked() {
                restart = true;
            }
        }
    });
    restart
}

/// Two-step "Reset all settings": first click arms for 4 s, second
/// confirms. Returns true on confirm.
/// Discord Rich Presence on/off + live status.
fn discord_card(ui: &mut egui::Ui, on: &mut bool, share: &mut rsoverlay_core::config::DiscordCfg, accent: egui::Color32) {
    use rsoverlay_discord::Status;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::CHAT, "Discord", accent);
        ui.add_space(6.0);
        let status = rsoverlay_discord::status();
        ui.add_enabled_ui(status != Status::Unavailable, |ui| {
            ui.checkbox(on, "Show RSOverlay on my Discord profile").on_hover_text(
                "Friends see \"Using RSOverlay\" (Desktop / In VR) with a timer. Needs the Discord desktop app, with User Settings › Activity Privacy › Share my activity on.",
            );
        });
        ui.add_enabled_ui(status != Status::Unavailable && *on, |ui| {
            ui.indent("discord-share", |ui| {
                ui.checkbox(&mut share.show_windows, "Number of open windows").on_hover_text("\"In VR · 3 windows open\" — a count only, never window titles");
                ui.checkbox(&mut share.show_battery, "Headset battery").on_hover_text("\"Headset 82% · ~1h 40m left\"");
                ui.checkbox(&mut share.show_music, "What I'm listening to").on_hover_text("Song and artist, visible to everyone who can see your profile");
                ui.checkbox(&mut share.show_button, "\"Get RSOverlay\" button").on_hover_text("Lets friends open the project page (Discord hides it on your own view)");
            });
        });
        let (text, color) = match status {
            Status::Unavailable => ("Not available in this build", theme::FG_FAINT),
            Status::Off => ("Off", theme::FG_MUTED),
            Status::Waiting => ("Waiting for Discord — is the desktop app running?", theme::FG_MUTED),
            Status::Connected => ("Connected — showing on your profile", accent),
        };
        ui.label(egui::RichText::new(text).size(theme::SIZE_CAPTION + 1.0).color(color));
        // Status changes come from the worker thread: keep it fresh.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    });
}

fn reset_card(ui: &mut egui::Ui, armed_at: &mut Option<f64>) -> bool {
    let now = ui.input(|i| i.time);
    if armed_at.is_some_and(|t| now - t > 4.0) {
        *armed_at = None;
    }
    let mut confirmed = false;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::SETTINGS, "Reset", theme::DANGER);
        ui.add_space(6.0);
        let (label, color) = if armed_at.is_some() {
            ("Tap again to reset everything", theme::DANGER)
        } else {
            ("Reset all settings", theme::FG_PRIMARY)
        };
        let btn = egui::Button::new(egui::RichText::new(label).color(color)).min_size(egui::vec2(ui.available_width(), 36.0));
        if ui.add(btn).on_hover_text("Accent, cards, placement, night light, VR, macros — back to defaults").clicked() {
            if armed_at.is_some() {
                *armed_at = None;
                confirmed = true;
            } else {
                *armed_at = Some(now);
            }
        }
        if armed_at.is_some() {
            ui.ctx().request_repaint();
        }
    });
    confirmed
}

#[allow(clippy::too_many_arguments)]
fn accessibility_card(ui: &mut egui::Ui, steady: &mut f32, scale: &mut f32, lasers: &mut bool, laser_angle: &mut f32, pointer_size: &mut f32, reduce_motion: &mut bool, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::SETTINGS, "Accessibility", accent);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("VR text size"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(scale, 0.8..=1.6).fixed_decimals(1).suffix("×"));
        });
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Steady pointer"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(steady, 0.0..=1.0).fixed_decimals(2).custom_formatter(|v, _| match v {
                v if v < 0.01 => "Off".into(),
                v => format!("{:.0}%", v * 100.0),
            }));
        })
        .response
        .on_hover_text("For shaky hands: steadies the laser while you hold it still, and stays quick when you move on purpose. The beam, cursor, keys and clicks all follow the steadied laser.");
        ui.checkbox(lasers, "Laser beams").on_hover_text("Draw a beam from each controller while it points at a panel");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Laser angle"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(laser_angle, -30.0..=30.0).suffix("°").fixed_decimals(0));
        })
        .response
        .on_hover_text("Tilt the laser up/down to match how you hold your controller");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Pointer size"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(pointer_size, 0.5..=2.5).fixed_decimals(1).suffix("×"));
        })
        .response
        .on_hover_text("Size of the laser cursor and beam");
        ui.checkbox(reduce_motion, "Reduce motion").on_hover_text("Make page, toast and highlight transitions instant");
        ui.label(egui::RichText::new("Moving/resizing a panel always shows an accent outline + size readout, with a haptic tick on grab and release.")
            .size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED));
    });
}

fn vr_card(ui: &mut egui::Ui, left_handed: &mut bool, clip: &mut f32, wrist_opacity: &mut f32, hide_wrist: &mut bool, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::POINTER, "VR", accent);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Laser hand"));
            for (lh, label) in [(false, "Right"), (true, "Left")] {
                let sel = *left_handed == lh;
                if ui.selectable_label(sel, egui::RichText::new(label).color(if sel { theme::BG_ROOT } else { theme::FG_MUTED })).clicked() {
                    *left_handed = lh;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Hide beyond"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(clip, 30.0..=90.0).suffix("°").fixed_decimals(0));
        })
        .response
        .on_hover_text("Panels fade out when you look at them from further off-axis than this");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Wrist opacity"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(wrist_opacity, 0.2..=1.0).fixed_decimals(2));
        });
        ui.checkbox(hide_wrist, "Hide wrist toolbar")
            .on_hover_text("No wrist overlay at all. Open the dashboard with a double-press of A / X (or the menu button) on the wrist hand.");
    });
}

/// Gamepad (XInput) shortcuts, with "Set" capture: press the buttons
/// together, let go, and that combination is saved.
fn gamepad_card(
    ui: &mut egui::Ui,
    pad: &mut rsoverlay_core::config::GamepadCfg,
    held: u16,
    capture: &mut Option<(usize, u16)>,
    accent: egui::Color32,
) {
    use rsoverlay_input::gamepad::chord_name;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::CONTROLLER, "Gamepad", accent);
        ui.add_space(6.0);
        ui.checkbox(&mut pad.enabled, "Gamepad shortcuts")
            .on_hover_text("For sims and seated play with a controller: button combinations that open the dashboard, recenter or show the keyboard. Off by default so your game's controls are never touched.");
        // Capture: collect everything pressed until all buttons are let go.
        if let Some((i, acc)) = capture.as_mut() {
            if held != 0 {
                *acc |= held;
            } else if *acc != 0 {
                let chord = *acc;
                match *i {
                    0 => pad.dashboard = chord,
                    1 => pad.recenter = chord,
                    _ => pad.keyboard = chord,
                }
                *capture = None;
            }
        }
        if !pad.enabled {
            *capture = None;
            return;
        }
        ui.vertical(|ui| {
            let capturing = capture.map(|(i, _)| i);
            for (i, label) in ["Dashboard", "Recenter", "Keyboard"].into_iter().enumerate() {
                let chord = [pad.dashboard, pad.recenter, pad.keyboard][i];
                ui.horizontal(|ui| {
                    ui.label(theme::body_muted(label));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if capturing == Some(i) {
                            if ui.button("Cancel").clicked() {
                                *capture = None;
                            }
                            ui.label(egui::RichText::new("press the buttons together…").color(accent));
                        } else {
                            if ui.button("Set").on_hover_text("Then press the buttons together on your gamepad").clicked() {
                                *capture = Some((i, 0));
                            }
                            if chord != 0 && ui.button("Clear").clicked() {
                                match i {
                                    0 => pad.dashboard = 0,
                                    1 => pad.recenter = 0,
                                    _ => pad.keyboard = 0,
                                }
                            }
                            ui.label(egui::RichText::new(chord_name(chord)).monospace());
                        }
                    });
                });
            }
        });
        if pad.enabled && held != 0 {
            ui.label(theme::caption(&format!("Pressed now: {}", chord_name(held))));
        }
    });
}

/// How panels show, hide, move and render in VR.
#[allow(clippy::too_many_arguments)]
fn vr_panels_card(
    ui: &mut egui::Ui,
    hide_with_steamvr: &mut bool,
    grab_tilt: &mut bool,
    axis_lock: &mut bool,
    render_scale: &mut f32,
    background_fps: &mut f32,
    ptr: &mut PointerPrefs,
    accent: egui::Color32,
) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::LAYERS, "VR panels", accent);
        ui.add_space(6.0);
        ui.checkbox(hide_with_steamvr, "Hide while the SteamVR menu is open")
            .on_hover_text("Fade all RSOverlay panels out while SteamVR's own dashboard is up");
        ui.checkbox(grab_tilt, "Tilt panels with the controller")
            .on_hover_text("While grabbing, the panel rotates with your hand. Off: it stays upright and faces you.");
        ui.checkbox(&mut ptr.autohide_toolbars, "Auto-hide window toolbars")
            .on_hover_text("A window's toolbar (dock, opacity, size, close) only appears while your laser is on it. Cleaner for watching a game or video.");
        ui.checkbox(axis_lock, "One stick action at a time")
            .on_hover_text("While grabbing: push/pull (stick up/down) and resize (stick left/right) don't mix. The direction you push first wins until the stick returns to centre.");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Grab smoothing"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(&mut ptr.grab_smoothing, 0.0..=1.0).fixed_decimals(2));
        })
        .response
        .on_hover_text("Steadier panels while you carry them (higher = calmer, a little more lag)");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Vibration"));
            crate::theme::fit_slider(ui);
            let mut pct = (ptr.haptics * 100.0).round();
            if ui.add(egui::Slider::new(&mut pct, 0.0..=150.0).fixed_decimals(0).suffix("%")).changed() {
                ptr.haptics = pct / 100.0;
            }
        })
        .response
        .on_hover_text("Controller vibration on clicks and grabs (0% = off)");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Resolution"));
            for (v, label, tip) in [
                (1.0, "Standard", "Default sharpness"),
                (1.5, "High", "Sharper text for high-resolution headsets (more GPU memory)"),
                (2.0, "Ultra", "Sharpest text, e.g. Bigscreen Beyond or Pimax (about 4× the GPU memory)"),
            ] {
                let sel = (*render_scale - v).abs() < 0.01;
                if ui
                    .selectable_label(sel, egui::RichText::new(label).color(if sel { theme::BG_ROOT } else { theme::FG_MUTED }))
                    .on_hover_text(tip)
                    .clicked()
                {
                    *render_scale = v;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Unwatched panels"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(background_fps, 1.0..=90.0).fixed_decimals(0).suffix(" fps"));
        })
        .response
        .on_hover_text("How often panels you aren't looking at refresh. Lower saves GPU for your game; raise it to watch a video out of the corner of your eye.");
    });
}

/// How the wrist looks: clock / date format, background image, and
/// the album-art accent.
fn wrist_look_card(
    ui: &mut egui::Ui,
    clock: &mut String,
    date: &mut String,
    bg: &mut crate::backgrounds::WristBackground,
    media_theme: &mut bool,
    accent: egui::Color32,
) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::CLOCK, "Wrist look", accent);
        ui.add_space(6.0);
        for (label, value, table) in [("Clock", &mut *clock, &crate::locale::CLOCK_FORMATS[..]), ("Date", &mut *date, &crate::locale::DATE_FORMATS[..])] {
            ui.horizontal(|ui| {
                ui.label(theme::body_muted(label));
                for (slug, text) in table {
                    let sel = value.as_str() == *slug;
                    if ui.selectable_label(sel, egui::RichText::new(*text).color(if sel { theme::BG_ROOT } else { theme::FG_MUTED })).clicked() {
                        *value = slug.to_string();
                    }
                }
            });
        }
        // Background image behind the clock + media.
        if !bg.scanned {
            bg.scanned = true;
            bg.files = crate::backgrounds::list(&crate::backgrounds::dir());
        }
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Background"));
            let shown = if bg.file.is_empty() { "None".to_string() } else { bg.file.clone() };
            egui::ComboBox::from_id_source("wrist-bg").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut bg.file, String::new(), "None");
                for f in bg.files.clone() {
                    ui.selectable_value(&mut bg.file, f.clone(), f);
                }
            });
            if ui.button("Refresh").clicked() {
                bg.files = crate::backgrounds::list(&crate::backgrounds::dir());
            }
            if ui.button("Open folder").on_hover_text("Drop PNG or JPG images here, then Refresh").clicked() {
                let d = crate::backgrounds::dir();
                let _ = std::fs::create_dir_all(&d);
                crate::backgrounds::open_folder(&d);
                bg.files = crate::backgrounds::list(&d);
            }
        });
        if !bg.file.is_empty() {
            ui.horizontal(|ui| {
                ui.label(theme::body_muted("Darken"));
                crate::theme::fit_slider(ui);
                ui.add(egui::Slider::new(&mut bg.dim, 0.0..=0.9).fixed_decimals(2));
            })
            .response
            .on_hover_text("Darkens the image so the clock stays easy to read");
        }
        ui.checkbox(media_theme, "Media theme").on_hover_text("Accent colour follows the album art while music plays");
    });
}

/// What the wrist shows. Modules and status chips up front; the button
/// lists fold away (they're also editable in VR: More, then Edit).
fn wrist_toolbar_card(ui: &mut egui::Ui, mask: &mut u16, compact: &mut u32, accent: egui::Color32) {
    use crate::wrist::compact as c;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::GRID, "Wrist toolbar", accent);
        ui.add_space(6.0);
        ui.label(theme::body_muted("Compact wrist shows"))
            .on_hover_text("What the wrist shows before you press More. In VR: More, then Edit, and tap items to show / hide them.");
        ui.horizontal_wrapped(|ui| {
            for &(bit, name) in c::SECTIONS {
                compact_toggle(ui, compact, bit, name);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::caption("Status chips"));
            for &(bit, name) in c::CHIPS {
                compact_toggle(ui, compact, bit, name);
            }
        });
        egui::CollapsingHeader::new(theme::body_muted("Customise buttons")).id_source("wrist-buttons").show(ui, |ui| {
            ui.label(theme::caption("Compact wrist"));
            ui.horizontal_wrapped(|ui| {
                for (i, name) in c::ACTION_NAMES.iter().enumerate() {
                    compact_toggle(ui, compact, c::action_bit(i), name);
                }
            });
            ui.add_space(4.0);
            ui.label(theme::caption("Full toolbar (after More)"));
            ui.horizontal_wrapped(|ui| {
                for (i, name) in c::ACTION_NAMES.iter().enumerate() {
                    let mut on = *mask & (1 << i) != 0;
                    if ui.checkbox(&mut on, *name).changed() {
                        *mask ^= 1 << i;
                        if *mask & c::ALL_ACTIONS == 0 {
                            *mask |= 1 << 3; // keep at least the dashboard button
                        }
                    }
                }
            });
        });
        if ui.small_button("Default compact set").clicked() {
            *compact = rsoverlay_core::config::DEFAULT_WRIST_COMPACT;
        }
    });
}

fn compact_toggle(ui: &mut egui::Ui, compact: &mut u32, bit: u32, name: &str) {
    let mut on = *compact & bit != 0;
    if ui.checkbox(&mut on, name).changed() {
        *compact ^= bit;
    }
}

/// VR keyboard look + typing sounds.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardPrefs {
    pub opacity: f32,
    pub sounds: bool,
    pub volume: f32,
    /// Pack folder name; empty = built-in clicks.
    pub pack: String,
    /// Packs found under the sounds folder (rescanned on Refresh).
    pub packs: Vec<String>,
}

impl Default for KeyboardPrefs {
    fn default() -> Self {
        Self { opacity: 1.0, sounds: true, volume: 0.4, pack: String::new(), packs: Vec::new() }
    }
}

fn keyboard_card(ui: &mut egui::Ui, kb: &mut KeyboardPrefs, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::KEYBOARD, "Keyboard", accent);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Opacity"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(&mut kb.opacity, 0.2..=1.0).fixed_decimals(2));
        });
        ui.checkbox(&mut kb.sounds, "Typing sounds").on_hover_text("A short click on every key press of the VR keyboard");
        ui.add_enabled_ui(kb.sounds, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::body_muted("Volume"));
                crate::theme::fit_slider(ui);
                ui.add(egui::Slider::new(&mut kb.volume, 0.0..=1.0).fixed_decimals(2));
            });
            ui.horizontal(|ui| {
                ui.label(theme::body_muted("Sound pack"));
                let classic = rsoverlay_audio::typing::CLASSIC_PACK;
                let current = match kb.pack.as_str() {
                    "" => "Thock (built-in)".to_string(),
                    p if p == classic => "Classic clicks".to_string(),
                    p => p.to_string(),
                };
                egui::ComboBox::from_id_source("sound-pack").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut kb.pack, String::new(), "Thock (built-in)");
                    ui.selectable_value(&mut kb.pack, classic.to_string(), "Classic clicks");
                    for p in &kb.packs {
                        ui.selectable_value(&mut kb.pack, p.clone(), p);
                    }
                });
                if ui.small_button("Refresh").clicked() {
                    kb.packs = rsoverlay_audio::typing::list_packs(&rsoverlay_core::config::sounds_dir());
                }
            })
            .response
            .on_hover_text(format!(
                "Add a pack: a folder in {} with key.wav, space.wav, enter.wav, backspace.wav and/or modifier.wav (16-bit PCM). Missing files use the built-in click.",
                rsoverlay_core::config::sounds_dir().display()
            ));
        });
    });
}

/// Laser click helpers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerPrefs {
    pub long_press: bool,
    pub long_press_s: f32,
    pub double_click_s: f32,
    /// Controller vibration strength (0 = off, 1 = default).
    pub haptics: f32,
    /// Grab smoothing: 0 = raw hand motion, 1 = heavy dampening.
    pub grab_smoothing: f32,
    /// The real mouse follows the laser over a window (off: it only
    /// moves when you click or drag).
    pub mouse_follows: bool,
    /// Windows get touch input (the trigger is a finger) instead of
    /// mouse clicks.
    pub touch_input: bool,
    /// Experimental: the game doesn't get controller input while the
    /// laser is on a panel.
    pub block_game_input: bool,
    /// Experimental: SteamVR's own laser drives the panels.
    pub steamvr_laser: bool,
    /// Trigger pull that clicks (0.2..1; 1 = the trigger's own click).
    pub click_point: f32,
    /// Window toolbars only while the laser is on the window.
    pub autohide_toolbars: bool,
}

impl Default for PointerPrefs {
    fn default() -> Self {
        Self { long_press: false, long_press_s: 0.6, double_click_s: 0.4, haptics: 1.0, grab_smoothing: 0.3, mouse_follows: true, touch_input: false, block_game_input: false, steamvr_laser: false, click_point: 0.7, autohide_toolbars: false }
    }
}

impl PointerPrefs {
    /// What the laser state machine uses (0 = off).
    pub fn long_press_secs(&self) -> f32 {
        if self.long_press { self.long_press_s } else { 0.0 }
    }
}

/// VR toast placement + alert sound slots.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationPrefs {
    pub size_m: f32,
    pub offset_y: f32,
    pub distance_m: f32,
    pub volume: f32,
    /// Per [`rsoverlay_audio::alerts::AlertKind::ALL`]: "builtin", "none"
    /// or a file name in the sounds folder.
    pub sounds: [String; 4],
    /// WAVs found in the sounds folder (rescanned on Refresh).
    pub files: Vec<String>,
    /// Muted source apps: history only, no toast or sound.
    pub muted: Vec<String>,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self { size_m: 0.30, offset_y: -0.12, distance_m: 0.65, volume: 0.6, sounds: std::array::from_fn(|_| "builtin".into()), files: Vec::new(), muted: Vec::new() }
    }
}

impl NotificationPrefs {
    pub fn sound_for(&self, kind: rsoverlay_audio::alerts::AlertKind) -> &str {
        let i = rsoverlay_audio::alerts::AlertKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
        &self.sounds[i]
    }
}

fn notifications_card(
    ui: &mut egui::Ui,
    n: &mut NotificationPrefs,
    test: &mut Vec<(rsoverlay_audio::alerts::AlertKind, f32)>,
    accent: egui::Color32,
) {
    use rsoverlay_audio::alerts::{AlertKind, BUILTIN, NONE};
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::BELL, "Notifications", accent);
        ui.add_space(6.0);
        ui.label(theme::body_muted("In VR, toasts float in front of you:"));
        let slider = |ui: &mut egui::Ui, label: &str, v: &mut f32, r: std::ops::RangeInclusive<f32>, suffix: &str| {
            ui.horizontal(|ui| {
                ui.label(theme::body_muted(label));
                crate::theme::fit_slider(ui);
                ui.add(egui::Slider::new(v, r).fixed_decimals(2).suffix(suffix));
            });
        };
        slider(ui, "Size", &mut n.size_m, 0.15..=0.6, " m");
        slider(ui, "Height", &mut n.offset_y, -0.4..=0.2, " m");
        slider(ui, "Distance", &mut n.distance_m, 0.3..=1.5, " m");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Sound volume"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(&mut n.volume, 0.0..=1.0).fixed_decimals(2));
        });
        egui::Grid::new("alert-sounds").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
        for (i, kind) in AlertKind::ALL.into_iter().enumerate() {
            {
                ui.label(theme::body_muted(kind.label()));
                let shown = match n.sounds[i].as_str() {
                    BUILTIN => "Built-in".to_string(),
                    NONE => "Silent".to_string(),
                    f => f.to_string(),
                };
                egui::ComboBox::from_id_source(("alert-sound", i)).selected_text(shown).width(180.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut n.sounds[i], BUILTIN.to_string(), "Built-in");
                    ui.selectable_value(&mut n.sounds[i], NONE.to_string(), "Silent");
                    for f in &n.files {
                        ui.selectable_value(&mut n.sounds[i], f.clone(), f);
                    }
                });
                if ui.small_button("▶").on_hover_text("Test").clicked() {
                    test.push((kind, 1.0));
                }
                ui.end_row();
            }
        }
        });
        ui.horizontal(|ui| {
            let dir = rsoverlay_core::config::sounds_dir();
            if ui.small_button("Refresh").clicked() {
                n.files = rsoverlay_audio::alerts::list_sound_files(&dir);
            }
            if ui.small_button("Open sounds folder").clicked() {
                let _ = std::fs::create_dir_all(&dir);
                #[cfg(windows)]
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        })
        .response
        .on_hover_text(format!(
            "Drop .wav files (16-bit PCM, up to 2 MB) into {} and press Refresh. Apps sending notifications pick the category (default / warning / error); you pick the sound.",
            rsoverlay_core::config::sounds_dir().display()
        ));
    });
}

/// How desktop windows in VR receive the laser.
fn window_input_card(ui: &mut egui::Ui, ptr: &mut PointerPrefs, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::MONITOR, "Window input", accent);
        ui.add_space(6.0);
        ui.checkbox(&mut ptr.mouse_follows, "Mouse follows the laser")
            .on_hover_text("On: pointing at a window moves your real mouse (hover menus, tooltips). Off: the mouse only moves when you click or drag, so it stays put while you look around.");
        ui.checkbox(&mut ptr.touch_input, "Touch input")
            .on_hover_text("The trigger touches the window like a finger (tap, drag, swipe) instead of clicking a mouse. For touch-friendly apps; right / middle click and scrolling still use the mouse.");
        ui.checkbox(&mut ptr.block_game_input, "Block game input while pointing (experimental)")
            .on_hover_text("While your laser is on a panel, the game doesn't get your trigger / button presses (no accidental shots or menu clicks). Uses SteamVR's laser-mouse mode, which may show SteamVR's own pointer too. Please report how it behaves.");
        ui.checkbox(&mut ptr.steamvr_laser, "Use SteamVR's laser (experimental)")
            .on_hover_text("SteamVR draws the laser and decides clicks on RSOverlay's panels. The game doesn't get controller input while you point at a panel. Known to miss clicks on some setups; off uses RSOverlay's own laser.");
    });
}

fn pointer_card(ui: &mut egui::Ui, scroll: &mut f32, ptr: &mut PointerPrefs, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::POINTER, "Pointer", accent);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Click point"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(&mut ptr.click_point, 0.3..=1.0).fixed_decimals(2).custom_formatter(|v, _| match v {
                v if v >= 0.995 => "Full click".into(),
                v => format!("{:.0}%", v * 100.0),
            }));
        })
        .response
        .on_hover_text("How far you pull the trigger to click. Full click = only the trigger's own click (Index). Lower = lighter clicks.");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Scroll speed"));
            crate::theme::fit_slider(ui);
            ui.add(egui::Slider::new(scroll, 0.25..=3.0).fixed_decimals(2).suffix("×"));
        });
        ui.checkbox(&mut ptr.long_press, "Long-press for right click")
            .on_hover_text("Hold the trigger still to right-click. Moving first still drags; letting go early still clicks.");
        ui.add_enabled_ui(ptr.long_press, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::body_muted("Long-press time"));
                crate::theme::fit_slider(ui);
                ui.add(egui::Slider::new(&mut ptr.long_press_s, 0.3..=1.5).fixed_decimals(1).suffix(" s"));
            });
        });
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Double-click delay"));
            crate::theme::fit_slider(ui);
            let mut ms = ptr.double_click_s * 1000.0;
            if ui.add(egui::Slider::new(&mut ms, 0.0..=800.0).fixed_decimals(0).suffix(" ms")).changed() {
                ptr.double_click_s = ms / 1000.0;
            }
        })
        .response
        .on_hover_text("A second click this soon after the first, and close to it, lands exactly on it, so hand shake doesn't break double-clicks. 0 = off.");
    });
}

fn minute_of_day() -> u16 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    (now.hour() * 60 + now.minute()) as u16
}

fn media_card(ui: &mut egui::Ui, info: Option<&MediaInfo>, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::PULSE, "Now playing", accent);
        ui.add_space(6.0);
        match info {
            Some(m) if !m.title.is_empty() => {
                ui.label(theme::card_title(&m.title));
                if !m.artist.is_empty() {
                    ui.label(theme::body_muted(&m.artist));
                }
                ui.add_space(8.0);
                media_controls(ui, m.playing, accent);
            }
            _ => {
                ui.label(theme::body_muted("nothing playing"));
                ui.add_space(8.0);
                // Controls stay visible even when SMTC reports nothing —
                // some sessions report "stopped" while the key presses
                // still start playback.
                media_controls(ui, false, accent);
            }
        }
    });
}

fn media_controls(ui: &mut egui::Ui, playing: bool, accent: egui::Color32) {
    ui.horizontal(|ui| {
        let btn = |label: &str| {
            egui::Button::new(
                egui::RichText::new(label)
                    .color(theme::FG_PRIMARY)
                    .size(16.0),
            )
            .min_size(egui::vec2(32.0, 28.0))
        };
        if ui.add(btn("⏮")).on_hover_text("Previous track").clicked() {
            media::send(MediaCommand::Previous);
        }
        let play_pause = if playing { "⏸" } else { "▶" };
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new(play_pause)
                        .color(accent)
                        .size(18.0),
                )
                .min_size(egui::vec2(40.0, 30.0)),
            )
            .on_hover_text(if playing { "Pause" } else { "Play" })
            .clicked()
        {
            media::send(MediaCommand::PlayPause);
        }
        if ui.add(btn("⏭")).on_hover_text("Next track").clicked() {
            media::send(MediaCommand::Next);
        }
    });
}

pub(crate) fn card_header(
    ui: &mut egui::Ui,
    source: egui::ImageSource<'static>,
    label: &str,
    accent: egui::Color32,
) {
    ui.horizontal(|ui| {
        ui.add(icon(source, 18.0, crate::theme::FG_MUTED));
        ui.add_space(6.0);
        ui.label(theme::card_title(label));
        crate::widgets::popout_button(ui, label, accent);
    });
}

/// RSOverlay's own footprint, measured live — the "how light are we"
/// card. Only our own numbers; no claims we can't measure here.
fn performance_card(
    ui: &mut egui::Ui,
    snap: Option<&rsoverlay_sysinfo::Snapshot>,
    frame_ms: f32,
    hist: &VecDeque<f32>,
    fps: f32,
    notifs: u64,
    accent: egui::Color32,
) {
    use rsoverlay_sysinfo::format_bytes;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::PULSE, "RSOverlay performance", accent);
        ui.label(theme::body_muted("RSOverlay's own footprint, for the curious. Your PC and game: the PC performance card."));
        ui.add_space(8.0);
        let own = snap.map(|s| s.own).unwrap_or_default();
        let tiles = [
            ("CPU", own.cpu_pct.map_or("—".into(), |c| format!("{c:.1}%"))),
            ("RAM", if own.ram > 0 { format_bytes(own.ram) } else { "—".into() }),
            ("VRAM", own.vram.map_or("—".into(), format_bytes)),
            ("Frame", format!("{frame_ms:.2} ms")),
        ];
        ui.columns(4, |cols| {
            for (c, (k, v)) in cols.iter_mut().zip(tiles) {
                c.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(v).size(20.0).strong().color(accent));
                    ui.label(egui::RichText::new(k).size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED));
                });
            }
        });
        ui.add_space(8.0);
        // Frame-time sparkline (last ~2 s of frames).
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 8.0, crate::theme::BG_RAISED);
        if hist.len() > 1 {
            let top = hist.iter().copied().fold(1.0_f32, f32::max);
            let pts: Vec<egui::Pos2> = hist
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    egui::pos2(
                        r.left() + 6.0 + (r.width() - 12.0) * i as f32 / (hist.len() - 1) as f32,
                        r.bottom() - 6.0 - (r.height() - 12.0) * (v / top).min(1.0),
                    )
                })
                .collect();
            ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.5_f32, accent)));
            ui.painter().text(r.right_top() + egui::vec2(-8.0, 6.0), egui::Align2::RIGHT_TOP, format!("peak {top:.1} ms"),
                egui::FontId::monospace(theme::SIZE_CAPTION), theme::FG_MUTED);
        }
        ui.add_space(8.0);
        for badge in [
            "GPU-to-GPU capture — zero CPU frame copies",
            "Native Rust — no browser engine",
            "Hidden panels cost nothing — not rendered",
        ] {
            ui.horizontal_top(|ui| {
                ui.painter().circle_filled(ui.cursor().min + egui::vec2(4.0, 9.0), 3.0, accent);
                ui.add_space(14.0);
                ui.add(egui::Label::new(egui::RichText::new(badge).size(theme::SIZE_CAPTION + 2.0).color(theme::FG_PRIMARY)).wrap(true));
            });
        }
        ui.add_space(6.0);
        kv(ui, "overlay fps", &format!("{fps:.0}"));
        kv(ui, "notifications", &notifs.to_string());
    });
}

#[allow(dead_code)]
fn stats_card(ui: &mut egui::Ui, frame_count: u64, fps: f32, notifs: u64, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::SETTINGS, "Stats", accent);
        ui.add_space(6.0);
        kv(ui, "frame", &frame_count.to_string());
        kv(ui, "fps", &format!("{:.1}", fps));
        kv(ui, "notifications", &notifs.to_string());
        kv(ui, "ws bind", "127.0.0.1:42069");
    });
}

fn placement_card(ui: &mut egui::Ui, p: &mut Placement, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::POINTER, "Placement", accent);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for a in [Anchor::Wrist, Anchor::Hmd, Anchor::World] {
                let selected = p.anchor == a;
                let label = egui::RichText::new(a.label())
                    .color(if selected { theme::BG_ROOT } else { theme::FG_MUTED });
                if ui.selectable_label(selected, label).clicked() {
                    p.anchor = a;
                }
            }
        });
        ui.add_space(6.0);
        placement_slider(ui, "X", &mut p.offset_x, -0.6, 0.6);
        placement_slider(ui, "Y", &mut p.offset_y, 0.8, 2.0);
        placement_slider(ui, "Z", &mut p.offset_z, -1.0, 0.0);
        ui.add_space(4.0);
        placement_slider(ui, "W", &mut p.size_w, 0.1, 0.8);
        placement_slider(ui, "H", &mut p.size_h, 0.08, 0.6);
        ui.add_space(4.0);
        placement_slider(ui, "Curve", &mut p.curvature, 0.0, 1.0);
        placement_slider(ui, "Opacity", &mut p.opacity, 0.2, 1.0);
    });
}

fn placement_slider(ui: &mut egui::Ui, label: &str, v: &mut f32, lo: f32, hi: f32) {
    ui.horizontal(|ui| {
        ui.label(theme::body_muted(label));
        ui.add(
            egui::Slider::new(v, lo..=hi)
                .fixed_decimals(2)
                .show_value(true),
        );
    });
}

/// Scrollable history of notifications received this session. Returns
/// true when the user hit "Clear all" so the caller can wipe history +
/// any still-visible toasts in one shot.
/// Muted apps are capped so a misbehaving client can't grow the config.
const MAX_MUTED: usize = 50;

fn history_card(
    ui: &mut egui::Ui,
    history: &VecDeque<HistoryEntry>,
    muted: &mut Vec<String>,
    accent: egui::Color32,
) -> bool {
    let mut cleared = false;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            card_header(ui, icons::BELL, "Notifications", accent);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let clear = ui.add_enabled(
                    !history.is_empty(),
                    egui::Button::new(
                        egui::RichText::new("Clear all")
                            .color(if history.is_empty() {
                                theme::FG_MUTED
                            } else {
                                accent
                            })
                            .size(theme::SIZE_CAPTION)
                            .extra_letter_spacing(1.0),
                    ),
                );
                if clear.clicked() {
                    cleared = true;
                }
            });
        });
        ui.add_space(6.0);
        if !muted.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::body_muted("Muted:"));
                let mut unmute = None;
                for app in muted.iter() {
                    if ui.small_button(format!("{app}  ✕")).on_hover_text("Unmute: show toasts and play sounds again").clicked() {
                        unmute = Some(app.clone());
                    }
                }
                if let Some(a) = unmute {
                    muted.retain(|m| *m != a);
                }
            });
            ui.add_space(4.0);
        }
        if history.is_empty() {
            ui.label(theme::body_muted("no notifications yet"));
            return;
        }
        egui::ScrollArea::vertical()
            .id_source("history")
            .max_height(560.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut mute = None;
                for entry in history.iter().rev() {
                    if history_entry_row(ui, entry, muted) {
                        mute = Some(entry.n.source_app.clone());
                    }
                    ui.add_space(4.0);
                }
                if let Some(app) = mute.filter(|_| muted.len() < MAX_MUTED) {
                    muted.push(app.chars().take(64).collect());
                }
            });
    });
    cleared
}

/// One history line. Returns true when "Mute <app>" was clicked.
fn history_entry_row(ui: &mut egui::Ui, entry: &HistoryEntry, muted: &[String]) -> bool {
    let rel = relative_time(entry.received.elapsed());
    let app = entry.n.source_app.as_str();
    let mut mute = false;
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            if !entry.n.title.is_empty() {
                ui.label(
                    egui::RichText::new(&entry.n.title)
                        .strong()
                        .color(theme::FG_PRIMARY),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(rel)
                        .size(theme::SIZE_CAPTION)
                        .color(theme::FG_MUTED),
                );
                if !app.is_empty() && !muted.iter().any(|m| m == app) {
                    mute = ui
                        .small_button("Mute")
                        .on_hover_text(format!("Mute {app}: its notifications go to history only, no toast or sound"))
                        .clicked();
                }
                if !app.is_empty() {
                    ui.label(egui::RichText::new(app).size(theme::SIZE_CAPTION).color(theme::FG_MUTED));
                }
            });
        });
        if !entry.n.content.is_empty() {
            ui.label(theme::body_muted(&entry.n.content));
        }
    });
    mute
}

pub fn relative_time_secs(secs: u64) -> String {
    relative_time(Duration::from_secs(secs))
}

fn relative_time(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Returns `true` when the user clicked the "Reset defaults" button —
/// the caller resets its DevView to defaults + re-saves config.
#[allow(clippy::too_many_arguments)]
fn appearance_card(
    ui: &mut egui::Ui,
    accent: &mut egui::Color32,
    show_mirror: &mut bool,
    show_media: &mut bool,
    show_chat: &mut bool,
    show_keyboard: &mut bool,
    show_history: &mut bool,
    show_battery: &mut bool,
    show_macros: &mut bool,
    show_volume: &mut bool,
    show_system: &mut bool,
    language: &mut crate::i18n::Lang,
    keyboard_layout: &mut crate::keyboard_layout::LayoutId,
) -> bool {
    let mut reset_clicked = false;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            card_header(ui, icons::SETTINGS, "Appearance", *accent);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn = egui::Button::new(
                    egui::RichText::new("Reset defaults")
                        .color(theme::FG_MUTED)
                        .size(theme::SIZE_CAPTION)
                        .extra_letter_spacing(1.0),
                );
                if ui
                    .add(btn)
                    .on_hover_text(
                        "Clear accent, placement, card toggles back to defaults",
                    )
                    .clicked()
                {
                    reset_clicked = true;
                }
            });
        });
        ui.add_space(6.0);
        // One row: all accent swatches + custom color picker, inline.
        ui.horizontal_wrapped(|ui| {
            for (name, preset) in theme::ACCENT_PRESETS {
                let selected = *accent == *preset;
                let size = egui::vec2(20.0, 20.0);
                let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                let painter = ui.painter();
                let center = rect.center();
                painter.circle_filled(center, 8.0, *preset);
                if selected {
                    painter.circle_stroke(
                        center,
                        10.0,
                        egui::Stroke::new(1.5_f32, theme::FG_PRIMARY),
                    );
                }
                resp.on_hover_text(*name).clicked().then(|| *accent = *preset);
            }
            ui.color_edit_button_srgba(accent);
        });
        ui.add_space(6.0);
        // One row: all card show/hide checkboxes side by side.
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(show_mirror, "Mirror");
            ui.checkbox(show_media, "Media");
            ui.checkbox(show_chat, "Chat");
            ui.checkbox(show_keyboard, "Keyboard");
            ui.checkbox(show_history, "History");
            ui.checkbox(show_battery, "Battery");
            ui.checkbox(show_macros, "Macros");
            ui.checkbox(show_volume, "Volume");
            ui.checkbox(show_system, "PC performance");
        });
        ui.add_space(8.0);
        // Language picker — shows the native-script name so a Russian
        // user sees Русский. Hidden until the UI text is actually
        // translated (see `i18n::UI_TRANSLATED`).
        if crate::i18n::UI_TRANSLATED {
            egui::ComboBox::from_label("Language")
                .selected_text(language.native_name())
                .show_ui(ui, |ui| {
                    for l in crate::i18n::Lang::all() {
                        ui.selectable_value(language, *l, l.native_name());
                    }
                });
        }
        egui::ComboBox::from_label("Layout")
            .selected_text(keyboard_layout.label())
            .show_ui(ui, |ui| {
                for l in crate::keyboard_layout::LayoutId::all() {
                    ui.selectable_value(keyboard_layout, *l, l.label());
                }
            });
    });
    reset_clicked
}

fn chatbox_card(
    ui: &mut egui::Ui,
    draft: &mut String,
    share_now_playing: &mut bool,
    accent: egui::Color32,
) -> Option<rsoverlay_osc::ChatMessage> {
    let mut result = None;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::CHAT, "VRChat chatbox", accent);
        ui.add_space(6.0);
        let edit = ui.add(
            egui::TextEdit::singleline(draft)
                .desired_width(f32::INFINITY)
                .hint_text("message VRChat chatbox…"),
        );
        // Own row: inside a justified column a bare button would stretch
        // to full width.
        let clicked = ui
            .horizontal(|ui| ui.button(egui::RichText::new("send").color(accent)).clicked())
            .inner;
        let submit = (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || clicked;
        if submit && !draft.trim().is_empty() {
            result = Some(rsoverlay_osc::ChatMessage {
                text: draft.clone(),
                send_immediately: true,
                play_notification: true,
            });
            draft.clear();
        }
        ui.add_space(4.0);
        ui.checkbox(share_now_playing, "share now playing")
            .on_hover_text("Post ♪ Title — Artist to the chatbox on track change");
    });
    result
}

fn forward_toggle(ui: &mut egui::Ui, on: &mut bool, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::POINTER, "Input forwarding", accent);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.checkbox(on, "");
            ui.label(theme::body(if *on {
                "clicks on the mirror reach the real desktop"
            } else {
                "clicks on the mirror are inert"
            }));
        });
    });
}

fn kv(ui: &mut egui::Ui, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(theme::body_muted(k));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(egui::RichText::new(v).color(theme::FG_PRIMARY).size(theme::SIZE_BODY));
        });
    });
}

fn mirror_card(
    ui: &mut egui::Ui,
    mirror: Option<MirrorHandle>,
    sources: &mut SourcePicker,
    forward_input: bool,
    accent: egui::Color32,
) -> Option<MirrorHit> {
    let mut hit = None;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            card_header(ui, icons::MONITOR, "Desktop", accent);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                source_combo(ui, sources);
            });
        });
        ui.add_space(6.0);
        match mirror {
            Some(m) => {
                let avail = ui.available_size();
                let aspect = m.width as f32 / m.height as f32;
                let max_w = avail.x;
                let max_h = (avail.y - 4.0).max(160.0);
                let (w, h) = fit(aspect, max_w, max_h);
                let sense = if forward_input {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::hover()
                };
                let response = ui.add(
                    egui::Image::new((m.id, egui::vec2(w, h))).uv(m.uv).sense(sense),
                );
                paint_mirror_cursor(ui, response.rect, m.cursor);
                if forward_input {
                    if let Some(pos) = response.hover_pos().or_else(|| response.interact_pointer_pos()) {
                        let rect = response.rect;
                        let u = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                        let v = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                        let px = (u * m.width as f32) as i32;
                        let py = (v * m.height as f32) as i32;
                        let clicked = response.clicked();
                        hit = Some(MirrorHit { x: px, y: py, clicked });
                    }
                }
            }
            None => {
                ui.label(theme::body_muted("no monitor captured"));
            }
        }
    });
    hit
}

fn source_combo(ui: &mut egui::Ui, sources: &mut SourcePicker) {
    let selected = sources.current.as_ref().map_or_else(|| "none".to_string(), |s| s.label());
    let open = egui::ComboBox::from_id_source("capture-source")
        .width(240.0)
        .selected_text(truncate_label(&selected, 34))
        .show_ui(ui, |ui| {
            for choice in &sources.choices {
                let is_current = sources.current.as_ref() == Some(choice);
                if ui.selectable_label(is_current, truncate_label(&choice.label(), 48)).clicked() && !is_current {
                    sources.request = Some(choice.clone());
                }
            }
        })
        .inner
        .is_some();
    if open {
        sources.wants_refresh = true;
    }
}

fn truncate_label(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}…", &s[..i]),
    }
}

fn fit(aspect: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    let w_by_h = max_h * aspect;
    if w_by_h <= max_w {
        (w_by_h, max_h)
    } else {
        (max_w, max_w / aspect)
    }
}

fn toast_card(ui: &mut egui::Ui, n: &Notification, accent: egui::Color32) {
    theme::pill().show(ui, |ui| {
        ui.set_width(260.0);
        ui.horizontal(|ui| {
            ui.add(icon(icons::BELL, 26.0, accent));
            ui.add_space(8.0);
            ui.vertical(|ui| {
                if !n.title.is_empty() {
                    ui.label(theme::card_title(&n.title));
                }
                if !n.content.is_empty() {
                    ui.label(theme::body(&n.content));
                }
                if !n.source_app.is_empty() {
                    ui.label(theme::caption(&n.source_app));
                }
            });
        });
    });
}

#[cfg(test)]
mod laser_integration_tests {
    use super::*;
    use crate::laser::{Hand, HandSample, LaserInput};
    use crate::shell::Page;

    const PANEL: egui::Vec2 = egui::vec2(1365.0, 853.0);

    fn view() -> DevView {
        // No worker threads in tests: hide the cards that spawn them.
        DevView { show_volume: false, show_system: false, ..Default::default() }
    }

    /// Run one frame fed only by laser samples.
    fn frame(ctx: &egui::Context, dev: &mut DevView, laser: &mut LaserInput, t: &mut f64, hit: Option<egui::Pos2>, trigger: f32) {
        let lf = laser.update(&[HandSample { hand: Hand::Right, hit, trigger, stick_x: 0.0, stick_y: 0.0, button: egui::PointerButton::Primary }], 1.0 / 90.0);
        *t += 1.0 / 90.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, PANEL)),
            time: Some(*t),
            events: lf.events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| dev.ui_with(ctx, None));
    }

    fn click(ctx: &egui::Context, dev: &mut DevView, laser: &mut LaserInput, t: &mut f64, at: egui::Pos2) {
        frame(ctx, dev, laser, t, Some(at), 0.0); // hover
        frame(ctx, dev, laser, t, Some(at), 0.9); // pull
        frame(ctx, dev, laser, t, Some(at), 0.0); // release
    }

    /// First rail y (scanning down) where a laser click opens Settings.
    fn settings_y(ctx: &egui::Context, dev: &mut DevView, laser: &mut LaserInput, t: &mut f64) -> Option<f32> {
        (60..800).step_by(6).map(|y| y as f32).find(|&y| {
            dev.page = Page::Home;
            click(ctx, dev, laser, t, egui::pos2(80.0, y));
            dev.page == Page::Settings
        })
    }

    #[test]
    fn laser_click_drives_nav_rail() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let (mut dev, mut laser, mut t) = (view(), LaserInput::default(), 0.0);
        frame(&ctx, &mut dev, &mut laser, &mut t, None, 0.0);
        let y = settings_y(&ctx, &mut dev, &mut laser, &mut t);
        assert!(y.is_some(), "no laser click on the rail opened Settings");
    }

    #[test]
    fn held_trigger_swept_onto_rail_does_not_click() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let (mut dev, mut laser, mut t) = (view(), LaserInput::default(), 0.0);
        frame(&ctx, &mut dev, &mut laser, &mut t, None, 0.0);
        let y = settings_y(&ctx, &mut dev, &mut laser, &mut t).expect("rail item");
        dev.page = Page::Home;
        let at = egui::pos2(80.0, y);
        frame(&ctx, &mut dev, &mut laser, &mut t, None, 0.9); // pulled off-panel
        frame(&ctx, &mut dev, &mut laser, &mut t, Some(at), 0.9); // swept on, still held
        frame(&ctx, &mut dev, &mut laser, &mut t, Some(at), 0.0); // released
        assert_eq!(dev.page, Page::Home);
    }
}

#[cfg(test)]
mod search_integration_tests {
    use super::*;
    use crate::search::Target;
    use crate::shell::Page;

    #[test]
    fn picking_a_setting_jumps_and_flashes_its_card() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut dev = DevView { show_volume: false, show_system: false, ..Default::default() };
        let mut t = 0.0;
        let mut frame = |dev: &mut DevView| {
            t += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1365.0, 853.0))),
                time: Some(t),
                ..Default::default()
            };
            let _ = ctx.run(input, |c| dev.ui_with(c, None));
        };
        frame(&mut dev);
        dev.apply_search(&ctx, Target::Card(Page::Settings, "Accessibility"));
        assert_eq!(dev.page, Page::Settings);
        for _ in 0..3 {
            frame(&mut dev);
        }
        // The card's header hook consumed the request and scrolled to it.
        let h = ctx.data(|d| d.get_temp::<(String, f64, bool)>(egui::Id::new("rso-highlight")));
        assert_eq!(h.map(|(c, _, scrolled)| (c, scrolled)), Some(("Accessibility".to_string(), true)));
    }

    #[test]
    fn search_opens_the_advanced_section_to_reach_its_cards() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut dev = DevView { show_volume: false, show_system: false, ..Default::default() };
        let mut t = 0.0;
        let mut frame = |dev: &mut DevView| {
            t += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1365.0, 853.0))),
                time: Some(t),
                ..Default::default()
            };
            let _ = ctx.run(input, |c| dev.ui_with(c, None));
        };
        frame(&mut dev);
        // Administrator lives in the collapsed Advanced section.
        dev.apply_search(&ctx, Target::Card(Page::Settings, "Administrator"));
        for _ in 0..3 {
            frame(&mut dev);
        }
        let h = ctx.data(|d| d.get_temp::<(String, f64, bool)>(egui::Id::new("rso-highlight")));
        assert_eq!(h.map(|(c, _, scrolled)| (c, scrolled)), Some(("Administrator".to_string(), true)), "card was drawn and scrolled to");
    }
}

#[cfg(test)]
mod mute_tests {
    use super::*;

    #[test]
    fn muted_app_goes_to_history_only() {
        let mut dev = DevView::default();
        dev.notif.muted = vec!["Chatty".into()];
        dev.push_notification(Notification { title: "spam".into(), source_app: "Chatty".into(), ..Default::default() });
        assert_eq!(dev.history.len(), 1);
        assert!(dev.toasts.is_empty());
        assert_eq!(dev.unread, 0);
        assert!(dev.pending_sounds.is_empty());
        dev.push_notification(Notification { title: "hi".into(), source_app: "Friend".into(), ..Default::default() });
        assert_eq!(dev.toasts.len(), 1);
        assert_eq!(dev.unread, 1);
    }
}

#[cfg(test)]
mod gamepad_capture_tests {
    use super::*;
    use rsoverlay_input::gamepad::button as b;

    /// Draw the Gamepad settings card once with `held` as the live pad state.
    fn frame(ctx: &egui::Context, dev: &mut DevView, t: &mut f64, held: u16) {
        dev.gamepad_held = held;
        *t += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 900.0))),
            time: Some(*t),
            ..Default::default()
        };
        let _ = ctx.run(input, |c| {
            egui::CentralPanel::default().show(c, |ui| {
                gamepad_card(ui, &mut dev.gamepad, dev.gamepad_held, &mut dev.gamepad_capture, egui::Color32::WHITE);
            });
        });
    }

    #[test]
    fn set_captures_the_combination_pressed_together() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut dev = DevView::default();
        let mut t = 0.0;
        dev.gamepad.enabled = true;
        // "Set" on Recenter, then press Back, add Start, let go.
        dev.gamepad_capture = Some((1, 0));
        for held in [0, b::BACK, b::BACK | b::START, b::START, 0] {
            frame(&ctx, &mut dev, &mut t, held);
        }
        assert_eq!(dev.gamepad.recenter, b::BACK | b::START);
        assert_eq!(dev.gamepad_capture, None, "capture ends on release");
        // The other shortcuts are untouched.
        assert_eq!(dev.gamepad.dashboard, rsoverlay_core::config::GamepadCfg::default().dashboard);
    }

    #[test]
    fn turning_shortcuts_off_cancels_a_capture() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut dev = DevView::default();
        let mut t = 0.0;
        dev.gamepad.enabled = false;
        dev.gamepad_capture = Some((0, 0));
        frame(&ctx, &mut dev, &mut t, b::A);
        assert_eq!(dev.gamepad_capture, None);
        assert_eq!(dev.gamepad.dashboard, rsoverlay_core::config::GamepadCfg::default().dashboard);
    }
}

#[cfg(test)]
mod windows_button_tests {
    use super::DevView;

    #[test]
    fn windows_button_hides_the_keyboard_too_and_brings_it_back() {
        let mut d = DevView { windows_up: true, show_keyboard: true, ..Default::default() };
        d.toggle_windows();
        assert!(!d.windows_up && !d.show_keyboard, "one press hides both");
        d.toggle_windows();
        assert!(d.windows_up && d.show_keyboard, "keyboard comes back with the windows");
        // Keyboard wasn't open: it stays closed when the windows return.
        let mut d = DevView { windows_up: true, show_keyboard: false, ..Default::default() };
        d.toggle_windows();
        d.toggle_windows();
        assert!(d.windows_up && !d.show_keyboard);
        // Closed with its own button while hidden: not brought back.
        let mut d = DevView { windows_up: true, show_keyboard: true, ..Default::default() };
        d.toggle_windows();
        d.kb_hidden_with_windows = false;
        d.toggle_windows();
        assert!(!d.show_keyboard);
    }
}
