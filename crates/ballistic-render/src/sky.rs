//! The sky box, like the original's *TT Sky* (research findings 19).
//!
//! - Square side faces whose corners lie on a circle of the sky's radius
//!   around the camera, the first one centred behind the level's start
//!   direction (Virtools −Z), the others going round so that each is left of
//!   the previous one seen from inside; plus a bottom face. With four sides
//!   that's a cube.
//! - It follows the camera's position, not its rotation, and is drawn first,
//!   without depth.
//! - Its projection widens the camera's horizontal field of view by
//!   `distortion` of the way to 180° (`TT_Gravity_RT.dll` `0x10006220`).

use ballistic_bundle as bb;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::{Camera, Gpu, texture::GpuTexture};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: Mat4,
    color: [f32; 4],
}

/// One textured face: its 6 vertices in the buffer.
struct Face {
    texture: wgpu::BindGroup,
    first: u32,
}

pub struct SkyRenderer {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    faces: Vec<Face>,
    distortion: f32,
    color: [f32; 4],
}

/// The faces' corners, in bundle space (right-handed, Y up), with UVs.
/// `sides` faces, then the bottom.
pub fn sky_mesh(params: &bb::SkyParams) -> Vec<[([f32; 3], [f32; 2]); 4]> {
    let n = params.sides.len().max(3);
    #[expect(clippy::cast_precision_loss, reason = "a handful of sides")]
    let step = std::f32::consts::TAU / n as f32;
    let r = params.radius;
    let chord = r * (2.0 - 2.0 * step.cos()).sqrt();
    let h = params.height.unwrap_or(chord);
    let (lo, hi) = (params.y - h * 0.5, params.y + h * 0.5);
    // Virtools angles from 225° (5π/4, a constant in TT Sky), x = cos,
    // z = sin; the bundle mirrors Z.
    let corner = |i: usize| {
        #[expect(clippy::cast_precision_loss, reason = "a handful of sides")]
        let a = 5.0 * std::f32::consts::FRAC_PI_4 + step * i as f32;
        (r * a.cos(), -r * a.sin())
    };
    let mut quads = Vec::new();
    for i in 0..n {
        // Seen from inside, corner i is on the face's right, i + 1 on its left.
        let (rx, rz) = corner(i);
        let (lx, lz) = corner(i + 1);
        quads.push([
            ([lx, hi, lz], [0.0, 0.0]),
            ([rx, hi, rz], [1.0, 0.0]),
            ([rx, lo, rz], [1.0, 1.0]),
            ([lx, lo, lz], [0.0, 1.0]),
        ]);
    }
    if params.bottom.is_some() {
        // Seen from inside looking down: the image's top towards the third
        // face (Virtools +Z), its right towards the second (+X). UVs from the
        // corner directions, as TT Sky maps it.
        let uv = |x: f32, z: f32| {
            let len = (x * x + z * z).sqrt().max(f32::EPSILON);
            let s = std::f32::consts::FRAC_1_SQRT_2;
            [0.5 + x / len * s, 0.5 + z / len * s]
        };
        let corners: Vec<(f32, f32)> = (0..4).map(|i| corner(i * n / 4)).collect();
        let p = |(x, z): (f32, f32)| ([x, lo, z], uv(x, z));
        if n == 4 {
            quads.push([p(corners[0]), p(corners[1]), p(corners[2]), p(corners[3])]);
        }
    }
    quads
}

