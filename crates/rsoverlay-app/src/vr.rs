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
    use rsoverlay_openvr::{pose_translation, DeviceBattery, DeviceKind, OpenVr};
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
    /// Battery props change slowly; polling 64 device slots every
    /// frame would be wasted IPC to vrserver.
    const BATTERY_POLL: Duration = Duration::from_secs(5);

    pub fn run(
        _notify: Option<NotifyChannel>,
        rt: Handle,
        config: rsoverlay_core::Config,
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
        overlay
            .set_width(handle, p.size_w)
            .context("SetOverlayWidthInMeters")?;
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

        overlay.show(handle).ok();
        info!(
            "OpenVR overlay registered at (0, 1.5, -1.5) — \
             dashboard streaming via DirectX 12"
        );

        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                r.store(false, Ordering::SeqCst);
            }
        });

        let mut frame = 0u64;
        while running.load(Ordering::SeqCst) {
            // 1. Render the dashboard into the overlay texture. No window
            // input — we pass an empty RawInput. Interaction via laser
            // pointer / controller lands in the next milestone.
            frame = frame.wrapping_add(1);
            dev_view.frame = frame;
            if let Some(sys) = &system {
                if last_battery_poll.is_none_or(|t| t.elapsed() >= BATTERY_POLL) {
                    last_battery_poll = Some(Instant::now());
                    dev_view.set_batteries(sys.batteries().iter().map(reading).collect());
                }
            }
            let raw_input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(
                        OVERLAY_TEX_W as f32 / 1.5,
                        OVERLAY_TEX_H as f32 / 1.5,
                    ),
                )),
                ..Default::default()
            };
            egui_panel.render(
                &gpu.device,
                &gpu.queue,
                &overlay_view,
                raw_input,
                |ctx| dev_view.ui_with(ctx, injector.as_ref()),
            );

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
) -> anyhow::Result<()> {
    anyhow::bail!("VR mode is Windows-only in this milestone; Linux OpenVR support is scaffolded but not wired");
}
