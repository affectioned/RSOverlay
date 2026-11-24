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
}

impl MirrorHandle {
    pub fn full(id: egui::TextureId, width: u32, height: u32) -> Self {
        Self { id, width, height, uv: egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), cursor: None }
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
    /// App icons per window (raw pixels from the app; textures are made
    /// lazily per egui context).
    pub icons: std::collections::HashMap<isize, egui::ColorImage>,
    icon_tex: std::collections::HashMap<isize, egui::TextureHandle>,
}

impl SourcePicker {
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
    pub show_history: bool,
    pub show_battery: bool,
    pub battery_warn_pct: u8,
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
    pub run_as_admin: bool,
    /// Set by the app: is this process elevated right now?
    pub elevated: bool,
    /// "Restart as admin now" clicked; the app relaunches via UAC.
    pub restart_elevated_requested: bool,
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
    last_media_poll: Instant,
    chat_draft: String,
    pending_chat: Option<rsoverlay_osc::ChatMessage>,
}

impl Default for DevView {
    fn default() -> Self {
        Self {
            frame: 0,
            last_fps: 0.0,
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
            show_history: false,
            show_battery: true,
            battery_warn_pct: 20,
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
            run_as_admin: false,
            elevated: false,
            restart_elevated_requested: false,
            reset_armed_at: None,
            night_light: crate::nightlight::NightLight::default(),
            tint: [1.0; 4],
            last_frame_time: None,
            history: VecDeque::new(),
            notifications_received: 0,
            media: None,
            last_media_poll: Instant::now() - Duration::from_secs(10),
            chat_draft: String::new(),
            pending_chat: None,
        }
    }
}

