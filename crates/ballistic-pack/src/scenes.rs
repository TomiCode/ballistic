//! Converts decoded Virtools scene objects into engine-native [`Scene`]s.
//!
//! This is where the coordinate system changes: left-handed → right-handed by
//! mirroring Z, with triangle winding reversed to preserve facing.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckfile::{CkFile, ClassId};
use ckscene::{Decoded, Entity3d, decode_object};

/// Resolves a texture file name from a material (e.g. `Floor_Top_Flat.bmp`)
/// to a bundle texture key, or `None` if no such image is being packed.
pub trait TextureResolver {
    fn resolve(&self, file_name: &str) -> Option<String>;
}

pub struct Converted {
    pub scene: bb::Scene,
    pub warnings: Vec<String>,
}

/// The level init's *set Env. Mapping* (`TT_ReflectionMapping` on each member
/// of `group`, with `FixCube` at `camera` as the camera; research findings
/// 31). Once, so the texture coordinates are fixed: for each vertex, in its
/// object's frame, the direction to the camera reflected about the normal,
/// `r`, gives ((r.x + 1) / 2, (r.z + 1) / 2) in the original's left-handed
/// space. A mesh shared by several members keeps the last one's.
pub fn reflection_map(scene: &mut bb::Scene, group: &str, camera: [f32; 3]) {
    let Some(g) = scene.groups.iter().find(|g| g.name == group) else {
        return;
    };
    let members = g.members.clone();
    for i in members {
        let Some(o) = scene.objects.get(i as usize) else {
            continue;
        };
        let Some(mesh) = o.mesh.and_then(|m| scene.meshes.get_mut(m as usize)) else {
            continue;
        };
        let cam = to_local(&o.transform, camera);
        for v in mesh.vertex_data.as_chunks_mut::<32>().0 {
            let f = |k: usize| f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap_or([0; 4]));
            let (p, n) = ([f(0), f(1), f(2)], [f(3), f(4), f(5)]);
            let d = normalize([cam[0] - p[0], cam[1] - p[1], cam[2] - p[2]]);
            let dn = 2.0 * (d[0] * n[0] + d[1] * n[1] + d[2] * n[2]);
            let r = normalize([dn * n[0] - d[0], dn * n[1] - d[1], dn * n[2] - d[2]]);
            // Bundle space mirrors Z: the original's r.z is −r.z here.
            let uv = [f32::midpoint(r[0], 1.0), f32::midpoint(1.0, -r[2])];
            v[24..28].copy_from_slice(&uv[0].to_le_bytes());
            v[28..32].copy_from_slice(&uv[1].to_le_bytes());
        }
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 { v.map(|x| x / l) } else { v }
}

/// A world point in the frame of an object whose column-major world matrix
/// is `m` (rotation and scale, then translation).
fn to_local(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    let d = [p[0] - m[3][0], p[1] - m[3][1], p[2] - m[3][2]];
    // Solve M3 · x = d with the 3×3 part (columns m[0..3]).
    let c = |j: usize| [m[j][0], m[j][1], m[j][2]];
    let (a, b, e) = (c(0), c(1), c(2));
    let cross = |u: [f32; 3], w: [f32; 3]| {
        [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ]
    };
    let dot = |u: [f32; 3], w: [f32; 3]| u[0] * w[0] + u[1] * w[1] + u[2] * w[2];
    let det = dot(a, cross(b, e));
    if det.abs() < 1e-12 {
        return d;
    }
    [
        dot(d, cross(b, e)) / det,
        dot(a, cross(d, e)) / det,
        dot(a, cross(b, d)) / det,
    ]
}

/// `true` if the file contains renderable geometry worth packing as a scene.
pub fn has_geometry(file: &CkFile) -> bool {
    file.objects
        .iter()
        .any(|o| o.class_id == ClassId::MESH && o.chunk.is_some())
}

