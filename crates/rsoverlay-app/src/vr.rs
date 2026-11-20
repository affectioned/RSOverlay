//! SteamVR runtime path.
//!
//! Scope of this milestone: hit the P0 acceptance — "red square appears 2 m
//! in front of the HMD in SteamVR, Ctrl+C shuts down with no leaked overlay
//! handles."
//!
//! Implementation: stand up a dedicated D3D11 device, create a BGRA8 render
//! target texture, clear it to red every frame, and submit it to OpenVR via
//! `SetOverlayTexture` with `eType = DirectX`. A richer wgpu-rendered overlay
//! (egui, mirror, notifications) follows once we wire the DXGI shared-handle
//! interop with the preview-mode wgpu render target.

#[cfg(windows)]
mod imp {
    use anyhow::{anyhow, Context, Result};
    use rsoverlay_notify::NotifyChannel;
    use rsoverlay_openvr::{pose_translation, OpenVr};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::runtime::Handle;
    use tracing::{info, warn};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
        ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

    const TEX_SIZE: u32 = 512;
    const CLEAR_RGBA: [f32; 4] = [0.85, 0.07, 0.14, 1.0];

    pub fn run(_notify: Option<NotifyChannel>, rt: Handle) -> Result<()> {
        let vr = OpenVr::init().context("OpenVR init")?;
        let overlay = vr.overlay().context("acquiring IVROverlay")?;

        let handle = overlay
            .create("rsoverlay.main", "RSOverlay")
            .context("CreateOverlay")?;
        overlay.set_width(handle, 1.0).ok();
        let pose = pose_translation(0.0, 1.5, -2.0);
        overlay.set_transform_absolute(handle, &pose).ok();

        // Build D3D11 device + overlay texture.
        let (device, context, texture, rtv) = create_overlay_texture(TEX_SIZE)?;

        // Fill once — OpenVR holds a reference to the texture via SetOverlayTexture.
        unsafe { context.ClearRenderTargetView(&rtv, &CLEAR_RGBA) };

        let tex_ptr: *mut c_void = texture_raw_ptr(&texture);
        unsafe {
            overlay
                .set_texture_d3d11(handle, tex_ptr)
                .context("SetOverlayTexture")?;
        }
        overlay.show(handle).ok();
        info!("overlay registered at (0, 1.5, -2) with red {TEX_SIZE}x{TEX_SIZE} texture");

        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                r.store(false, Ordering::SeqCst);
            }
        });

        // Keep the texture alive and the main thread responsive while OpenVR
        // continues compositing. ClearRenderTargetView again periodically so
        // the compositor sees us as live.
        while running.load(Ordering::SeqCst) {
            unsafe { context.ClearRenderTargetView(&rtv, &CLEAR_RGBA) };
            unsafe {
                if let Err(e) = overlay.set_texture_d3d11(handle, tex_ptr) {
                    warn!("resubmitting overlay texture failed: {e:#}");
                }
            }
            std::thread::sleep(Duration::from_millis(16));
        }

        overlay.hide(handle).ok();
        overlay.clear_texture(handle).ok();
        overlay.destroy(handle).ok();

        // Explicit drops to guarantee the D3D11 objects outlive the final
        // OpenVR submission (OpenVR takes its own reference, but we keep the
        // order explicit for the reader).
        drop(rtv);
        drop(texture);
        drop(context);
        drop(device);

        info!("clean shutdown");
        Ok(())
    }

    fn create_overlay_texture(
        size: u32,
    ) -> Result<(
        ID3D11Device,
        ID3D11DeviceContext,
        ID3D11Texture2D,
        ID3D11RenderTargetView,
    )> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut feature_level = D3D_FEATURE_LEVEL::default();
            let fl = [D3D_FEATURE_LEVEL_11_0];
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&fl),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut feature_level),
                Some(&mut context),
            )
            .context("D3D11CreateDevice")?;
            let device = device.ok_or_else(|| anyhow!("D3D11CreateDevice returned null"))?;
            let context = context.ok_or_else(|| anyhow!("no immediate context"))?;

            let desc = D3D11_TEXTURE2D_DESC {
                Width: size,
                Height: size,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut tex: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .context("CreateTexture2D overlay")?;
            let texture = tex.ok_or_else(|| anyhow!("CreateTexture2D returned null"))?;

            let mut rtv: Option<ID3D11RenderTargetView> = None;
            device
                .CreateRenderTargetView(&texture, None, Some(&mut rtv))
                .context("CreateRenderTargetView")?;
            let rtv = rtv.ok_or_else(|| anyhow!("CreateRenderTargetView returned null"))?;

            Ok((device, context, texture, rtv))
        }
    }

    fn texture_raw_ptr(tex: &ID3D11Texture2D) -> *mut c_void {
        use windows::core::Interface;
        tex.as_raw()
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