impl DevView {
    pub fn push_notification(&mut self, n: Notification) {
        const MAX_HISTORY: usize = 50;
        self.notifications_received += 1;
        self.unread += 1;
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
    pub fn clear_all_notifications(&mut self) {
        self.toasts.clear();
        self.history.clear();
        self.unread = 0;
    }

    /// Replace the battery readings (the app polls OpenVR every few
    /// seconds) and raise a toast for any device that just went low.
    /// Alerts fire even with the card hidden — that's the point of them.
    pub fn set_batteries(&mut self, readings: Vec<BatteryReading>) {
        for n in self.low_battery.update(&readings, self.battery_warn_pct) {
            self.push_notification(n);
        }
        let msgs = self.battery_osc.messages(&readings, Instant::now());
        self.pending_osc.extend(msgs);
        self.batteries = readings;
    }

    /// Widget popped out from a card's ↗ this frame (VR dashboard).
    pub fn take_widget_request(&mut self, ctx: &egui::Context) -> Option<crate::widgets::WidgetKind> {
        crate::widgets::take_popout_request(ctx)
    }

    /// A floating widget panel: tab strip + the live card + dock bar.
    pub fn widget_ui(
        &mut self,
        ctx: &egui::Context,
        tabs: &[crate::widgets::WidgetKind],
        active: usize,
        attach: crate::window_panel::Attach,
        injector: Option<&rsoverlay_input::InputInjector>,
    ) -> crate::widgets::WidgetAction {
        use crate::widgets::WidgetKind as K;
        self.tick_state(ctx);
        crate::widgets::set_popout_enabled(ctx, false);
        let accent = self.accent;
        if ctx.style().visuals.selection.bg_fill != accent {
            ctx.style_mut(|st| st.visuals.selection.bg_fill = accent);
        }
        crate::widgets::frame(ctx, tabs, active, attach, accent, |ui, k| match k {
            K::NowPlaying => media_card(ui, self.media.as_ref(), accent),
            K::System => self.system.show(ui, self.last_fps, accent),
            K::Volume => self.volume.show(ui, accent),
            K::Battery => crate::battery::card(ui, &self.batteries, &mut self.battery_warn_pct, &mut self.battery_osc, accent),
            K::Macros => self.macros.card(ui, injector, accent),
            K::Chat => {
                if let Some(msg) = chatbox_card(ui, &mut self.chat_draft, &mut self.now_playing.enabled, accent) {
                    self.pending_chat = Some(msg);
                }
            }
            K::Notifications => {
                if history_card(ui, &self.history, accent) {
                    self.clear_all_notifications();
                }
            }
        })
    }

    /// The wrist watch-face (VR). Shares all state with the dashboard;
    /// returns what the wearer tapped.
    pub fn wrist_ui(&mut self, ctx: &egui::Context) -> crate::wrist::WristAction {
        self.tick_state(ctx);
        let playing = self.media.as_ref().is_some_and(|m| m.playing);
        let track = self.media.as_ref().and_then(crate::now_playing::format_line);
        crate::wrist::show(ctx, &self.batteries, playing, track.as_deref(), self.unread, self.accent)
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

    fn maybe_refresh_media(&mut self) {
        if self.last_media_poll.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_media_poll = Instant::now();
        self.media = media::poll();
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
        let now = Instant::now();
        let accent = self.accent;
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

        // VR: the dashboard is a rounded, softly bordered, slightly
        // translucent glass slab (panels draw transparent on top of it);
        // desktop keeps the edge-to-edge AMOLED black.
        let root = if self.dock_layout {
            let slab = ctx.screen_rect().shrink(4.0);
            let bg = ctx.layer_painter(egui::LayerId::background());
            bg.rect_filled(slab, 30.0, egui::Color32::from_black_alpha(236));
            bg.rect_stroke(slab, 30.0, egui::Stroke::new(1.5_f32, accent.gamma_multiply(0.28)));
            egui::Frame::none()
        } else {
            egui::Frame::none().fill(theme::BG_ROOT)
        };
        egui::TopBottomPanel::top("header")
            .frame(root.inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 14.0, bottom: 10.0 }))
            .show_separator_line(false)
            .show(ctx, |ui| {
                let act = crate::shell::header(ui, &self.batteries, self.show_keyboard, self.unread, accent);
                if act.toggle_keyboard {
                    self.show_keyboard = !self.show_keyboard;
                }
                if act.open_notifications {
                    self.page = crate::shell::Page::Notifications;
                }
            });
        if self.dock_layout {
            egui::TopBottomPanel::bottom("dock")
                .frame(root.inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 6.0, bottom: 14.0 }))
                .show_separator_line(false)
                .show(ctx, |ui| crate::shell::dock(ui, &mut self.page, self.unread, accent));
        } else {
        egui::SidePanel::left("nav")
            .exact_width(crate::shell::RAIL_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(root.inner_margin(egui::Margin { left: 12.0, right: 0.0, top: 8.0, bottom: 12.0 }))
            .show(ctx, |ui| {
                crate::shell::nav_rail(ui, &mut self.page, self.unread, accent);
            });
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

        if self.show_keyboard && !self.dock_layout {
            if let Some(inj) = injector {
                egui::Area::new(egui::Id::new("virtual-keyboard"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -theme::GUTTER))
                    .show(ctx, |ui| {
                        theme::card().show(ui, |ui| {
                            card_header(ui, icons::CHAT, "Keyboard", accent);
                            ui.add_space(6.0);
                            crate::keyboard::show(ui, &mut self.keyboard, inj, accent);
                        });
                    });
            }
        }

        // Toasts: each in its own Area so it can slide independently.
        // Enter from the right, exit to the right in the last 220 ms,
        // and survivors glide up into the gap a departing toast leaves.
        // Stacked below the header so they never cover the clock.
        let t_now = ctx.input(|i| i.time);
        let mut y = 84.0;
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
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-theme::GUTTER + slide, ty))
                .interactable(false)
                .show(ctx, |ui| toast_card(ui, &t.n, accent));
            t.height = resp.response.rect.height();
            y += t.height + 8.0;
        }

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl DevView {
    fn page_body(
        &mut self,
        ui: &mut egui::Ui,
        injector: Option<&rsoverlay_input::InputInjector>,
        accent: egui::Color32,
    ) {
        use crate::shell::{page_title, Page};
        page_title(ui, self.page.label());
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
                        self.system.show(l, self.last_fps, accent);
                        l.add_space(theme::GUTTER);
                    }
                    if self.show_volume {
                        self.volume.show(l, accent);
                        l.add_space(theme::GUTTER);
                    }
                    if self.show_battery {
                        crate::battery::card(r, &self.batteries, &mut self.battery_warn_pct, &mut self.battery_osc, accent);
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
            Page::Notifications => {
                if history_card(ui, &self.history, accent) {
                    self.clear_all_notifications();
                }
            }
            Page::Settings => {
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
                        &mut self.reduce_motion,
                        &mut self.language,
                        &mut self.keyboard.layout,
                    ) {
                        self.reset_requested = true;
                    }
                    l.add_space(theme::GUTTER);
                    self.night_light.card(l, minute_of_day(), accent);
                    l.add_space(theme::GUTTER);
                    vr_card(l, &mut self.vr_left_handed, &mut self.vr_clip_angle, &mut self.vr_wrist_opacity, accent);
                    l.add_space(theme::GUTTER);
                    accessibility_card(l, &mut self.vr_ui_scale, &mut self.vr_show_lasers, &mut self.reduce_motion, accent);
                    l.add_space(theme::GUTTER);
                    if admin_card(l, &mut self.run_as_admin, self.elevated, accent) {
                        self.restart_elevated_requested = true;
                    }
                    placement_card(r, &mut self.placement, accent);
                    r.add_space(theme::GUTTER);
                    stats_card(r, self.frame, self.last_fps, self.notifications_received, accent);
                    r.add_space(theme::GUTTER);
                    if reset_card(r, &mut self.reset_armed_at) {
                        self.reset_requested = true;
                    }
                    lp.allocate_rect(l.min_rect(), egui::Sense::hover());
                    rp.allocate_rect(r.min_rect(), egui::Sense::hover());
                });
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

