//! Unit tests on synthetic chunks laid out like the game's files.

use ckfile::{ClassId, StateChunk};

use crate::*;

/// Builds chunk data from `(identifier, payload)` sections.
fn chunk(class: ClassId, sections: &[(u32, Vec<u32>)]) -> StateChunk {
    let mut data = Vec::new();
    for (i, (id, payload)) in sections.iter().enumerate() {
        let next = if i + 1 == sections.len() {
            0
        } else {
            (data.len() + 2 + payload.len()) as u32
        };
        data.push(*id);
        data.push(next);
        data.extend_from_slice(payload);
    }
    StateChunk {
        class_id: class,
        data_version: 10,
        chunk_version: 7,
        options: ckfile::chunk_options::FILE,
        data,
        object_positions: vec![],
        sub_chunk_positions: vec![],
        manager_positions: vec![],
    }
}

fn f(v: f32) -> u32 {
    v.to_bits()
}

#[test]
fn entity_with_parent_and_hidden_marker() {
    let mut t = vec![0x2_0000, 0x42_000B];
    t.extend(
        [
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0,
        ]
        .map(f),
    );
    t.push(7); // parent
    let c = chunk(
        ClassId::ENTITY_3D,
        &[(0x4, vec![]), (0x4000, vec![3, 1, 3]), (0x10_0000, t)],
    );
    let e = Entity3d::decode(&c).unwrap();
    assert!(e.hidden);
    assert_eq!(e.current_mesh, Some(3));
    assert_eq!(e.meshes, vec![3]);
    assert_eq!(e.parent, Some(7));
    assert_eq!(e.transform_point([1.0, 2.0, 3.0]), [11.0, 22.0, 33.0]);
}

#[test]
fn material_packed_modes() {
    let c = chunk(
        ClassId::MATERIAL,
        &[(
            0x1000,
            vec![
                0xFFFF_FFFF,
                0xFF19_1919,
                0xFFE5_E5E5,
                0xFF00_0000,
                f(25.0),
                u32::MAX,
                0,
                0x1326_5224,
                0x0105_041E,
            ],
        )],
    );
    let m = Material::decode(&c).unwrap();
    assert_eq!(m.texture, None);
    assert_eq!(m.texture_blend, TextureBlend::ModulateAlpha);
    assert_eq!(m.min_filter, FilterMode::Linear);
    assert_eq!(m.src_blend, BlendFactor::SrcAlpha);
    assert_eq!(m.dest_blend, BlendFactor::InvSrcAlpha);
    assert_eq!(m.shade_mode, ShadeMode::Gouraud);
    assert_eq!(m.fill_mode, FillMode::Solid);
    assert_eq!(m.address_mode, AddressMode::Wrap);
    assert_eq!(m.z_func, CompareFunc::LessEqual);
    assert_eq!(m.alpha_func, CompareFunc::Greater);
    assert_eq!(m.alpha_ref, 1);
    assert!(m.alpha_test() && m.alpha_blend() && m.z_write() && !m.two_sided());
    assert_eq!(argb_to_rgba(m.ambient)[0], 25.0 / 255.0);
}

#[test]
fn mesh_with_shared_colours_and_normals() {
    // One triangle, normals present, single diffuse/specular (flags = 0x3).
    let mut v = vec![3, 0x3, 0];
    v.extend([0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0].map(f)); // positions
    v.extend([0xFFFF_FFFF, 0]); // diffuse, specular
    v.extend([0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0].map(f)); // normals
    v.extend([0.0, 0.0, 1.0, 0.0, 0.0, 1.0].map(f)); // uvs
    let c = chunk(
        ClassId::MESH,
        &[
            (0x2000, vec![0x3_C00A]),
            (0x10_0000, vec![1, 5, 0]),
            (0x1_0000, vec![1, 0x0001_0000, 0x0000_0002]),
            (0x2_0000, v),
        ],
    );
    let m = Mesh::decode(&c).unwrap();
    assert_eq!(m.positions.len(), 3);
    assert_eq!(m.diffuse, vec![0xFFFF_FFFF; 3]);
    assert_eq!(
        m.faces,
        vec![Face {
            indices: [0, 1, 2],
            group: 0
        }]
    );
    assert_eq!(m.material_groups, vec![Some(5)]);
    assert_eq!(m.uvs[1], [1.0, 0.0]);
    let n = m.generate_normals();
    assert!((n[0][1].abs() - 1.0).abs() < 1e-6);
}

#[test]
fn mesh_rejects_out_of_range_face() {
    let mut v = vec![1, 0xF, 0];
    v.extend([0.0, 0.0, 0.0].map(f));
    v.extend([0, 0, f(0.0), f(0.0)]);
    let c = chunk(
        ClassId::MESH,
        &[
            (0x10_0000, vec![0]),
            (0x1_0000, vec![1, 0x0001_0000, 0x0000_0002]),
            (0x2_0000, v),
        ],
    );
    assert!(Mesh::decode(&c).is_err());
}

#[test]
fn texture_file_name() {
    let mut files = vec![1, 19];
    let mut name = b"Floor_Top_Flat.bmp\0".to_vec();
    name.resize(20, 0);
    files.extend(
        name.chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])),
    );
    let c = chunk(
        ClassId::TEXTURE,
        &[(0x1_0000, files), (0x2F_F000, vec![0x3_0200, 0, 6])],
    );
    let t = Texture::decode(&c).unwrap();
    assert_eq!(t.file(), Some("Floor_Top_Flat.bmp"));
    assert_eq!(t.video_format, Some(6));
}
