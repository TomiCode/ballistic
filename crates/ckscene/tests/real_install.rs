//! Decoding tests against a real install (`BALLANCE_DIR`); skipped otherwise.

use std::path::PathBuf;

use ckfile::{CkFile, ClassId};
use ckscene::{Decoded, Entity3d, Group, Mesh, decode_object};

fn install_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BALLANCE_DIR")?);
    dir.join("base.cmo").is_file().then_some(dir)
}

fn level(dir: &std::path::Path, n: u32) -> CkFile {
    CkFile::open(dir.join(format!("3D Entities/Level/Level_{n:02}.NMO"))).unwrap()
}

#[test]
fn all_levels_decode_and_reference_valid_objects() {
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    for n in 1..=12 {
        let f = level(&dir, n);
        for (i, o) in f.objects.iter().enumerate() {
            let Some(d) = decode_object(o) else { continue };
            let d = d.unwrap_or_else(|e| panic!("level {n} object #{i}: {e}"));
            let class_of = |r: usize| f.objects[r].class_id;
            match d {
                Decoded::Entity(e) => {
                    if let Some(m) = e.current_mesh {
                        assert_eq!(class_of(m), ClassId::MESH);
                    }
                }
                Decoded::Mesh(m) => {
                    for g in m.material_groups.iter().flatten() {
                        assert_eq!(class_of(*g), ClassId::MATERIAL);
                    }
                }
                Decoded::Material(m) => {
                    if let Some(t) = m.texture {
                        assert_eq!(class_of(t), ClassId::TEXTURE);
                    }
                }
                Decoded::Texture(t) => {
                    assert!(t.file().is_some(), "level texture without file");
                }
                Decoded::Entity2d(e) => {
                    if let Some(m) = e.material {
                        assert_eq!(class_of(m), ClassId::MATERIAL);
                    }
                }
                Decoded::Group(_) => {}
            }
        }
    }
}

#[test]
fn hud_entities_decode() {
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    let f = ckfile::CkFile::open(dir.join("3D Entities").join("Camera.nmo")).unwrap();
    let find = |name: &str| {
        let i = f
            .objects
            .iter()
            .position(|o| o.name.as_deref() == Some(name))
            .unwrap();
        match decode_object(&f.objects[i]).unwrap().unwrap() {
            Decoded::Entity2d(e) => e,
            other => panic!("{name}: {other:?}"),
        }
    };
    let ball = find("Interface_Life_Kugel");
    assert!((ball.rect[0] - 0.9496).abs() < 1e-4, "{ball:?}");
    assert_eq!(ball.z_order, 9);
    assert!(ball.hidden);
    let m = ball.material.unwrap();
    assert_eq!(f.objects[m].name.as_deref(), Some("Interface_Life"));
    let digits = find("Interface_Points_digits");
    assert_eq!(digits.material, None);
    assert_eq!(digits.source, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(digits.z_order, 15);
}

#[test]
fn level_one_floors_are_placed_in_world_space() {
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    let f = level(&dir, 1);
    let group = f
        .objects
        .iter()
        .find(|o| o.name.as_deref() == Some("Phys_Floors"))
        .unwrap();
    let floors = Group::decode(group.chunk.as_ref().unwrap()).unwrap();
    assert!(floors.members.len() >= 5);

    for &m in &floors.members {
        let e = Entity3d::decode(f.objects[m].chunk.as_ref().unwrap()).unwrap();
        let mesh =
            Mesh::decode(f.objects[e.current_mesh.unwrap()].chunk.as_ref().unwrap()).unwrap();
        assert_ne!(mesh.faces.len(), 0);
        // Floors are not centred at the origin: the transform must move them.
        let w = e.transform_point(mesh.positions[0]);
        assert!(w.iter().any(|c| c.abs() > 1.0));
        assert!(w.iter().all(|c| c.is_finite() && c.abs() < 10_000.0));
    }
}