fn accessibility_card(ui: &mut egui::Ui, scale: &mut f32, lasers: &mut bool, reduce_motion: &mut bool, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::SETTINGS, "Accessibility", accent);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("VR text size"));
            ui.add(egui::Slider::new(scale, 0.8..=1.6).fixed_decimals(1).suffix("×"));
        });
        ui.checkbox(lasers, "Laser beams").on_hover_text("Draw a beam from each controller while it points at a panel");
        ui.checkbox(reduce_motion, "Reduce motion").on_hover_text("Make page, toast and highlight transitions instant");
        ui.label(egui::RichText::new("Moving/resizing a panel always shows a glowing outline + size readout, with a haptic tick on grab and release.")
            .size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED));
    });
}

fn vr_card(ui: &mut egui::Ui, left_handed: &mut bool, clip: &mut f32, wrist_opacity: &mut f32, accent: egui::Color32) {
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
            ui.add(egui::Slider::new(clip, 30.0..=90.0).suffix("°").fixed_decimals(0));
        })
        .response
        .on_hover_text("Panels fade out when you look at them from further off-axis than this");
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("Wrist opacity"));
            ui.add(egui::Slider::new(wrist_opacity, 0.2..=1.0).fixed_decimals(2));
        });
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

fn card_header(
    ui: &mut egui::Ui,
    source: egui::ImageSource<'static>,
    label: &str,
    accent: egui::Color32,
) {
    ui.horizontal(|ui| {
        ui.add(icon(source, 18.0, accent));
        ui.add_space(6.0);
        ui.label(theme::caption(label));
        crate::widgets::popout_button(ui, label, accent);
    });
}

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
fn history_card(
    ui: &mut egui::Ui,
    history: &VecDeque<HistoryEntry>,
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
        if history.is_empty() {
            ui.label(theme::body_muted("no notifications yet"));
            return;
        }
        egui::ScrollArea::vertical()
            .id_source("history")
            .max_height(560.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for entry in history.iter().rev() {
                    history_entry_row(ui, entry);
                    ui.add_space(4.0);
                }
            });
    });
    cleared
}

fn history_entry_row(ui: &mut egui::Ui, entry: &HistoryEntry) {
    let rel = relative_time(entry.received.elapsed());
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
            });
        });
        if !entry.n.content.is_empty() {
            ui.label(theme::body_muted(&entry.n.content));
        }
    });
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
    reduce_motion: &mut bool,
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
            ui.checkbox(show_system, "System");
        });
        ui.add_space(4.0);
        ui.checkbox(reduce_motion, "Reduce motion")
            .on_hover_text("Make page, toast and highlight transitions instant");
        ui.add_space(8.0);
        // Language picker — shows the native-script name so a Russian
        // user sees Русский.
        egui::ComboBox::from_label("Language")
            .selected_text(language.native_name())
            .show_ui(ui, |ui| {
                for l in crate::i18n::Lang::all() {
                    ui.selectable_value(language, *l, l.native_name());
                }
            });
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
        let lf = laser.update(&[HandSample { hand: Hand::Right, hit, trigger, stick_y: 0.0, button: egui::PointerButton::Primary }], 1.0 / 90.0);
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
        let (mut dev, mut laser, mut t) = (view(), LaserInput::default(), 0.0);
        frame(&ctx, &mut dev, &mut laser, &mut t, None, 0.0);
        let y = settings_y(&ctx, &mut dev, &mut laser, &mut t);
        assert!(y.is_some(), "no laser click on the rail opened Settings");
    }

    #[test]
    fn held_trigger_swept_onto_rail_does_not_click() {
        let ctx = egui::Context::default();
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
