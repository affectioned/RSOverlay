//! VR Preview: the headset view, on the desktop.
//!
//! What this is for: design decisions without putting the headset on.
//! A perspective-correct 3D scene places every RSOverlay panel the way
//! the VR shell does (wrist on the off hand, dashboard in front,
//! keyboard low and tilted, windows and widgets around you), each one
//! rendered through the same `EguiPanel` pipeline and the same UI code
//! as VR mode, so what you see here is what the headset shows.
//!
//! Layout presets show RSOverlay the way people actually use it (keys
//! 1-6 switch live, `--scene` picks one at launch):
//!
//! 1. work       editor + notes + terminal arc, stats widget, keyboard
//! 2. gaming     game guide to the side, media / battery widgets, wrist up
//! 3. social     dashboard on Chat, keyboard, notifications + media
//! 4. dashboard  the dashboard with the wrist and a popped-out widget
//! 5. wrist-compact   wrist close-up, compact set (laser on More)
//! 6. wrist-expanded  wrist close-up, everything shown (laser on Less)
//!
//! Window contents are mock apps (`preview_mock`) and media is a demo
//! track, so screenshots never contain the user's real desktop or
//! listening history. Mouse drag looks around, scroll dollies, R resets
//! the view, Esc exits.

use crate::preview_mock::MockApp;
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use rsoverlay_core::RuntimeMode;
use rsoverlay_notify::NotifyChannel;
use rsoverlay_render::{GpuCtx, OverlayTarget};
use rsoverlay_ui::{
    laser::Hand,
    shell::Page,
    views::DevView,
    vrkeyboard::VrKeyboard,
    widgets::WidgetKind,
    window_panel::{self, Attach},
    EguiPanel,
};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};
use wgpu::util::DeviceExt;
use winit::{
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::WindowBuilder,
};

/// Physical pixels, so `--screenshot` is exactly 1920×1080 on any
/// display scaling (Steam store screenshot size).
const INITIAL_WIN_W: u32 = 1920;
const INITIAL_WIN_H: u32 = 1080;
/// Standing eye height; the viewer faces -Z.
const HEAD: Vec3 = Vec3::new(0.0, 1.6, 0.0);

// Panel textures, pixels-per-point and widths: the VR shell's values
// (`vr.rs`), so text and layout match the headset exactly.
const DASH: PanelSize = PanelSize { tex: (2048, 1280), ppp: 2.0 };
const WRIST: PanelSize = PanelSize { tex: (1200, 1040), ppp: 2.0 };
const WIN: PanelSize = PanelSize { tex: (1920, 1200), ppp: 1.5 };
const WID: PanelSize = PanelSize { tex: (1200, 1000), ppp: 2.0 };
const KB: PanelSize = PanelSize { tex: (2048, 800), ppp: 2.0 };
const BEAM_TEX: (u32, u32) = (256, 16);
const BEAM_THICKNESS_M: f32 = 0.0035;

#[derive(Copy, Clone)]
struct PanelSize {
    tex: (u32, u32),
    ppp: f32,
}

#[derive(Clone)]
enum Content {
    Dashboard(Page),
    Wrist { expanded: bool },
    Keyboard,
    Widgets(Vec<WidgetKind>),
    Window(MockApp),
}

/// One panel in a preset: what it shows, where it is, and where the
/// laser points on it (uv), if anywhere.
struct Spec {
    content: Content,
    size: PanelSize,
    width_m: f32,
    pose: Mat4,
    laser: Option<(f32, f32)>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Preset {
    Work,
    Gaming,
    Social,
    Dashboard,
    WristCompact,
    WristExpanded,
}

impl Preset {
    const ALL: [Preset; 6] = [Preset::Work, Preset::Gaming, Preset::Social, Preset::Dashboard, Preset::WristCompact, Preset::WristExpanded];