impl SkyRenderer {
    /// `faces`: the side textures in order, then the bottom's (if any).
    #[allow(clippy::too_many_lines, reason = "wgpu setup, start to finish")]
    pub fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        params: &bb::SkyParams,
        faces: &[GpuTexture],
    ) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("sky.wgsl"));
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky texture"),
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
            label: Some("sky"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&layout),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        // Clamped, like the original's sky materials.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let mut vertices = Vec::new();
        let mut out = Vec::new();
        for (quad, texture) in sky_mesh(params).iter().zip(faces) {
            #[expect(clippy::cast_possible_truncation, reason = "a few faces")]
            let first = vertices.len() as u32;
            let v = |i: usize| Vertex {
                position: quad[i].0,
                uv: quad[i].1,
            };
            vertices.extend_from_slice(&[v(0), v(3), v(2), v(0), v(2), v(1)]);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sky face"),
                layout: &texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            out.push(Face {
                texture: group,
                first,
            });
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sky vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Self {
            pipeline,
            globals,
            globals_group,
            vertices,
            faces: out,
            distortion: params.distortion,
            color: params.color,
        }
    }

    /// The sky's projection: the camera's, with the horizontal field of view
    /// widened by `distortion` of the way to 180°.
    pub fn projection(camera: &Camera, aspect: f32, distortion: f32) -> Mat4 {
        let fov_x = 2.0 * ((camera.fov_y * 0.5).tan() * aspect).atan();
        let wide = fov_x + (std::f32::consts::PI - fov_x) * distortion;
        let fov_y = 2.0 * ((wide * 0.5).tan() / aspect).atan();
        glam::camera::rh::proj::directx::perspective(fov_y, aspect, 1.0, 1000.0)
    }

    /// Clears `color` to `background` and draws the sky.
    pub fn render(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
        background: [f64; 4],
    ) {
        let view = glam::camera::rh::view::look_to_mat4(Vec3::ZERO, camera.forward(), Vec3::Y);
        let globals = Globals {
            view_proj: Self::projection(camera, aspect, self.distortion) * view,
            color: self.color,
        };
        gpu.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let [r, g, b, a] = background;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for f in &self.faces {
            pass.set_bind_group(1, &f.texture, &[]);
            pass.draw(f.first..f.first + 6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> bb::SkyParams {
        bb::SkyParams {
            radius: 100.0,
            distortion: 0.15,
            height: None,
            y: 0.0,
            color: [1.0; 4],
            sides: ["_back", "_right", "_front", "_left"]
                .map(String::from)
                .to_vec(),
            top: None,
            bottom: Some("_down".into()),
        }
    }

    fn centre(q: &[([f32; 3], [f32; 2]); 4]) -> Vec3 {
        q.iter().map(|(p, _)| Vec3::from(*p)).sum::<Vec3>() / 4.0
    }

    #[test]
    fn four_sides_make_a_cube_around_the_camera() {
        let m = sky_mesh(&params());
        assert_eq!(m.len(), 5);
        let half = 100.0 * std::f32::consts::FRAC_1_SQRT_2;
        // Back (Virtools −Z) is bundle +Z; then Right +X, Front −Z, Left −X.
        let expect = [Vec3::Z, Vec3::X, Vec3::NEG_Z, Vec3::NEG_X];
        for (q, dir) in m.iter().zip(expect) {
            assert!((centre(q) - dir * half).length() < 1e-3, "{:?}", centre(q));
            // Square.
            let w = Vec3::from(q[0].0).distance(Vec3::from(q[1].0));
            let h = Vec3::from(q[0].0).distance(Vec3::from(q[3].0));
            assert!((w - h).abs() < 1e-3 && (w - 2.0 * half).abs() < 1e-3);
        }
        assert!((centre(&m[4]).y + half).abs() < 1e-3);
    }

    #[test]
    fn faces_join_left_to_right_seen_from_inside() {
        // Each face's left edge (u = 0) is the next face's right edge (u = 1).
        let m = sky_mesh(&params());
        for i in 0..4 {
            let next = &m[(i + 1) % 4];
            let near = |a: [f32; 3], b: [f32; 3]| Vec3::from(a).distance(Vec3::from(b)) < 1e-3;
            assert!(near(m[i][0].0, next[1].0) && near(m[i][3].0, next[2].0));
        }
    }

    #[test]
    fn the_bottom_s_top_edge_meets_the_front() {
        let m = sky_mesh(&params());
        let front = &m[2];
        let bottom = &m[4];
        // The bottom's corners with v = 0 (its top edge) are the front's
        // lower corners.
        let top_edge: Vec<[f32; 3]> = bottom
            .iter()
            .filter(|(_, uv)| uv[1] < 0.01)
            .map(|(p, _)| *p)
            .collect();
        assert_eq!(top_edge.len(), 2);
        for p in [front[2].0, front[3].0] {
            assert!(
                top_edge
                    .iter()
                    .any(|q| Vec3::from(*q).distance(Vec3::from(p)) < 1e-3)
            );
        }
    }

    #[test]
    fn distortion_widens_the_view() {
        let cam = Camera::default();
        let p = SkyRenderer::projection(&cam, 4.0 / 3.0, 0.15);
        let base = cam.projection(4.0 / 3.0);
        assert!(p.x_axis.x < base.x_axis.x);
        // The aspect ratio is kept.
        assert!((p.y_axis.y / p.x_axis.x - 4.0 / 3.0).abs() < 1e-4);
    }
}
