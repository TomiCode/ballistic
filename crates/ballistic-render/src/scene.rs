//! GPU-side scene: uploads a bundle [`Scene`] and draws it.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use ballistic_bundle::{self as bb, Scene};
use glam::{Mat3, Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::texture::{self, GpuTexture};
use crate::{Camera, DEPTH_FORMAT, Gpu, Result};

/// Which triangle faces to cull. The front face is counter-clockwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CullMode {
    /// Cull back faces, except on two-sided materials (normal mode).
    Back,
    /// Draw everything.
    None,
    /// Cull front faces (debugging winding).
    Front,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    pub show_hidden: bool,
    pub cull: CullMode,
    pub clear_color: [f64; 4],
    /// Clear colour and depth before drawing (false to draw over a previous scene).
    pub clear: bool,
    /// Direction *towards* the light.
    pub light_dir: Vec3,
    pub light_color: Vec3,
    pub ambient: Vec3,
    /// `(start, end, colour)` for linear distance fog.
    pub fog: Option<(f32, f32, Vec3)>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            show_hidden: false,
            cull: CullMode::Back,
            clear_color: [0.30, 0.36, 0.45, 1.0],
            clear: true,
            light_dir: Vec3::new(0.35, 1.0, 0.25).normalize(),
            light_color: Vec3::splat(0.85),
            ambient: Vec3::splat(0.45),
            fog: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobalsUniform {
    view_proj: Mat4,
    view: Mat4,
    camera_pos: Vec4,
    light_dir: Vec4,
    light_color: Vec4,
    ambient: Vec4,
    fog_color: Vec4,
    fog: Vec4,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ObjectUniform {
    model: Mat4,
    normal: Mat4,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniform {
    diffuse: [f32; 4],
    ambient: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    blend: Option<(bb::BlendFactor, bb::BlendFactor)>,
    z_write: bool,
    z_func: bb::CompareFunc,
    cull: Option<wgpu::Face>,
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    blend: Option<(bb::BlendFactor, bb::BlendFactor)>,
    z_write: bool,
    z_func: bb::CompareFunc,
    two_sided: bool,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    submeshes: Vec<(Range<u32>, Option<u32>)>,
}

struct DrawObject {
    /// Index in `Scene::objects`.
    scene_index: u32,
    mesh: u32,
    uniform_offset: u32,
    hidden: bool,
    center: Vec3,
    /// Local-space bounds centre (to recompute `center` after moves).
    local_center: Vec3,
}

pub struct SceneRenderer {
    target_format: wgpu::TextureFormat,
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    globals_buffer: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    object_group: wgpu::BindGroup,
    object_buffer: wgpu::Buffer,
    materials: Vec<GpuMaterial>,
    default_material: GpuMaterial,
    meshes: Vec<GpuMesh>,
    objects: Vec<DrawObject>,
    /// World-space bounds of visible geometry (of all geometry if nothing is visible).
    pub bounds: (Vec3, Vec3),
    /// Position of the level start marker (`PS_Levelstart` group), if any.
    pub start_position: Option<Vec3>,
    pub triangle_count: u64,
}

/// Loads texture bytes (KTX2) by bundle key.
pub type TextureLoader<'a> = dyn FnMut(&str) -> Option<Vec<u8>> + 'a;

impl SceneRenderer {
    /// Uploads `scene`. Returns the renderer and non-fatal warnings.
    #[allow(clippy::too_many_lines)] // linear GPU setup sequence
    pub fn new(
        gpu: &Gpu,
        scene: &Scene,
        target_format: wgpu::TextureFormat,
        load_texture: &mut TextureLoader<'_>,
    ) -> Result<(Self, Vec<String>)> {
        let device = &gpu.device;
        let mut warnings = Vec::new();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });

        let uniform_entry = |binding, dynamic| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, false)],
        });
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object"),
            entries: &[uniform_entry(0, true)],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                uniform_entry(0, false),
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[
                Some(&globals_layout),
                Some(&object_layout),
                Some(&material_layout),
            ],
            immediate_size: 0,
        });

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<GlobalsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        // Materials.
        let mut samplers: HashMap<(bb::AddressMode, bool), wgpu::Sampler> = HashMap::new();
        let mut textures: HashMap<String, Option<Arc<GpuTexture>>> = HashMap::new();
        let white = Arc::new(texture::white(gpu));
        let mut make_material = |m: &bb::Material, warnings: &mut Vec<String>| -> GpuMaterial {
            let tex = match &m.texture {
                Some(key) => textures
                    .entry(key.clone())
                    .or_insert_with(|| {
                        let Some(bytes) = load_texture(key) else {
                            warnings.push(format!("texture {key} missing from bundle"));
                            return None;
                        };
                        texture::upload_ktx2(gpu, key, &bytes)
                            .map_err(|e| warnings.push(e.to_string()))
                            .ok()
                            .map(Arc::new)
                    })
                    .clone()
                    .unwrap_or_else(|| white.clone()),
                None => white.clone(),
            };
            let sampler = samplers
                .entry((m.address_mode, m.linear_filter))
                .or_insert_with(|| make_sampler(device, m.address_mode, m.linear_filter));
            let uniform = MaterialUniform {
                diffuse: m.diffuse,
                ambient: m.ambient,
                emissive: m.emissive,
                params: [
                    m.alpha_test.map_or(0.0, |(_, r)| r),
                    m.alpha_test
                        .map_or(0.0, |(f, _)| f32::from(compare_code(f))),
                    f32::from(blend_code(m.texture_blend)),
                    f32::from(u8::from(m.environment_map)),
                ],
            };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&m.name),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&m.name),
                layout: &material_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&tex.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            GpuMaterial {
                bind_group,
                blend: m.blend.then_some((m.src_blend, m.dest_blend)),
                z_write: m.z_write,
                z_func: m.z_func,
                two_sided: m.two_sided,
            }
        };
        let materials: Vec<GpuMaterial> = scene
            .materials
            .iter()
            .map(|m| make_material(m, &mut warnings))
            .collect();
        let default_material = make_material(&default_material(), &mut warnings);

        // Meshes.
        let mut triangle_count = 0u64;
        let meshes: Vec<GpuMesh> = scene
            .meshes
            .iter()
            .map(|m| {
                triangle_count += (m.index_data.len() / 6) as u64;
                GpuMesh {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&m.name),
                        contents: &m.vertex_data,
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    // Index buffers must be 4-byte aligned in size.
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&m.name),
                        contents: &padded(&m.index_data),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    submeshes: m
                        .submeshes
                        .iter()
                        .map(|s| (s.first_index..s.first_index + s.index_count, s.material))
                        .collect(),
                }
            })
            .collect();

        // Objects with meshes, one dynamic uniform slot each.
        let align = device.limits().min_uniform_buffer_offset_alignment;
        let stride = (std::mem::size_of::<ObjectUniform>() as u32).next_multiple_of(align);
        let mut object_bytes = Vec::new();
        let mut objects = Vec::new();
        let (mut bmin, mut bmax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        // Fallback bounds over hidden objects too (prefab files hide everything).
        let (mut amin, mut amax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (scene_index, o) in scene.objects.iter().enumerate() {
            let Some(mesh) = o.mesh else { continue };
            let model = Mat4::from_cols_array_2d(&o.transform);
            let normal = Mat4::from_mat3(Mat3::from_mat4(model).inverse().transpose());
            let offset = object_bytes.len() as u32;
            object_bytes.extend_from_slice(bytemuck::bytes_of(&ObjectUniform { model, normal }));
            object_bytes.resize((offset + stride) as usize, 0);
            let src = &scene.meshes[mesh as usize];
            let (lo, hi) = (Vec3::from(src.bounds_min), Vec3::from(src.bounds_max));
            let mut center = Vec3::ZERO;
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                );
                let w = model.transform_point3(corner);
                center += w / 8.0;
                amin = amin.min(w);
                amax = amax.max(w);
                if !o.hidden {
                    bmin = bmin.min(w);
                    bmax = bmax.max(w);
                }
            }
            objects.push(DrawObject {
                scene_index: scene_index as u32,
                mesh,
                uniform_offset: offset,
                hidden: o.hidden,
                center,
                local_center: (lo + hi) * 0.5,
            });
        }
        if bmin.x > bmax.x {
            (bmin, bmax) = if amin.x <= amax.x {
                (amin, amax)
            } else {
                (Vec3::ZERO, Vec3::ZERO)
            };
        }
        if object_bytes.is_empty() {
            object_bytes.resize(stride as usize, 0);
        }
        let object_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("objects"),
            contents: &object_bytes,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let object_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("objects"),
            layout: &object_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &object_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
                }),
            }],
        });

        let start_position = scene
            .groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case("PS_Levelstart"))
            .and_then(|g| g.members.first())
            .map(|&i| Vec4::from(scene.objects[i as usize].transform[3]).truncate());

        Ok((
            Self {
                target_format,
                shader,
                layout,
                pipelines: HashMap::new(),
                globals_buffer,
                globals_group,
                object_group,
                object_buffer,
                materials,
                default_material,
                meshes,
                objects,
                bounds: (bmin, bmax),
                start_position,
                triangle_count,
            },
            warnings,
        ))
    }

    /// Moves a scene object (by its index in `Scene::objects`). No-op for
    /// objects without a mesh.
    pub fn set_object_transform(&mut self, gpu: &Gpu, scene_index: usize, model: Mat4) {
        let Some(o) = self
            .objects
            .iter_mut()
            .find(|o| o.scene_index as usize == scene_index)
        else {
            return;
        };
        let normal = Mat4::from_mat3(Mat3::from_mat4(model).inverse().transpose());
        o.center = model.transform_point3(o.local_center);
        gpu.queue.write_buffer(
            &self.object_buffer,
            u64::from(o.uniform_offset),
            bytemuck::bytes_of(&ObjectUniform { model, normal }),
        );
    }

    /// Shows or hides a scene object regardless of its saved hidden flag
    /// (`RenderOptions::show_hidden` still shows everything).
    pub fn set_object_hidden(&mut self, scene_index: usize, hidden: bool) {
        for o in &mut self.objects {
            if o.scene_index as usize == scene_index {
                o.hidden = hidden;
            }
        }
    }

    fn material(&self, idx: Option<u32>) -> &GpuMaterial {
        idx.and_then(|i| self.materials.get(i as usize))
            .unwrap_or(&self.default_material)
    }

    fn key(m: &GpuMaterial, cull: CullMode) -> PipelineKey {
        PipelineKey {
            blend: m.blend,
            z_write: m.z_write,
            z_func: m.z_func,
            cull: match (cull, m.two_sided) {
                (CullMode::None, _) | (CullMode::Back, true) => None,
                (CullMode::Back, false) => Some(wgpu::Face::Back),
                (CullMode::Front, _) => Some(wgpu::Face::Front),
            },
        }
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, key: PipelineKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let blend = key.blend.map(|(src, dst)| {
            let c = wgpu::BlendComponent {
                src_factor: blend_factor(src),
                dst_factor: blend_factor(dst),
                operation: wgpu::BlendOperation::Add,
            };
            wgpu::BlendState { color: c, alpha: c }
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<bb::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: key.cull,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(key.z_write),
                depth_compare: Some(compare_function(key.z_func)),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline);
    }

    /// Records the scene into `encoder`, clearing `color` and `depth`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
        opts: &RenderOptions,
    ) {
        let view = camera.view();
        let (fog_start, fog_end, fog_color, fog_on) = match opts.fog {
            Some((s, e, c)) => (s, e, c, 1.0),
            None => (0.0, 0.0, Vec3::ZERO, 0.0),
        };
        let globals = GlobalsUniform {
            view_proj: camera.projection(aspect) * view,
            view,
            camera_pos: camera.position.extend(1.0),
            light_dir: opts.light_dir.normalize_or(Vec3::Y).extend(0.0),
            light_color: opts.light_color.extend(1.0),
            ambient: opts.ambient.extend(1.0),
            fog_color: fog_color.extend(1.0),
            fog: Vec4::new(fog_start, fog_end, fog_on, 0.0),
        };
        gpu.queue
            .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        // Build the draw list: opaque in scene order, blended back to front.
        let mut opaque = Vec::new();
        let mut blended = Vec::new();
        for (oi, o) in self.objects.iter().enumerate() {
            if o.hidden && !opts.show_hidden {
                continue;
            }
            for (si, (_, mat)) in self.meshes[o.mesh as usize].submeshes.iter().enumerate() {
                let m = self.material(*mat);
                let key = Self::key(m, opts.cull);
                if m.blend.is_some() {
                    let d = o.center.distance_squared(camera.position);
                    blended.push((d, oi, si, key));
                } else {
                    opaque.push((oi, si, key));
                }
            }
        }
        blended.sort_by(|a, b| b.0.total_cmp(&a.0));
        for key in opaque
            .iter()
            .map(|d| d.2)
            .chain(blended.iter().map(|d| d.3))
        {
            self.ensure_pipeline(&gpu.device, key);
        }

        let [r, g, b, a] = opts.clear_color;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if opts.clear {
                        wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a })
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: if opts.clear {
                        wgpu::LoadOp::Clear(1.0)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.globals_group, &[]);

        let mut current: Option<PipelineKey> = None;
        let draws = opaque
            .iter()
            .copied()
            .chain(blended.iter().map(|&(_, oi, si, k)| (oi, si, k)));
        for (oi, si, key) in draws {
            if current != Some(key) {
                pass.set_pipeline(&self.pipelines[&key]);
                current = Some(key);
            }
            let o = &self.objects[oi];
            let mesh = &self.meshes[o.mesh as usize];
            let (range, mat) = &mesh.submeshes[si];
            pass.set_bind_group(1, &self.object_group, &[o.uniform_offset]);
            pass.set_bind_group(2, &self.material(*mat).bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(range.clone(), 0, 0..1);
        }
    }
}

