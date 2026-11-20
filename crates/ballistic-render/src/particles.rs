//! Particles: unlit triangles (texture × vertex colour), depth-tested but
//! not writing depth, blended per batch, like the original's particle
//! systems (research findings 20).

use std::collections::HashMap;

use ballistic_bundle as bb;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::{Camera, DEPTH_FORMAT, Gpu, scene::blend_factor, texture::GpuTexture};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct ParticleVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// Triangles sharing a texture and blend.
pub struct ParticleBatch<'a> {
    /// From [`ParticleRenderer::add_texture`].
    pub texture: usize,
    pub blend: (bb::BlendFactor, bb::BlendFactor),
    pub vertices: &'a [ParticleVertex],
}

pub struct ParticleRenderer {
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    texture_layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    pipelines: HashMap<(bb::BlendFactor, bb::BlendFactor), wgpu::RenderPipeline>,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    textures: Vec<wgpu::BindGroup>,
    vertices: Option<wgpu::Buffer>,
}

impl ParticleRenderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("particles.wgsl"));
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle texture"),
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particles"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle globals"),
            size: std::mem::size_of::<Mat4>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particles"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            shader,
            layout,
            texture_layout,
            format,
            pipelines: HashMap::new(),
            globals,
            globals_group,
            sampler,
            textures: Vec::new(),
            vertices: None,
        }
    }

    /// Makes a texture available to batches; returns its index.
    pub fn add_texture(&mut self, gpu: &Gpu, texture: &GpuTexture) -> usize {
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle texture"),
            layout: &self.texture_layout,
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

    /// A plain white texture (for untextured systems).
    pub fn add_white(&mut self, gpu: &Gpu) -> usize {
        let white = crate::texture::white(gpu);
        self.add_texture(gpu, &white)
    }

    fn pipeline(&mut self, gpu: &Gpu, blend: (bb::BlendFactor, bb::BlendFactor)) {
        if self.pipelines.contains_key(&blend) {
            return;
        }
        let c = wgpu::BlendComponent {
            src_factor: blend_factor(blend.0),
            dst_factor: blend_factor(blend.1),
            operation: wgpu::BlendOperation::Add,
        };
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let p = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("particles"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ParticleVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.format,
                        blend: Some(wgpu::BlendState { color: c, alpha: c }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert(blend, p);
    }

    /// Draws `batches` over `color`, tested against `depth`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
        batches: &[ParticleBatch<'_>],
    ) {
        let batches: Vec<&ParticleBatch<'_>> = batches
            .iter()
            .filter(|b| !b.vertices.is_empty() && b.texture < self.textures.len())
            .collect();
        if batches.is_empty() {
            return;
        }
        for b in &batches {
            self.pipeline(gpu, b.blend);
        }
        let view_proj = camera.projection(aspect) * camera.view();
        gpu.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&view_proj));
        let all: Vec<ParticleVertex> = batches
            .iter()
            .flat_map(|b| b.vertices.iter().copied())
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&all);
        if self
            .vertices
            .as_ref()
            .is_none_or(|v| v.size() < bytes.len() as u64)
        {
            self.vertices = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle vertices"),
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
            label: Some("particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.globals_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        let mut first = 0u32;
        for b in batches {
            #[expect(clippy::cast_possible_truncation, reason = "a few thousand vertices")]
            let n = b.vertices.len() as u32;
            if let Some(p) = self.pipelines.get(&b.blend) {
                pass.set_pipeline(p);
                pass.set_bind_group(1, &self.textures[b.texture], &[]);
                pass.draw(first..first + n, 0..1);
            }
            first += n;
        }
    }
}