    fn from_arg(s: Option<&str>) -> Self {
        match s.unwrap_or("dashboard") {
            "work" => Self::Work,
            "gaming" => Self::Gaming,
            "social" => Self::Social,
            "wrist" | "wrist-compact" => Self::WristCompact,
            "wrist-expanded" => Self::WristExpanded,
            _ => Self::Dashboard,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Gaming => "gaming",
            Self::Social => "social",
            Self::Dashboard => "dashboard",
            Self::WristCompact => "wrist-compact",
            Self::WristExpanded => "wrist-expanded",
        }
    }

    /// (yaw, pitch, dolly) the preset is framed from.
    fn camera(self) -> (f32, f32, f32) {
        match self {
            Self::Work => (0.0, -0.20, -0.12),
            Self::Gaming => (-0.12, -0.30, 0.0),
            Self::Social => (0.0, -0.30, 0.0),
            Self::Dashboard => (-0.08, -0.16, 0.0),
            Self::WristCompact => (-0.46, -0.60, 0.20),
            // Taller (two rows of status chips): look a little lower, step back.
            Self::WristExpanded => (-0.46, -0.62, 0.13),
        }
    }

    fn panels(self) -> Vec<Spec> {
        let wrist = |expanded: bool, pos: Vec3, laser| Spec {
            content: Content::Wrist { expanded },
            size: WRIST,
            width_m: 0.24,
            pose: face_head(pos),
            laser,
        };
        let keyboard = |pos: Vec3, laser| Spec { content: Content::Keyboard, size: KB, width_m: 0.75, pose: tilted(pos, -40.0), laser };
        let window = |app, yaw, dist, h, w, laser| Spec {
            content: Content::Window(app),
            size: WIN,
            width_m: w,
            pose: face_head(arc(yaw, dist, h)),
            laser,
        };
        let widget = |tabs: Vec<WidgetKind>, yaw, dist, h| Spec {
            content: Content::Widgets(tabs),
            size: WID,
            width_m: 0.34,
            pose: face_head(arc(yaw, dist, h)),
            laser: None,
        };
        let dash = |page, yaw, dist, h, laser| Spec {
            content: Content::Dashboard(page),
            size: DASH,
            width_m: 0.85,
            pose: face_head(arc(yaw, dist, h)),
            laser,
        };
        match self {
            // A workday in VR: code front and centre, notes and a
            // terminal either side, stats up high, keyboard under hand.
            Self::Work => vec![
                window(MockApp::Notes, -44.0, 1.0, 1.58, 0.72, None),
                window(MockApp::Editor, 0.0, 1.05, 1.60, 1.05, Some((0.46, 0.34))),
                window(MockApp::Terminal, 44.0, 1.0, 1.50, 0.72, None),
                widget(vec![WidgetKind::System], 30.0, 0.95, 2.02),
                keyboard(arc(0.0, 0.48, 1.16), None),
                wrist(false, Vec3::new(-0.26, 1.08, -0.34), None),
            ],
            // Mid-game: a guide pinned off to the side, glanceable
            // widgets, the wrist up for a quick check.
            Self::Gaming => vec![
                window(MockApp::Guide, 34.0, 1.35, 1.60, 0.80, None),
                widget(vec![WidgetKind::NowPlaying, WidgetKind::Macros], -42.0, 0.75, 1.36),
                widget(vec![WidgetKind::Battery], -14.0, 1.15, 1.74),
                wrist(true, Vec3::new(-0.10, 1.30, -0.36), Some((0.70, 0.11))),
            ],
            // Social VR: chatting through the dashboard and keyboard,
            // notifications and music to the sides.
            Self::Social => vec![
                dash(Page::Chat, 0.0, 0.78, 1.52, None),
                widget(vec![WidgetKind::Notifications], -40.0, 0.80, 1.58),
                widget(vec![WidgetKind::NowPlaying], 40.0, 0.80, 1.55),
                keyboard(arc(0.0, 0.46, 1.17), Some((0.42, 0.55))),
                wrist(false, Vec3::new(-0.27, 1.06, -0.32), None),
            ],
            Self::Dashboard => vec![
                widget(vec![WidgetKind::System, WidgetKind::NowPlaying], -46.0, 0.75, 1.52),
                dash(Page::Home, 0.0, 0.72, 1.48, Some((0.07, 0.27))),
                wrist(false, Vec3::new(-0.27, 1.07, -0.33), None),
            ],
            Self::WristCompact => vec![wrist(false, Vec3::new(-0.15, 1.30, -0.30), Some((0.90, 0.07)))],
            Self::WristExpanded => vec![wrist(true, Vec3::new(-0.15, 1.30, -0.30), Some((0.90, 0.07)))],
        }
    }
}

/// A point on a circle around the head: `yaw_deg` right of straight
/// ahead, `dist` metres out, at `height`.
fn arc(yaw_deg: f32, dist: f32, height: f32) -> Vec3 {
    let a = yaw_deg.to_radians();
    Vec3::new(a.sin() * dist, height, -a.cos() * dist)
}

/// Upright panel at `pos` facing the head (no roll).
fn face_head(pos: Vec3) -> Mat4 {
    let z = (HEAD - pos).normalize();
    let x = Vec3::Y.cross(z).normalize();
    let y = z.cross(x);
    Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), pos.extend(1.0))
}