/// Converts a file's scene. With `all_meshes`, meshes no object uses are kept
/// too (prefabs: their collision meshes are referenced only by scripts).
pub fn convert(
    file: &CkFile,
    textures: &dyn TextureResolver,
    all_meshes: bool,
) -> Result<Converted> {
    let decoded: Vec<Option<Decoded>> = file
        .objects
        .iter()
        .enumerate()
        .map(|(i, o)| {
            decode_object(o)
                .transpose()
                .with_context(|| format!("object #{i} {:?}", o.name))
        })
        .collect::<Result<_>>()?;

    let mut cx = Cx {
        file,
        decoded: &decoded,
        textures,
        meshes: Vec::new(),
        mesh_map: HashMap::new(),
        materials: Vec::new(),
        material_map: HashMap::new(),
        warnings: Vec::new(),
    };

    // Scene objects, in file order.
    let entity_map: BTreeMap<usize, u32> = decoded
        .iter()
        .enumerate()
        .filter(|(_, d)| matches!(d, Some(Decoded::Entity(_))))
        .enumerate()
        .map(|(n, (i, _))| (i, n as u32))
        .collect();

    let mut objects = Vec::with_capacity(entity_map.len());
    for &i in entity_map.keys() {
        let Some(Decoded::Entity(e)) = &decoded[i] else {
            unreachable!()
        };
        let kind = match file.objects[i].class_id {
            ClassId::LIGHT | ClassId::TARGET_LIGHT => bb::ObjectKind::Light,
            ClassId::CAMERA | ClassId::TARGET_CAMERA => bb::ObjectKind::Camera,
            ClassId::SPRITE_3D => bb::ObjectKind::Sprite,
            _ => bb::ObjectKind::Entity,
        };
        let sprite = match (kind, file.objects[i].chunk.as_ref()) {
            (bb::ObjectKind::Sprite, Some(chunk)) => {
                let s = ckscene::Sprite3d::decode(chunk)?;
                Some(bb::Sprite {
                    offset: s.offset,
                    uv: s.uv,
                    material: s.material.and_then(|m| cx.material(m)),
                })
            }
            _ => None,
        };
        let mesh = match e.current_mesh {
            Some(m) => cx.mesh(m),
            None => None,
        };
        objects.push(bb::SceneObject {
            name: name_of(file, i),
            kind,
            transform: convert_matrix(e),
            mesh,
            parent: e.parent.and_then(|p| entity_map.get(&p).copied()),
            hidden: e.hidden,
            sprite,
        });
    }

    if all_meshes {
        for (i, d) in decoded.iter().enumerate() {
            if matches!(d, Some(Decoded::Mesh(_))) {
                cx.mesh(i);
            }
        }
    }

    let mut groups = Vec::new();
    for (i, d) in decoded.iter().enumerate() {
        let Some(Decoded::Group(g)) = d else { continue };
        let members: Vec<u32> = g
            .members
            .iter()
            .filter_map(|m| entity_map.get(m).copied())
            .collect();
        // `All_*` groups are editor conveniences listing every object in the
        // file (scripts, materials, …); only their 3D members are kept, silently.
        let name = name_of(file, i);
        if members.len() != g.members.len() && !name.starts_with("All_") {
            cx.warnings.push(format!(
                "group {name:?}: {} non-3D member(s) dropped",
                g.members.len() - members.len()
            ));
        }
        groups.push(bb::Group { name, members });
    }

    Ok(Converted {
        scene: bb::Scene {
            meshes: cx.meshes,
            materials: cx.materials,
            objects,
            groups,
        },
        warnings: cx.warnings,
    })
}

struct Cx<'a> {
    file: &'a CkFile,
    decoded: &'a [Option<Decoded>],
    textures: &'a dyn TextureResolver,
    meshes: Vec<bb::Mesh>,
    mesh_map: HashMap<usize, Option<u32>>,
    materials: Vec<bb::Material>,
    material_map: HashMap<usize, Option<u32>>,
    warnings: Vec<String>,
}

