//! Tests against a real Ballance install. These need `BALLANCE_DIR` to point at
//! the game's root directory (the one containing `base.cmo`) and are skipped
//! otherwise. No game data is stored in this repository.

use std::path::{Path, PathBuf};

use ckfile::{CkFile, ClassId, PluginCategory};

fn install_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BALLANCE_DIR")?);
    if dir.join("base.cmo").is_file() {
        Some(dir)
    } else {
        eprintln!("BALLANCE_DIR set but base.cmo not found there; skipping");
        None
    }
}

macro_rules! require_install {
    () => {
        match install_dir() {
            Some(d) => d,
            None => {
                eprintln!("skipping: BALLANCE_DIR not set");
                return;
            }
        }
    };
}

fn composition_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            composition_files(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("nmo") || e.eq_ignore_ascii_case("cmo"))
        {
            out.push(p);
        }
    }
}

#[test]
fn every_file_and_chunk_parses() {
    let dir = require_install!();
    let mut files = Vec::new();
    composition_files(&dir, &mut files);
    assert!(
        files.len() >= 50,
        "expected a full install, found {} files",
        files.len()
    );

    for path in &files {
        let f = CkFile::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(f.objects.len(), f.header.object_count as usize);
        for (i, o) in f.objects.iter().enumerate() {
            let Some(c) = &o.chunk else { continue };
            c.sections()
                .unwrap_or_else(|e| panic!("{} object #{i}: {e}", path.display()));
            for sub in c
                .sub_chunks()
                .unwrap_or_else(|e| panic!("{} object #{i}: {e}", path.display()))
            {
                sub.sections().unwrap();
            }
        }
    }
}

#[test]
fn level_one_structure() {
    let dir = require_install!();
    let f = CkFile::open(dir.join("3D Entities/Level/Level_01.NMO")).unwrap();
    assert_eq!(f.header.ck_version, 0x1302_2002);
    assert!(f.plugin_dependencies.iter().all(|d| d.guids.is_empty()));

    // Texture file names are stored as length-prefixed strings.
    let tex = f
        .objects
        .iter()
        .find(|o| o.class_id == ClassId::TEXTURE && o.name.as_deref() == Some("Floor_Top_Flat"))
        .expect("Floor_Top_Flat texture");
    let mut r = tex
        .chunk
        .as_ref()
        .unwrap()
        .section(0x10000)
        .unwrap()
        .unwrap();
    assert_eq!(r.u32().unwrap(), 1, "slot count");
    assert_eq!(r.string().unwrap().as_deref(), Some("Floor_Top_Flat.bmp"));

    // Group members are file indices of 3D objects.
    let group = f
        .objects
        .iter()
        .find(|o| o.class_id == ClassId::GROUP && o.name.as_deref() == Some("P_Extra_Point"))
        .expect("P_Extra_Point group");
    let mut r = group
        .chunk
        .as_ref()
        .unwrap()
        .section(0xFFFFF)
        .unwrap()
        .unwrap();
    let n = r.u32().unwrap();
    assert!(n > 0);
    for _ in 0..n {
        let member = f
            .object(r.object_ref().unwrap())
            .expect("valid member index");
        assert_eq!(member.class_id, ClassId::OBJECT_3D);
        assert!(
            member
                .name
                .as_deref()
                .unwrap()
                .starts_with("P_Extra_Point_")
        );
    }
}

#[test]
fn script_files_declare_behavior_dependencies() {
    let dir = require_install!();
    let f = CkFile::open(dir.join("3D Entities/Gameplay.nmo")).unwrap();
    assert!(f.behavior_guids().count() > 50);
    assert!(
        f.plugin_dependencies
            .iter()
            .any(|d| d.category == PluginCategory::Manager)
    );
    assert!(f.objects.iter().any(|o| o.class_id == ClassId::BEHAVIOR));
}
