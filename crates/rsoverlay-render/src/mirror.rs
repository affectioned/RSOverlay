//! Desktop mirror texture — a wgpu texture that gets updated each frame
//! from a captured DXGI frame.

use crate::GpuCtx;
use anyhow::{anyhow, Result};
use rsoverlay_capture::Frame;

pub struct MirrorTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl MirrorTexture {
    pub fn new(gpu: &GpuCtx, width: u32, height: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mirror"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view, width, height }
    }

    /// Upload one captured BGRA frame into the texture.
    pub fn upload(&self, gpu: &GpuCtx, frame: &Frame) -> Result<()> {
        if frame.width != self.width || frame.height != self.height {
            return Err(anyhow!(
                "frame {}x{} does not match mirror texture {}x{}",
                frame.width,
                frame.height,
                self.width,
                self.height
            ));
        }
        gpu.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.bgra,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(frame.row_pitch),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }
}