impl Cx<'_> {
    fn mesh(&mut self, idx: usize) -> Option<u32> {
        if let Some(&m) = self.mesh_map.get(&idx) {
            return m;
        }
        let out = if let Some(Some(Decoded::Mesh(m))) = self.decoded.get(idx) {
            let mesh = self.convert_mesh(idx, m);
            self.meshes.push(mesh);
            Some(self.meshes.len() as u32 - 1)
        } else {
            self.warnings
                .push(format!("reference #{idx} is not a mesh; ignored"));
            None
        };
        self.mesh_map.insert(idx, out);
        out
    }

    fn convert_mesh(&mut self, idx: usize, m: &ckscene::Mesh) -> bb::Mesh {
        let normals = m.normals.clone().unwrap_or_else(|| m.generate_normals());
        let vertices: Vec<bb::Vertex> = m
            .positions
            .iter()
            .zip(&normals)
            .zip(&m.uvs)
            .map(|((p, n), uv)| bb::Vertex {
                position: mirror(*p),
                normal: mirror(*n),
                uv: *uv,
            })
            .collect();

        let mut bounds_min = [f32::MAX; 3];
        let mut bounds_max = [f32::MIN; 3];
        for v in &vertices {
            for k in 0..3 {
                bounds_min[k] = bounds_min[k].min(v.position[k]);
                bounds_max[k] = bounds_max[k].max(v.position[k]);
            }
        }
        if vertices.is_empty() {
            bounds_min = [0.0; 3];
            bounds_max = [0.0; 3];
        }

        // Group faces by material group (stable), reversing winding.
        let mut order: Vec<&ckscene::Face> = m.faces.iter().collect();
        order.sort_by_key(|f| f.group);
        let mut indices = Vec::with_capacity(order.len() * 3);
        let mut submeshes = Vec::new();
        let mut start = 0usize;
        while start < order.len() {
            let group = order[start].group;
            let end = start
                + order[start..]
                    .iter()
                    .take_while(|f| f.group == group)
                    .count();
            let first_index = indices.len() as u32;
            for f in &order[start..end] {
                let [a, b, c] = f.indices;
                indices.extend_from_slice(&[a, c, b]);
            }
            let material = match m.material_groups.get(usize::from(group)).copied().flatten() {
                Some(mat) => self.material(mat),
                None => None,
            };
            submeshes.push(bb::Submesh {
                first_index,
                index_count: indices.len() as u32 - first_index,
                material,
            });
            start = end;
        }

        bb::Mesh {
            name: name_of(self.file, idx),
            vertex_data: bytemuck_bytes(&vertices),
            index_data: indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
            submeshes,
            bounds_min,
            bounds_max,
            visible: m.is_visible(),
        }
    }

    fn material(&mut self, idx: usize) -> Option<u32> {
        if let Some(&m) = self.material_map.get(&idx) {
            return m;
        }
        let out = if let Some(Some(Decoded::Material(m))) = self.decoded.get(idx) {
            let mat = self.convert_material(idx, m);
            self.materials.push(mat);
            Some(self.materials.len() as u32 - 1)
        } else {
            self.warnings
                .push(format!("reference #{idx} is not a material; ignored"));
            None
        };
        self.material_map.insert(idx, out);
        out
    }

    fn convert_material(&mut self, idx: usize, m: &ckscene::Material) -> bb::Material {
        let name = name_of(self.file, idx);
        let texture = m.texture.and_then(|t| match self.decoded.get(t) {
            Some(Some(Decoded::Texture(tex))) => {
                let file = tex.file()?;
                let key = self.textures.resolve(file);
                if key.is_none() {
                    self.warnings.push(format!(
                        "material {name:?}: texture file {file:?} not found"
                    ));
                }
                key
            }
            _ => None,
        });
        let rgba = ckscene::argb_to_rgba;
        bb::Material {
            diffuse: rgba(m.diffuse),
            ambient: rgba(m.ambient),
            specular: rgba(m.specular),
            emissive: rgba(m.emissive),
            specular_power: m.specular_power,
            texture,
            texture_blend: texture_blend(m.texture_blend),
            address_mode: address_mode(m.address_mode),
            linear_filter: !matches!(m.mag_filter, ckscene::FilterMode::Nearest),
            src_blend: blend_factor(m.src_blend, bb::BlendFactor::One),
            dest_blend: blend_factor(m.dest_blend, bb::BlendFactor::Zero),
            blend: m.alpha_blend(),
            alpha_test: m
                .alpha_test()
                .then(|| (compare(m.alpha_func), f32::from(m.alpha_ref) / 255.0)),
            z_write: m.z_write(),
            z_func: compare(m.z_func),
            two_sided: m.two_sided(),
            flat_shading: m.shade_mode == ckscene::ShadeMode::Flat,
            wireframe: m.fill_mode == ckscene::FillMode::Wireframe,
            environment_map: m.effect.is_some(),
            name,
        }
    }
}