/// Facing the head, then pitched about its own horizontal axis
/// (keyboard tilted up, wrist turned toward the face).
fn tilted(pos: Vec3, pitch_deg: f32) -> Mat4 {
    let flat = (HEAD - pos) * Vec3::new(1.0, 0.0, 1.0);
    let z = flat.normalize();
    let x = Vec3::Y.cross(z).normalize();
    let y = z.cross(x);
    Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), pos.extend(1.0)) * Mat4::from_rotation_x(pitch_deg.to_radians())
}

/// A live panel: its own egui context + texture + GPU quad.
struct Panel {
    spec: Spec,
    egui: EguiPanel,
    target: OverlayTarget,
    points: egui::Vec2,
    quad: Quad,
}

impl Panel {
    fn height_m(&self) -> f32 {
        self.spec.width_m * self.spec.size.tex.1 as f32 / self.spec.size.tex.0 as f32
    }

    /// World point under panel uv (u right, v down).
    fn world_at(&self, (u, v): (f32, f32)) -> Vec3 {
        self.spec.pose.transform_point3(Vec3::new((u - 0.5) * self.spec.width_m, (0.5 - v) * self.height_m(), 0.0))
    }
}

struct Quad {
    buf: wgpu::Buffer,
    bg: wgpu::BindGroup,
}

