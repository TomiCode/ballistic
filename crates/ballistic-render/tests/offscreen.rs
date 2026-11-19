//! Off-screen rendering of a synthetic scene. Skipped when no GPU adapter exists
//! (e.g. some CI machines).

use ballistic_bundle as bb;
use ballistic_render::{Camera, CullMode, Gpu, RenderOptions, RenderTarget, SceneRenderer};
use glam::Vec3;

/// A unit quad at the origin facing +Y, wound counter-clockwise seen from above
/// (the bundle's front-face convention).
fn quad_scene(material: bb::Material) -> bb::Scene {
    let v = |x: f32, z: f32| bb::Vertex {
        position: [x, 0.0, z],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    };
    let verts = [v(-1.0, -1.0), v(-1.0, 1.0), v(1.0, 1.0), v(1.0, -1.0)];
    // Seen from +Y looking down -Y (screen right = +X, screen up = -Z): CCW.
    let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
    let bytes = |s: &[f32]| s.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let vertex_data = verts
        .iter()
        .flat_map(|v| {
            bytes(&[v.position, v.normal].concat())
                .into_iter()
                .chain(bytes(&v.uv))
        })
        .collect();
    bb::Scene {
        meshes: vec![bb::Mesh {
            name: "quad".into(),
            vertex_data,
            index_data: indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
            submeshes: vec![bb::Submesh {
                first_index: 0,
                index_count: 6,
                material: Some(0),
            }],
            bounds_min: [-1.0, 0.0, -1.0],
            bounds_max: [1.0, 0.0, 1.0],
        }],
        materials: vec![material],
        objects: vec![bb::SceneObject {
            name: "quad".into(),
            kind: bb::ObjectKind::Entity,
            transform: glam::Mat4::IDENTITY.to_cols_array_2d(),
            mesh: Some(0),
            parent: None,
            hidden: false,
        }],
        groups: vec![],
    }
}

fn red_material() -> bb::Material {
    bb::Material {
        name: "red".into(),
        diffuse: [1.0, 0.0, 0.0, 1.0],
        ambient: [1.0, 0.0, 0.0, 1.0],
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

/// Renders the quad from above and returns the centre pixel.
fn centre_pixel(gpu: &Gpu, material: bb::Material, cull: CullMode) -> [u8; 4] {
    let format = ballistic_render::OFFSCREEN_FORMAT;
    let (mut r, warnings) =
        SceneRenderer::new(gpu, &quad_scene(material), format, &mut |_| None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut cam = Camera {
        position: Vec3::new(0.0, 5.0, 0.001),
        ..Camera::default()
    };
    cam.look_at(Vec3::ZERO);
    let target = RenderTarget::new(gpu, 64, 64, format);
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    let opts = RenderOptions {
        cull,
        clear_color: [0.0, 0.0, 1.0, 1.0],
        ..RenderOptions::default()
    };
    r.render(
        gpu,
        &mut enc,
        &target.color_view,
        &target.depth_view,
        &cam,
        1.0,
        &opts,
    );
    gpu.queue.submit([enc.finish()]);
    let px = target.read_rgba(gpu).unwrap();
    let i = (32 * 64 + 32) * 4;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

fn gpu() -> Option<Gpu> {
    match pollster::block_on(Gpu::headless()) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

#[test]
fn front_faces_are_ccw_and_culling_works() {
    let Some(gpu) = gpu() else { return };
    let drawn = centre_pixel(&gpu, red_material(), CullMode::Back);
    assert!(drawn[0] > 200 && drawn[2] < 50, "quad not drawn: {drawn:?}");

    let culled = centre_pixel(&gpu, red_material(), CullMode::Front);
    assert!(
        culled[2] > 200 && culled[0] < 50,
        "front face not culled: {culled:?}"
    );
}

#[test]
fn two_sided_materials_ignore_back_culling() {
    let Some(gpu) = gpu() else { return };
    // View the quad from below: it's a back face.
    let mut m = red_material();
    m.two_sided = true;
    let format = ballistic_render::OFFSCREEN_FORMAT;
    let (mut r, _) = SceneRenderer::new(&gpu, &quad_scene(m), format, &mut |_| None).unwrap();
    let mut cam = Camera {
        position: Vec3::new(0.0, -5.0, 0.001),
        ..Camera::default()
    };
    cam.look_at(Vec3::ZERO);
    let target = RenderTarget::new(&gpu, 32, 32, format);
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    r.render(
        &gpu,
        &mut enc,
        &target.color_view,
        &target.depth_view,
        &cam,
        1.0,
        &RenderOptions {
            clear_color: [0.0, 0.0, 1.0, 1.0],
            ..RenderOptions::default()
        },
    );
    gpu.queue.submit([enc.finish()]);
    let px = target.read_rgba(&gpu).unwrap();
    let i = (16 * 32 + 16) * 4;
    assert!(
        px[i] > 100,
        "two-sided back face not drawn: {:?}",
        &px[i..i + 4]
    );
}

#[test]
fn alpha_test_discards() {
    let Some(gpu) = gpu() else { return };
    let mut m = red_material();
    m.diffuse[3] = 0.2;
    m.alpha_test = Some((bb::CompareFunc::Greater, 0.5));
    let px = centre_pixel(&gpu, m, CullMode::Back);
    assert!(
        px[2] > 200 && px[0] < 50,
        "alpha-tested fragment drawn: {px:?}"
    );
}