fn padded(bytes: &[u8]) -> Vec<u8> {
    let mut v = bytes.to_vec();
    v.resize(bytes.len().next_multiple_of(4).max(4), 0);
    v
}

fn default_material() -> bb::Material {
    bb::Material {
        name: "default".into(),
        diffuse: [0.8, 0.8, 0.8, 1.0],
        ambient: [0.3, 0.3, 0.3, 1.0],
        specular: [0.0; 4],
        emissive: [0.0, 0.0, 0.0, 1.0],
        specular_power: 0.0,
        texture: None,
        texture_blend: bb::TextureBlend::Modulate,
        address_mode: bb::AddressMode::Wrap,
        linear_filter: true,
        src_blend: bb::BlendFactor::One,
        dest_blend: bb::BlendFactor::Zero,
        blend: false,
        alpha_test: None,
        z_write: true,
        z_func: bb::CompareFunc::LessEqual,
        two_sided: false,
        flat_shading: false,
        wireframe: false,
        environment_map: false,
    }
}

fn make_sampler(device: &wgpu::Device, address: bb::AddressMode, linear: bool) -> wgpu::Sampler {
    let mode = match address {
        bb::AddressMode::Wrap => wgpu::AddressMode::Repeat,
        bb::AddressMode::Mirror | bb::AddressMode::MirrorOnce => wgpu::AddressMode::MirrorRepeat,
        bb::AddressMode::Clamp | bb::AddressMode::Border => wgpu::AddressMode::ClampToEdge,
    };
    let filter = if linear {
        wgpu::FilterMode::Linear
    } else {
        wgpu::FilterMode::Nearest
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: None,
        address_mode_u: mode,
        address_mode_v: mode,
        address_mode_w: mode,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: if linear {
            wgpu::MipmapFilterMode::Linear
        } else {
            wgpu::MipmapFilterMode::Nearest
        },
        lod_min_clamp: 0.0,
        lod_max_clamp: 32.0,
        compare: None,
        anisotropy_clamp: if linear { 16 } else { 1 },
        border_color: None,
    })
}