pub fn run(
    _notify: Option<NotifyChannel>,
    test: crate::preview::TestHooks,
    app_config: rsoverlay_core::Config,
    _config_path: std::path::PathBuf,
) -> Result<()> {
    let event_loop = EventLoop::new().context("event loop")?;
    // Store shots need the exact size; otherwise fit the monitor's work
    // area (never under the taskbar), centred.
    let want = winit::dpi::PhysicalSize::new(INITIAL_WIN_W, INITIAL_WIN_H);
    let mut builder = WindowBuilder::new();
    builder = match &test.screenshot {
        Some(_) => builder.with_inner_size(want),
        None => {
            let scale = event_loop.primary_monitor().map_or(1.0, |m| m.scale_factor());
            let l = want.to_logical::<f64>(scale);
            let (size, pos) = crate::preview::fit_to_monitor(&event_loop, (l.width, l.height));
            let b = builder.with_inner_size(size);
            match pos {
                Some(p) => b.with_position(p),
                None => b,
            }
        }
    };
    let window = Arc::new(
        builder
            .with_title("RSOverlay — VR Preview  (1 work · 2 gaming · 3 social · 4 dashboard · 5 wrist · 6 wrist expanded · R reset view)")
            .with_min_inner_size(winit::dpi::LogicalSize::new(800u32, 600u32))
            .build(&event_loop)
            .context("creating VR preview window")?,
    );

    let gpu = pollster::block_on(GpuCtx::new(RuntimeMode::Preview))?;
    let surface = gpu.instance.create_surface(window.clone()).context("creating wgpu surface")?;
    let size = window.inner_size();
    let caps = surface.get_capabilities(&gpu.adapter);
    let surface_format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
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

    // The preview has no headset to read batteries or a media session
    // from: demo data stands in (and keeps screenshots private).
    let mut hooks = test.clone();
    hooks.demo_batteries = true;
    let mut dev_view = DevView::default();
    crate::preview::seed_dev_view(&mut dev_view, &app_config, &hooks);
    dev_view.dock_layout = true;
    dev_view.set_demo_media("Northern Lights", "Ambient Works");
    // A made-up cover for the Media theme (shown on the wrist scenes).
    dev_view.set_demo_art();
    // The volume mixer lists the PC's real audio apps; keep it out.
    dev_view.show_volume = false;
    let mut keyboard = VrKeyboard::default();

    let scene = SceneRenderer::new(&gpu, surface_format)?;
    let beam_tex = OverlayTarget::new(&gpu, BEAM_TEX.0, BEAM_TEX.1);
    {
        let mut e = EguiPanel::new(&gpu.device, beam_tex.format, BEAM_TEX.0, BEAM_TEX.1);
        e.set_pixels_per_point(1.0);
        e.set_clear(wgpu::Color::TRANSPARENT);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(BEAM_TEX.0 as f32, BEAM_TEX.1 as f32))),
            ..Default::default()
        };
        e.render(&gpu.device, &gpu.queue, &beam_tex.view, raw, rsoverlay_ui::shell::paint_beam);
    }
    let beam = scene.quad(&gpu, &beam_tex.view);

    let mut preset = Preset::from_arg(test.scene.as_deref());
    // Wrist close-ups show the Media theme's album cover; the wide scenes
    // keep the default accent so the overview reads as one product.
    let media_theme = |p: Preset| matches!(p, Preset::WristCompact | Preset::WristExpanded);
    dev_view.media_theme = media_theme(preset);
    let mut panels = build(&gpu, &scene, preset);
    let mut camera = OrbitCamera::framing(preset);
    let mut input = InputState::default();
    let started = Instant::now();
    let mut last = Instant::now();
    let mut frame_counter: u64 = 0;
    let screenshot_path = test.screenshot.clone();
    let exit_after = test.exit_after;

    info!("VR preview: scene '{}' — 1-6 switch presets, drag to look, scroll to dolly, R resets, Esc exits", preset.name());

    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run(move |event, elwt| match event {
        Event::WindowEvent { event, window_id } if window_id == window.id() => match event {
            WindowEvent::CloseRequested => elwt.exit(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => match event.logical_key {
                Key::Named(NamedKey::Escape) => elwt.exit(),
                Key::Character(ref c) => {
                    let c = c.as_str();
                    if let Some(p) = c.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| Preset::ALL.get(i)) {
                        preset = *p;
                        dev_view.media_theme = media_theme(preset);
                        panels = build(&gpu, &scene, preset);
                        camera = OrbitCamera::framing(preset);
                        info!("VR preview: scene '{}'", preset.name());
                    } else if c.eq_ignore_ascii_case("r") {
                        camera = OrbitCamera::framing(preset);
                    }
                }
                _ => {}
            },
            WindowEvent::Resized(new_size) => {
                config.width = new_size.width.max(1);
                config.height = new_size.height.max(1);
                surface.configure(&gpu.device, &config);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                input.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let new = (position.x as f32, position.y as f32);
                if input.dragging {
                    camera.orbit(new.0 - input.last.0, new.1 - input.last.1);
                }
                input.last = new;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y as f32) / 50.0,
                };
                camera.dolly(dy);
            }
            WindowEvent::RedrawRequested => {
                let time = started.elapsed().as_secs_f64();
                let dt = last.elapsed().as_secs_f32();
                last = Instant::now();
                dev_view.frame = dev_view.frame.wrapping_add(1);
                dev_view.last_fps = 1.0 / dt.max(1e-5);
                dev_view.show_keyboard = panels.iter().any(|p| matches!(p.spec.content, Content::Keyboard));
                // Pop-up toasts land on top of whatever card is there; the
                // scenes show notifications in their widget instead.
                dev_view.dismiss_toasts();
                let accent = dev_view.effective_accent();

                // 1. Every panel draws its real UI into its texture.
                for p in panels.iter_mut() {
                    let cursor = p.spec.laser.map(|(u, v)| egui::pos2(u * p.points.x, v * p.points.y));
                    let raw = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, p.points)),
                        time: Some(time),
                        predicted_dt: dt.clamp(1.0 / 240.0, 0.1),
                        events: cursor.map(egui::Event::PointerMoved).into_iter().collect(),
                        ..Default::default()
                    };
                    let content = p.spec.content.clone();
                    p.egui.render(&gpu.device, &gpu.queue, &p.target.view, raw, |ctx| {
                        match content {
                            Content::Dashboard(page) => {
                                dev_view.page = page;
                                dev_view.ui_with(ctx, None);
                            }
                            Content::Wrist { expanded } => {
                                dev_view.wrist_expanded = expanded;
                                let _ = dev_view.wrist_ui(ctx);
                            }
                            Content::Keyboard => {
                                let hands = [(Hand::Right, cursor, 0.0)];
                                let _ = keyboard.frame(ctx, &hands, time, accent, false);
                            }
                            Content::Widgets(tabs) => {
                                let _ = dev_view.widget_ui(ctx, &tabs, 0, Attach::World, false, None, None);
                            }
                            Content::Window(app) => {
                                egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                                    let full = ui.max_rect();
                                    let bar = egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - window_panel::TOOLBAR_H), full.max);
                                    let body = egui::Rect::from_min_max(full.min, egui::pos2(full.right(), bar.top() - 6.0));
                                    app.paint(ui, body);
                                    let mut opacity = 1.0;
                                    let _ = window_panel::toolbar(ui, bar, app.title(), None, Attach::World, false, None, Some(window_panel::WindowMenu { resizable: true, smooth: false, tilt: Some(false), snap: Some(true) }), Some(&mut opacity), accent);
                                });
                            }
                        }
                        rsoverlay_ui::laser::paint_cursor(ctx, cursor.map(|c| (c, false)), accent);
                    });
                }

                // 2. The 3D scene: floor, panels far-to-near, beams.
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
                let eye = camera.eye();
                let mut order: Vec<usize> = (0..panels.len()).collect();
                let dist = |p: &Panel| p.spec.pose.w_axis.truncate().distance(eye);
                order.sort_by(|&a, &b| dist(&panels[b]).total_cmp(&dist(&panels[a])));
                let mut draws: Vec<&Quad> = Vec::new();
                for &i in &order {
                    let p = &panels[i];
                    scene.write_quad(&gpu, &p.quad, p.spec.pose, p.spec.width_m, p.height_m(), [1.0; 4]);
                    draws.push(&p.quad);
                }
                // One beam, from the right hand to whatever it points at.
                if let Some(p) = panels.iter().find(|p| p.spec.laser.is_some()) {
                    let to = p.world_at(p.spec.laser.unwrap_or_default());
                    let from = Vec3::new(0.20, 1.12, -0.22);
                    let (pose, len) = beam_pose(from, to, eye);
                    let a = accent;
                    let tint = [a.r() as f32 / 255.0, a.g() as f32 / 255.0, a.b() as f32 / 255.0, 0.9];
                    scene.write_quad(&gpu, &beam, pose, len, BEAM_THICKNESS_M, tint);
                    draws.push(&beam);
                }
                let aspect = config.width as f32 / config.height.max(1) as f32;
                scene.render(&gpu, &camera, accent, aspect, &view, &draws);

                frame_counter = frame_counter.wrapping_add(1);
                if exit_after == Some(frame_counter) {
                    if let Some(path) = &screenshot_path {
                        match crate::preview::save_surface_as_png(&gpu, &frame.texture, config.width, config.height, path) {
                            Ok(n) => info!("wrote screenshot {} ({} bytes)", path.display(), n),
                            Err(e) => warn!("screenshot failed: {e:#}"),
                        }
                    }
                    info!("--exit-after {frame_counter} reached; clean exit");
                    elwt.exit();
                }
                frame.present();
            }
            _ => {}
        },
        Event::AboutToWait => window.request_redraw(),
        _ => {}
    })?;
    Ok(())
}

