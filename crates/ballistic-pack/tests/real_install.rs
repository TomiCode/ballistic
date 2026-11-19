//! End-to-end: pack a real install (`BALLANCE_DIR`) and read the bundle back.
//! Skipped when the variable isn't set.

use std::path::PathBuf;

use ballistic_bundle::{AudioEncoding, BundleReader, TextureEncoding};
use ballistic_pack::pack;
use ckscene::{Decoded, decode_object};

fn install_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BALLANCE_DIR")?);
    dir.join("base.cmo").is_file().then_some(dir)
}

#[test]
fn pack_and_read_back() {
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    let out = std::env::temp_dir().join(format!("ballistic-test-{}.bpk", std::process::id()));
    let report = pack::build(&pack::Options {
        from: Some(dir.clone()),
        out: out.clone(),
        texture_encoding: TextureEncoding::Bc,
        audio_encoding: AudioEncoding::Vorbis,
        audio_quality: 0.4,
    })
    .unwrap();
    assert_eq!(report.manifest.source.variant, "gog");

    let mut r = BundleReader::open(&out).unwrap();
    let m = r.manifest().clone();
    assert!(m.scenes.iter().any(|s| s.key == "level/level_12"));
    assert!(m.textures.iter().any(|t| t.key == "sky/sky_a_back"));
    assert!(m.sounds.iter().any(|s| s.key == "misc_checkpoint"));

    // Every texture is a valid KTX2 with the advertised mip chain.
    for t in &m.textures {
        let bytes = r.texture(&t.key).unwrap();
        let k = ktx2::Reader::new(&bytes[..]).unwrap();
        assert_eq!(k.header().level_count, t.mip_levels, "{}", t.key);
        assert_eq!(k.header().pixel_width, t.width);
    }
    // Every sound is Ogg.
    for s in &m.sounds {
        assert_eq!(&r.sound(&s.key).unwrap()[..4], b"OggS", "{}", s.key);
    }

    // World-space geometry of Level 1 equals the original's, mirrored in Z.
    let scene = r.scene("level/level_01").unwrap();
    let ck = ckfile::CkFile::open(dir.join("3D Entities/Level/Level_01.NMO")).unwrap();
    let decoded: Vec<Option<Decoded>> = ck
        .objects
        .iter()
        .map(|o| decode_object(o).transpose().unwrap())
        .collect();
    let entities: Vec<&ckscene::Entity3d> = decoded
        .iter()
        .filter_map(|d| match d {
            Some(Decoded::Entity(e)) => Some(e),
            _ => None,
        })
        .collect();
    assert_eq!(entities.len(), scene.objects.len());

    let mut checked = 0;
    for (obj, e) in scene.objects.iter().zip(&entities) {
        let (Some(mi), Some(ck_mesh)) = (obj.mesh, e.current_mesh) else {
            continue;
        };
        let Some(Decoded::Mesh(src)) = &decoded[ck_mesh] else {
            panic!("not a mesh")
        };
        let verts = scene.meshes[mi as usize].vertices();
        assert_eq!(verts.len(), src.positions.len());
        let t = obj.transform;
        for (v, p) in verts.iter().zip(&src.positions) {
            let q = v.position;
            let world: [f32; 3] =
                std::array::from_fn(|r| (0..3).map(|c| t[c][r] * q[c]).sum::<f32>() + t[3][r]);
            let orig = e.transform_point(*p);
            let expected = [orig[0], orig[1], -orig[2]];
            for k in 0..3 {
                assert!(
                    (world[k] - expected[k]).abs() < 1e-3,
                    "{}: {world:?} vs {expected:?}",
                    obj.name
                );
            }
            checked += 1;
        }
        // Index count matches face count, all faces assigned to submeshes.
        let mesh = &scene.meshes[mi as usize];
        let covered: u32 = mesh.submeshes.iter().map(|s| s.index_count).sum();
        assert_eq!(covered as usize, src.faces.len() * 3);
        assert_eq!(mesh.indices().len(), src.faces.len() * 3);
    }
    assert!(checked > 10_000, "only {checked} vertices checked");

    // Groups survived with their members.
    let floors = scene
        .groups
        .iter()
        .find(|g| g.name == "Phys_Floors")
        .unwrap();
    assert_ne!(floors.members.len(), 0);

    drop(r);
    let _ = std::fs::remove_file(out);
}
