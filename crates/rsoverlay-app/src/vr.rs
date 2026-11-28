//! SteamVR runtime path — XSO-style presentation:
//!
//! - **Wrist watch** parented to the left controller: time, battery,
//!   and Menu / Keyboard / Play / Alerts buttons. Always there when you
//!   glance at your hand.
//! - **Dashboard**, hidden until summoned (wrist Menu button or the left
//!   B / menu button). Spawns in front of you facing you; hold grip
//!   while pointing at it to carry it. SteamVR-style bottom dock.
//! - **Lasers** from both hands, routed to whichever panel each ray hits
//!   first; trigger clicks with a haptic tick, stick scrolls.
//!
//! Each panel is an egui context rendering into a wgpu DX12 texture that
//! OpenVR samples directly (no readback).

#[cfg(windows)]
mod imp {
    use anyhow::{anyhow, Context, Result};
    use rsoverlay_notify::NotifyChannel;
    use glam::{Affine3A, Vec3A};
    use crate::vr_logic::*;
    use rsoverlay_openvr::{pose_offset_pitch, DeviceBattery, DeviceKind, Hand, HmdMatrix34_t, OpenVr};
    use rsoverlay_ui::laser::{self, HandSample, LaserInput};
    use rsoverlay_render::GpuCtx;
    use crate::mirror_source::{fetch_icons, list_sources, GpuMirror};
    use rsoverlay_ui::widgets::WidgetKind;
    use rsoverlay_ui::window_panel::{self, Attach};
    use rsoverlay_ui::{battery::BatteryReading, views::DevView, EguiPanel};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::runtime::Handle;
    use tracing::{info, warn};

    /// Click tick length. Short enough to read as a "detent", not a buzz.
    const CLICK_HAPTIC_US: u16 = 1200;

    /// Wrist watch: small texture, 2 texels/pt → 512 × 256 pt face.
    /// Wrist toolbar stack: 600 × 520 pt at 2 texels/pt (room for every
    /// module at once in edit mode, including two rows of status chips).
    const WRIST_TEX: (u32, u32) = (1200, 1040);
    const WRIST_PPP: f32 = 2.0;
    const WRIST_WIDTH_M: f32 = 0.24;
    /// Wrist mount, in the left controller's frame (−Z = pointing
    /// direction, +Y = top of the controller, +Z = back toward the
    /// wrist): just above the back of the hand, face pitched 60° up
    /// toward the wearer — glance at your wrist and it's there.
    const WRIST_OFFSET: [f32; 3] = [0.0, 0.03, 0.12];
    const WRIST_PITCH_DEG: f32 = -60.0;

    /// Summoned dashboard: 1024 × 640 pt at 2 texels/pt, 85 cm wide,
    /// placed 70 cm ahead and 15 cm below eye line.
    const DASH_TEX: (u32, u32) = (2048, 1280);
    const DASH_PPP: f32 = 2.0;
    const DASH_WIDTH_M: f32 = 0.85;
    const DASH_DISTANCE_M: f32 = 0.70;
    const DASH_DROP_M: f32 = -0.15;

    /// Floating window overlays: 1280 × 800 pt at 1.5 texels/pt, 90 cm
    /// wide, opened 90 cm ahead (behind the dashboard), fanned sideways.
    const WIN_TEX: (u32, u32) = (1920, 1200);
    const WIN_PPP: f32 = 1.5;
    const WIN_WIDTH_M: f32 = 0.90;
    const MAX_WINDOWS: usize = 6;
    /// Monitor wall: panel width, arc radius (eye to panel) and the gap
    /// between neighbours along the arc.
    const WALL_WIDTH_M: f32 = 1.1;
    const WALL_RADIUS_M: f32 = 1.25;
    const WALL_GAP_M: f32 = 0.04;
    /// Big screen: main monitor, wide and gently curved, a little further.
    const BIG_WIDTH_M: f32 = 2.2;
    const BIG_DISTANCE_M: f32 = 1.6;
    const BIG_CURVATURE: f32 = 0.15;
    /// Widget panels (popped-out cards): 600 × 500 pt, 34 cm wide.
    const WID_TEX: (u32, u32) = (1200, 1000);
    const WID_PPP: f32 = 2.0;
    const WID_WIDTH_M: f32 = 0.34;
    const MAX_FLOATING: usize = 14;
    /// Drop a widget within this distance of another to merge them.
    const MERGE_DIST_M: f32 = 0.18;
    /// Window edge docking: a dropped window whose edge lands within
    /// this distance of a neighbour's opposite edge snaps flush to it.
    const SNAP_DIST_M: f32 = 0.12;
    /// …only if the two are roughly coplanar (depth apart, in the
    /// neighbour's frame) and facing the same way (cos of max angle).
    const SNAP_DEPTH_M: f32 = 0.15;
    const SNAP_COS: f32 = 0.866;
    /// Centre-align along the shared edge when this close to aligned.
    const SNAP_ALIGN_M: f32 = 0.08;
    /// Seam left between docked windows.
    const SNAP_GAP_M: f32 = 0.008;
    /// Panel indices: 0 wrist, 1 dashboard, 2 keyboard, WIN0.. windows.
    const WIN0: usize = 3;
    /// Notification toasts: 600 × 400 pt at 2 texels/pt, head-locked.
    const TOAST_TEX: (u32, u32) = (1200, 800);
    const TOAST_PPP: f32 = 2.0;
    /// VR keyboard: 1024 × 400 pt at 2 texels/pt, 75 cm wide, summoned
    /// 45 cm ahead and 40 cm below eye line, tilted up toward you.
    const KB_TEX: (u32, u32) = (2048, 800);
    const KB_PPP: f32 = 2.0;
    const KB_WIDTH_M: f32 = 0.75;
    const KB_DISTANCE_M: f32 = 0.45;
    const KB_DROP_M: f32 = -0.40;
    const KB_TILT_DEG: f32 = -40.0;

    /// XSO-style manipulation while gripping a panel.
    /// Stick Y: push/pull, exponential (×e per second at full deflection
    /// × rate) so it's fine up close and quick far away.
    const PUSH_RATE: f32 = 1.4;
    /// Stick X: scale, same exponential feel.
    const SCALE_RATE: f32 = 1.2;
    const GRAB_DIST: (f32, f32) = (0.20, 8.0);
    const PANEL_WIDTH: (f32, f32) = (0.15, 4.0);
    // Panels outside your view cone re-render at Settings › VR panels ›
    // Unwatched panels fps instead of every frame (XSO: "active"
    // overlays are the ones you look at).

    /// Laser beam strip: 256 × 16 texels, 3.5 mm thick at any length.
    const BEAM_TEX: (u32, u32) = (256, 16);
    const BEAM_THICKNESS_M: f32 = 0.0035;
    /// Laser cursor: its own small overlay on the beam tip.
    const CURSOR_TEX: (u32, u32) = (64, 64);
    const CURSOR_M: f32 = 0.016;
    /// Grab / release haptic: a firmer tick than a click.
    const GRAB_HAPTIC_S: f32 = 0.025;

    /// View-angle fade time constant (XSO-style auto-hide).
    const FADE_TAU: f32 = 0.08;
    /// Focus mode: opacity of the windows you aren't working in.
    const FOCUS_DIM: f32 = 0.3;

    /// How often to re-check which controller the wrist is parented to
    /// (controllers connect late / swap roles).
    const ATTACH_POLL: Duration = Duration::from_secs(1);

    /// Battery props change slowly; polling 64 device slots every
    /// frame would be wasted IPC to vrserver.
    const BATTERY_POLL: Duration = Duration::from_secs(5);

    /// One SteamVR overlay with its own texture, egui context and laser
    /// state. The wrist watch and the dashboard are two of these.
    struct VrPanel {
        handle: rsoverlay_openvr::VROverlayHandle_t,
        _tex: wgpu::Texture,
        view: wgpu::TextureView,
        raw_resource: *mut c_void,
        raw_queue: *mut c_void,
        egui: EguiPanel,
        laser: LaserInput,
        /// Panel size in egui points.
        points: egui::Vec2,
        visible: bool,
        applied_tint: [f32; 4],
        width_m: f32,
        /// 0..1 view-angle fade (multiplies alpha).
        fade: f32,
        /// Texture size now (base size x render scale).
        tex: (u32, u32),
        base_tex: (u32, u32),
        base_ppp: f32,
        scale: f32,
        /// Settings > VR panels > Resolution: texels multiplier.
        render_k: f32,
        /// The texture replaced by the last resolution change, kept alive
        /// a few frames so the compositor never samples a freed one.
        retired: Option<(wgpu::Texture, u32)>,
        /// Inside the view-angle cone this frame.
        in_view: bool,
        last_render: f64,
    }

    impl VrPanel {
        #[allow(clippy::too_many_arguments)]
        fn new(
            overlay: &rsoverlay_openvr::Overlay<'_>,
            gpu: &GpuCtx,
            key: &str,
            name: &str,
            tex: (u32, u32),
            ppp: f32,
            width_m: f32,
            curvature: f32,
        ) -> Result<Self> {
            let handle = overlay.create(key, name).with_context(|| format!("CreateOverlay {key}"))?;
            overlay.set_width(handle, width_m).context("SetOverlayWidthInMeters")?;
            if let Err(e) = overlay.set_curvature(handle, curvature) {
                warn!("SetOverlayCurvature: {e:#}");
            }
            let (t, view, raw_resource, raw_queue) = build_shared_overlay_texture(gpu, tex.0, tex.1)?;
            let mut egui = EguiPanel::new(&gpu.device, t.format(), tex.0, tex.1);
            egui.set_pixels_per_point(ppp);
            // Transparent clear: rounded corners show the world behind.
            egui.set_clear(wgpu::Color::TRANSPARENT);
            Ok(Self {
                handle,
                _tex: t,
                view,
                raw_resource,
                raw_queue,
                egui,
                laser: LaserInput::default(),
                points: egui::vec2(tex.0 as f32 / ppp, tex.1 as f32 / ppp),
                visible: false,
                applied_tint: [f32::NAN; 4],
                width_m,
                fade: 1.0,
                tex,
                base_tex: tex,
                base_ppp: ppp,
                scale: 1.0,
                render_k: 1.0,
                retired: None,
                in_view: true,
                last_render: f64::NEG_INFINITY,
            })
        }

        /// Accessibility text size: fewer, bigger egui points per texel.
        fn apply_scale(&mut self, scale: f32) {
            if (scale - self.scale).abs() > 1e-3 {
                self.scale = scale;
                self.apply_ppp();
            }
        }

        /// Same layout in points, `render_k` x the texels: sharper text
        /// on high-resolution headsets.
        fn apply_ppp(&mut self) {
            let ppp = self.base_ppp * self.scale * self.render_k;
            self.egui.set_pixels_per_point(ppp);
            self.points = egui::vec2(self.tex.0 as f32 / ppp, self.tex.1 as f32 / ppp);
        }

        /// Rebuild the shared texture at `k` x the base size (0.5..=2).
        fn set_render_scale(&mut self, gpu: &GpuCtx, k: f32) {
            let k = k.clamp(RENDER_K.0, RENDER_K.1);
            if (k - self.render_k).abs() < 1e-3 {
                return;
            }
            let tex = ((self.base_tex.0 as f32 * k).round() as u32, (self.base_tex.1 as f32 * k).round() as u32);
            match build_shared_overlay_texture(gpu, tex.0, tex.1) {
                Ok((t, view, raw_resource, raw_queue)) => {
                    let old = std::mem::replace(&mut self._tex, t);
                    self.retired = Some((old, 0));
                    self.view = view;
                    self.raw_resource = raw_resource;
                    self.raw_queue = raw_queue;
                    self.tex = tex;
                    self.render_k = k;
                    self.egui.resize(tex.0, tex.1);
                    self.apply_ppp();
                    // Redraw now, not at the background rate.
                    self.last_render = f64::NEG_INFINITY;
                }
                Err(e) => warn!("panel resolution {k}x: {e:#}"),
            }
        }

        fn set_width(&mut self, overlay: &rsoverlay_openvr::Overlay<'_>, w: f32) {
            let w = w.clamp(PANEL_WIDTH.0, PANEL_WIDTH.1);
            if (w - self.width_m).abs() > self.width_m * 0.002 {
                if let Err(e) = overlay.set_width(self.handle, w) {
                    warn!("resize panel: {e:#}");
                }
                self.width_m = w;
            }
        }

        fn set_visible(&mut self, overlay: &rsoverlay_openvr::Overlay<'_>, on: bool) {
            if on != self.visible {
                let r = if on { overlay.show(self.handle) } else { overlay.hide(self.handle) };
                if let Err(e) = r {
                    warn!("show/hide overlay: {e:#}");
                }
                self.visible = on;
            }
        }