fn build(gpu: &GpuCtx, scene: &SceneRenderer, preset: Preset) -> Vec<Panel> {
    preset
        .panels()
        .into_iter()
        .map(|spec| {
            let (w, h) = spec.size.tex;
            let target = OverlayTarget::new(gpu, w, h);
            let mut egui = EguiPanel::new(&gpu.device, target.format, w, h);
            egui.set_pixels_per_point(spec.size.ppp);
            // Transparent like the VR panels: rounded corners show the
            // world behind.
            egui.set_clear(wgpu::Color::TRANSPARENT);
            let points = egui::vec2(w as f32, h as f32) / spec.size.ppp;
            let quad = scene.quad(gpu, &target.view);
            Panel { spec, egui, target, points, quad }
        })
        .collect()
}

/// A camera-facing strip from `from` to `to`; returns (pose, length).
fn beam_pose(from: Vec3, to: Vec3, eye: Vec3) -> (Mat4, f32) {
    let d = to - from;
    let len = d.length().max(1e-4);
    let x = d / len;
    let mid = from + d * 0.5;
    let to_eye = eye - mid;
    let z = (to_eye - x * to_eye.dot(x)).normalize_or_zero();
    let z = if z == Vec3::ZERO { Vec3::Z } else { z };
    let y = z.cross(x);
    (Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), mid.extend(1.0)), len)
}

