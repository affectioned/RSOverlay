//! Mipmapped copy of a captured texture, so drawing a big desktop into a
//! smaller panel averages texels instead of skipping them.
//!
//! A 2560- or 3840-wide monitor drawn into a ~1920-texel window panel is
//! minified 1.3–2x (more on a small panel). Plain bilinear sampling of a
//! single-level texture only reads 4 texels per pixel, so past ~1.5x it
//! drops detail: text shimmers and edges crawl, which reads as a
//! low-bitrate stream. Each refresh copies the capture into level 0 and
//! box-filters each next level from the previous one; the sampler blends
//! between levels (trilinear).

use crate::GpuCtx;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
/// Enough for 32x minification; smaller levels are never sampled.
const MAX_LEVELS: u32 = 6;

const SHADER: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the target.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    return o;
}

// Bilinear at the centre of each 2x2 block = their average. The views
// are sRGB, so the average is taken in linear light.
@fragment
fn fs(v: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(src, smp, v.uv, 0.0);
}
"#;

/// Number of levels for a `w` x `h` texture.
pub fn level_count(w: u32, h: u32) -> u32 {
    (32 - w.max(h).max(1).leading_zeros()).min(MAX_LEVELS)
}

pub struct MipMirror {
    pub texture: wgpu::Texture,
    /// All levels; what egui samples.
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pipeline: wgpu::RenderPipeline,
    /// Bind group i reads level i, for rendering level i + 1.
    downs: Vec<(wgpu::BindGroup, wgpu::TextureView)>,
}

impl MipMirror {
    pub fn new(gpu: &GpuCtx, width: u32, height: u32) -> Self {
        let levels = level_count(width, height);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mirror-mips"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mirror-mips"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mirror-mips"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mirror-mips"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mirror-mips"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: "vs", buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mirror-mips"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let level = |i: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: i, mip_level_count: Some(1), ..Default::default() })
        };
        let downs = (0..levels.saturating_sub(1))
            .map(|i| {
                let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mirror-mips"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&level(i)) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                    ],
                });
                (bind, level(i + 1))
            })
            .collect();
        Self { texture, view, width, height, pipeline, downs }
    }

    /// Copy `src` (same size, BGRA8 sRGB, COPY_SRC) into level 0 and
    /// rebuild the smaller levels. One submit; queue order puts it
    /// before anything drawn with the result afterwards.
    pub fn refresh(&self, gpu: &GpuCtx, src: &wgpu::Texture) {
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mirror-mips") });
        enc.copy_texture_to_texture(
            src.as_image_copy(),
            self.texture.as_image_copy(),
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        for (bind, target) in &self.downs {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mirror-mips"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(enc.finish()));
    }

    /// How egui should sample it: trilinear.
    pub fn sampler() -> wgpu::SamplerDescriptor<'static> {
        wgpu::SamplerDescriptor {
            label: Some("mirror-trilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_the_long_side() {
        assert_eq!(level_count(1, 1), 1);
        assert_eq!(level_count(2, 1), 2);
        assert_eq!(level_count(1920, 1080), MAX_LEVELS);
        assert_eq!(level_count(16, 9), 5);
        assert_eq!(level_count(0, 0), 1);
    }

    /// On the real GPU: one-texel black / white stripes (small text at
    /// desktop resolution) must come out mid-grey one level down, not
    /// all-black or all-white as point-ish sampling would give.
    #[test]
    fn stripes_average_to_grey_on_the_gpu() {
        let Ok(gpu) = pollster::block_on(GpuCtx::new(rsoverlay_core::RuntimeMode::Preview)) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let (w, h) = (64u32, 64u32);
        let src = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("stripes"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let px: Vec<u8> = (0..w * h).flat_map(|i| if (i % w) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        gpu.queue.write_texture(
            src.as_image_copy(),
            &px,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let mips = MipMirror::new(&gpu, w, h);
        mips.refresh(&gpu, &src);
        // Read level 1 (32 x 32; 128-byte rows padded to 256).
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: 256 * 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture { texture: &mips.texture, mip_level: 1, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::ImageCopyBuffer { buffer: &buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(32) } },
            wgpu::Extent3d { width: 32, height: 32, depth_or_array_layers: 1 },
        );
        gpu.queue.submit(Some(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::Maintain::Wait);
        let data = buf.slice(..).get_mapped_range();
        for y in 0..32usize {
            for x in 0..32usize {
                let p = &data[y * 256 + x * 4..][..4];
                // Linear 0.5 encoded as sRGB = 188.
                assert!(p[..3].iter().all(|&c| (180..=196).contains(&c)), "texel ({x},{y}) = {p:?}, want grey");
                assert_eq!(p[3], 255);
            }
        }
    }
}