fn name_of(file: &CkFile, idx: usize) -> String {
    file.objects[idx].name.clone().unwrap_or_default()
}

fn mirror(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[1], -v[2]]
}

/// Row-vector left-handed matrix → column-major, column-vector right-handed
/// matrix. Mirroring is `S·M·S` with `S = diag(1, 1, -1, 1)`. The transpose
/// for column vectors is free: row `i` of the row-vector matrix becomes column
/// `i` of the column-major array.
fn convert_matrix(e: &Entity3d) -> [[f32; 4]; 4] {
    let s = [1.0, 1.0, -1.0, 1.0];
    let m = e.world_matrix;
    std::array::from_fn(|i| std::array::from_fn(|j| s[i] * m[i][j] * s[j]))
}

fn bytemuck_bytes(v: &[bb::Vertex]) -> Vec<u8> {
    v.iter()
        .flat_map(|v| {
            v.position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn texture_blend(t: ckscene::TextureBlend) -> bb::TextureBlend {
    use ckscene::TextureBlend as T;
    match t {
        T::Decal => bb::TextureBlend::Decal,
        T::DecalAlpha => bb::TextureBlend::DecalAlpha,
        T::ModulateAlpha => bb::TextureBlend::ModulateAlpha,
        T::DecalMask => bb::TextureBlend::DecalMask,
        T::ModulateMask => bb::TextureBlend::ModulateMask,
        T::Copy => bb::TextureBlend::Copy,
        T::Add => bb::TextureBlend::Add,
        T::Modulate | T::Unknown(_) => bb::TextureBlend::Modulate,
    }
}

fn address_mode(a: ckscene::AddressMode) -> bb::AddressMode {
    use ckscene::AddressMode as A;
    match a {
        A::Mirror => bb::AddressMode::Mirror,
        A::Clamp => bb::AddressMode::Clamp,
        A::Border => bb::AddressMode::Border,
        A::MirrorOnce => bb::AddressMode::MirrorOnce,
        A::Wrap | A::Unknown(_) => bb::AddressMode::Wrap,
    }
}

fn blend_factor(b: ckscene::BlendFactor, fallback: bb::BlendFactor) -> bb::BlendFactor {
    use ckscene::BlendFactor as B;
    match b {
        B::Zero => bb::BlendFactor::Zero,
        B::One => bb::BlendFactor::One,
        B::SrcColor => bb::BlendFactor::SrcColor,
        B::InvSrcColor => bb::BlendFactor::InvSrcColor,
        B::SrcAlpha => bb::BlendFactor::SrcAlpha,
        B::InvSrcAlpha => bb::BlendFactor::InvSrcAlpha,
        B::DestAlpha => bb::BlendFactor::DestAlpha,
        B::InvDestAlpha => bb::BlendFactor::InvDestAlpha,
        B::DestColor => bb::BlendFactor::DestColor,
        B::InvDestColor => bb::BlendFactor::InvDestColor,
        B::SrcAlphaSat => bb::BlendFactor::SrcAlphaSat,
        B::Unknown(_) => fallback,
    }
}

fn compare(c: ckscene::CompareFunc) -> bb::CompareFunc {
    use ckscene::CompareFunc as C;
    match c {
        C::Never => bb::CompareFunc::Never,
        C::Less => bb::CompareFunc::Less,
        C::Equal => bb::CompareFunc::Equal,
        C::LessEqual | C::Unknown(_) => bb::CompareFunc::LessEqual,
        C::Greater => bb::CompareFunc::Greater,
        C::NotEqual => bb::CompareFunc::NotEqual,
        C::GreaterEqual => bb::CompareFunc::GreaterEqual,
        C::Always => bb::CompareFunc::Always,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_mirroring_keeps_points_consistent() {
        // A rotation of 90° around Y plus a translation, in the original's
        // row-vector convention.
        let e = Entity3d {
            current_mesh: None,
            meshes: vec![],
            world_matrix: [
                [0.0, 0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [10.0, 2.0, 3.0, 1.0],
            ],
            flags: 0,
            parent: None,
            hidden: false,
        };
        let p = [1.0, 2.0, 3.0];
        // Expected: transform in the original space, then mirror.
        let expected = mirror(e.transform_point(p));
        // Actual: mirror, then apply the converted column-major matrix.
        let m = convert_matrix(&e);
        let q = mirror(p);
        let actual: [f32; 3] =
            std::array::from_fn(|r| (0..3).map(|c| m[c][r] * q[c]).sum::<f32>() + m[3][r]);
        for k in 0..3 {
            assert!(
                (expected[k] - actual[k]).abs() < 1e-5,
                "{expected:?} vs {actual:?}"
            );
        }
    }
}

#[cfg(test)]
mod reflection_tests {
    use super::*;

    fn vertex(p: [f32; 3], n: [f32; 3]) -> Vec<u8> {
        p.iter()
            .chain(n.iter())
            .chain([0.0f32, 0.0].iter())
            .flat_map(|x| x.to_le_bytes())
            .collect()
    }

    fn uv(scene: &bb::Scene, i: usize) -> [f32; 2] {
        let v = &scene.meshes[0].vertex_data[i * 32..i * 32 + 32];
        let f = |k: usize| f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap());
        [f(6), f(7)]
    }

    #[test]
    fn reflection_mapping_looks_from_the_camera() {
        let mut data = vertex([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        data.extend(vertex([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        let mut scene = bb::Scene {
            meshes: vec![bb::Mesh {
                name: "rail".into(),
                vertex_data: data,
                index_data: Vec::new(),
                submeshes: Vec::new(),
                bounds_min: [0.0; 3],
                bounds_max: [0.0; 3],
                visible: true,
            }],
            materials: Vec::new(),
            objects: vec![bb::SceneObject {
                name: "Rail".into(),
                kind: bb::ObjectKind::Entity,
                // At (0, -10, 0): the camera at the origin is straight above.
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, -10.0, 0.0, 1.0],
                ],
                mesh: Some(0),
                parent: None,
                hidden: false,
                sprite: None,
            }],
            groups: vec![bb::Group {
                name: "Phys_FloorRails".into(),
                members: vec![0],
            }],
        };
        reflection_map(&mut scene, "Phys_FloorRails", [0.0; 3]);
        // Facing the camera: reflected straight back up, the middle.
        assert_eq!(uv(&scene, 0), [0.5, 0.5]);
        // Facing +X with the camera above: reflected straight down
        // (r = (0, −1, 0)): x and z in the middle too.
        let [u, v] = uv(&scene, 1);
        assert!((u - 0.5).abs() < 1e-5 && (v - 0.5).abs() < 1e-5);
        // Moved to (−10, −10, 0): the camera is up and to +X, so the +X face
        // reflects it down and out: r = (0.71, −0.71, 0).
        scene.objects[0].transform[3] = [-10.0, -10.0, 0.0, 1.0];
        reflection_map(&mut scene, "Phys_FloorRails", [0.0; 3]);
        let [u, v] = uv(&scene, 1);
        assert!(
            (u - 0.8536).abs() < 1e-3 && (v - 0.5).abs() < 1e-5,
            "{u} {v}"
        );
    }
}