        /// Run one egui frame + hand the texture to the compositor.
        fn render(
            &mut self,
            gpu: &GpuCtx,
            overlay: &rsoverlay_openvr::Overlay<'_>,
            time: f64,
            dt: f32,
            events: Vec<egui::Event>,
            build: impl FnOnce(&egui::Context),
        ) {
            self.last_render = time;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.points)),
                // Explicit clock: motion + toast timers read ctx time.
                time: Some(time),
                predicted_dt: dt.clamp(1.0 / 240.0, 0.1),
                events,
                ..Default::default()
            };
            self.egui.render(&gpu.device, &gpu.queue, &self.view, raw, build);
            unsafe {
                if let Err(e) = overlay.set_texture_d3d12(self.handle, self.raw_resource, self.raw_queue) {
                    warn!("SetOverlayTexture (D3D12): {e:#}");
                }
            }
            // The compositor has had several frames on the new texture.
            if let Some((_, n)) = &mut self.retired {
                *n += 1;
                if *n > 8 {
                    self.retired = None;
                }
            }
        }

        /// Ease the view-angle fade toward shown/hidden.
        fn update_fade(&mut self, shown: bool, dt: f32) {
            self.in_view = shown;
            let target = if shown { 1.0 } else { 0.0 };
            let k = 1.0 - (-dt.max(0.0) / FADE_TAU).exp();
            self.fade += (target - self.fade) * k;
            if (self.fade - target).abs() < 0.01 {
                self.fade = target;
            }
        }

        /// Can the laser hit this panel? (Faded panels don't catch rays.)
        fn interactive(&self) -> bool {
            self.visible && self.fade > 0.5
        }

        /// Night light + opacity (× view-angle fade), re-sent only on
        /// change (IPC per call).
        fn apply_tint(&mut self, overlay: &rsoverlay_openvr::Overlay<'_>, tint: [f32; 4]) {
            let tint = [tint[0], tint[1], tint[2], tint[3] * self.fade];
            if tint.iter().zip(self.applied_tint).any(|(a, b)| (a - b).abs() > 1e-3 || b.is_nan()) {
                let _ = overlay.set_color(self.handle, [tint[0], tint[1], tint[2]]);
                let _ = overlay.set_alpha(self.handle, tint[3]);
                self.applied_tint = tint;
            }
        }

        fn destroy(&self, overlay: &rsoverlay_openvr::Overlay<'_>) {
            overlay.hide(self.handle).ok();
            overlay.clear_texture(self.handle).ok();
            overlay.destroy(self.handle).ok();
        }
    }

    fn to_affine(m: &HmdMatrix34_t) -> Affine3A {
        let m = &m.m;
        Affine3A::from_cols(
            Vec3A::new(m[0][0], m[1][0], m[2][0]),
            Vec3A::new(m[0][1], m[1][1], m[2][1]),
            Vec3A::new(m[0][2], m[1][2], m[2][2]),
            Vec3A::new(m[0][3], m[1][3], m[2][3]),
        )
    }

    fn from_affine(a: &Affine3A) -> HmdMatrix34_t {
        let (x, y, z, t) = (a.matrix3.x_axis, a.matrix3.y_axis, a.matrix3.z_axis, a.translation);
        HmdMatrix34_t {
            m: [[x.x, y.x, z.x, t.x], [x.y, y.y, z.y, t.y], [x.z, y.z, z.z, t.z]],
        }
    }

    /// Where a freshly summoned dashboard goes: in front of the HMD along
    /// its *horizontal* gaze, a little below eye line, upright and facing
    /// the wearer (overlays are visible from their +Z side).
    fn summon_pose(hmd: &HmdMatrix34_t) -> Affine3A {
        let h = to_affine(hmd);
        let fwd = -h.matrix3.z_axis;
        let flat = Vec3A::new(fwd.x, 0.0, fwd.z).try_normalize().unwrap_or(Vec3A::NEG_Z);
        let pos = h.translation + flat * DASH_DISTANCE_M + Vec3A::new(0.0, DASH_DROP_M, 0.0);
        let z = -flat; // face back toward the wearer
        let y = Vec3A::Y;
        let x = y.cross(z);
        Affine3A::from_cols(x, y, z, pos)
    }

    /// A floating panel, dockable to world / head / hands: either a
    /// captured window (`mirror`) or a tab group of widgets (`widgets`).
    struct WinOverlay {
        panel: VrPanel,
        mirror: Option<GpuMirror>,
        widgets: Option<(Vec<WidgetKind>, usize)>,
        attach: Attach,
        /// World pose while docked to World.
        world: Affine3A,
        /// (device index, device-local pose) while docked to head/hand.
        rel: Option<(u32, Affine3A)>,
        last_hit: Option<(i32, i32)>,
        /// UIPI: target elevated and we aren't (re-checked every second).
        input_blocked: bool,
        blocked_checked: Option<Instant>,
        /// App icon, uploaded into this panel's own egui context.
        icon: Option<egui::TextureHandle>,
        /// Mouse button held down in the real window (press → drag → up).
        down: Option<rsoverlay_input::MouseButton>,
        /// Fractional wheel delta not yet sent (WHEEL_DELTA units).
        wheel_acc: f32,
        /// Same, horizontal (tilt-wheel).
        hwheel_acc: f32,
        /// Touch input: a finger is down on the real window.
        touching: bool,
        /// Last time the laser was on this window (toolbar auto-hide).
        toolbar_seen: f64,
        /// Stable id (survives index shifts when others close).
        seq: u32,
        /// Edge-docked group: members move together while one is grabbed.
        group: Option<u32>,
        /// Captured-content opacity (toolbar slider), 0.1..=1.
        opacity: f32,
        /// Stays up with the dashboard closed (XSO layout mode).
        pinned: bool,
        /// Overlay curvature (0 = flat).
        curvature: f32,
        /// Full frame rate even when not looked at (Window menu).
        smooth: bool,
        /// Auto resolution: debounced texture-size changes.
        res: ResSettle,
        /// Share of the panel's width the capture filled last frame.
        content_frac: f32,
    }

    impl WinOverlay {
        /// Panel size in metres (width, height): the overlay keeps its
        /// texture's aspect.
        fn size_m(&self) -> (f32, f32) {
            let (tw, th) = self.panel.tex;
            (self.panel.width_m, self.panel.width_m * th as f32 / tw.max(1) as f32)
        }
    }

    /// An in-progress grab (XSO model): the panel rides the laser at
    /// `dist`, keeps the grabbed spot under the beam, faces the HMD.
    struct Grab {
        hand: Hand,
        /// 1 = dashboard, 2.. = window overlays.
        panel: usize,
        dist: f32,
        /// Panel centre minus grab point, in panel-local axes, per metre
        /// of panel width (so it scales with the panel).
        offset: Vec3A,
        /// Two-hand scale: (other hand, start hand-separation, start width).
        second: Option<(Hand, f32, f32)>,
        smoothed: Affine3A,
        /// Edge-docked windows riding along: (seq, pose relative to the
        /// grabbed panel).
        followers: Vec<(u32, Affine3A)>,
        /// Grabbed panel's size (m) at grab start: followers beyond its
        /// edges are pushed out / pulled in as it's resized.
        size0: (f32, f32),
        /// Settings > VR panels > Tilt with the controller: the panel's
        /// rotation relative to the controller at grab start.
        tilt: Option<glam::Quat>,
        /// One-stick-action lock: Some(true) = scaling, Some(false) =
        /// push/pull, None = stick centred (nothing chosen yet).
        axis: Option<bool>,
    }











    fn open_window(
        overlay: &rsoverlay_openvr::Overlay<'_>,
        gpu: &GpuCtx,
        seq: u32,
        src: rsoverlay_ui::views::CaptureSource,
        world: Affine3A,
        icons: &std::collections::HashMap<isize, egui::ColorImage>,
    ) -> Result<WinOverlay> {
        let key = format!("rsoverlay.win.{seq}");
        let mut panel = VrPanel::new(overlay, gpu, &key, &src.label(), WIN_TEX, WIN_PPP, WIN_WIDTH_M, 0.0)?;
        let mirror = match GpuMirror::start(gpu, &mut panel.egui, src.clone()) {
            Ok(m) => m,
            Err(e) => {
                panel.destroy(overlay);
                return Err(e);
            }
        };
        let icon = match &src {
            rsoverlay_ui::views::CaptureSource::Window { hwnd, .. } => icons
                .get(hwnd)
                .map(|img| panel.egui.ctx.load_texture("window-icon", img.clone(), egui::TextureOptions::LINEAR)),
            _ => None,
        };
        let _ = overlay.set_transform_absolute(panel.handle, &from_affine(&world));
        panel.set_visible(overlay, true);
        info!("window overlay opened: {}", src.label());
        Ok(WinOverlay {
            panel,
            mirror: Some(mirror),
            widgets: None,
            attach: Attach::World,
            world,
            rel: None,
            last_hit: None,
            input_blocked: false,
            blocked_checked: None,
            icon,
            down: None,
            wheel_acc: 0.0,
            hwheel_acc: 0.0,
            touching: false,
            toolbar_seen: f64::NEG_INFINITY,
            seq,
            group: None,
            opacity: 1.0,
            pinned: false,
            curvature: 0.0,
            smooth: false,
            res: ResSettle::default(),
            content_frac: 1.0,
        })
    }

    fn attach_slug(a: Attach) -> &'static str {
        match a {
            Attach::World => "world",
            Attach::Head => "head",
            Attach::LeftHand => "left",
            Attach::RightHand => "right",
        }
    }

    fn attach_from_slug(s: &str) -> Attach {
        match s {
            "head" => Attach::Head,
            "left" => Attach::LeftHand,
            "right" => Attach::RightHand,
            _ => Attach::World,
        }
    }

    /// Snapshot the floating arrangement into a layout.
    fn capture_layout(wins: &[WinOverlay], kb: Option<(Affine3A, f32)>) -> rsoverlay_core::layouts::Layout {
        use rsoverlay_core::layouts::{KeyboardPose, Layout, LayoutPanel};
        let panels = wins
            .iter()
            .map(|w| {
                let (kind, title, monitor, widgets) = match (&w.mirror, &w.widgets) {
                    (_, Some((tabs, _))) => ("widgets", String::new(), 0, tabs.iter().map(|k| k.slug().to_string()).collect()),
                    (Some(m), _) => match &m.source {
                        rsoverlay_ui::views::CaptureSource::Monitor(i) => ("monitor", String::new(), *i, Vec::new()),
                        rsoverlay_ui::views::CaptureSource::Window { title, .. } => ("window", title.clone(), 0, Vec::new()),
                    },
                    _ => ("widgets", String::new(), 0, Vec::new()),
                };
                let pose = match w.rel {
                    Some((_, rel)) => rel,
                    None => w.world,
                };
                LayoutPanel {
                    kind: kind.into(),
                    title,
                    monitor,
                    widgets,
                    attach: attach_slug(w.attach).into(),
                    pose: from_affine(&pose).m,
                    width: w.panel.width_m,
                    group: w.group,
                    opacity: (w.opacity < 1.0).then_some(w.opacity),
                    pinned: w.pinned,
                    curvature: (w.curvature > 0.0).then_some(w.curvature),
                    smooth: w.smooth,
                }
            })
            .collect();
        Layout {
            name: String::new(),
            saved_unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            panels,
            keyboard: kb.map(|(p, width)| KeyboardPose { pose: from_affine(&p).m, width }),
        }
    }

    fn new_widget(
        overlay: &rsoverlay_openvr::Overlay<'_>,
        gpu: &GpuCtx,
        seq: u32,
        tabs: Vec<WidgetKind>,
        world: Affine3A,
    ) -> Result<WinOverlay> {
        let key = format!("rsoverlay.widget.{seq}");
        let mut panel = VrPanel::new(overlay, gpu, &key, "RSOverlay Widget", WID_TEX, WID_PPP, WID_WIDTH_M, 0.0)?;
        overlay.set_transform_absolute(panel.handle, &from_affine(&world)).ok();
        panel.set_visible(overlay, true);
        Ok(WinOverlay {
            panel,
            mirror: None,
            widgets: Some((tabs, 0)),
            attach: Attach::World,
            world,
            rel: None,
            last_hit: None,
            input_blocked: false,
            blocked_checked: None,
            icon: None,
            down: None,
            wheel_acc: 0.0,
            hwheel_acc: 0.0,
            touching: false,
            toolbar_seen: f64::NEG_INFINITY,
            seq,
            group: None,
            opacity: 1.0,
            pinned: false,
            curvature: 0.0,
            smooth: false,
            res: ResSettle::default(),
            content_frac: 1.0,
        })
    }

    /// Current world pose of a tracked device: 0 = HMD, else a controller.
    fn device_pose(dev: u32, hmd: Option<&HmdMatrix34_t>, controllers: &[rsoverlay_openvr::Controller]) -> Option<Affine3A> {
        if dev == 0 {
            hmd.map(to_affine)
        } else {
            controllers.iter().find(|c| c.index == dev).map(|c| to_affine(&c.pose))
        }
    }

    fn win_world(w: &WinOverlay, hmd: Option<&HmdMatrix34_t>, controllers: &[rsoverlay_openvr::Controller]) -> Affine3A {
        match w.rel {
            None => w.world,
            Some((dev, rel)) => device_pose(dev, hmd, controllers).map_or(w.world, |d| d * rel),
        }
    }

    /// Where a dropped panel should snap so one of its edges sits flush
    /// against a neighbour's opposite edge (left / right / top / bottom).
    /// `others` = (index, pose, (width, height) m). Returns the chosen
    /// neighbour and the snapped pose (the neighbour's orientation, so
    /// docked windows form one flat surface). O(n) over `others`.
    fn snap_pose(moving: &Affine3A, size: (f32, f32), others: &[(usize, Affine3A, (f32, f32))]) -> Option<(usize, Affine3A)> {
        let (mw, mh) = size;
        let mut best: Option<(f32, usize, Affine3A)> = None;
        for &(j, o, (ow, oh)) in others {
            if moving.matrix3.z_axis.dot(o.matrix3.z_axis) < SNAP_COS {
                continue;
            }
            let local = o.inverse().transform_point3a(moving.translation);
            if local.z.abs() > SNAP_DEPTH_M {
                continue;
            }
            // Along the shared edge: snap to centred when close, else keep
            // the offset as long as the edges still overlap.
            let align = |v: f32, limit: f32| -> Option<f32> {
                if v.abs() < SNAP_ALIGN_M {
                    Some(0.0)
                } else if v.abs() < limit {
                    Some(v)
                } else {
                    None
                }
            };
            let sx = (ow + mw) / 2.0 + SNAP_GAP_M;
            let sy = (oh + mh) / 2.0 + SNAP_GAP_M;
            let along_y = align(local.y, (oh + mh) / 2.0);
            let along_x = align(local.x, (ow + mw) / 2.0);
            let candidates = [along_y.map(|y| (sx, y)), along_y.map(|y| (-sx, y)), along_x.map(|x| (x, sy)), along_x.map(|x| (x, -sy))];
            for (tx, ty) in candidates.into_iter().flatten() {
                let d = ((local.x - tx).powi(2) + (local.y - ty).powi(2)).sqrt();
                if d < SNAP_DIST_M && best.as_ref().is_none_or(|b| d < b.0) {
                    let pose = Affine3A { translation: o.transform_point3a(Vec3A::new(tx, ty, 0.0)), ..o };
                    best = Some((d, j, pose));
                }
            }
        }
        best.map(|(_, j, p)| (j, p))
    }

    /// A docked neighbour's pose (relative to the grabbed panel) after the
    /// grabbed panel resized from `old` to `new` (w, h m) about its centre:
    /// anything beyond an edge moves by that edge's shift, so seams stay
    /// flush; anything overlapping the panel's span on an axis stays put.
    fn push_follower(rel: Affine3A, old: (f32, f32), new: (f32, f32)) -> Affine3A {
        let shift = |v: f32, half_old: f32, half_new: f32| {
            if v.abs() >= half_old - 1e-3 {
                v + v.signum() * (half_new - half_old)
            } else {
                v
            }
        };
        let t = rel.translation;
        Affine3A {
            translation: Vec3A::new(shift(t.x, old.0 / 2.0, new.0 / 2.0), shift(t.y, old.1 / 2.0, new.1 / 2.0), t.z),
            ..rel
        }
    }

    /// Dissolve edge-dock groups left with a single member. O(n²), n ≤ 14.
    fn prune_groups(wins: &mut [WinOverlay]) {
        for k in 0..wins.len() {
            if let Some(g) = wins[k].group {
                if wins.iter().filter(|w| w.group == Some(g)).count() < 2 {
                    wins[k].group = None;
                }
            }
        }
    }

    /// SteamVR Input manifest + default bindings (XSOverlay-compatible
    /// layout), written next to config.toml so SteamVR can read them and
    /// users can rebind in SteamVR's controller-binding UI.
    const MANIFEST: &[(&str, &str)] = &[
        ("actions.json", include_str!("../steamvr/actions.json")),
        ("bindings_knuckles.json", include_str!("../steamvr/bindings_knuckles.json")),
        ("bindings_oculus_touch.json", include_str!("../steamvr/bindings_oculus_touch.json")),
        ("bindings_vive_controller.json", include_str!("../steamvr/bindings_vive_controller.json")),
        ("bindings_holographic_controller.json", include_str!("../steamvr/bindings_holographic_controller.json")),
    ];

    /// SteamVR app key RSOverlay registers under.
    const APP_KEY: &str = "rsoverlay.overlay";

    /// The .vrmanifest that makes RSOverlay a SteamVR overlay app (the
    /// same mechanism XSOverlay uses): SteamVR lists it under startup
    /// overlay apps and launches this exe with `--vr` alongside itself.
    fn app_manifest(exe: &std::path::Path, actions: &std::path::Path) -> String {
        let doc = serde_json::json!({
            "source": "builtin",
            "applications": [{
                "app_key": APP_KEY,
                "launch_type": "binary",
                "binary_path_windows": exe.to_string_lossy(),
                "arguments": "--vr",
                "is_dashboard_overlay": true,
                "action_manifest_path": actions.to_string_lossy(),
                "strings": { "en_us": {
                    "name": "RSOverlay",
                    "description": "Open-source desktop, window and widget overlay for SteamVR"
                } }
            }]
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }

    /// Write + register the app manifest. Failures only cost auto-launch.
    fn register_with_steamvr(vr: &OpenVr, dir: &std::path::Path, actions: &std::path::Path) {
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => return warn!("auto-launch: can't locate our exe: {e}"),
        };
        let path = dir.join("rsoverlay.vrmanifest");
        if let Err(e) = std::fs::write(&path, app_manifest(&exe, actions)) {
            return warn!("auto-launch: writing {}: {e}", path.display());
        }
        match vr.register_app(&path, APP_KEY) {
            Ok(r) if r.newly_installed => info!("registered with SteamVR; auto-launch {}", if r.auto_launch { "on" } else { "off" }),
            Ok(r) => info!("SteamVR app registration refreshed; auto-launch {}", if r.auto_launch { "on" } else { "off (SteamVR › Settings › Startup)" }),
            Err(e) => warn!("auto-launch: SteamVR registration failed: {e}"),
        }
    }

    fn write_manifest(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(dir)?;
        for (name, body) in MANIFEST {
            std::fs::write(dir.join(name), body)?;
        }
        Ok(dir.join("actions.json"))
    }

    pub fn run(
        mut notify: Option<NotifyChannel>,
        rt: Handle,
        config: rsoverlay_core::Config,
        config_path: std::path::PathBuf,
        osc: Option<rsoverlay_osc::OscClient>,
    ) -> Result<()> {
        let vr = OpenVr::init().context("OpenVR init")?;
        let overlay = vr.overlay().context("acquiring IVROverlay")?;
        let system = vr
            .system()
            .map_err(|e| warn!("IVRSystem unavailable — no controllers / battery: {e}"))
            .ok();

        let gpu = pollster::block_on(GpuCtx::new(rsoverlay_core::RuntimeMode::Vr))
            .context("building wgpu GpuCtx")?;

        let mut wrist = VrPanel::new(&overlay, &gpu, "rsoverlay.wrist", "RSOverlay Wrist", WRIST_TEX, WRIST_PPP, WRIST_WIDTH_M, 0.0)?;
        let mut dash = VrPanel::new(
            &overlay,
            &gpu,
            "rsoverlay.dashboard",
            "RSOverlay",
            DASH_TEX,
            DASH_PPP,
            DASH_WIDTH_M,
            config.placement.curvature,
        )?;

        let mut dev_view = DevView::default();
        crate::preview::seed_dev_view(&mut dev_view, &config, &crate::preview::TestHooks::default());
        dev_view.updates.supported = crate::UPDATE_CHECKS;
        info!("elevated: {}", dev_view.elevated);
        dev_view.dock_layout = true;
        // VR mode persists like the desktop modes: edits in the headset
        // survive a restart, hand-edits to config.toml hot-reload.
        let mut persister = crate::preview::ConfigPersister::new(config_path.clone(), config.clone());
        let config_base = config.clone();
        let injector = rsoverlay_input::InputInjector::new().ok();
        // SteamVR Input with XSO-style default bindings; the legacy
        // controller API is the fallback if the manifest can't load.
        let steamvr_dir = config_path.parent().map(|p| p.join("steamvr")).unwrap_or_else(|| "steamvr".into());
        let input = match write_manifest(&steamvr_dir) {
            Ok(path) => {
                register_with_steamvr(&vr, &steamvr_dir, &path);
                vr.input(&path)
            }
                .map_err(|e| warn!("SteamVR Input unavailable ({e}); using legacy controller input"))
                .ok(),
            Err(e) => {
                warn!("writing SteamVR action manifest: {e}");
                None
            }
        };
        if input.is_some() {
            info!("SteamVR Input active — bindings rebindable in SteamVR › Controller Settings › RSOverlay");
        }

        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                r.store(false, Ordering::SeqCst);
            }
        });

        let mut fps = crate::preview::FpsCounter::new();
        let started = Instant::now();
        let mut last_frame = Instant::now();
        let mut last_battery_poll: Option<Instant> = None;
        let mut last_attach_check: Option<Instant> = None;
        let mut wrist_parent: Option<u32> = None;
        let mut wrist_on: Hand = Hand::Left;
        // Wrist pose relative to its controller; grabbing the wrist edits it.
        let mut wrist_rel = {
            let [x, y, z] = WRIST_OFFSET;
            to_affine(&pose_offset_pitch(x, y, z, WRIST_PITCH_DEG))
        };
        let mut dash_pose: Option<Affine3A> = None;
        // Dashboard docked to a device (head / hand): (device, pose in its
        // space). The compositor carries it; `dash_pose` follows each frame.
        let mut dash_rel: Option<(u32, Affine3A)> = None;
        let mut grab: Option<Grab> = None;
        let mut wins: Vec<WinOverlay> = Vec::new();
        let layouts_path = rsoverlay_core::layouts::Layouts::path();
        dev_view.layouts = rsoverlay_core::layouts::Layouts::load(&layouts_path);
        // The favourite layout loads once controllers/HMD are tracked.
        let mut favorite_pending = dev_view.layouts.favorite;
        let mut kb = VrPanel::new(&overlay, &gpu, "rsoverlay.keyboard", "RSOverlay Keyboard", KB_TEX, KB_PPP, KB_WIDTH_M, 0.0)?;
        for (i, h) in [(0, wrist.handle), (1, dash.handle), (2, kb.handle)] {
            let _ = overlay.set_sort_order(h, layer(i));
        }
        // Laser beams: static white strip textures rendered once, tinted
        // and stretched per frame. Sorted above every panel.
        let mut beams = Vec::with_capacity(2);
        for side in ["left", "right"] {
            let mut b = VrPanel::new(&overlay, &gpu, &format!("rsoverlay.beam.{side}"), "RSOverlay Laser", BEAM_TEX, 1.0, 1.0, 0.0)?;
            b.render(&gpu, &overlay, 0.0, 0.016, Vec::new(), rsoverlay_ui::shell::paint_beam);
            let _ = overlay.set_sort_order(b.handle, 100);
            beams.push(b);
        }
        let mut beam_tint = [[f32::NAN; 4]; 2];
        // Laser cursors (one per hand), above the beams.
        let mut cursors = Vec::with_capacity(2);
        for side in ["left", "right"] {
            let mut c = VrPanel::new(&overlay, &gpu, &format!("rsoverlay.cursor.{side}"), "RSOverlay Cursor", CURSOR_TEX, 1.0, CURSOR_M, 0.0)?;
            c.egui.set_clear(wgpu::Color::TRANSPARENT);
            c.render(&gpu, &overlay, 0.0, 0.016, Vec::new(), rsoverlay_ui::shell::paint_cursor_dot);
            let _ = overlay.set_sort_order(c.handle, 101);
            cursors.push(c);
        }
        let mut cursor_tint = [[f32::NAN; 4]; 2];
        let mut wrist_own_hit_logged = false;
        // What each hand's laser was on last frame (diagnostics).
        let mut laser_on: [Option<usize>; 2] = [None, None];
        let mut prev_trig = [false; 2];
        let mut keyboard = rsoverlay_ui::vrkeyboard::VrKeyboard::default();
        // Restore hands-on state from last session.
        {
            let st = &dev_view.vr_state;
            if let Some(m) = st.wrist_pose {
                wrist_rel = to_affine(&HmdMatrix34_t { m });
            }
            if let Some(w) = st.wrist_width {
                wrist.set_width(&overlay, w);
            }
            keyboard.locked_pos = st.kb_pinned;
            keyboard.auto_repeat = st.kb_auto_repeat;
            keyboard.full = st.kb_full;
            keyboard.vrchat = st.kb_vrchat;
        }
        // Last session: reopened once tracking is up, unless a starred
        // layout is set (that wins). Autosaved every few seconds after.
        let mut session_pending = dev_view
            .layouts
            .last_session
            .clone()
            .filter(|l| favorite_pending.is_none() && dev_view.restore_session && !l.panels.is_empty());
        let mut session_checked = Instant::now();
        let mut kb_pose: Option<Affine3A> = None;
        // VR keyboard → RSOverlay's own text fields: events typed this
        // frame go to the panel whose field has focus (1 = dashboard,
        // WIN0.. = widget groups), delivered on its next render.
        let mut typed: Vec<egui::Event> = Vec::new();
        let mut typed_target: Option<usize> = None;
        let mut prev_text_target: Option<usize> = None;
        let mut win_seq = 0u32;
        let mut next_group = 0u32;
        let mut typing_sounds = rsoverlay_audio::typing::TypingSounds::new();
        let mut thumbs = crate::mirror_source::ThumbWorker::spawn();
        // Device icons from the user's SteamVR install (decoded once each).
        let mut device_icons = crate::device_icons::DeviceIcons::default();
        // Focus mode: the window being worked in (by seq).
        let mut focus_win: Option<u32> = None;
        // Were windows open last frame (Focus auto-off when the last closes).
        let mut had_windows = false;
        // Block game input: the panel whose overlay currently carries the
        // input-capture flag.
        let mut capture_on: Option<rsoverlay_openvr::VROverlayHandle_t> = None;
        // Gamepad shortcuts (only polled while enabled).
        let mut gamepads = rsoverlay_input::gamepad::Gamepads::default();
        let mut pad_prev: u16 = 0;
        // SteamVR frame timing (game fps, GPU time, reprojection). Only the
        // layout-verified compositor table is used; None = no VR stats.
        let compositor = vr.compositor().map_err(|e| info!("frame stats unavailable: {e}")).ok();
        let mut last_perf: Option<Instant> = None;
        // Headset pixels per radian (auto resolution, Readable size).
        let mut px_per_rad: Option<f32> = None;
        // Head-locked toast panel, shown only while toasts are up.
        let mut toasts = VrPanel::new(&overlay, &gpu, "rsoverlay.toasts", "RSOverlay Notifications", TOAST_TEX, TOAST_PPP, 0.30, 0.0)?;
        let _ = overlay.set_sort_order(toasts.handle, 60);
        let mut toast_place: Option<(f32, f32, f32)> = None;
        dev_view.toasts_external = true;
        let sounds_dir = rsoverlay_core::config::sounds_dir();
        let mut prev_menu = false;
        // Steady pointer filters, [left, right].
        let mut steady = [RayStabilizer::default(); 2];
        // The PC mouse: yields laser hover while you use it.
        let mut mouse_owner = MouseYield::default();
        // Keyboard on SteamVR's own laser (vr_logic::SteamKb).
        let mut steam_kb = SteamKb::default();
        let mut kb_mouse_scale = egui::Vec2::ZERO;
        let mut steam_kb_logged = (false, false);
        // Window count last frame (a new one shows the windows).
        let mut prev_win_count = 0usize;
        dev_view.windows_up = true;
        let mut prev_grip = [false; 2];
        let mut frame = 0u64;
        info!("VR shell up — wrist on the off hand; double-press left A/X (or the wrist Menu button) shows the dashboard");

        while running.load(Ordering::SeqCst) {
            let frame_start = Instant::now();
            // SteamVR is shutting down: leave with it (auto-launched
            // overlays are expected to; a straggler blocks the exit).
            if system.as_ref().is_some_and(|s| s.quit_requested()) {
                info!("SteamVR is quitting; exiting");
                break;
            }
            frame = frame.wrapping_add(1);
            dev_view.frame = frame;
            dev_view.last_fps = fps.tick();
            let dt = last_frame.elapsed().as_secs_f32();
            last_frame = Instant::now();
            let time = started.elapsed().as_secs_f64();

            if let Some(sys) = &system {
                if last_battery_poll.is_none_or(|t| t.elapsed() >= BATTERY_POLL) {
                    last_battery_poll = Some(Instant::now());
                    let use_icons = dev_view.steamvr_icons;
                    let readings = sys
                        .batteries()
                        .iter()
                        .map(|d| BatteryReading {
                            icon: if use_icons { device_icons.for_device(d.icon.as_deref(), d.resource_root.as_deref()) } else { None },
                            ..reading(d)
                        })
                        .collect();
                    dev_view.set_batteries(readings);

                }
            }

            // ── Wrist parenting: the non-dominant hand; re-resolved each
            // second (late controllers, handedness switched in Settings).
            let want_hand = if dev_view.vr_left_handed { Hand::Right } else { Hand::Left };
            if want_hand != wrist_on {
                wrist_on = want_hand;
                wrist_parent = None;
                last_attach_check = None;
                // Mirror the mount across the controller's YZ plane.
                wrist_rel.translation.x = -wrist_rel.translation.x;
            }
            if last_attach_check.is_none_or(|t| t.elapsed() >= ATTACH_POLL) {
                last_attach_check = Some(Instant::now());
                let target = system.as_ref().and_then(|s| s.hand_index(wrist_on));
                if target != wrist_parent {
                    if let Some(dev) = target {
                        match overlay.set_transform_device_relative(wrist.handle, dev, &from_affine(&wrist_rel)) {
                            Ok(()) => info!("wrist attached to {wrist_on:?} controller (device {dev})"),
                            Err(e) => warn!("wrist attach: {e:#}"),
                        }
                    }
                    wrist_parent = target;
                }
            }
            // Settings › VR › Hide wrist toolbar applies immediately.
            wrist.set_visible(&overlay, wrist_parent.is_some() && !dev_view.vr_hide_wrist);

            // Poses predicted to when this frame reaches the eyes: the
            // time the compositor draws the controller-parented beam and
            // cursor at, so the laser clicks exactly where it's drawn
            // (SteamVR's own keyboard does the same).
            let predict = system.as_ref().map_or(0.0, |s| s.seconds_to_photons());
            mouse_owner.observe(rsoverlay_capture::window::cursor_pos(), time);
            let (hmd, mut controllers) = system.as_ref().map(|s| s.tracking(predict)).unwrap_or_default();
            // Buttons/axes from SteamVR Input (user's bindings), poses
            // from the tracking API. [left, right].
            let actions = input.as_ref().map(|i| i.poll(predict));
            let mut mods = [(false, false); 2];
            // Dedicated right / middle click buttons (pressed = clicking).
            let mut direct = [(false, false); 2];
            let mut toggle_edit = false;
            if let Some(a) = &actions {
                for c in controllers.iter_mut() {
                    let h = &a[hand_slot(c.hand)];
                    // The binding's click threshold is authoritative.
                    c.trigger = if h.click { 1.0 } else { 0.0 };
                    c.grip = h.grab;
                    c.stick = h.stick;
                    c.menu = false;
                    mods[hand_slot(c.hand)] = (h.right_mod, h.middle_mod);
                    direct[hand_slot(c.hand)] = (h.right_click, h.middle_click);
                    // Aim from SteamVR's tip pose: the intended pointing
                    // direction per controller model (raw pose points
                    // noticeably off on Index).
                    if let Some(aim) = h.aim {
                        let (o, d) = rsoverlay_openvr::ray_from_pose(&aim);
                        c.origin = o;
                        c.dir = d;
                    }
                }
                toggle_edit = a.iter().any(|h| h.toggle_edit);
            }

            // ── Summon / dismiss: left B / menu button edge.
            // User laser-angle fine-tune: pitch the ray about the
            // controller's right axis.
            if dev_view.vr_laser_angle.abs() > 0.01 {
                for c in controllers.iter_mut() {
                    let right = to_affine(&c.pose).matrix3.x_axis;
                    let rot = glam::Quat::from_axis_angle(glam::Vec3::from(right), dev_view.vr_laser_angle.to_radians());
                    let d = rot * glam::Vec3::from(c.dir);
                    c.dir = d.into();
                }
            }

            // Steady pointer (Accessibility): the ray itself is smoothed,
            // so beam, cursor, keyboard and clicks agree.
            for c in controllers.iter_mut() {
                let s = hand_slot(c.hand);
                let (o, d) = steady[s].step(Vec3A::from(c.origin), Vec3A::from(c.dir), dev_view.cursor_smoothing, dt);
                c.origin = o.into();
                c.dir = d.into();
            }

            // Show/hide the dashboard: XSO's double-press left A / X via
            // SteamVR Input; legacy fallback = off-hand menu button.
            let menu_now = controllers.iter().any(|c| c.hand == wrist_on && c.menu);
            let mut toggle_dash = toggle_edit || (menu_now && !prev_menu);
            prev_menu = menu_now;

            if let Some((d, rel)) = dash_rel {
                if let Some(p) = device_pose(d, hmd.as_ref(), &controllers) {
                    dash_pose = Some(p * rel);
                }
            }

            // ── View-angle auto-hide (XSO behaviour) for every panel; a
            // panel being grabbed never fades.
            // SteamVR's own menu open: everything of ours steps aside.
            let steamvr_menu = dev_view.vr_hide_with_steamvr && overlay.is_dashboard_visible();
            if steamvr_menu && grab.is_some() {
                grab = None;
            }
            // Windows button: independent of the dashboard; a newly
            // opened window brings them back up.
            dev_view.dashboard_open = dash.visible;
            dev_view.windows_up = windows_up_after(dev_view.windows_up, prev_win_count, wins.len());
            prev_win_count = wins.len();
            if let Some(h) = hmd.as_ref().map(to_affine) {
                let clip = dev_view.vr_clip_angle;
                let grabbed = grab.as_ref().map(|g| g.panel);
                let wrist_world = wrist_parent
                    .and_then(|d| device_pose(d, hmd.as_ref(), &controllers))
                    .map(|p| p * wrist_rel);
                let up = !steamvr_menu;
                wrist.update_fade(up && (grabbed == Some(0) || wrist_world.is_some_and(|w| in_view(&h, &w, clip))), dt);
                dash.update_fade(up && (grabbed == Some(1) || dash_pose.is_some_and(|p| in_view(&h, &p, clip))), dt);
                kb.update_fade(up && (grabbed == Some(2) || kb_pose.is_some_and(|p| in_view(&h, &p, clip))), dt);
                for (k, w) in wins.iter_mut().enumerate() {
                    let p = win_world(w, hmd.as_ref(), &controllers);
                    w.panel.update_fade(window_shown(steamvr_menu, dev_view.windows_up, w.pinned, grabbed == Some(k + WIN0), in_view(&h, &p, clip)), dt);
                }
            }

            // ── Laser routing: each hand's ray goes to the nearest panel.
            // Panels: 0 = wrist (never the left hand — it wears it),
            // 1 = dashboard, 2.. = window overlays.
            let n = WIN0 + wins.len();
            let mut samples: Vec<Vec<HandSample>> = (0..n).map(|_| Vec::new()).collect();
            let mut hit_by: Vec<[bool; 2]> = vec![[false; 2]; n];
            // Per hand: nearest panel + hit distance (for grab start).
            let mut hand_hit: [Option<(usize, f32)>; 2] = [None, None];
            for c in &controllers {
                let hand = match c.hand {
                    Hand::Left => laser::Hand::Left,
                    Hand::Right => laser::Hand::Right,
                };
                let target = |i: usize| -> (rsoverlay_openvr::VROverlayHandle_t, bool, egui::Vec2) {
                    match i {
                        0 => (wrist.handle, wrist.interactive() && c.hand != wrist_on, wrist.points),
                        1 => (dash.handle, dash.interactive(), dash.points),
                        2 => (kb.handle, kb.interactive(), kb.points),
                        k => (wins[k - WIN0].panel.handle, wins[k - WIN0].panel.interactive(), wins[k - WIN0].panel.points),
                    }
                };
                let mut best: Option<(usize, [f32; 2], f32)> = None;
                for i in 0..n {
                    let (h, visible, _) = target(i);
                    if !visible {
                        continue;
                    }
                    let mut hit = overlay.intersect(h, c.origin, c.dir);
                    // The wrist rides a controller: also test it with our
                    // own ray-vs-quad math at the pose we mount it at, so
                    // a missed SteamVR intersection can't make it dead.
                    if hit.is_none() && i == 0 {
                        if let Some(p) = wrist_parent.and_then(|d| device_pose(d, hmd.as_ref(), &controllers)) {
                            let quad = SimOverlay { pose: p * wrist_rel, width: wrist.width_m, aspect: wrist.tex.1 as f32 / wrist.tex.0 as f32 };
                            hit = quad.intersect(Vec3A::from(c.origin), Vec3A::from(c.dir).normalize_or_zero());
                            if hit.is_some() && !wrist_own_hit_logged {
                                wrist_own_hit_logged = true;
                                warn!("wrist: SteamVR's intersection missed where the wrist is; using our own hit test");
                            }
                        }
                    }
                    if let Some((uv, d)) = hit {
                        if best.is_none_or(|b| hit_beats((i, d), (b.0, b.2))) {
                            best = Some((i, uv, d));
                        }
                    }
                }
                hand_hit[hand_slot(c.hand)] = best.map(|b| (b.0, b.2));
                // Diagnostics: where a keyboard press lands (panel points).
                let s = hand_slot(c.hand);
                let pressed = c.trigger > 0.5;
                if pressed && !prev_trig[s] {
                    if let Some((2, uv, d)) = best {
                        let p = LaserInput::uv_to_points(uv, kb.points);
                        info!("keyboard press {:?}: uv ({:.3},{:.3}) d {d:.3} → ({:.0},{:.0}) pt of {:.0}x{:.0}", c.hand, uv[0], uv[1], p.x, p.y, kb.points.x, kb.points.y);
                    }
                }
                prev_trig[s] = pressed;
                // Diagnostics: what each laser is on, logged on change,
                // with the wrist / dashboard state (tester: no beam on
                // either).
                let now_on = best.map(|b| b.0);
                if now_on != laser_on[hand_slot(c.hand)] {
                    laser_on[hand_slot(c.hand)] = now_on;
                    let name = match now_on {
                        None => "nothing".to_string(),
                        Some(0) => "wrist".into(),
                        Some(1) => "dashboard".into(),
                        Some(2) => "keyboard".into(),
                        Some(k) => format!("window {}", k - WIN0 + 1),
                    };
                    // Geometry check: SteamVR's hit vs our own on the panel
                    // we placed, and where the cursor lands relative to its
                    // surface (+ = in front, toward you).
                    let panel_pose = match now_on {
                        Some(0) => wrist_parent.and_then(|d| device_pose(d, hmd.as_ref(), &controllers)).map(|p| (p * wrist_rel, wrist.width_m, wrist.tex)),
                        Some(1) => dash_pose.map(|p| (p, dash.width_m, dash.tex)),
                        Some(2) => kb_pose.map(|p| (p, kb.width_m, kb.tex)),
                        _ => None,
                    };
                    let geo = match (panel_pose, best, hmd.as_ref().map(|h| to_affine(h).translation)) {
                        (Some((p, w, tex)), Some((_, uv, d)), Some(e)) => {
                            let quad = SimOverlay { pose: p, width: w, aspect: tex.1 as f32 / tex.0 as f32 };
                            let ours = quad.intersect(Vec3A::from(c.origin), Vec3A::from(c.dir).normalize_or_zero());
                            let cur = cursor_pose(Vec3A::from(c.origin), Vec3A::from(c.dir), d, e).translation;
                            let front = (cur - p.translation).dot(p.matrix3.z_axis);
                            format!(
                                " | steamvr uv ({:.3},{:.3}) d {d:.3}; ours {:?}; cursor {:+.4} m off the surface",
                                uv[0], uv[1],
                                ours.map(|(u, dd)| format!("uv ({:.3},{:.3}) d {dd:.3}", u[0], u[1])),
                                front
                            )
                        }
                        _ => String::new(),
                    };
                    info!(
                        "laser {:?} → {name} (wrist shown {} fade {:.2}; dashboard shown {} fade {:.2}){geo}",
                        c.hand, wrist.visible, wrist.fade, dash.visible, dash.fade
                    );
                }
                for (i, list) in samples.iter_mut().enumerate() {
                    let hit = best.filter(|b| b.0 == i).map(|b| LaserInput::uv_to_points(b.1, target(i).2));
                    hit_by[i][hand_slot(c.hand)] = hit.is_some();
                    let (rm, mm) = mods[hand_slot(c.hand)];
                    let (trigger, button) = match direct[hand_slot(c.hand)] {
                        // A dedicated button clicks on its own.
                        (true, _) => (1.0, egui::PointerButton::Secondary),
                        (_, true) => (1.0, egui::PointerButton::Middle),
                        _ if rm => (c.trigger, egui::PointerButton::Secondary),
                        _ if mm => (c.trigger, egui::PointerButton::Middle),
                        _ => (c.trigger, egui::PointerButton::Primary),
                    };
                    list.push(HandSample { hand, hit, trigger, stick_x: c.stick[0], stick_y: c.stick[1], button });
                }
            }
            let mut samples = samples.into_iter();
            let mut wrist_samples = samples.next().unwrap_or_default();
            let mut dash_samples = samples.next().unwrap_or_default();
            let kb_samples = samples.next().unwrap_or_default();
            let mut win_samples: Vec<Vec<HandSample>> = samples.collect();

            // ── Grab (XSO model). Grip on a panel: it rides the laser,
            // faces you, stays level. While held: stick Y push/pull,
            // stick X scale; a second hand gripping the same panel
            // scales by hand separation.
            let eye = hmd.as_ref().map(|h| to_affine(h).translation);
            let mut released: Option<usize> = None;
            let mut released_hand: Option<Hand> = None;
            let mut grabbed_hand: Option<Hand> = None;
            let mut hud: Option<(usize, rsoverlay_ui::shell::GrabHud)> = None;
            for c in &controllers {
                let s = hand_slot(c.hand);
                let pressed = c.grip && !prev_grip[s];
                prev_grip[s] = c.grip;
                if !pressed {
                    continue;
                }
                match (&mut grab, hand_hit[s]) {
                    // Second hand on the already-grabbed panel → two-hand scale.
                    (Some(g), Some((i, _))) if g.hand != c.hand && g.panel == i && g.second.is_none() => {
                        if let Some(other) = controllers.iter().find(|o| o.hand == g.hand) {
                            let sep = (to_affine(&c.pose).translation - to_affine(&other.pose).translation).length();
                            let w = match i {
                                0 => wrist.width_m,
                                1 => dash.width_m,
                                2 => kb.width_m,
                                k => wins[k - WIN0].panel.width_m,
                            };
                            g.second = Some((c.hand, sep.max(0.01), w));
                        }
                    }
                    (None, Some((i, dist))) => {
                        let current = if i == 0 {
                            wrist_parent
                                .and_then(|d| device_pose(d, hmd.as_ref(), &controllers))
                                .map(|p| p * wrist_rel)
                        } else if i == 1 {
                            // Like windows: grabbing a docked dashboard undocks it.
                            if dash_rel.take().is_some() {
                                dev_view.dash_attach = Attach::World;
                            }
                            dash_pose
                        } else if i == 2 {
                            // A pinned keyboard can't be moved.
                            kb_pose.filter(|_| !keyboard.locked_pos)
                        } else {
                            // Grabbing a docked window undocks it to World.
                            let w = &mut wins[i - WIN0];
                            let p = win_world(w, hmd.as_ref(), &controllers);
                            w.rel = None;
                            w.attach = Attach::World;
                            w.world = p;
                            Some(p)
                        };
                        if let Some(p) = current {
                            let hit = Vec3A::from(c.origin) + Vec3A::from(c.dir) * dist;
                            let width = match i {
                                0 => wrist.width_m,
                                1 => dash.width_m,
                                2 => kb.width_m,
                                k => wins[k - WIN0].panel.width_m,
                            };
                            let offset = p.matrix3.transpose() * (p.translation - hit) / width;
                            // Edge-docked neighbours ride along.
                            let followers = match i.checked_sub(WIN0).and_then(|k| wins[k].group.map(|g| (k, g))) {
                                Some((k, g)) => wins
                                    .iter()
                                    .enumerate()
                                    .filter(|&(j, w)| j != k && w.group == Some(g) && w.attach == Attach::World)
                                    .map(|(_, w)| (w.seq, p.inverse() * w.world))
                                    .collect(),
                                None => Vec::new(),
                            };
                            let size0 = i.checked_sub(WIN0).map_or((width, 0.0), |k| wins[k].size_m());
                            let tilt = dev_view.vr_grab_tilt.then(|| {
                                let (_, hand_rot, _) = to_affine(&c.pose).to_scale_rotation_translation();
                                let (_, panel_rot, _) = p.to_scale_rotation_translation();
                                hand_rot.inverse() * panel_rot
                            });
                            grab = Some(Grab { hand: c.hand, panel: i, dist, offset, second: None, smoothed: p, followers, size0, tilt, axis: None });
                            grabbed_hand = Some(c.hand);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(g) = &mut grab {
                let main = controllers.iter().find(|c| c.hand == g.hand).copied();
                match main {
                    Some(c) if c.grip => {
                        let panel_w = match g.panel {
                            0 => wrist.width_m,
                            1 => dash.width_m,
                            2 => kb.width_m,
                            k => wins[k - WIN0].panel.width_m,
                        };
                        let mut width = panel_w;
                        // Two-hand scale wins over stick scale while active.
                        let two = g.second.and_then(|(h2, sep0, w0)| {
                            controllers.iter().find(|o| o.hand == h2 && o.grip).map(|o| {
                                let sep = (to_affine(&c.pose).translation - to_affine(&o.pose).translation).length();
                                w0 * sep / sep0
                            })
                        });
                        let (sx, sy) = if dev_view.vr_stick_axis_lock {
                            lock_axis(&mut g.axis, stick(c.stick[0]), stick(c.stick[1]))
                        } else {
                            (stick(c.stick[0]), stick(c.stick[1]))
                        };
                        let resizing = two.is_some() || sx != 0.0;
                        match two {
                            Some(w) => width = w,
                            None => {
                                g.second = None;
                                width *= (sx * SCALE_RATE * dt).exp();
                                g.dist = (g.dist * (sy * PUSH_RATE * dt).exp()).clamp(GRAB_DIST.0, GRAB_DIST.1);
                            }
                        }
                        let width = width.clamp(PANEL_WIDTH.0, PANEL_WIDTH.1);
                        hud = Some((g.panel, rsoverlay_ui::shell::GrabHud { width_m: width, dist_m: g.dist, resizing }));
                        let hit = Vec3A::from(c.origin) + Vec3A::from(c.dir) * g.dist;
                        // Orient first (from the grab point), then hang the
                        // panel off it by the scaled grab offset.
                        let (_, hand_rot, _) = to_affine(&c.pose).to_scale_rotation_translation();
                        let target = grab_target(hit, g.offset, width, g.tilt, hand_rot, eye);
                        g.smoothed = smooth(g.smoothed, target, dt, dev_view.pointer.grab_smoothing);
                        let p = g.smoothed;
                        if g.panel == 0 {
                            // The wrist stays parented: store the new pose
                            // relative to its controller.
                            if let Some((dev, hand_pose)) =
                                wrist_parent.and_then(|d| device_pose(d, hmd.as_ref(), &controllers).map(|hp| (d, hp)))
                            {
                                wrist_rel = hand_pose.inverse() * p;
                                if let Err(e) = overlay.set_transform_device_relative(wrist.handle, dev, &from_affine(&wrist_rel)) {
                                    warn!("move wrist: {e:#}");
                                }
                            }
                            wrist.set_width(&overlay, width);
                        } else if let Err(e) = overlay.set_transform_absolute(
                            match g.panel {
                                1 => dash.handle,
                                2 => kb.handle,
                                k => wins[k - WIN0].panel.handle,
                            },
                            &from_affine(&p),
                        ) {
                            warn!("move panel: {e:#}");
                        }
                        if g.panel == 0 {
                        } else if g.panel == 1 {
                            dash_pose = Some(p);
                            dash.set_width(&overlay, width);
                        } else if g.panel == 2 {
                            kb_pose = Some(p);
                            kb.set_width(&overlay, width);
                        } else {
                            let k = g.panel - WIN0;
                            wins[k].world = p;
                            wins[k].panel.set_width(&overlay, width);
                            let size = wins[k].size_m();
                            for (seq, rel) in &g.followers {
                                if let Some(f) = wins.iter_mut().find(|w| w.seq == *seq) {
                                    f.world = p * push_follower(*rel, g.size0, size);
                                    if let Err(e) = overlay.set_transform_absolute(f.panel.handle, &from_affine(&f.world)) {
                                        warn!("move docked window: {e:#}");
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        let g = grab.take();
                        released = g.as_ref().map(|g| g.panel);
                        released_hand = g.map(|g| g.hand);
                    }
                }
            }
            // Dropped a widget onto another widget → merge into one tab group.
            let mut merged = false;
            if let Some(i) = released.filter(|&i| i >= WIN0) {
                let k = i - WIN0;
                if wins.get(k).is_some_and(|w| w.widgets.is_some()) {
                    let here = win_world(&wins[k], hmd.as_ref(), &controllers).translation;
                    let target = (0..wins.len()).find(|&j| {
                        j != k
                            && wins[j].widgets.is_some()
                            && (win_world(&wins[j], hmd.as_ref(), &controllers).translation - here).length() < MERGE_DIST_M
                    });
                    if let Some(j) = target {
                        let mut w = wins.remove(k);
                        let j = if j > k { j - 1 } else { j };
                        if let (Some((moved, _)), Some((tabs, active))) = (w.widgets.take(), wins[j].widgets.as_mut()) {
                            for t in moved {
                                if !tabs.contains(&t) {
                                    tabs.push(t);
                                }
                            }
                            *active = tabs.len() - 1;
                        }
                        w.panel.destroy(&overlay);
                        merged = true;
                        info!("merged widget panels");
                    }
                }
            }
            // Dropped a window / widget beside another → snap edges flush and
            // link them (they move together until Unlink). Skipped after a
            // merge: the dropped panel is gone and indices have shifted.
            if let Some(k) = released.filter(|&i| i >= WIN0 && !merged).map(|i| i - WIN0) {
                if wins.get(k).is_some_and(|w| w.attach == Attach::World) {
                    let own = wins[k].group;
                    let others: Vec<_> = wins
                        .iter()
                        .enumerate()
                        .filter(|&(j, w)| j != k && w.attach == Attach::World && (own.is_none() || w.group != own))
                        .map(|(j, w)| (j, w.world, w.size_m()))
                        .collect();
                    if let Some((j, pose)) = snap_pose(&wins[k].world, wins[k].size_m(), &others) {
                        // Shift the dropped window's whole group by the snap.
                        let delta = pose * wins[k].world.inverse();
                        let gid = wins[j].group.unwrap_or_else(|| {
                            next_group += 1;
                            next_group
                        });
                        for (m, w) in wins.iter_mut().enumerate() {
                            if m == k || (own.is_some() && w.group == own) {
                                w.world = delta * w.world;
                                if let Err(e) = overlay.set_transform_absolute(w.panel.handle, &from_affine(&w.world)) {
                                    warn!("dock window: {e:#}");
                                }
                                w.group = Some(gid);
                            }
                        }
                        wins[j].group = Some(gid);
                        if let (Some(h), Some(i)) = (released_hand, &input) {
                            i.haptic(h, GRAB_HAPTIC_S, dev_view.pointer.haptics);
                        }
                        info!("docked panel edges");
                    }
                }
            }
            // The grabbing hand's stick drives push/scale, not scrolling.
            if let Some(g) = &grab {
                let h = match g.hand {
                    Hand::Left => laser::Hand::Left,
                    Hand::Right => laser::Hand::Right,
                };
                for list in std::iter::once(&mut wrist_samples).chain(std::iter::once(&mut dash_samples)).chain(win_samples.iter_mut()) {
                    for smp in list.iter_mut().filter(|s| s.hand == h) {
                        smp.stick_x = 0.0;
                        smp.stick_y = 0.0;
                    }
                }
            }

            // ── Keyboard on SteamVR's laser: engaged while our laser
            // reaches it; SteamVR's mouse events then drive the keys.
            if kb.points != kb_mouse_scale {
                kb_mouse_scale = kb.points;
                if let Err(e) = overlay.set_mouse_input(kb.handle, kb.points.x, kb.points.y) {
                    warn!("keyboard on SteamVR's laser: {e:#}");
                }
            }
            let kb_events: Vec<(usize, KbMouse)> = overlay
                .mouse_events(kb.handle, kb.points.y)
                .into_iter()
                .filter(|e| e.button <= 1)
                .map(|e| {
                    let hand = controllers.iter().find(|c| c.index == e.device).map_or(1, |c| hand_slot(c.hand));
                    let m = match e.kind {
                        rsoverlay_openvr::MouseKind::Move => KbMouse::Move(e.x, e.y),
                        rsoverlay_openvr::MouseKind::Down => KbMouse::Down(e.x, e.y),
                        rsoverlay_openvr::MouseKind::Up => KbMouse::Up,
                    };
                    (hand, m)
                })
                .collect();
            let ray_on_kb = (0..2).find(|&s| hand_hit[s].is_some_and(|(i, _)| i == 2)).filter(|_| kb.visible);
            let kb_steam = steam_kb.step(ray_on_kb, &kb_events, time);
            if kb_steam.steam && !kb_events.is_empty() && !steam_kb_logged.0 {
                steam_kb_logged.0 = true;
                info!("keyboard: on SteamVR's laser");
            }
            if ray_on_kb.is_some() && !kb_steam.steam && !steam_kb_logged.1 {
                steam_kb_logged.1 = true;
                warn!("keyboard: SteamVR's laser didn't answer; using RSOverlay's");
            }

            // ── Block game input while the laser is on a panel (opt-in):
            // the flag sits only on the panel being pointed at.
            // The keyboard on SteamVR's laser always needs the flag.
            let want_capture = kb_steam
                .capture
                .then_some(2)
                .or_else(|| dev_view.pointer.block_game_input.then(|| hand_hit.iter().flatten().next().map(|(i, _)| *i)).flatten())
                .map(|i| match i {
                    0 => wrist.handle,
                    1 => dash.handle,
                    2 => kb.handle,
                    k => wins[k - WIN0].panel.handle,
                });
            if want_capture != capture_on {
                if let Some(h) = capture_on {
                    let _ = overlay.set_input_capture(h, false);
                }
                if let Some(h) = want_capture {
                    if let Err(e) = overlay.set_input_capture(h, true) {
                        warn!("block game input: {e:#}");
                    }
                }
                capture_on = want_capture;
            }

            // ── Laser beams: drawn while a hand points at any panel; bright
            // while the trigger is held. Billboarded around their own axis
            // so they never go edge-on.
            for c in &controllers {
                let s = hand_slot(c.hand);
                let b = &mut beams[s];
                // On the keyboard SteamVR draws its own laser.
                let hit = hand_hit[s].filter(|(i, _)| dev_view.vr_show_lasers && !(*i == 2 && kb_steam.steam));
                b.set_visible(&overlay, hit.is_some());
                let (Some((_, len)), Some(e)) = (hit, eye) else { continue };
                let o = Vec3A::from(c.origin);
                let d = Vec3A::from(c.dir);
                let len = len.max(0.05);
                let mid = o + d * (len * 0.5);
                let to_eye = e - mid;
                let z = (to_eye - d * to_eye.dot(d)).try_normalize().unwrap_or(Vec3A::Y);
                let pose = Affine3A::from_cols(d, z.cross(d), z, mid);
                let _ = overlay.set_width(b.handle, len);
                let _ = overlay.set_texel_aspect(b.handle, len * BEAM_TEX.1 as f32 / (BEAM_TEX.0 as f32 * BEAM_THICKNESS_M * dev_view.vr_pointer_size));
                // Parent to the controller: the compositor re-poses it
                // every frame with the freshest tracking, so the beam
                // never trails the hand (absolute poses lagged a frame).
                let rel = to_affine(&c.pose).inverse() * pose;
                let _ = overlay.set_transform_device_relative(b.handle, c.index, &from_affine(&rel));
                let a = dev_view.effective_accent();
                let tint = [
                    a.r() as f32 / 255.0,
                    a.g() as f32 / 255.0,
                    a.b() as f32 / 255.0,
                    if c.trigger > 0.5 { 0.95 } else { 0.55 },
                ];
                if tint != beam_tint[s] {
                    let _ = overlay.set_color(b.handle, [tint[0], tint[1], tint[2]]);
                    let _ = overlay.set_alpha(b.handle, tint[3]);
                    beam_tint[s] = tint;
                }
            }
            for (i, b) in beams.iter_mut().enumerate() {
                if !controllers.iter().any(|c| hand_slot(c.hand) == i) {
                    b.set_visible(&overlay, false);
                }
            }
            // ── Laser cursors: on the beam tip, parented to the
            // controller like the beam (see vr_logic::cursor_pose).
            for (s, cur) in cursors.iter_mut().enumerate() {
                let c = controllers.iter().find(|c| hand_slot(c.hand) == s);
                // Wrist and dashboard draw their own cursor in-panel:
                // SteamVR showed this overlay under them (tester; the log
                // put it 1 mm in front of the wrist, still unseen).
                let hit = hand_hit[s].filter(|(i, _)| *i >= WIN0 || (*i == 2 && !kb_steam.steam));
                let place = c.zip(hit).zip(eye);
                cur.set_visible(&overlay, place.is_some());
                let Some(((c, (_, dist)), e)) = place else { continue };
                let pose = cursor_pose(Vec3A::from(c.origin), Vec3A::from(c.dir), dist, e);
                let pressed = c.trigger > 0.5;
                let _ = overlay.set_width(cur.handle, CURSOR_M * dev_view.vr_pointer_size * if pressed { 0.8 } else { 1.0 });
                let rel = to_affine(&c.pose).inverse() * pose;
                let _ = overlay.set_transform_device_relative(cur.handle, c.index, &from_affine(&rel));
                let a = dev_view.effective_accent();
                let tint = [a.r() as f32 / 255.0, a.g() as f32 / 255.0, a.b() as f32 / 255.0, 1.0];
                if tint != cursor_tint[s] {
                    let _ = overlay.set_color(cur.handle, [tint[0], tint[1], tint[2]]);
                    cursor_tint[s] = tint;
                }
            }

            // Grab / release: a firmer haptic tick than a click.
            for h in grabbed_hand.into_iter().chain(released_hand) {
                match &input {
                    Some(i) => i.haptic(h, GRAB_HAPTIC_S, 0.6 * dev_view.pointer.haptics),
                    None => {
                        if let (Some(sys), Some(c)) = (&system, controllers.iter().find(|c| c.hand == h)) {
                            sys.haptic(c.index, (2500.0 * dev_view.pointer.haptics) as u16);
                        }
                    }
                }
            }

            // Pointer settings (Settings › Pointer) on every panel's laser.
            for p in [&mut wrist.laser, &mut dash.laser].into_iter().chain(wins.iter_mut().map(|w| &mut w.panel.laser)) {
                // The ray is already steadied (RayStabilizer): smoothing
                // here again would put clicks off the drawn cursor.
                p.smoothing = 0.0;
                p.scroll_mult = dev_view.scroll_speed;
                p.long_press_s = dev_view.pointer.long_press_secs();
                p.double_click_s = dev_view.pointer.double_click_s;
            }

            // Accessibility text size.
            let scale = dev_view.vr_ui_scale.clamp(0.8, 1.6);
            wrist.apply_scale(scale);
            dash.apply_scale(scale);
            kb.apply_scale(scale);
            let k = dev_view.vr_render_scale;
            for p in [&mut wrist, &mut dash, &mut kb, &mut toasts] {
                p.set_render_scale(&gpu, k);
            }
            // Window captures: native resolution — one capture pixel
            // per texel (see vr_logic::native_render_k).
            let held = grab.as_ref().map(|g| g.panel);
            for (i, w) in wins.iter_mut().enumerate() {
                match w.mirror.as_ref().map(|m| m.handle().width) {
                    Some(src) => {
                        let drawn = w.content_frac * w.panel.tex.0 as f32;
                        let want = native_render_k(w.panel.render_k, src, drawn);
                        if let Some(nk) = w.res.step(w.panel.render_k, want, time, held == Some(i + WIN0)) {
                            info!("window {}: {} px capture → {:.0} texel panel", w.seq, src, w.panel.base_tex.0 as f32 * nk);
                            w.panel.set_render_scale(&gpu, nk);
                        }
                    }
                    None => w.panel.set_render_scale(&gpu, k),
                }
            }
            for w in &mut wins {
                w.panel.apply_scale(scale);
            }

            // ── Wrist frame.
            let wf = wrist.laser.update(&wrist_samples, dt);
            let mut act = rsoverlay_ui::wrist::WristAction::default();
            if wrist.visible {
                let accent = dev_view.effective_accent();
                let cursor = wf.cursor;
                wrist.render(&gpu, &overlay, time, dt, wf.events, |ctx| {
                    act = dev_view.wrist_ui(ctx);
                    if let Some((0, h)) = hud {
                        rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                    }
                    laser::paint_cursor(ctx, cursor, accent);
                });
            }
            // Gamepad shortcuts drive the same actions as the wrist.
            let pad_now = if dev_view.gamepad.enabled { gamepads.poll() } else { 0 };
            dev_view.gamepad_held = pad_now;
            {
                use rsoverlay_input::gamepad::chord_pressed;
                let g = &dev_view.gamepad;
                act.toggle_dashboard |= chord_pressed(pad_prev, pad_now, g.dashboard);
                act.recenter |= chord_pressed(pad_prev, pad_now, g.recenter);
                act.toggle_keyboard ^= chord_pressed(pad_prev, pad_now, g.keyboard);
            }
            pad_prev = pad_now;
            toggle_dash |= act.toggle_dashboard;
            if act.toggle_keyboard {
                dev_view.show_keyboard = !dev_view.show_keyboard;
            }
            if let Some(p) = act.open_page {
                if dash.visible && dev_view.page == p {
                    // Same button again: hide it (toggle, like XSO).
                    toggle_dash = true;
                } else {
                    dev_view.page = p;
                    if !dash.visible {
                        toggle_dash = true;
                    }
                }
            }
            if let Some(l) = act.layout {
                dev_view.layout_request = Some(l);
            }
            if act.popout_stats {
                // Toggle: close an open stats widget, else pop one out.
                match wins.iter().position(|w| w.widgets.as_ref().is_some_and(|(t, _)| t.contains(&WidgetKind::System))) {
                    Some(k) => {
                        let w = wins.remove(k);
                        if grab.as_ref().is_some_and(|g| g.panel >= WIN0) {
                            grab = None;
                        }
                        w.panel.destroy(&overlay);
                    }
                    None => rsoverlay_ui::widgets::request_popout(&dash.egui.ctx, WidgetKind::System),
                }
            }
            if act.close_all {
                grab = None;
                for mut w in wins.drain(..) {
                    if let Some(m) = w.mirror.take() {
                        m.shutdown(&mut w.panel.egui);
                    }
                    w.panel.destroy(&overlay);
                }
            }
            if let (true, Some(h)) = (act.recenter, &hmd) {
                // Gather every world-docked panel into an arc in front of
                // you (XSO's recenter); hand/head-docked ones stay put.
                let base = summon_pose(h);
                let eye = to_affine(h).translation;
                let world: Vec<usize> = (0..wins.len()).filter(|&k| wins[k].attach == Attach::World).collect();
                let n = world.len() as f32;
                for (j, &k) in world.iter().enumerate() {
                    let t = if n > 1.0 { j as f32 / (n - 1.0) - 0.5 } else { 0.0 };
                    let pos = base.translation - base.matrix3.z_axis * 0.25 + base.matrix3.x_axis * (t * 0.55 * n.min(4.0));
                    let p = face(pos, eye);
                    let _ = overlay.set_transform_absolute(wins[k].panel.handle, &from_affine(&p));
                    wins[k].world = p;
                }
                if kb.visible && !keyboard.locked_pos {
                    let fwd = -base.matrix3.z_axis;
                    let pos = eye + Vec3A::new(fwd.x, 0.0, fwd.z).normalize_or_zero() * KB_DISTANCE_M + Vec3A::new(0.0, KB_DROP_M, 0.0);
                    let p = Affine3A { translation: pos, ..base } * Affine3A::from_rotation_x(KB_TILT_DEG.to_radians());
                    let p = Affine3A { translation: pos, ..p };
                    let _ = overlay.set_transform_absolute(kb.handle, &from_affine(&p));
                    kb_pose = Some(p);
                }
            }

            // ── Dashboard bar: close + dock, the same as every window.
            if std::mem::take(&mut dev_view.dash_close) && dash.visible {
                toggle_dash = true;
            }
            if let (Some(a), Some(cur)) = (dev_view.dash_dock.take(), dash_pose) {
                let dev = match a {
                    Attach::World => None,
                    Attach::Head => Some(0),
                    Attach::LeftHand => controllers.iter().find(|c| c.hand == Hand::Left).map(|c| c.index),
                    Attach::RightHand => controllers.iter().find(|c| c.hand == Hand::Right).map(|c| c.index),
                };
                match dev.and_then(|d| device_pose(d, hmd.as_ref(), &controllers).map(|p| (d, p))) {
                    Some((d, p)) => {
                        let rel = p.inverse() * cur;
                        match overlay.set_transform_device_relative(dash.handle, d, &from_affine(&rel)) {
                            Ok(()) => {
                                dash_rel = Some((d, rel));
                                dev_view.dash_attach = a;
                            }
                            Err(e) => warn!("dock dashboard: {e:#}"),
                        }
                    }
                    None if a == Attach::World => match overlay.set_transform_absolute(dash.handle, &from_affine(&cur)) {
                        Ok(()) => {
                            dash_rel = None;
                            dev_view.dash_attach = a;
                        }
                        Err(e) => warn!("dock dashboard: {e:#}"),
                    },
                    None => warn!("dock dashboard to {}: device not tracked", a.label()),
                }
            }

            // ── Summon / dismiss the dashboard.
            if toggle_dash {
                let show = !dash.visible;
                if show && dash_rel.is_none() {
                    // Re-summon in front of you every time, XSO-style; a
                    // grabbed position lasts until the next dismiss. A
                    // docked dashboard stays where it's docked.
                    if let Some(h) = &hmd {
                        let p = summon_pose(h);
                        if let Err(e) = overlay.set_transform_absolute(dash.handle, &from_affine(&p)) {
                            warn!("place dashboard: {e:#}");
                        }
                        dash_pose = Some(p);
                    }
                } else if grab.as_ref().is_some_and(|g| g.panel == 1) {
                    grab = None;
                }
                dash.set_visible(&overlay, show);
            }

            // ── Dashboard frame (only while visible — hidden costs nothing).
            let mut df = dash.laser.update(&dash_samples, dt);
            if typed_target == Some(1) {
                df.events.append(&mut typed);
            }
            if dash.visible {
                let accent = dev_view.effective_accent();
                let inj = injector.as_ref();
                let cursor = df.cursor;
                dash.render(&gpu, &overlay, time, dt, df.events, |ctx| {
                    dev_view.ui_with(ctx, inj);
                    if let Some((1, h)) = hud {
                        rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                    }
                    laser::paint_cursor(ctx, cursor, accent);
                });
            }

            // ── Card ↗ pop-out → new widget panel left of the dashboard.
            if let Some(kind) = dev_view.take_widget_request(&dash.egui.ctx) {
                if wins.len() >= MAX_FLOATING {
                    warn!("floating panel limit ({MAX_FLOATING}) reached");
                } else if let Some(base) = dash_pose.or_else(|| hmd.as_ref().map(summon_pose)) {
                    let n = wins.iter().filter(|w| w.widgets.is_some()).count() as f32;
                    let world = Affine3A {
                        translation: base.translation - base.matrix3.x_axis * (DASH_WIDTH_M * 0.5 + WID_WIDTH_M * 0.6)
                            + base.matrix3.y_axis * (0.25 - 0.3 * (n % 3.0))
                            + base.matrix3.z_axis * 0.05,
                        ..base
                    };
                    win_seq += 1;
                    match new_widget(&overlay, &gpu, win_seq, vec![kind], world) {
                        Ok(w) => {
                            info!("widget popped out: {}", kind.label());
                            wins.push(w);
                        }
                        Err(e) => warn!("pop out widget: {e:#}"),
                    }
                }
            }

            // ── Keyboard overlay (own panel, both hands type).
            if dev_view.show_keyboard != kb.visible {
                if dev_view.show_keyboard {
                    if let Some(h) = &hmd {
                        // Ahead and low, face tilted up toward you.
                        let base = summon_pose(h);
                        let fwd = -base.matrix3.z_axis;
                        let pos = to_affine(h).translation + Vec3A::new(fwd.x, 0.0, fwd.z).normalize_or_zero() * KB_DISTANCE_M
                            + Vec3A::new(0.0, KB_DROP_M, 0.0);
                        let tilt = Affine3A::from_rotation_x(KB_TILT_DEG.to_radians());
                        let p = Affine3A { translation: pos, ..base } * tilt;
                        let p = Affine3A { translation: pos, ..p };
                        let _ = overlay.set_transform_absolute(kb.handle, &from_affine(&p));
                        kb_pose = Some(p);
                    }
                } else {
                    keyboard.release_all();
                    if grab.as_ref().is_some_and(|g| g.panel == 2) {
                        grab = None;
                    }
                }
                kb.set_visible(&overlay, dev_view.show_keyboard);
            }
            // Which RSOverlay panel has a focused text field (if any)?
            let text_target = if dash.visible && dash.egui.ctx.wants_keyboard_input() {
                Some(1)
            } else {
                wins.iter().position(|w| w.panel.egui.ctx.wants_keyboard_input()).map(|k| k + WIN0)
            };
            // Focusing a field opens the keyboard (once — closing it stays closed).
            if text_target.is_some() && text_target != prev_text_target && !dev_view.show_keyboard {
                dev_view.show_keyboard = true;
            }
            prev_text_target = text_target;
            let internal = text_target.is_some();
            let mut kb_haptics = Vec::new();
            if kb.visible {
                let hands: Vec<(laser::Hand, Option<egui::Pos2>, f32)> = if kb_steam.steam {
                    let hand = if kb_steam.hand == 0 { laser::Hand::Left } else { laser::Hand::Right };
                    vec![(hand, kb_steam.pos.map(|(x, y)| egui::pos2(x, y)), if kb_steam.pressed { 1.0 } else { 0.0 })]
                } else {
                    kb_samples.iter().map(|s| (s.hand, s.hit, s.trigger)).collect()
                };
                let accent = dev_view.effective_accent();
                let mut kf = rsoverlay_ui::vrkeyboard::KbFrame::default();
                kb.render(&gpu, &overlay, time, dt, Vec::new(), |ctx| {
                    kf = keyboard.frame(ctx, &hands, time, accent, internal);
                    if let Some((2, h)) = hud {
                        rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                    }
                });
                kb_haptics = kf.haptic;
                if dev_view.kb_prefs.sounds && !kf.sounds.is_empty() {
                    let pack = Some(dev_view.kb_prefs.pack.as_str()).filter(|p| !p.is_empty());
                    typing_sounds.configure(&sounds_dir, pack, dev_view.kb_prefs.volume);
                    // One click per frame is plenty (two hands on one frame
                    // would just cut each other off).
                    if let Some(k) = kf.sounds.last() {
                        typing_sounds.play(*k);
                    }
                }
                if let Some(msg) = kf.chat.take() {
                    dev_view.send_chat(msg);
                }
                if !kf.text.is_empty() {
                    typed.extend(kf.text);
                    typed_target = text_target;
                }
                if kf.close {
                    dev_view.show_keyboard = false;
                }
            }

            // ── Desktop page: list refresh + open requests.
            if std::mem::take(&mut dev_view.sources.wants_refresh) {
                dev_view.sources.choices = list_sources();
                fetch_icons(&dev_view.sources.choices, &mut dev_view.sources.icons);
                crate::mirror_source::fetch_apps(&dev_view.sources.choices, &mut dev_view.sources.apps);
                thumbs.request(&dev_view.sources.choices);
            }
            thumbs.collect(&mut dev_view.sources);
            // ── Monitor wall: every monitor, arced around you, left to
            // right as they sit on the desk.
            if std::mem::take(&mut dev_view.sources.wall_request) {
                if let Some(h) = &hmd {
                    let monitors: Vec<_> = list_sources()
                        .into_iter()
                        .filter(|s| matches!(s, rsoverlay_ui::views::CaptureSource::Monitor(_)))
                        .collect();
                    let base = summon_pose(h);
                    for src in monitors {
                        let open = wins.iter().any(|w| w.mirror.as_ref().is_some_and(|m| m.source == src));
                        if open {
                            continue;
                        }
                        if wins.iter().filter(|w| w.mirror.is_some()).count() >= MAX_WINDOWS || wins.len() >= MAX_FLOATING {
                            warn!("monitor wall: window limit ({MAX_WINDOWS}) reached");
                            break;
                        }
                        win_seq += 1;
                        match open_window(&overlay, &gpu, win_seq, src, base, &dev_view.sources.icons) {
                            Ok(w) => wins.push(w),
                            Err(e) => warn!("monitor wall: {e:#}"),
                        }
                    }
                    // Arrange every open monitor panel, by desktop x.
                    let mut wall: Vec<(i32, usize)> = wins
                        .iter()
                        .enumerate()
                        .filter_map(|(k, w)| {
                            let m = w.mirror.as_ref()?;
                            matches!(m.source, rsoverlay_ui::views::CaptureSource::Monitor(_)).then(|| (m.geom().map_or(0, |g| g.origin_x), k))
                        })
                        .collect();
                    wall.sort();
                    let eye = to_affine(h).translation;
                    let fwd = -to_affine(h).matrix3.z_axis;
                    let widths = vec![WALL_WIDTH_M; wall.len()];
                    for (pose, (_, k)) in arc_poses(eye, fwd, &widths, WALL_RADIUS_M, WALL_GAP_M, DASH_DROP_M).into_iter().zip(wall) {
                        let w = &mut wins[k];
                        w.rel = None;
                        w.attach = Attach::World;
                        w.group = None;
                        w.world = pose;
                        w.pinned = true;
                        w.panel.set_width(&overlay, WALL_WIDTH_M);
                        if let Err(e) = overlay.set_transform_absolute(w.panel.handle, &from_affine(&pose)) {
                            warn!("monitor wall: place: {e:#}");
                        }
                    }
                }
            }
            // ── Big screen: the main monitor, large and curved, ahead.
            if std::mem::take(&mut dev_view.sources.big_screen_request) {
                if let Some(h) = &hmd {
                    let main = rsoverlay_ui::views::CaptureSource::Monitor(0);
                    let k = match wins.iter().position(|w| w.mirror.as_ref().is_some_and(|m| m.source == main)) {
                        Some(k) => Some(k),
                        None if wins.iter().filter(|w| w.mirror.is_some()).count() < MAX_WINDOWS && wins.len() < MAX_FLOATING => {
                            win_seq += 1;
                            match open_window(&overlay, &gpu, win_seq, main, summon_pose(h), &dev_view.sources.icons) {
                                Ok(w) => {
                                    wins.push(w);
                                    Some(wins.len() - 1)
                                }
                                Err(e) => {
                                    warn!("big screen: {e:#}");
                                    None
                                }
                            }
                        }
                        None => {
                            warn!("big screen: window limit ({MAX_WINDOWS}) reached");
                            None
                        }
                    };
                    if let Some(k) = k {
                        let eye = to_affine(h).translation;
                        let fwd = -to_affine(h).matrix3.z_axis;
                        let pose = arc_poses(eye, fwd, &[BIG_WIDTH_M], BIG_DISTANCE_M, 0.0, DASH_DROP_M)[0];
                        let w = &mut wins[k];
                        w.rel = None;
                        w.attach = Attach::World;
                        w.group = None;
                        w.world = pose;
                        w.pinned = true;
                        w.curvature = BIG_CURVATURE;
                        w.panel.set_width(&overlay, BIG_WIDTH_M);
                        let _ = overlay.set_curvature(w.panel.handle, BIG_CURVATURE);
                        if let Err(e) = overlay.set_transform_absolute(w.panel.handle, &from_affine(&pose)) {
                            warn!("big screen: place: {e:#}");
                        }
                    }
                }
            }
            if let Some(src) = dev_view.sources.request.take() {
                if wins.iter().filter(|w| w.mirror.is_some()).count() >= MAX_WINDOWS || wins.len() >= MAX_FLOATING {
                    warn!("window overlay limit ({MAX_WINDOWS}) reached");
                } else if let Some(h) = &hmd {
                    // Behind the dashboard, fanned to the right per
                    // already-open window.
                    let base = summon_pose(h);
                    let fwd = -base.matrix3.z_axis;
                    let side = base.matrix3.x_axis * (0.25 * wins.len() as f32);
                    let world = Affine3A { translation: base.translation + fwd * 0.20 + side, ..base };
                    win_seq += 1;
                    match open_window(&overlay, &gpu, win_seq, src, world, &dev_view.sources.icons) {
                        Ok(w) => wins.push(w),
                        Err(e) => warn!("window overlay: {e:#}"),
                    }
                }
            }

            // ── Window overlays.
            let mut close: Vec<usize> = Vec::new();
            let mut splits: Vec<(usize, WidgetKind)> = Vec::new();
            let mut win_haptics = Vec::new();
            for (k, w) in wins.iter_mut().enumerate() {
                let mut lf = w.panel.laser.update(win_samples.get(k).map_or(&[][..], |v| v.as_slice()), dt);
                if typed_target == Some(k + WIN0) {
                    lf.events.append(&mut typed);
                }
                win_haptics.extend(lf.haptic);
                // Looked-away panels refresh ~5×/s: the GPU only works
                // hard for what you're looking at.
                let background_s = 1.0 / f64::from(dev_view.vr_background_fps.clamp(1.0, 90.0));
                // Focus mode: dimmed windows are background too.
                let dimmed = focus_win.is_some_and(|f| f != w.seq);
                if !should_render(w.panel.in_view, dimmed, w.smooth, time - w.panel.last_render, background_s, !lf.events.is_empty()) {
                    continue;
                }
                let accent = dev_view.effective_accent();
                let mut act = window_panel::WindowPanelAction::default();
                if let Some((tabs, active)) = w.widgets.clone() {
                    // Widget group: tabs + live card + dock bar.
                    let (attach, linked) = (w.attach, w.group.is_some());
                    let pinned = Some(w.pinned);
                    let inj = injector.as_ref();
                    let mut wa = rsoverlay_ui::widgets::WidgetAction::default();
                    w.panel.render(&gpu, &overlay, time, dt, lf.events, |ctx| {
                        wa = dev_view.widget_ui(ctx, &tabs, active, attach, linked, pinned, inj);
                        if let Some((_, h)) = hud.filter(|(i, _)| *i == k + WIN0) {
                            rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                        }
                    });
                    if let Some(i) = wa.select {
                        w.widgets = Some((tabs.clone(), i));
                    }
                    if wa.split && tabs.len() > 1 {
                        splits.push((k, tabs[active]));
                    }
                    act.attach = wa.attach;
                    act.close = wa.close;
                    act.unlink = wa.unlink;
                    act.toggle_pin = wa.toggle_pin;
                } else if let Some(mirror) = &w.mirror {
                    if w.blocked_checked.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                        w.blocked_checked = Some(Instant::now());
                        w.input_blocked = mirror.window_hwnd().is_some_and(rsoverlay_input::elevation::input_blocked);
                    }
                    let (title, handle_m, attach, blocked) =
                        (mirror.source.label(), mirror.handle(), w.attach, w.input_blocked);
                    let touch_mode = dev_view.pointer.touch_input;
                    let icon = w.icon.as_ref().map(|t| t.id());
                    let (linked, opacity, pinned, smooth) = (w.group.is_some(), w.opacity, Some(w.pinned), w.smooth);
                    // Auto-hide: the toolbar shows while the laser is on the
                    // window or it's held, and lingers a moment after.
                    if lf.cursor.is_some() || grab.as_ref().is_some_and(|g| g.panel == k + WIN0) {
                        w.toolbar_seen = time;
                    }
                    let show_toolbar = toolbar_visible(dev_view.pointer.autohide_toolbars, time, w.toolbar_seen);
                    let events = lf.events.clone();
                    w.panel.render(&gpu, &overlay, time, dt, lf.events, |ctx| {
                        mirror.refresh(&gpu);
                        act = window_panel::show(ctx, &title, Some(handle_m), attach, blocked, icon, linked, opacity, pinned, show_toolbar, smooth, accent);
                        if let Some((_, h)) = hud.filter(|(i, _)| *i == k + WIN0) {
                            rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                        }
                    });
                    if let Some(r) = act.image_rect {
                        w.content_frac = (r.width() / w.panel.points.x).clamp(0.05, 1.0);
                    }
                    // The laser is the real mouse on this window: moves,
                    // separate down/up (drag, select, hold), right/middle
                    // via the click modifiers, stick → wheel.
                    if let (Some(rect), Some(inj), Some(geom), false) = (act.image_rect, injector.as_ref(), mirror.geom(), blocked) {
                        let (cw, ch) = (handle_m.width as f32, handle_m.height as f32);
                        let to_px = |p: egui::Pos2| {
                            let u = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                            let v = ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                            ((u * cw) as i32, (v * ch) as i32)
                        };
                        for ev in &events {
                            match ev {
                                // Hover moves the real mouse only with "Mouse follows the
                                // laser"; a held button (drag) always does.
                                egui::Event::PointerMoved(p) if rect.contains(*p) || w.down.is_some() || w.touching => {
                                    let (x, y) = to_px(*p);
                                    if w.last_hit != Some((x, y)) {
                                        if w.touching {
                                            // The finger slides (drag / swipe).
                                            let _ = rsoverlay_input::touch::touch(&geom, x, y, rsoverlay_input::touch::Phase::Move);
                                        } else if w.down.is_some() {
                                            // Dragging: always follow.
                                            let _ = inj.move_to(&geom, x, y);
                                            mouse_owner.moved((geom.origin_x + x, geom.origin_y + y));
                                        } else if dev_view.pointer.mouse_follows && !touch_mode {
                                            // Hover: only past the tremor deadband,
                                            // and never while you use the PC mouse.
                                            if !beyond_deadband(w.last_hit, (x, y)) {
                                                continue;
                                            }
                                            if mouse_owner.may_hover(time) {
                                                let _ = inj.move_to(&geom, x, y);
                                                mouse_owner.moved((geom.origin_x + x, geom.origin_y + y));
                                            }
                                        }
                                        w.last_hit = Some((x, y));
                                    }
                                }
                                // Touch input: the trigger is a finger (right / middle
                                // clicks and the wheel stay on the mouse).
                                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } if touch_mode && (w.touching || rect.contains(*pos)) => {
                                    let (x, y) = to_px(*pos);
                                    if *pressed && !w.touching {
                                        if let Some(hwnd) = mirror.window_hwnd() {
                                            rsoverlay_capture::window::focus_window(hwnd);
                                        }
                                        w.touching = rsoverlay_input::touch::touch(&geom, x, y, rsoverlay_input::touch::Phase::Down)
                                            .map_err(|e| warn!("touch: {e:#}"))
                                            .is_ok();
                                    } else if !*pressed && w.touching {
                                        let _ = rsoverlay_input::touch::touch(&geom, x, y, rsoverlay_input::touch::Phase::Up);
                                        w.touching = false;
                                    }
                                }
                                egui::Event::PointerButton { pos, button, pressed, .. } => {
                                    let btn = match button {
                                        egui::PointerButton::Secondary => rsoverlay_input::MouseButton::Right,
                                        egui::PointerButton::Middle => rsoverlay_input::MouseButton::Middle,
                                        _ => rsoverlay_input::MouseButton::Left,
                                    };
                                    let (x, y) = to_px(*pos);
                                    if *pressed && rect.contains(*pos) {
                                        if let Some(hwnd) = mirror.window_hwnd() {
                                            rsoverlay_capture::window::focus_window(hwnd);
                                        }
                                        let _ = inj.button(&geom, x, y, btn, true);
                                        w.down = Some(btn);
                                    } else if !*pressed {
                                        if let Some(held) = w.down.take() {
                                            let _ = inj.button(&geom, x, y, held, false);
                                        }
                                    }
                                }
                                egui::Event::Scroll(v) if w.last_hit.is_some() => {
                                    // The wheel goes where the real cursor is: bring
                                    // it to the laser first when it doesn't follow.
                                    if let (false, Some((x, y))) = (dev_view.pointer.mouse_follows, w.last_hit) {
                                        let _ = inj.move_to(&geom, x, y);
                                    }
                                    // ~2 wheel units per point: smooth in
                                    // modern apps, one notch per ~60 pt.
                                    w.wheel_acc += v.y * 2.0;
                                    let send = (w.wheel_acc / 30.0).trunc() * 30.0;
                                    if send != 0.0 {
                                        let _ = inj.wheel(send as i32);
                                        w.wheel_acc -= send;
                                    }
                                    // Sideways: egui's -x is "scroll right".
                                    w.hwheel_acc -= v.x * 2.0;
                                    let send = (w.hwheel_acc / 30.0).trunc() * 30.0;
                                    if send != 0.0 {
                                        let _ = inj.hwheel(send as i32);
                                        w.hwheel_acc -= send;
                                    }
                                }
                                egui::Event::PointerGone => w.last_hit = None,
                                _ => {}
                            }
                        }
                    }
                }
                if act.unlink {
                    w.group = None;
                }
                w.pinned ^= act.toggle_pin;
                w.smooth ^= act.toggle_smooth;
                // Readable size: widen (or shrink) the panel so the text
                // is legible from where you are (see vr_logic).
                if act.readable {
                    let src = w.mirror.as_ref().map(|m| m.handle().width);
                    let head = hmd.as_ref().map(to_affine);
                    if let (Some(src), Some(ppr), Some(h)) = (src, px_per_rad, head) {
                        let p = win_world(w, hmd.as_ref(), &controllers);
                        let dist = (p.translation - h.translation).length();
                        let (width, capped) = readable_width(src, w.content_frac, dist, ppr);
                        info!("readable size: {src} px at {dist:.2} m → {width:.2} m wide{}", if capped { " (capped at 90°)" } else { "" });
                        w.panel.set_width(&overlay, width);
                    }
                }
                if let Some(o) = act.opacity {
                    w.opacity = o.clamp(window_panel::MIN_CONTENT_OPACITY, 1.0);
                }
                if let (Some((rw, rh)), Some(hwnd)) = (act.resize, w.mirror.as_ref().and_then(|m| m.window_hwnd())) {
                    match rsoverlay_capture::window::resize_window(hwnd, rw, rh) {
                        Ok(()) => info!("resized window to {rw}x{rh}"),
                        Err(e) => warn!("resize window: {e:#}"),
                    }
                }
                if let Some(a) = act.attach {
                    // Docked to head / hand → no longer part of a window wall.
                    w.group = None;
                    let cur = win_world(w, hmd.as_ref(), &controllers);
                    let dev = match a {
                        Attach::World => None,
                        Attach::Head => Some(0),
                        Attach::LeftHand => controllers.iter().find(|c| c.hand == Hand::Left).map(|c| c.index),
                        Attach::RightHand => controllers.iter().find(|c| c.hand == Hand::Right).map(|c| c.index),
                    };
                    let res = match dev.and_then(|d| device_pose(d, hmd.as_ref(), &controllers).map(|p| (d, p))) {
                        Some((d, p)) => {
                            let rel = p.inverse() * cur;
                            w.rel = Some((d, rel));
                            overlay.set_transform_device_relative(w.panel.handle, d, &from_affine(&rel))
                        }
                        None if a == Attach::World => {
                            w.rel = None;
                            w.world = cur;
                            overlay.set_transform_absolute(w.panel.handle, &from_affine(&cur))
                        }
                        None => {
                            warn!("dock to {}: device not tracked", a.label());
                            continue;
                        }
                    };
                    match res {
                        Ok(()) => w.attach = a,
                        Err(e) => warn!("dock window: {e:#}"),
                    }
                }
                if act.close {
                    close.push(k);
                }
            }
            for k in close.into_iter().rev() {
                let mut w = wins.remove(k);
                if grab.as_ref().is_some_and(|g| g.panel >= WIN0) {
                    grab = None;
                }
                if let Some(m) = w.mirror.take() {
                    m.shutdown(&mut w.panel.egui);
                }
                w.panel.destroy(&overlay);
            }
            // Unlink / re-dock / close can leave a one-window "group".
            prune_groups(&mut wins);
            // Split: the active tab leaves its group as a new panel beside it.
            for (k, kind) in splits.into_iter().rev() {
                if wins.len() >= MAX_FLOATING {
                    break;
                }
                let Some((tabs, active)) = wins.get_mut(k).and_then(|w| w.widgets.as_mut()) else { continue };
                tabs.retain(|t| *t != kind);
                *active = (*active).min(tabs.len().saturating_sub(1));
                let base = win_world(&wins[k], hmd.as_ref(), &controllers);
                let world = Affine3A { translation: base.translation + base.matrix3.x_axis * (WID_WIDTH_M + 0.04), ..base };
                win_seq += 1;
                match new_widget(&overlay, &gpu, win_seq, vec![kind], world) {
                    Ok(w) => wins.push(w),
                    Err(e) => warn!("split widget: {e:#}"),
                }
            }
            dev_view.open_windows = wins.iter().filter_map(|w| w.mirror.as_ref().map(|m| m.source.clone())).collect();

            // ── Layouts: save / load / favourite / clear. The favourite
            // loads once tracking is up.
            let mut load_now: Option<(String, rsoverlay_core::layouts::Layout)> = None;
            if hmd.is_some() {
                if let Some(i) = favorite_pending.take() {
                    if dev_view.layouts.slot(i).is_some() {
                        dev_view.layout_request = Some(rsoverlay_ui::layouts_page::LayoutAction::Load(i));
                    }
                }
                if let Some(l) = session_pending.take() {
                    load_now = Some(("last session".into(), l));
                }
            }
            if let Some(action) = dev_view.layout_request.take() {
                use rsoverlay_ui::layouts_page::LayoutAction;
                match action {
                    LayoutAction::Save(i) => {
                        let kbp = kb.visible.then_some(kb_pose).flatten().map(|p| (p, kb.width_m));
                        let layout = capture_layout(&wins, kbp);
                        info!("layout saved to slot {} ({} panels)", i + 1, layout.panels.len());
                        dev_view.layouts.set(i, Some(layout));
                    }
                    LayoutAction::Clear(i) => dev_view.layouts.set(i, None),
                    LayoutAction::ToggleFavorite(i) => {
                        dev_view.layouts.favorite = if dev_view.layouts.favorite == Some(i) { None } else { Some(i) };
                    }
                    LayoutAction::Load(i) => {
                        load_now = dev_view.layouts.slot(i).cloned().map(|l| (format!("slot {}", i + 1), l));
                    }
                }
                if let Err(e) = dev_view.layouts.save(&layouts_path) {
                    warn!("saving layouts: {e:#}");
                }
            }
            if let Some((what, layout)) = load_now.take() {
                for mut w in wins.drain(..) {
                    if let Some(m) = w.mirror.take() {
                        m.shutdown(&mut w.panel.egui);
                    }
                    w.panel.destroy(&overlay);
                }
                grab = None;
                let sources = list_sources();
                fetch_icons(&sources, &mut dev_view.sources.icons);
                // Saved group ids are per-layout; offset them past
                // any id handed out this session.
                let group_base = next_group;
                for p in &layout.panels {
                    let pose = to_affine(&HmdMatrix34_t { m: p.pose });
                    let attach = attach_from_slug(&p.attach);
                    let dev = match attach {
                        Attach::World => None,
                        Attach::Head => Some(0),
                        Attach::LeftHand => system.as_ref().and_then(|s| s.hand_index(Hand::Left)),
                        Attach::RightHand => system.as_ref().and_then(|s| s.hand_index(Hand::Right)),
                    };
                    // Relative poses need a world fallback for
                    // the first frame / missing device.
                    let world = dev
                        .and_then(|d| device_pose(d, hmd.as_ref(), &controllers))
                        .map_or(pose, |d| d * pose);
                    win_seq += 1;
                    let opened = match p.kind.as_str() {
                        "widgets" => {
                            let tabs: Vec<WidgetKind> = p.widgets.iter().filter_map(|s| WidgetKind::from_slug(s)).collect();
                            (!tabs.is_empty()).then(|| new_widget(&overlay, &gpu, win_seq, tabs, world))
                        }
                        "monitor" => Some(open_window(&overlay, &gpu, win_seq, rsoverlay_ui::views::CaptureSource::Monitor(p.monitor), world, &dev_view.sources.icons)),
                        _ => {
                            // Exact title, else same leading text
                            // (titles change: "Discord — #general").
                            let head: String = p.title.chars().take(12).collect();
                            sources
                                .iter()
                                .find(|s| s.label() == p.title)
                                .or_else(|| sources.iter().find(|s| matches!(s, rsoverlay_ui::views::CaptureSource::Window { .. }) && !head.is_empty() && s.label().starts_with(&head)))
                                .map(|src| open_window(&overlay, &gpu, win_seq, src.clone(), world, &dev_view.sources.icons))
                        }
                    };
                    match opened {
                        Some(Ok(mut w)) => {
                            w.panel.set_width(&overlay, p.width);
                            if let (Some(d), false) = (dev, attach == Attach::World) {
                                if overlay.set_transform_device_relative(w.panel.handle, d, &from_affine(&pose)).is_ok() {
                                    w.rel = Some((d, pose));
                                    w.attach = attach;
                                }
                            }
                            w.world = world;
                            w.group = p.group.filter(|_| attach == Attach::World).map(|g| group_base + 1 + g);
                            w.opacity = p.opacity.unwrap_or(1.0).clamp(window_panel::MIN_CONTENT_OPACITY, 1.0);
                            w.pinned = p.pinned;
                            w.smooth = p.smooth;
                            if let Some(c) = p.curvature.filter(|c| *c > 0.0) {
                                w.curvature = c.min(1.0);
                                let _ = overlay.set_curvature(w.panel.handle, w.curvature);
                            }
                            next_group = next_group.max(w.group.unwrap_or(0));
                            wins.push(w);
                        }
                        Some(Err(e)) => warn!("layout: reopening {:?}: {e:#}", p.title),
                        None => warn!("layout: window {:?} isn't open — skipped", p.title),
                    }
                }
                if let Some(k) = &layout.keyboard {
                    let p = to_affine(&HmdMatrix34_t { m: k.pose });
                    let _ = overlay.set_transform_absolute(kb.handle, &from_affine(&p));
                    kb.set_width(&overlay, k.width);
                    kb_pose = Some(p);
                    dev_view.show_keyboard = true;
                    kb.set_visible(&overlay, true);
                }
                // A docked partner whose window isn't open any more.
                prune_groups(&mut wins);
                info!("layout {what} loaded ({} panels)", wins.len());
            }

            // ── Session autosave: the open arrangement, every few seconds
            // when it changed (and once more at shutdown).
            if session_pending.is_none() && favorite_pending.is_none() && session_checked.elapsed() >= Duration::from_secs(3) {
                session_checked = Instant::now();
                let kbp = kb.visible.then_some(kb_pose).flatten().map(|p| (p, kb.width_m));
                let now = capture_layout(&wins, kbp);
                if !dev_view.layouts.last_session.as_ref().is_some_and(|l| l.same_arrangement(&now)) {
                    dev_view.layouts.last_session = Some(now);
                    if let Err(e) = dev_view.layouts.save(&layouts_path) {
                        warn!("saving session: {e:#}");
                    }
                }
            }

            // Mirror hands-on state into the config (autosaved, debounced).
            dev_view.vr_state = rsoverlay_core::config::VrState {
                wrist_pose: Some(from_affine(&wrist_rel).m),
                wrist_width: Some(wrist.width_m),
                kb_pinned: keyboard.locked_pos,
                kb_auto_repeat: keyboard.auto_repeat,
                kb_full: keyboard.full,
                kb_vrchat: keyboard.vrchat,
            };

            // ── Haptic ticks for clicks on any panel.
            if let Some(sys) = &system {
                for h in [wf.haptic, df.haptic].into_iter().flatten().chain(win_haptics).chain(kb_haptics) {
                    let want = match h {
                        laser::Hand::Left => Hand::Left,
                        laser::Hand::Right => Hand::Right,
                    };
                    match &input {
                        Some(i) => i.haptic(want, 0.012, 0.35 * dev_view.pointer.haptics),
                        None => {
                            if let Some(c) = controllers.iter().find(|c| c.hand == want) {
                                sys.haptic(c.index, (f32::from(CLICK_HAPTIC_US) * dev_view.pointer.haptics) as u16);
                            }
                        }
                    }
                }
            }

            // ── OSC out: chatbox + avatar parameters.
            if let Some(client) = &osc {
                for (name, value) in dev_view.take_pending_osc() {
                    if let Err(e) = client.send_param(name, value) {
                        warn!("osc: {e:#}");
                    }
                }
                if let Some(msg) = dev_view.take_pending_chat() {
                    if let Err(e) = client.send_chat(msg) {
                        warn!("osc: {e:#}");
                    }
                }
            }

            // ── Settings lifecycle: reset, external edits, persistence,
            // elevated restart.
            if dev_view.reset_requested {
                crate::preview::seed_dev_view(&mut dev_view, &rsoverlay_core::Config::default(), &crate::preview::TestHooks::default());
                dev_view.reset_requested = false;
            }
            if let Some(cfg) = persister.poll_external_change() {
                crate::preview::seed_dev_view(&mut dev_view, &cfg, &crate::preview::TestHooks::default());
            }
            persister.tick(&crate::preview::config_from_dev_view(&dev_view, &config_base));
            if std::mem::take(&mut dev_view.updates.restart_requested) {
                persister.tick(&crate::preview::config_from_dev_view(&dev_view, &config_base));
                persister.flush();
                match rsoverlay_core::update::spawn_updated() {
                    Ok(()) => {
                        info!("restarting on the new version");
                        running.store(false, Ordering::SeqCst);
                    }
                    Err(e) => warn!("restart after update: {e:#}"),
                }
            }
            if std::mem::take(&mut dev_view.restart_elevated_requested) {
                persister.tick(&crate::preview::config_from_dev_view(&dev_view, &config_base));
                persister.flush();
                let args: Vec<String> = std::env::args().skip(1).collect();
                if rsoverlay_input::elevation::relaunch_elevated(&args) {
                    info!("relaunching elevated");
                    running.store(false, Ordering::SeqCst);
                } else {
                    warn!("elevated relaunch cancelled or failed");
                }
            }

            // ── Notifications: drain the server, sounds, head-locked toasts.
            if let Some(n) = notify.as_mut() {
                while let Ok(notif) = n.rx.try_recv() {
                    dev_view.push_notification(notif);
                }
            }
            crate::preview::play_alert_sounds(&mut dev_view);
            if last_perf.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
                last_perf = Some(Instant::now());
                px_per_rad = system.as_ref().and_then(|s| s.pixels_per_radian());
                let hz = system.as_ref().and_then(|s| s.display_hz());
                dev_view.vr_perf = compositor.as_ref().zip(hz).and_then(|(c, hz)| {
                    let frames: Vec<rsoverlay_core::perf::Frame> = c
                        .frame_samples(90)
                        .into_iter()
                        .map(|f| rsoverlay_core::perf::Frame { interval_ms: f.interval_ms, gpu_ms: f.gpu_ms, cpu_ms: f.cpu_ms, presents: f.presents, dropped: f.dropped })
                        .collect();
                    rsoverlay_core::perf::summarize(&frames, hz)
                });
            }
            let want_toasts = dev_view.has_toasts() && hmd.is_some();
            if want_toasts {
                let place = (dev_view.notif.size_m, dev_view.notif.offset_y, dev_view.notif.distance_m);
                if toast_place != Some(place) {
                    toast_place = Some(place);
                    toasts.set_width(&overlay, place.0);
                    // Straight ahead of the eyes, facing them (+Z toward the HMD).
                    let pose = Affine3A::from_translation(glam::Vec3::new(0.0, place.1, -place.2));
                    if let Err(e) = overlay.set_transform_device_relative(toasts.handle, 0, &from_affine(&pose)) {
                        warn!("place toasts: {e:#}");
                    }
                }
                toasts.render(&gpu, &overlay, time, dt, Vec::new(), |ctx| dev_view.toasts_ui(ctx));
            }
            toasts.set_visible(&overlay, want_toasts);

            dev_view.record_frame_ms(frame_start.elapsed().as_secs_f32() * 1000.0);

            let tint = dev_view.overlay_tint();
            wrist.apply_tint(&overlay, [tint[0], tint[1], tint[2], tint[3] * dev_view.vr_wrist_opacity]);
            dash.apply_tint(&overlay, tint);
            kb.apply_tint(&overlay, [tint[0], tint[1], tint[2], tint[3] * dev_view.kb_prefs.opacity]);
            // Focus mode: everything but the window you work in steps back.
            // Nothing left to focus on (every window closed): Focus ends.
            if dev_view.focus_mode && wins.iter().all(|w| w.mirror.is_none()) && had_windows {
                dev_view.focus_mode = false;
                info!("focus mode off: no windows left");
            }
            had_windows = wins.iter().any(|w| w.mirror.is_some());
            focus_win = match (dev_view.focus_mode, hmd.as_ref().map(to_affine)) {
                (true, Some(h)) => {
                    let laser = hand_hit.iter().flatten().find_map(|(i, _)| i.checked_sub(WIN0)).and_then(|k| wins.get(k)).map(|w| w.seq);
                    let by_gaze: Vec<(u32, f32)> = wins
                        .iter()
                        .filter(|w| w.panel.visible && w.panel.fade > 0.5)
                        .map(|w| (w.seq, gaze_angle(&h, win_world(w, hmd.as_ref(), &controllers).translation)))
                        .collect();
                    pick_focus(focus_win, laser, &by_gaze)
                }
                _ => None,
            };
            for w in &mut wins {
                let dim = if focus_win.is_some_and(|f| f != w.seq) { FOCUS_DIM } else { 1.0 };
                w.panel.apply_tint(&overlay, [tint[0], tint[1], tint[2], tint[3] * dim]);
            }

            std::thread::sleep(Duration::from_millis(11));
        }

        // Persist everything before tearing down: the debounced config
        // write may still be pending, and the session should be exact.
        persister.tick(&crate::preview::config_from_dev_view(&dev_view, &config_base));
        persister.flush();
        if session_pending.is_none() && favorite_pending.is_none() {
            let kbp = kb.visible.then_some(kb_pose).flatten().map(|p| (p, kb.width_m));
            dev_view.layouts.last_session = Some(capture_layout(&wins, kbp));
            if let Err(e) = dev_view.layouts.save(&layouts_path) {
                warn!("saving session: {e:#}");
            }
        }
        for mut w in wins.drain(..) {
            if let Some(m) = w.mirror.take() {
                m.shutdown(&mut w.panel.egui);
            }
            w.panel.destroy(&overlay);
        }
        keyboard.release_all();
        kb.destroy(&overlay);
        for b in &beams {
            b.destroy(&overlay);
        }
        for c in &cursors {
            c.destroy(&overlay);
        }
        wrist.destroy(&overlay);
        dash.destroy(&overlay);
        toasts.destroy(&overlay);
        info!("clean shutdown");
        Ok(())
    }

    fn hand_slot(h: Hand) -> usize {
        match h {
            Hand::Left => 0,
            Hand::Right => 1,
        }
    }

    fn reading(d: &DeviceBattery) -> BatteryReading {
        let label = match (d.kind, d.model.is_empty()) {
            (DeviceKind::Hmd, _) => "HMD".to_string(),
            (DeviceKind::LeftHand, _) => "Left hand".to_string(),
            (DeviceKind::RightHand, _) => "Right hand".to_string(),
            (DeviceKind::Controller, true) => "Controller".to_string(),
            (DeviceKind::Controller, false) => d.model.clone(),
            (DeviceKind::Tracker, true) => "Tracker".to_string(),
            (DeviceKind::Tracker, false) => format!("Tracker · {}", d.model),
        };
        BatteryReading {
            id: d.index,
            label,
            fraction: d.fraction,
            charging: d.charging,
            icon: None,
        }
    }

    /// Create a BGRA8 texture that wgpu renders into and OpenVR
    /// consumes. The resource is built via `wgpu-hal::dx12` so we can
    /// retain the raw `ID3D12Resource*` + `ID3D12CommandQueue*`
    /// pointers for OpenVR submission.
    fn build_shared_overlay_texture(
        gpu: &GpuCtx,
        width: u32,
        height: u32,
    ) -> Result<(wgpu::Texture, wgpu::TextureView, *mut c_void, *mut c_void)> {
        use wgpu::{
            Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        };

        // Access the raw D3D12 device + queue through wgpu-hal. Both
        // are needed: the device to create the committed resource, the
        // queue so OpenVR can sync with us.
        #[allow(clippy::type_complexity)]
        let built: Result<(d3d12::Resource, *mut c_void, *mut c_void)> = unsafe {
            gpu.device
                .as_hal::<wgpu_hal::api::Dx12, _, _>(|hal_device| -> Result<_> {
                    let hal_device = hal_device
                        .ok_or_else(|| anyhow!("wgpu is not on the DX12 backend"))?;
                    let raw_device = hal_device.raw_device(); // &d3d12::Device
                    let raw_queue = hal_device.raw_queue(); // &d3d12::CommandQueue

                    // CreateCommittedResource with a DEFAULT heap and
                    // RENDER_TARGET | SHADER_RESOURCE flags. OpenVR
                    // transitions state as needed on its own queue
                    // synchronization.
                    let mut raw_res: *mut winapi::um::d3d12::ID3D12Resource =
                        std::ptr::null_mut();
                    let heap_props = winapi::um::d3d12::D3D12_HEAP_PROPERTIES {
                        Type: winapi::um::d3d12::D3D12_HEAP_TYPE_DEFAULT,
                        CPUPageProperty:
                            winapi::um::d3d12::D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                        MemoryPoolPreference:
                            winapi::um::d3d12::D3D12_MEMORY_POOL_UNKNOWN,
                        CreationNodeMask: 1,
                        VisibleNodeMask: 1,
                    };
                    let desc = winapi::um::d3d12::D3D12_RESOURCE_DESC {
                        Dimension:
                            winapi::um::d3d12::D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                        Alignment: 0,
                        Width: width as u64,
                        Height: height,
                        DepthOrArraySize: 1,
                        MipLevels: 1,
                        Format: winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Layout:
                            winapi::um::d3d12::D3D12_TEXTURE_LAYOUT_UNKNOWN,
                        Flags: winapi::um::d3d12::D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                            | winapi::um::d3d12::D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
                    };
                    let iid = <winapi::um::d3d12::ID3D12Resource as winapi::Interface>::uuidof();
                    let hr = (*raw_device.as_mut_ptr()).CreateCommittedResource(
                        &heap_props,
                        winapi::um::d3d12::D3D12_HEAP_FLAG_NONE,
                        &desc,
                        winapi::um::d3d12::D3D12_RESOURCE_STATE_COMMON,
                        std::ptr::null(),
                        &iid,
                        &mut raw_res as *mut _ as *mut *mut c_void,
                    );
                    if hr < 0 || raw_res.is_null() {
                        return Err(anyhow!(
                            "CreateCommittedResource failed hr={hr:#x}"
                        ));
                    }
                    let res_void = raw_res as *mut c_void;
                    let queue_void = raw_queue.as_mut_ptr() as *mut c_void;
                    // `d3d12::Resource` is an alias for ComPtr<ID3D12Resource>.
                    let d3d12_resource: d3d12::Resource =
                        d3d12::ComPtr::from_raw(raw_res);
                    Ok((d3d12_resource, res_void, queue_void))
                })
                .ok_or_else(|| anyhow!("wgpu not on the wgpu-core backend"))?
        };
        let (d3d12_resource, res_void, queue_void) = built?;

        let hal_tex = unsafe {
            wgpu_hal::dx12::Device::texture_from_raw(
                d3d12_resource,
                TextureFormat::Bgra8Unorm,
                TextureDimension::D2,
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                1,
                1,
            )
        };

        let desc = TextureDescriptor {
            label: Some("vr-overlay-target"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let wgpu_tex = unsafe {
            gpu.device
                .create_texture_from_hal::<wgpu_hal::api::Dx12>(hal_tex, &desc)
        };
        let view = wgpu_tex.create_view(&wgpu::TextureViewDescriptor::default());

        Ok((wgpu_tex, view, res_void, queue_void))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rsoverlay_openvr::pose_offset_pitch;

        #[test]
        fn attach_slugs_roundtrip() {
            for a in Attach::ALL {
                assert_eq!(attach_from_slug(attach_slug(a)), a);
            }
            assert_eq!(attach_from_slug("garbage"), Attach::World);
        }

        const W: (f32, f32) = (0.9, 0.5625);

        #[test]
        fn window_dropped_near_right_edge_snaps_flush_and_centred() {
            let anchor = Affine3A::IDENTITY;
            // Slightly overlapping, a bit low, a bit in front.
            let moving = Affine3A::from_translation(glam::Vec3::new(0.85, -0.05, 0.04));
            let (j, p) = snap_pose(&moving, W, &[(7, anchor, W)]).expect("should snap");
            assert_eq!(j, 7);
            assert!((p.translation.x - (0.9 + SNAP_GAP_M)).abs() < 1e-5);
            assert!(p.translation.y.abs() < 1e-5 && p.translation.z.abs() < 1e-5);
        }

        #[test]
        fn window_dropped_above_snaps_to_top_edge_keeping_offset() {
            let moving = Affine3A::from_translation(glam::Vec3::new(0.3, 0.60, 0.0));
            let (_, p) = snap_pose(&moving, W, &[(0, Affine3A::IDENTITY, W)]).expect("should snap");
            assert!((p.translation.y - (0.5625 + SNAP_GAP_M)).abs() < 1e-5);
            assert!((p.translation.x - 0.3).abs() < 1e-5, "offset along the edge is kept");
        }

        #[test]
        fn far_away_or_angled_windows_do_not_snap() {
            let far = Affine3A::from_translation(glam::Vec3::new(1.4, 0.0, 0.0));
            assert!(snap_pose(&far, W, &[(0, Affine3A::IDENTITY, W)]).is_none());
            let deep = Affine3A::from_translation(glam::Vec3::new(0.9, 0.0, 0.4));
            assert!(snap_pose(&deep, W, &[(0, Affine3A::IDENTITY, W)]).is_none());
            let turned = Affine3A::from_rotation_translation(glam::Quat::from_rotation_y(1.0), glam::Vec3::new(0.9, 0.0, 0.0));
            assert!(snap_pose(&turned, W, &[(0, Affine3A::IDENTITY, W)]).is_none());
        }

        #[test]
        fn resizing_pushes_docked_neighbours_to_keep_seams_flush() {
            let at = |x: f32, y: f32| Affine3A::from_translation(glam::Vec3::new(x, y, 0.0));
            // Grabbed panel 0.9 × 0.5625 grows to 1.2 × 0.75.
            let (old, new) = (W, (1.2, 0.75));
            // Right neighbour: right edge moved +0.15.
            let right = push_follower(at(0.9, 0.0), old, new);
            assert!((right.translation.x - 1.05).abs() < 1e-5 && right.translation.y.abs() < 1e-5);
            // Neighbour below: bottom edge moved −0.09375; x within span stays.
            let below = push_follower(at(0.2, -0.5625), old, new);
            assert!((below.translation.y + 0.65625).abs() < 1e-5 && (below.translation.x - 0.2).abs() < 1e-5);
            // Shrinking pulls them back in.
            let back = push_follower(right, new, old);
            assert!((back.translation.x - 0.9).abs() < 1e-5);
        }

        #[test]
        fn snap_picks_the_nearest_neighbour() {
            let a = Affine3A::IDENTITY;
            let b = Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0));
            let moving = Affine3A::from_translation(glam::Vec3::new(1.05, 0.0, 0.0));
            let (j, _) = snap_pose(&moving, W, &[(0, a, W), (1, b, W)]).expect("should snap");
            assert_eq!(j, 1, "1.05 is 0.04 from b's left slot, 0.14 from a's right");
        }

        #[test]
        fn affine_roundtrip() {
            let m = pose_offset_pitch(0.1, 0.2, 0.3, -37.0);
            let back = from_affine(&to_affine(&m));
            for r in 0..3 {
                for c in 0..4 {
                    assert!((m.m[r][c] - back.m[r][c]).abs() < 1e-6);
                }
            }
        }

        #[test]
        fn face_points_visible_side_at_eye_and_stays_level() {
            let p = face(Vec3A::new(1.0, 1.0, -2.0), Vec3A::new(0.0, 1.7, 0.0));
            let to_eye = (Vec3A::new(0.0, 1.7, 0.0) - p.translation).normalize();
            assert!((p.matrix3.z_axis - to_eye).length() < 1e-5);
            assert!(p.matrix3.x_axis.y.abs() < 1e-6, "no roll: x axis stays horizontal");
        }

        #[test]
        fn smoothing_converges_and_is_frame_rate_independent() {
            let a = Affine3A::IDENTITY;
            let b = Affine3A::from_translation(glam::Vec3::new(1.0, 0.0, 0.0));
            let one = smooth(a, b, 0.02, 0.3);
            let two = smooth(smooth(a, b, 0.01, 0.3), b, 0.01, 0.3);
            assert!((one.translation - two.translation).length() < 1e-5);
            let mut p = a;
            for _ in 0..200 {
                p = smooth(p, b, 1.0 / 90.0, 0.3);
            }
            assert!((p.translation - b.translation).length() < 1e-3);
        }

        #[test]
        fn monitor_wall_arcs_around_you_left_to_right() {
            let eye = Vec3A::new(0.0, 1.6, 0.0);
            let p = arc_poses(eye, Vec3A::NEG_Z, &[1.0, 1.0, 1.0], 1.25, 0.05, 0.0);
            assert_eq!(p.len(), 3);
            for q in &p {
                // Same distance, eye height, facing the viewer.
                assert!(((q.translation - eye).length() - 1.25).abs() < 1e-4);
                assert!((q.translation.y - 1.6).abs() < 1e-4);
                let to_eye = (eye - q.translation).normalize();
                assert!(q.matrix3.z_axis.dot(to_eye) > 0.999);
            }
            // Middle straight ahead; left then right, symmetric.
            assert!(p[1].translation.x.abs() < 1e-4 && p[1].translation.z < 0.0);
            assert!(p[0].translation.x < 0.0 && p[2].translation.x > 0.0);
            assert!((p[0].translation.x + p[2].translation.x).abs() < 1e-4);
        }

        #[test]
        fn focus_follows_laser_then_gaze_with_hysteresis() {
            // Laser on a window: that one, whatever the gaze says.
            assert_eq!(pick_focus(Some(1), Some(2), &[(1, 0.0), (2, 40.0)]), Some(2));
            // Gaze: stays on the current window unless another is clearly nearer.
            assert_eq!(pick_focus(Some(1), None, &[(1, 10.0), (2, 6.0)]), Some(1));
            assert_eq!(pick_focus(Some(1), None, &[(1, 20.0), (2, 6.0)]), Some(2));
            assert_eq!(pick_focus(None, None, &[(1, 20.0), (2, 6.0)]), Some(2));
            assert_eq!(pick_focus(Some(1), None, &[]), None);
        }

        #[test]
        fn laser_hits_the_panel_drawn_on_top() {
            // Keyboard (drawn over windows) wins even when a window is nearer.
            assert!(hit_beats((2, 1.2), (WIN0, 0.8)));
            assert!(!hit_beats((WIN0, 0.8), (2, 1.2)));
            // Same layer: nearest wins.
            assert!(hit_beats((WIN0, 0.5), (WIN0 + 1, 0.9)));
        }

        #[test]
        fn stick_does_one_thing_at_a_time() {
            let mut axis = None;
            // Mostly sideways first: scaling, even as it drifts vertical.
            assert_eq!(lock_axis(&mut axis, 0.6, 0.2), (0.6, 0.0));
            assert_eq!(lock_axis(&mut axis, 0.3, 0.9), (0.3, 0.0));
            // Recentred: the next push picks again.
            assert_eq!(lock_axis(&mut axis, 0.0, 0.0), (0.0, 0.0));
            assert_eq!(lock_axis(&mut axis, 0.1, -0.7), (0.0, -0.7));
        }

        #[test]
        fn stick_dead_zone_rescales() {
            assert_eq!(stick(0.1), 0.0);
            assert!((stick(1.0) - 1.0).abs() < 1e-6);
            assert!((stick(-0.6) + 0.5).abs() < 1e-6);
        }

        #[test]
        fn summoned_dashboard_faces_wearer() {
            // HMD at 1.7 m looking down −Z, pitched down 30° — the panel
            // must still be upright, ahead on the floor plane, below eyes.
            let hmd = to_affine(&pose_offset_pitch(0.0, 1.7, 0.0, -30.0));
            let p = summon_pose(&from_affine(&hmd));
            assert!((p.translation.z + DASH_DISTANCE_M).abs() < 1e-5);
            assert!((p.translation.y - (1.7 + DASH_DROP_M)).abs() < 1e-5);
            assert!((p.matrix3.y_axis - Vec3A::Y).length() < 1e-6, "upright");
            // Overlay normal (+Z) points back toward the wearer (+Z here).
            assert!((p.matrix3.z_axis - Vec3A::Z).length() < 1e-6);
        }
    }
}

#[cfg(windows)]
pub use imp::run;

#[cfg(not(windows))]
pub fn run(
    _notify: Option<rsoverlay_notify::NotifyChannel>,
    _rt: tokio::runtime::Handle,
    _config: rsoverlay_core::Config,
    _osc: Option<rsoverlay_osc::OscClient>,
) -> anyhow::Result<()> {
    anyhow::bail!("VR mode is Windows-only in this milestone; Linux OpenVR support is scaffolded but not wired");
}
