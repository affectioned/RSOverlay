//! wgpu renderer for RSOverlay.
//!
//! Two public entry points:
//! - [`Renderer::new_headless`] — build a renderer that only writes into
//!   offscreen textures, for OpenVR texture submission.
//! - [`Renderer::new_surface`] — build a renderer that owns a wgpu surface,
//!   for the desktop preview window.

pub mod backend;
pub mod panel_view;

use anyhow::{Context, Result};
use rsoverlay_core::RuntimeMode;
use std::sync::Arc;
use wgpu::{Backends, InstanceDescriptor, InstanceFlags};

/// Shared wgpu objects. Cheaply cloneable.
#[derive(Clone)]
pub struct GpuCtx {
    pub instance: Arc<wgpu::Instance>,
    pub adapter: Arc<wgpu::Adapter>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
}

impl GpuCtx {
    pub async fn new(mode: RuntimeMode) -> Result<Self> {
        // For VR we must pick the DX12/DX11 adapter the HMD is on; for preview
        // we take whatever is primary.
        // wgpu 0.19 removed DX11 as a distinct backend; DX12 is what we hand to
        // OpenVR via the shared-handle interop (ID3D12Resource → ID3D11Texture2D
        // via resource sharing). Preview mode takes the primary adapter.
        let backends = if mode.is_vr() {
            Backends::DX12
        } else {
            Backends::PRIMARY
        };
        let instance = wgpu::Instance::new(InstanceDescriptor {
            backends,
            flags: InstanceFlags::default(),
            dx12_shader_compiler: wgpu::Dx12Compiler::Fxc,
            gles_minor_version: wgpu::Gles3MinorVersion::Automatic,
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .context("no suitable wgpu adapter")?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("rsoverlay-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .context("requesting wgpu device")?;
        Ok(Self {
            instance: Arc::new(instance),
            adapter: Arc::new(adapter),
            device: Arc::new(device),
            queue: Arc::new(queue),
        })
    }
}

/// A GPU texture the renderer draws into. In VR mode we open this as a shared
/// handle and hand the raw ID3D11Texture2D* to OpenVR. In preview mode we
/// blit it to the window surface.
pub struct OverlayTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
}

impl OverlayTarget {
    pub fn new(gpu: &GpuCtx, width: u32, height: u32) -> Self {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("overlay-target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view, width, height, format }
    }
}

/// Parameters needed to (re)clear a frame.
#[derive(Copy, Clone, Debug)]
pub struct ClearColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl ClearColor {
    pub const RED: Self = Self { r: 0.85, g: 0.07, b: 0.14, a: 1.0 };
    pub const BLACK: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
}

/// Clear the overlay target to a solid color. This is the P0 red-square path.
pub fn clear_target(gpu: &GpuCtx, target: &OverlayTarget, color: ClearColor) {
    let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("clear-target"),
    });
    {
        let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: color.r,
                        g: color.g,
                        b: color.b,
                        a: color.a,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    gpu.queue.submit(Some(enc.finish()));
}