fn blend_code(t: bb::TextureBlend) -> u8 {
    match t {
        bb::TextureBlend::Decal => 0,
        bb::TextureBlend::Modulate => 1,
        bb::TextureBlend::DecalAlpha => 2,
        bb::TextureBlend::ModulateAlpha => 3,
        bb::TextureBlend::DecalMask => 4,
        bb::TextureBlend::ModulateMask => 5,
        bb::TextureBlend::Copy => 6,
        bb::TextureBlend::Add => 7,
    }
}

fn compare_code(c: bb::CompareFunc) -> u8 {
    match c {
        bb::CompareFunc::Never => 1,
        bb::CompareFunc::Less => 2,
        bb::CompareFunc::Equal => 3,
        bb::CompareFunc::LessEqual => 4,
        bb::CompareFunc::Greater => 5,
        bb::CompareFunc::NotEqual => 6,
        bb::CompareFunc::GreaterEqual => 7,
        bb::CompareFunc::Always => 8,
    }
}

fn compare_function(c: bb::CompareFunc) -> wgpu::CompareFunction {
    match c {
        bb::CompareFunc::Never => wgpu::CompareFunction::Never,
        bb::CompareFunc::Less => wgpu::CompareFunction::Less,
        bb::CompareFunc::Equal => wgpu::CompareFunction::Equal,
        bb::CompareFunc::LessEqual => wgpu::CompareFunction::LessEqual,
        bb::CompareFunc::Greater => wgpu::CompareFunction::Greater,
        bb::CompareFunc::NotEqual => wgpu::CompareFunction::NotEqual,
        bb::CompareFunc::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
        bb::CompareFunc::Always => wgpu::CompareFunction::Always,
    }
}

fn blend_factor(f: bb::BlendFactor) -> wgpu::BlendFactor {
    use bb::BlendFactor as B;
    match f {
        B::Zero => wgpu::BlendFactor::Zero,
        B::One => wgpu::BlendFactor::One,
        B::SrcColor => wgpu::BlendFactor::Src,
        B::InvSrcColor => wgpu::BlendFactor::OneMinusSrc,
        B::SrcAlpha => wgpu::BlendFactor::SrcAlpha,
        B::InvSrcAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
        B::DestAlpha => wgpu::BlendFactor::DstAlpha,
        B::InvDestAlpha => wgpu::BlendFactor::OneMinusDstAlpha,
        B::DestColor => wgpu::BlendFactor::Dst,
        B::InvDestColor => wgpu::BlendFactor::OneMinusDst,
        B::SrcAlphaSat => wgpu::BlendFactor::SrcAlphaSaturated,
    }
}