#[derive(Default)]
struct InputState {
    dragging: bool,
    last: (f32, f32),
}

/// First-person look + dolly camera: a standing user turning their head.
struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    dolly: f32,
}

impl OrbitCamera {
    fn framing(preset: Preset) -> Self {
        let (yaw, pitch, dolly) = preset.camera();
        Self { yaw, pitch, dolly }
    }

    fn orbit(&mut self, dx: f32, dy: f32) {
        // Typical SteamVR mouse-look sensitivity: ~0.3° per screen pixel.
        self.yaw -= dx.to_radians() * 0.3;
        self.pitch -= dy.to_radians() * 0.3;
        self.pitch = self.pitch.clamp(-1.4, 1.4);
    }

    fn dolly(&mut self, amount: f32) {
        self.dolly = (self.dolly - amount * 0.1).clamp(-0.6, 1.2);
    }

    fn eye(&self) -> Vec3 {
        HEAD + self.forward() * self.dolly
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), -self.pitch.cos() * self.yaw.cos())
    }

    fn view(&self) -> Mat4 {
        let eye = self.eye();
        Mat4::look_at_rh(eye, eye + self.forward(), Vec3::Y)
    }

    fn proj(&self, aspect: f32) -> Mat4 {
        // ~100° horizontal is typical of modern HMDs.
        let hfov_rad = 100f32.to_radians();
        let vfov = 2.0 * ((hfov_rad * 0.5).tan() / aspect).atan();
        Mat4::perspective_rh(vfov, aspect, 0.05, 100.0)
    }
}

struct SceneRenderer {
    camera_buf: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    quad_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    quad_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
}

