//! SteamVR runtime path — renders the egui dashboard into a wgpu DX12
//! texture and submits it to OpenVR via the DirectX 12 texture type.
//!
//! No desktop window exists in this mode: everything the user sees is
//! submitted to the compositor. For debugging, launch `--vr-preview`
//! alongside to see the same dashboard in a desktop 3D scene.

#[cfg(windows)]
mod imp {
    use anyhow::{anyhow, Context, Result};
    use rsoverlay_notify::NotifyChannel;
    use rsoverlay_openvr::{pose_offset_pitch, pose_translation, DeviceBattery, DeviceKind, Hand, OpenVr};
    use rsoverlay_ui::laser::{self, HandSample, LaserInput};
    use rsoverlay_render::GpuCtx;
    use rsoverlay_ui::{battery::BatteryReading, views::DevView, EguiPanel};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::runtime::Handle;
    use tracing::{info, warn};

    const OVERLAY_TEX_W: u32 = 2048;
    const OVERLAY_TEX_H: u32 = 1280;
    /// Click tick length. Short enough to read as a "detent", not a buzz.
    const CLICK_HAPTIC_US: u16 = 1200;
    /// Texels per egui point in VR. Higher than the desktop's 1.5: the
    /// wrist panel is small in view, so the UI gets fewer, bigger points
    /// (≈ 910 × 570 pt) — compact, XSO-style density.
    const PPP: f32 = 2.25;

    /// Wrist mount, in the left controller's frame (−Z = pointing
    /// direction, +Y = top of the controller, +Z = back toward the
    /// wrist): sit just above the back of the hand, face tilted 60° up
    /// toward the wearer — glance at your wrist and it's there.
    const WRIST_OFFSET: [f32; 3] = [0.0, 0.03, 0.12];
    const WRIST_PITCH_DEG: f32 = -60.0;
    const WRIST_WIDTH_M: f32 = 0.40;
    /// HMD-locked: a comfortable reading distance, slightly below eye line.
    const HMD_OFFSET: [f32; 3] = [0.0, -0.10, -0.60];
    /// How often to re-check which device the panel is parented to
    /// (controllers connect late / swap roles).
    const ATTACH_POLL: Duration = Duration::from_secs(1);

    /// Battery props change slowly; polling 64 device slots every
    /// frame would be wasted IPC to vrserver.
    const BATTERY_POLL: Duration = Duration::from_secs(5);

