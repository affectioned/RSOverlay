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
    const WRIST_TEX: (u32, u32) = (1024, 512);
    const WRIST_PPP: f32 = 2.0;
    const WRIST_WIDTH_M: f32 = 0.17;
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
    /// Widget panels (popped-out cards): 600 × 500 pt, 34 cm wide.
    const WID_TEX: (u32, u32) = (1200, 1000);
    const WID_PPP: f32 = 2.0;
    const WID_WIDTH_M: f32 = 0.34;
    const MAX_FLOATING: usize = 14;
    /// Drop a widget within this distance of another to merge them.
    const MERGE_DIST_M: f32 = 0.18;
    /// Panel indices: 0 wrist, 1 dashboard, 2 keyboard, WIN0.. windows.
    const WIN0: usize = 3;
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
    const STICK_DEAD: f32 = 0.2;
    const GRAB_DIST: (f32, f32) = (0.20, 8.0);
    const PANEL_WIDTH: (f32, f32) = (0.15, 4.0);
    /// Pose smoothing time constant — kills hand tremor on big panels
    /// without feeling laggy.
    const GRAB_TAU: f32 = 0.05;
    /// Laser beam strip: 256 × 16 texels, 3.5 mm thick at any length.
    const BEAM_TEX: (u32, u32) = (256, 16);
    const BEAM_THICKNESS_M: f32 = 0.0035;
    /// Grab / release haptic: a firmer tick than a click.
    const GRAB_HAPTIC_S: f32 = 0.025;

    /// View-angle fade time constant (XSO-style auto-hide).
    const FADE_TAU: f32 = 0.08;

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
        tex: (u32, u32),
        base_ppp: f32,
        scale: f32,
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
                base_ppp: ppp,
                scale: 1.0,
            })
        }

        /// Accessibility text size: fewer, bigger egui points per texel.
        fn apply_scale(&mut self, scale: f32) {
            if (scale - self.scale).abs() > 1e-3 {
                self.scale = scale;
                let ppp = self.base_ppp * scale;
                self.egui.set_pixels_per_point(ppp);
                self.points = egui::vec2(self.tex.0 as f32 / ppp, self.tex.1 as f32 / ppp);
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
        }

        /// Ease the view-angle fade toward shown/hidden.
        fn update_fade(&mut self, shown: bool, dt: f32) {
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
    }

    /// XSO-style auto-hide: shown while the angle between the head's
    /// gaze and the panel's facing direction stays under `clip_deg`.
    /// Panels face the viewer along +Z, so straight-on is gaze == −Z.
    fn in_view(hmd: &Affine3A, panel: &Affine3A, clip_deg: f32) -> bool {
        let gaze = -hmd.matrix3.z_axis;
        let toward = -panel.matrix3.z_axis;
        gaze.dot(toward).clamp(-1.0, 1.0).acos().to_degrees() < clip_deg
    }

    /// Upright pose at `pos` whose +Z (visible face) points at `eye`.
    /// No roll — panels stay level however you twist the controller.
    fn face(pos: Vec3A, eye: Vec3A) -> Affine3A {
        let z = (eye - pos).try_normalize().unwrap_or(Vec3A::Z);
        let x = Vec3A::Y.cross(z).try_normalize().unwrap_or(Vec3A::X);
        let y = z.cross(x);
        Affine3A::from_cols(x, y, z, pos)
    }

    /// Frame-rate-independent exponential approach for poses.
    fn smooth(from: Affine3A, to: Affine3A, dt: f32) -> Affine3A {
        let k = 1.0 - (-dt.max(0.0) / GRAB_TAU).exp();
        let (_, r0, t0) = from.to_scale_rotation_translation();
        let (_, r1, t1) = to.to_scale_rotation_translation();
        Affine3A::from_rotation_translation(r0.slerp(r1, k), t0.lerp(t1, k))
    }

    fn stick(v: f32) -> f32 {
        if v.abs() < STICK_DEAD {
            0.0
        } else {
            v.signum() * (v.abs() - STICK_DEAD) / (1.0 - STICK_DEAD)
        }
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

    fn write_manifest(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(dir)?;
        for (name, body) in MANIFEST {
            std::fs::write(dir.join(name), body)?;
        }
        Ok(dir.join("actions.json"))
    }

    pub fn run(
        _notify: Option<NotifyChannel>,
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
            Ok(path) => vr
                .input(&path)
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
        let mut grab: Option<Grab> = None;
        let mut wins: Vec<WinOverlay> = Vec::new();
        let layouts_path = rsoverlay_core::layouts::Layouts::path();
        dev_view.layouts = rsoverlay_core::layouts::Layouts::load(&layouts_path);
        // The favourite layout loads once controllers/HMD are tracked.
        let mut favorite_pending = dev_view.layouts.favorite;
        let mut kb = VrPanel::new(&overlay, &gpu, "rsoverlay.keyboard", "RSOverlay Keyboard", KB_TEX, KB_PPP, KB_WIDTH_M, 0.0)?;
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
        let mut keyboard = rsoverlay_ui::vrkeyboard::VrKeyboard::default();
        let mut kb_pose: Option<Affine3A> = None;
        // VR keyboard → RSOverlay's own text fields: events typed this
        // frame go to the panel whose field has focus (1 = dashboard,
        // WIN0.. = widget groups), delivered on its next render.
        let mut typed: Vec<egui::Event> = Vec::new();
        let mut typed_target: Option<usize> = None;
        let mut prev_text_target: Option<usize> = None;
        let mut win_seq = 0u32;
        let mut prev_menu = false;
        let mut prev_grip = [false; 2];
        let mut frame = 0u64;
        info!("VR shell up — wrist on the off hand; double-press left A/X (or the wrist Menu button) shows the dashboard");

        while running.load(Ordering::SeqCst) {
            let frame_start = Instant::now();
            frame = frame.wrapping_add(1);
            dev_view.frame = frame;
            dev_view.last_fps = fps.tick();
            let dt = last_frame.elapsed().as_secs_f32();
            last_frame = Instant::now();
            let time = started.elapsed().as_secs_f64();

            if let Some(sys) = &system {
                if last_battery_poll.is_none_or(|t| t.elapsed() >= BATTERY_POLL) {
                    last_battery_poll = Some(Instant::now());
                    dev_view.set_batteries(sys.batteries().iter().map(reading).collect());
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
                wrist.set_visible(&overlay, wrist_parent.is_some());
            }

            let (hmd, mut controllers) = system.as_ref().map(|s| s.tracking()).unwrap_or_default();
            // Buttons/axes from SteamVR Input (user's bindings), poses
            // from the tracking API. [left, right].
            let actions = input.as_ref().map(|i| i.poll());
            let mut mods = [(false, false); 2];
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

            // Show/hide the dashboard: XSO's double-press left A / X via
            // SteamVR Input; legacy fallback = off-hand menu button.
            let menu_now = controllers.iter().any(|c| c.hand == wrist_on && c.menu);
            let mut toggle_dash = toggle_edit || (menu_now && !prev_menu);
            prev_menu = menu_now;

            // ── View-angle auto-hide (XSO behaviour) for every panel; a
            // panel being grabbed never fades.
            if let Some(h) = hmd.as_ref().map(to_affine) {
                let clip = dev_view.vr_clip_angle;
                let grabbed = grab.as_ref().map(|g| g.panel);
                let wrist_world = wrist_parent
                    .and_then(|d| device_pose(d, hmd.as_ref(), &controllers))
                    .map(|p| p * wrist_rel);
                wrist.update_fade(grabbed == Some(0) || wrist_world.is_some_and(|w| in_view(&h, &w, clip)), dt);
                dash.update_fade(grabbed == Some(1) || dash_pose.is_some_and(|p| in_view(&h, &p, clip)), dt);
                kb.update_fade(grabbed == Some(2) || kb_pose.is_some_and(|p| in_view(&h, &p, clip)), dt);
                for (k, w) in wins.iter_mut().enumerate() {
                    let p = win_world(w, hmd.as_ref(), &controllers);
                    w.panel.update_fade(grabbed == Some(k + WIN0) || in_view(&h, &p, clip), dt);
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
                    if let Some((uv, d)) = overlay.intersect(h, c.origin, c.dir) {
                        if best.is_none_or(|b| d < b.2) {
                            best = Some((i, uv, d));
                        }
                    }
                }
                hand_hit[hand_slot(c.hand)] = best.map(|b| (b.0, b.2));
                for (i, list) in samples.iter_mut().enumerate() {
                    let hit = best.filter(|b| b.0 == i).map(|b| LaserInput::uv_to_points(b.1, target(i).2));
                    hit_by[i][hand_slot(c.hand)] = hit.is_some();
                    let (rm, mm) = mods[hand_slot(c.hand)];
                    let button = if rm {
                        egui::PointerButton::Secondary
                    } else if mm {
                        egui::PointerButton::Middle
                    } else {
                        egui::PointerButton::Primary
                    };
                    list.push(HandSample { hand, hit, trigger: c.trigger, stick_y: c.stick[1], button });
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
                            grab = Some(Grab { hand: c.hand, panel: i, dist, offset, second: None, smoothed: p });
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
                        let resizing = two.is_some() || stick(c.stick[0]) != 0.0;
                        match two {
                            Some(w) => width = w,
                            None => {
                                g.second = None;
                                width *= (stick(c.stick[0]) * SCALE_RATE * dt).exp();
                                g.dist = (g.dist * (stick(c.stick[1]) * PUSH_RATE * dt).exp()).clamp(GRAB_DIST.0, GRAB_DIST.1);
                            }
                        }
                        let width = width.clamp(PANEL_WIDTH.0, PANEL_WIDTH.1);
                        hud = Some((g.panel, rsoverlay_ui::shell::GrabHud { width_m: width, dist_m: g.dist, resizing }));
                        let hit = Vec3A::from(c.origin) + Vec3A::from(c.dir) * g.dist;
                        // Orient first (from the grab point), then hang the
                        // panel off it by the scaled grab offset.
                        let mut target = face(hit, eye.unwrap_or(hit + Vec3A::Z));
                        target.translation = hit + target.matrix3 * (g.offset * width);
                        if let Some(e) = eye {
                            target = Affine3A { translation: target.translation, ..face(target.translation, e) };
                        }
                        g.smoothed = smooth(g.smoothed, target, dt);
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
                            wins[g.panel - WIN0].world = p;
                            wins[g.panel - WIN0].panel.set_width(&overlay, width);
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
                        info!("merged widget panels");
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
                        smp.stick_y = 0.0;
                    }
                }
            }

            // ── Laser beams: drawn while a hand points at any panel; bright
            // while the trigger is held. Billboarded around their own axis
            // so they never go edge-on.
            for c in &controllers {
                let s = hand_slot(c.hand);
                let b = &mut beams[s];
                let hit = hand_hit[s].filter(|_| dev_view.vr_show_lasers);
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
                let _ = overlay.set_texel_aspect(b.handle, len * BEAM_TEX.1 as f32 / (BEAM_TEX.0 as f32 * BEAM_THICKNESS_M));
                let _ = overlay.set_transform_absolute(b.handle, &from_affine(&pose));
                let a = dev_view.accent;
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

            // Grab / release: a firmer haptic tick than a click.
            for h in grabbed_hand.into_iter().chain(released_hand) {
                match &input {
                    Some(i) => i.haptic(h, GRAB_HAPTIC_S, 0.6),
                    None => {
                        if let (Some(sys), Some(c)) = (&system, controllers.iter().find(|c| c.hand == h)) {
                            sys.haptic(c.index, 2500);
                        }
                    }
                }
            }

            // Pointer settings (Settings › Pointer) on every panel's laser.
            for p in [&mut wrist.laser, &mut dash.laser].into_iter().chain(wins.iter_mut().map(|w| &mut w.panel.laser)) {
                p.smoothing = dev_view.cursor_smoothing;
                p.scroll_mult = dev_view.scroll_speed;
            }

            // Accessibility text size.
            let scale = dev_view.vr_ui_scale.clamp(0.8, 1.6);
            wrist.apply_scale(scale);
            dash.apply_scale(scale);
            kb.apply_scale(scale);
            for w in &mut wins {
                w.panel.apply_scale(scale);
            }

            // ── Wrist frame.
            let wf = wrist.laser.update(&wrist_samples, dt);
            let mut act = rsoverlay_ui::wrist::WristAction::default();
            if wrist.visible {
                let accent = dev_view.accent;
                let cursor = wf.cursor;
                wrist.render(&gpu, &overlay, time, dt, wf.events, |ctx| {
                    act = dev_view.wrist_ui(ctx);
                    if let Some((0, h)) = hud {
                        rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                    }
                    laser::paint_cursor(ctx, cursor, accent);
                });
            }
            toggle_dash |= act.toggle_dashboard;
            if act.toggle_keyboard {
                dev_view.show_keyboard = !dev_view.show_keyboard;
            }
            if act.open_notifications {
                dev_view.page = rsoverlay_ui::shell::Page::Notifications;
                if !dash.visible {
                    toggle_dash = true;
                }
            }

            // ── Summon / dismiss the dashboard.
            if toggle_dash {
                let show = !dash.visible;
                if show {
                    // Re-summon in front of you every time, XSO-style; a
                    // grabbed position lasts until the next dismiss.
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
                let accent = dev_view.accent;
                let cursor = df.cursor;
                let inj = injector.as_ref();
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
                } else if let Some(base) = dash_pose {
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
                let hands: Vec<(laser::Hand, Option<egui::Pos2>, f32)> =
                    kb_samples.iter().map(|s| (s.hand, s.hit, s.trigger)).collect();
                let accent = dev_view.accent;
                let mut kf = rsoverlay_ui::vrkeyboard::KbFrame::default();
                kb.render(&gpu, &overlay, time, dt, Vec::new(), |ctx| {
                    kf = keyboard.frame(ctx, &hands, time, accent, internal);
                    if let Some((2, h)) = hud {
                        rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                    }
                });
                kb_haptics = kf.haptic;
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
                let accent = dev_view.accent;
                let mut act = window_panel::WindowPanelAction::default();
                if let Some((tabs, active)) = w.widgets.clone() {
                    // Widget group: tabs + live card + dock bar.
                    let (attach, cursor) = (w.attach, lf.cursor);
                    let inj = injector.as_ref();
                    let mut wa = rsoverlay_ui::widgets::WidgetAction::default();
                    w.panel.render(&gpu, &overlay, time, dt, lf.events, |ctx| {
                        wa = dev_view.widget_ui(ctx, &tabs, active, attach, inj);
                        if let Some((_, h)) = hud.filter(|(i, _)| *i == k + WIN0) {
                            rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                        }
                        laser::paint_cursor(ctx, cursor, accent);
                    });
                    if let Some(i) = wa.select {
                        w.widgets = Some((tabs.clone(), i));
                    }
                    if wa.split && tabs.len() > 1 {
                        splits.push((k, tabs[active]));
                    }
                    act.attach = wa.attach;
                    act.close = wa.close;
                } else if let Some(mirror) = &w.mirror {
                    if w.blocked_checked.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                        w.blocked_checked = Some(Instant::now());
                        w.input_blocked = mirror.window_hwnd().is_some_and(rsoverlay_input::elevation::input_blocked);
                    }
                    let (title, handle_m, attach, cursor, blocked) =
                        (mirror.source.label(), mirror.handle(), w.attach, lf.cursor, w.input_blocked);
                    let icon = w.icon.as_ref().map(|t| t.id());
                    let events = lf.events.clone();
                    w.panel.render(&gpu, &overlay, time, dt, lf.events, |ctx| {
                        act = window_panel::show(ctx, &title, Some(handle_m), attach, blocked, icon, accent);
                        if let Some((_, h)) = hud.filter(|(i, _)| *i == k + WIN0) {
                            rsoverlay_ui::shell::paint_grab_hud(ctx, h, accent);
                        }
                        laser::paint_cursor(ctx, cursor, accent);
                    });
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
                                egui::Event::PointerMoved(p) if rect.contains(*p) || w.down.is_some() => {
                                    let (x, y) = to_px(*p);
                                    if w.last_hit != Some((x, y)) {
                                        let _ = inj.move_to(&geom, x, y);
                                        w.last_hit = Some((x, y));
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
                                    // ~2 wheel units per point: smooth in
                                    // modern apps, one notch per ~60 pt.
                                    w.wheel_acc += v.y * 2.0;
                                    let send = (w.wheel_acc / 30.0).trunc() * 30.0;
                                    if send != 0.0 {
                                        let _ = inj.wheel(send as i32);
                                        w.wheel_acc -= send;
                                    }
                                }
                                egui::Event::PointerGone => w.last_hit = None,
                                _ => {}
                            }
                        }
                    }
                }
                if let Some(a) = act.attach {
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
            if hmd.is_some() {
                if let Some(i) = favorite_pending.take() {
                    if dev_view.layouts.slot(i).is_some() {
                        dev_view.layout_request = Some(rsoverlay_ui::layouts_page::LayoutAction::Load(i));
                    }
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
                        if let Some(layout) = dev_view.layouts.slot(i).cloned() {
                            for mut w in wins.drain(..) {
                                if let Some(m) = w.mirror.take() {
                                    m.shutdown(&mut w.panel.egui);
                                }
                                w.panel.destroy(&overlay);
                            }
                            grab = None;
                            let sources = list_sources();
                            fetch_icons(&sources, &mut dev_view.sources.icons);
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
                            info!("layout slot {} loaded ({} panels)", i + 1, wins.len());
                        }
                    }
                }
                if let Err(e) = dev_view.layouts.save(&layouts_path) {
                    warn!("saving layouts: {e:#}");
                }
            }

            // ── Haptic ticks for clicks on any panel.
            if let Some(sys) = &system {
                for h in [wf.haptic, df.haptic].into_iter().flatten().chain(win_haptics).chain(kb_haptics) {
                    let want = match h {
                        laser::Hand::Left => Hand::Left,
                        laser::Hand::Right => Hand::Right,
                    };
                    match &input {
                        Some(i) => i.haptic(want, 0.012, 0.35),
                        None => {
                            if let Some(c) = controllers.iter().find(|c| c.hand == want) {
                                sys.haptic(c.index, CLICK_HAPTIC_US);
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

            dev_view.record_frame_ms(frame_start.elapsed().as_secs_f32() * 1000.0);

            let tint = dev_view.overlay_tint();
            wrist.apply_tint(&overlay, [tint[0], tint[1], tint[2], tint[3] * dev_view.vr_wrist_opacity]);
            dash.apply_tint(&overlay, tint);
            kb.apply_tint(&overlay, tint);
            for w in &mut wins {
                w.panel.apply_tint(&overlay, tint);
            }

            std::thread::sleep(Duration::from_millis(11));
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
        wrist.destroy(&overlay);
        dash.destroy(&overlay);
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
            let one = smooth(a, b, 0.02);
            let two = smooth(smooth(a, b, 0.01), b, 0.01);
            assert!((one.translation - two.translation).length() < 1e-5);
            let mut p = a;
            for _ in 0..200 {
                p = smooth(p, b, 1.0 / 90.0);
            }
            assert!((p.translation - b.translation).length() < 1e-3);
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
