//! egui-backed UI for RSOverlay panels.
//!
//! One [`EguiPanel`] renders an egui scene into a wgpu texture view — the
//! same `OverlayTarget` the renderer hands to OpenVR / the preview blit.

use egui_wgpu::{Renderer, ScreenDescriptor};
use std::sync::Arc;

pub mod battery;
pub mod i18n;
pub mod icons;
pub mod keyboard;
pub mod keyboard_layout;
pub mod locale;
pub mod media;
pub mod now_playing;
pub mod theme;
pub mod views;

pub struct EguiPanel {
    pub ctx: egui::Context,
    pub renderer: Renderer,
    width: u32,
    height: u32,
    ppp: f32,
}

impl EguiPanel {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let renderer = Renderer::new(device, output_format, None, 1);
        let ctx = egui::Context::default();
        // Pixels-per-point: the trade-off here is logical vertical budget
        // vs icon rasterization sharpness. At 1.5 the dashboard's 6 left-
        // column cards fit in a 1360px window comfortably, and because
        // the icon SVGs are now authored on 48-64px viewBoxes with 3-5px
        // strokes, resvg still produces crisp output at 18px logical ×
        // 1.5 = 27 physical pixels per icon.
        let ppp = 1.5;
        ctx.set_pixels_per_point(ppp);
        theme::install(&ctx);
        icons::install_loaders(&ctx);
        Self { ctx, renderer, width, height, ppp }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    /// Run a frame of egui and encode its render into the given texture view.
    pub fn render<F: FnOnce(&egui::Context)>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        raw_input: egui::RawInput,
        build_ui: F,
    ) {
        let output = self.ctx.run(raw_input, build_ui);
        let clipped = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        let desc = ScreenDescriptor {
            size_in_pixels: [self.width, self.height],
            pixels_per_point: self.ppp,
        };
        for (id, image_delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, image_delta);
        }
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui-encoder"),
        });
        self.renderer.update_buffers(device, queue, &mut enc, &clipped, &desc);
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // BG_ROOT = pure black (AMOLED).
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.renderer.render(&mut pass, &clipped, &desc);
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        queue.submit(Some(enc.finish()));
    }
}

/// Avoid leaking the wgpu `Arc<Device>` type through the public API by taking
/// it by reference above. Shared types live in `rsoverlay-core`.
#[allow(dead_code)]
type _Phantom = Arc<()>;