impl SceneRenderer {
    fn new(gpu: &GpuCtx, surface_format: wgpu::TextureFormat) -> Result<Self> {
        let d = &gpu.device;
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vr-scene-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let camera_buf = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vr-camera"),
            size: std::mem::size_of::<CameraGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let camera_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vr-camera-bgl"),
            entries: &[uniform(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT)],
        });
        let camera_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vr-camera-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buf.as_entire_binding() }],
        });
        let quad_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vr-quad-bgl"),
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vr-overlay-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let pipeline = |label, layouts: &[&wgpu::BindGroupLayout], vs, fs, blend| {
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: layouts, push_constant_ranges: &[] });
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: vs, buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: fs,
                    targets: &[Some(wgpu::ColorTargetState { format: surface_format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };
        // Panel textures are premultiplied (egui output over a
        // transparent clear), so blend them as such.
        let quad_pipeline = pipeline("vr-quad", &[&camera_bgl, &quad_bgl], "vs_quad", "fs_quad", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let grid_pipeline = pipeline("vr-grid", &[&camera_bgl], "vs_grid", "fs_grid", wgpu::BlendState::REPLACE);
        Ok(Self { camera_buf, camera_bg, quad_bgl, sampler, quad_pipeline, grid_pipeline })
    }

    fn quad(&self, gpu: &GpuCtx, texture: &wgpu::TextureView) -> Quad {
        let buf = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vr-quad"),
            contents: bytemuck::bytes_of(&QuadGpu { model: Mat4::IDENTITY.to_cols_array_2d(), params: [0.0; 4], tint: [1.0; 4] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vr-quad-bg"),
            layout: &self.quad_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        Quad { buf, bg }
    }

    fn write_quad(&self, gpu: &GpuCtx, q: &Quad, pose: Mat4, w: f32, h: f32, tint: [f32; 4]) {
        let data = QuadGpu { model: pose.to_cols_array_2d(), params: [0.0, w, h, 0.0], tint };
        gpu.queue.write_buffer(&q.buf, 0, bytemuck::bytes_of(&data));
    }

    fn render(&self, gpu: &GpuCtx, camera: &OrbitCamera, accent: egui::Color32, aspect: f32, view: &wgpu::TextureView, quads: &[&Quad]) {
        let vp = camera.proj(aspect) * camera.view();
        let eye = camera.eye();
        let cam = CameraGpu {
            view_proj: vp.to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 0.0],
            accent: [accent.r() as f32 / 255.0, accent.g() as f32 / 255.0, accent.b() as f32 / 255.0, 1.0],
        };
        gpu.queue.write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&cam));
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("vr-scene-enc") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vr-scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.grid_pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.draw(0..6, 0..1);
            pass.set_pipeline(&self.quad_pipeline);
            for q in quads {
                pass.set_bind_group(1, &q.bg, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        gpu.queue.submit(Some(enc.finish()));
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraGpu {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    accent: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadGpu {
    model: [[f32; 4]; 4],
    // x = curvature in [0,1], y = width in metres, z = height in
    // metres, w reserved.
    params: [f32; 4],
    // rgb = colour multiplier (beam tint), a = opacity.
    tint: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_face_the_viewer() {
        for preset in Preset::ALL {
            for s in preset.panels() {
                let pos = s.pose.w_axis.truncate();
                let normal = s.pose.z_axis.truncate();
                assert!(normal.dot((HEAD - pos).normalize()) > 0.5, "{preset:?}: a panel faces away");
            }
        }
    }

    #[test]
    fn beam_spans_its_endpoints() {
        let (from, to) = (Vec3::new(0.2, 1.1, -0.2), Vec3::new(0.0, 1.5, -1.0));
        let (pose, len) = beam_pose(from, to, HEAD);
        assert!((pose.transform_point3(Vec3::new(-len / 2.0, 0.0, 0.0)) - from).length() < 1e-4);
        assert!((pose.transform_point3(Vec3::new(len / 2.0, 0.0, 0.0)) - to).length() < 1e-4);
    }
}
