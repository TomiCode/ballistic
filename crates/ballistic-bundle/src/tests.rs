use std::io::Cursor;

use crate::*;

fn sample_scene() -> Scene {
    let verts = [
        Vertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [1.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [0.0, 0.0, -1.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 1.0],
        },
    ];
    Scene {
        meshes: vec![Mesh {
            name: "tri".into(),
            vertex_data: bytemuck::cast_slice(&verts).to_vec(),
            index_data: bytemuck::cast_slice(&[0u16, 2, 1]).to_vec(),
            submeshes: vec![Submesh {
                first_index: 0,
                index_count: 3,
                material: None,
            }],
            bounds_min: [0.0, 0.0, -1.0],
            bounds_max: [1.0, 0.0, 0.0],
        }],
        materials: vec![],
        objects: vec![SceneObject {
            name: "obj".into(),
            kind: ObjectKind::Entity,
            transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [5.0, 0.0, 0.0, 1.0],
            ],
            mesh: Some(0),
            parent: None,
            hidden: false,
        }],
        groups: vec![Group {
            name: "Phys_Floors".into(),
            members: vec![0],
        }],
    }
}

fn manifest() -> Manifest {
    Manifest {
        format: FORMAT_NAME.into(),
        format_version: FORMAT_VERSION,
        packer_version: "test".into(),
        source: SourceInfo {
            variant: "unknown".into(),
            verified_files: 0,
            modified_files: 0,
            unknown_files: 0,
        },
        options: PackOptions {
            texture_encoding: TextureEncoding::Bc,
            audio_encoding: AudioEncoding::Vorbis,
            audio_quality: 0.6,
        },
        scenes: vec![SceneEntry {
            key: "test".into(),
            objects: 1,
            meshes: 1,
            materials: 0,
        }],
        textures: vec![],
        sounds: vec![],
    }
}

fn build() -> Vec<u8> {
    let mut w = BundleWriter::new(Cursor::new(Vec::new()));
    w.add_scene("test", &sample_scene()).unwrap();
    w.add("textures/x.ktx2", b"not really ktx", true).unwrap();
    w.finish(&manifest()).unwrap().into_inner()
}

#[test]
fn round_trip() {
    let bytes = build();
    let mut r = BundleReader::new(Cursor::new(bytes)).unwrap();
    assert_eq!(r.manifest(), &manifest());
    let scene = r.scene("test").unwrap();
    assert_eq!(scene, sample_scene());
    assert_eq!(scene.meshes[0].indices(), vec![0, 2, 1]);
    assert_eq!(scene.meshes[0].vertices()[1].uv, [1.0, 0.0]);
    assert_eq!(r.texture("x").unwrap(), b"not really ktx");
}

#[test]
fn output_is_reproducible() {
    assert_eq!(build(), build());
}

#[test]
fn rejects_other_versions() {
    let mut m = manifest();
    m.format_version = 999;
    let w = BundleWriter::new(Cursor::new(Vec::new()));
    let bytes = w.finish(&m).unwrap().into_inner();
    let err = BundleReader::new(Cursor::new(bytes)).err().unwrap();
    assert!(matches!(
        err,
        Error::Unsupported {
            found_version: 999,
            ..
        }
    ));
}