    pub fn run(
        _notify: Option<NotifyChannel>,
        rt: Handle,
        config: rsoverlay_core::Config,
        osc: Option<rsoverlay_osc::OscClient>,
    ) -> Result<()> {
        let vr = OpenVr::init().context("OpenVR init")?;
        let overlay = vr.overlay().context("acquiring IVROverlay")?;
        // Optional: a runtime that can't serve IVRSystem_022 just loses
        // the battery card, not the whole overlay.
        let system = vr
            .system()
            .map_err(|e| warn!("IVRSystem unavailable, battery widget disabled: {e}"))
            .ok();

        let handle = overlay
            .create("rsoverlay.main", "RSOverlay")
            .context("CreateOverlay")?;
        // Use the persisted Placement from config. Wrist anchor still
        // resolves to the saved world-space offset until controller
        // pose polling lands in a follow-up milestone — that's when
        // anchor=Wrist will switch to SetOverlayTransformTrackedDevice
        // Relative with the user's non-dominant controller.
        let p = &config.placement;
        let anchor = config.placement.anchor;
        let width = if anchor == rsoverlay_core::PlacementAnchor::Wrist { WRIST_WIDTH_M } else { p.size_w };
        overlay
            .set_width(handle, width)
            .context("SetOverlayWidthInMeters")?;
        if let Err(e) = overlay.set_curvature(handle, p.curvature) {
            warn!("SetOverlayCurvature: {e:#}");
        }
        let pose = pose_translation(p.offset_x, p.offset_y, p.offset_z);
        overlay
            .set_transform_absolute(handle, &pose)
            .context("SetOverlayTransformAbsolute")?;

        // Set up wgpu DX12 for the overlay texture. Same path the
        // preview uses so what you see in `--vr-preview` is what the
        // HMD gets.
        let gpu = pollster::block_on(GpuCtx::new(rsoverlay_core::RuntimeMode::Vr))
            .context("building wgpu GpuCtx")?;

        // Build a D3D12 resource that wgpu owns through its normal API,
        // *and* capture its raw ID3D12Resource + CommandQueue pointers
        // so we can hand them to OpenVR each frame.
        let (overlay_tex, overlay_view, raw_resource, raw_queue) =
            build_shared_overlay_texture(&gpu, OVERLAY_TEX_W, OVERLAY_TEX_H)?;
        let overlay_format = overlay_tex.format();

        let mut egui_panel =
            EguiPanel::new(&gpu.device, overlay_format, OVERLAY_TEX_W, OVERLAY_TEX_H);
        egui_panel.set_pixels_per_point(PPP);
        let mut dev_view = DevView::default();
        crate::preview::seed_dev_view(
            &mut dev_view,
            &config,
            &crate::preview::TestHooks::default(),
        );
        let mut last_battery_poll: Option<Instant> = None;
        // Macros + keyboard fire global hotkeys; the overlay never takes
        // desktop focus, so they reach whatever app the user is in.
        let injector = rsoverlay_input::InputInjector::new().ok();
        // Last tint pushed to the compositor; only re-send on change
        // (each call is an IPC round-trip to vrcompositor).
        let mut applied_tint = [f32::NAN; 4];
        let mut laser = LaserInput::default();
        let mut fps = crate::preview::FpsCounter::new();
        // Which device the overlay is currently parented to (None =
        // absolute). Re-resolved every ATTACH_POLL.
        let mut attached_to: Option<u32> = None;
        let mut last_attach_check: Option<Instant> = None;
        let started = Instant::now();
        let mut last_frame = Instant::now();
        let panel = egui::vec2(OVERLAY_TEX_W as f32 / PPP, OVERLAY_TEX_H as f32 / PPP);

        overlay.show(handle).ok();
        info!("OpenVR overlay registered (anchor {:?}) — dashboard streaming via DirectX 12", anchor);

        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                r.store(false, Ordering::SeqCst);
            }
        });

        let mut frame = 0u64;
        while running.load(Ordering::SeqCst) {
            // 1. Render the dashboard into the overlay texture, driven by
            // the controller lasers (ray → ComputeOverlayIntersection →
            // egui pointer events).
            frame = frame.wrapping_add(1);
            dev_view.frame = frame;
            dev_view.last_fps = fps.tick();
            if let Some(sys) = &system {
                if last_battery_poll.is_none_or(|t| t.elapsed() >= BATTERY_POLL) {
                    last_battery_poll = Some(Instant::now());
                    dev_view.set_batteries(sys.batteries().iter().map(reading).collect());
                }
            }
            // 0. Keep the panel parented per the Placement anchor. The
            // compositor tracks the device for us; we only re-issue the
            // transform when the device index changes.
            if last_attach_check.is_none_or(|t| t.elapsed() >= ATTACH_POLL) {
                last_attach_check = Some(Instant::now());
                let target = match (anchor, &system) {
                    (rsoverlay_core::PlacementAnchor::Wrist, Some(s)) => s.hand_index(Hand::Left),
                    (rsoverlay_core::PlacementAnchor::Hmd, _) => Some(0), // HMD is always device 0
                    _ => None,
                };
                if target != attached_to {
                    let res = match (anchor, target) {
                        (rsoverlay_core::PlacementAnchor::Wrist, Some(dev)) => {
                            let [x, y, z] = WRIST_OFFSET;
                            overlay.set_transform_device_relative(handle, dev, &pose_offset_pitch(x, y, z, WRIST_PITCH_DEG))
                        }
                        (rsoverlay_core::PlacementAnchor::Hmd, Some(dev)) => {
                            let [x, y, z] = HMD_OFFSET;
                            overlay.set_transform_device_relative(handle, dev, &pose_translation(x, y, z))
                        }
                        _ => overlay.set_transform_absolute(handle, &pose),
                    };
                    match res {
                        Ok(()) => info!("overlay anchored to {:?} (device {:?})", anchor, target),
                        Err(e) => warn!("anchoring overlay: {e:#}"),
                    }
                    attached_to = target;
                }
            }

            let dt = last_frame.elapsed().as_secs_f32();
            last_frame = Instant::now();
            let controllers = system.as_ref().map(|s| s.controllers()).unwrap_or_default();
            let wrist_hand = (anchor == rsoverlay_core::PlacementAnchor::Wrist && attached_to.is_some()).then_some(Hand::Left);
            let samples: Vec<HandSample> = controllers
                .iter()
                .filter(|c| Some(c.hand) != wrist_hand)
                .map(|c| HandSample {
                    hand: match c.hand {
                        Hand::Left => laser::Hand::Left,
                        Hand::Right => laser::Hand::Right,
                    },
                    hit: overlay
                        .intersect(handle, c.origin, c.dir)
                        .map(|(uv, _dist)| LaserInput::uv_to_points(uv, panel)),
                    trigger: c.trigger,
                    stick_y: c.stick[1],
                })
                .collect();
            let lf = laser.update(&samples, dt);
            if let Some(h) = lf.haptic {
                info!("laser click ({h:?})");
            }
            if let (Some(h), Some(sys)) = (lf.haptic, &system) {
                let want = match h {
                    laser::Hand::Left => Hand::Left,
                    laser::Hand::Right => Hand::Right,
                };
                if let Some(c) = controllers.iter().find(|c| c.hand == want) {
                    sys.haptic(c.index, CLICK_HAPTIC_US);
                }
            }
            let raw_input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, panel)),
                // Explicit clock: the motion system (M34) and toasts read
                // ctx time, which otherwise doesn't advance without a
                // windowing integration.
                time: Some(started.elapsed().as_secs_f64()),
                predicted_dt: dt.clamp(1.0 / 240.0, 0.1),
                events: lf.events,
                ..Default::default()
            };
            let accent = dev_view.accent;
            egui_panel.render(
                &gpu.device,
                &gpu.queue,
                &overlay_view,
                raw_input,
                |ctx| {
                    dev_view.ui_with(ctx, injector.as_ref());
                    laser::paint_cursor(ctx, lf.cursor, accent);
                },
            );

            // 1a. OSC out: chatbox + avatar parameters (battery).
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

            // 1b. Night light + opacity, applied by the compositor.
            let tint = dev_view.overlay_tint();
            if tint.iter().zip(applied_tint).any(|(a, b)| (a - b).abs() > 1e-3 || b.is_nan()) {
                if let Err(e) = overlay.set_color(handle, [tint[0], tint[1], tint[2]]) {
                    warn!("SetOverlayColor: {e:#}");
                }
                if let Err(e) = overlay.set_alpha(handle, tint[3]) {
                    warn!("SetOverlayAlpha: {e:#}");
                }
                applied_tint = tint;
            }

            // 2. Hand the raw D3D12 resource + queue to OpenVR. The
            // compositor waits on the queue before sampling, so the
            // frame we just submitted is guaranteed-visible.
            unsafe {
                if let Err(e) = overlay.set_texture_d3d12(handle, raw_resource, raw_queue) {
                    warn!("SetOverlayTexture (D3D12): {e:#}");
                }
            }

            std::thread::sleep(Duration::from_millis(11));
        }

        overlay.hide(handle).ok();
        overlay.clear_texture(handle).ok();
        overlay.destroy(handle).ok();
        info!("clean shutdown");
        Ok(())
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
