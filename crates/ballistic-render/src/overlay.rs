//! Screen-space textured quads drawn over the scene (the HUD).
//!
//! Each quad is a pixel rectangle showing part of a texture, multiplied by a
//! colour and alpha blended, like the original's 2D entities (material
//! texture blend *Modulate Alpha*, *Src Alpha* / *Inv Src Alpha*).

use bytemuck::{Pod, Zeroable};

use crate::{Gpu, texture::GpuTexture};

/// A quad, in pixels from the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// Index returned by [`Overlay::add_texture`].
    pub texture: usize,
    /// Left, top, right, bottom in pixels.
    pub rect: [f32; 4],
    /// Left, top, right, bottom in texture coordinates.
    pub uv: [f32; 4],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

pub struct Overlay {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Vec<wgpu::BindGroup>,
    vertices: Option<wgpu::Buffer>,
}

impl Overlay {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("overlay.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let blend = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: blend,
                        alpha: blend,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("overlay"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            textures: Vec::new(),
            vertices: None,
        }
    }

    /// Makes a texture available to quads; returns its index.
    pub fn add_texture(&mut self, gpu: &Gpu, texture: &GpuTexture) -> usize {
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay texture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.textures.push(group);
        self.textures.len() - 1
    }

    /// Draws `quads` in order over `color` (`size` in pixels).
    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        size: (u32, u32),
        quads: &[Quad],
    ) {
        let quads: Vec<&Quad> = quads
            .iter()
            .filter(|q| q.texture < self.textures.len())
            .collect();
        if quads.is_empty() {
            return;
        }
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let clip = |x: f32, y: f32| [x / w * 2.0 - 1.0, 1.0 - y / h * 2.0];
        let mut vertices = Vec::with_capacity(quads.len() * 6);
        for q in &quads {
            let [l, t, r, b] = q.rect;
            let [u0, v0, u1, v1] = q.uv;
            let v = |x, y, u, v| Vertex {
                position: clip(x, y),
                uv: [u, v],
                color: q.color,
            };
            let (tl, tr, br, bl) = (
                v(l, t, u0, v0),
                v(r, t, u1, v0),
                v(r, b, u1, v1),
                v(l, b, u0, v1),
            );
            vertices.extend_from_slice(&[tl, bl, br, tl, br, tr]);
        }
        let bytes: &[u8] = bytemuck::cast_slice(&vertices);
        let fits = self
            .vertices
            .as_ref()
            .is_some_and(|b| b.size() >= bytes.len() as u64);
        if !fits {
            self.vertices = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("overlay vertices"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(buffer) = self.vertices.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(buffer, 0, bytes);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("overlay"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, buffer.slice(..));
        // Consecutive quads with the same texture in one draw.
        let mut start = 0;
        while start < quads.len() {
            let texture = quads[start].texture;
            let end = quads[start..]
                .iter()
                .position(|q| q.texture != texture)
                .map_or(quads.len(), |n| start + n);
            pass.set_bind_group(0, &self.textures[texture], &[]);
            #[expect(clippy::cast_possible_truncation, reason = "a few hundred quads")]
            pass.draw((start * 6) as u32..(end * 6) as u32, 0..1);
            start = end;
        }
    }
}
