//! Script decoding against a real install (`BALLANCE_DIR`); skipped otherwise.

use std::path::{Path, PathBuf};

use ckfile::CkFile;
use ckscript::describe::Describer;
use ckscript::{ScriptObject, Scripts};

fn install_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BALLANCE_DIR")?);
    dir.join("base.cmo").is_file().then_some(dir)
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
fn every_script_decodes_and_validates() {
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    let mut files = Vec::new();
    composition_files(&dir, &mut files);
    let mut behaviors = 0;
    for p in &files {
        let f = CkFile::open(p).unwrap();
        let s = Scripts::decode(&f).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let problems = s.validate(&f);
        assert!(problems.is_empty(), "{}: {problems:#?}", p.display());
        behaviors += s
            .objects
            .iter()
            .filter(|o| matches!(o, Some(ScriptObject::Behavior(_))))
            .count();
        assert!(
            s.roots.iter().all(|(_, owner)| owner.is_some()),
            "{}",
            p.display()
        );
    }
    // The census from the original survey.
    assert_eq!(behaviors, 4469);
}

#[test]
fn every_building_block_is_in_the_catalogue() {
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    let mut files = Vec::new();
    composition_files(&dir, &mut files);
    let mut c = ckscript::conform::Conformance::default();
    for p in &files {
        let f = CkFile::open(p).unwrap();
        let s = Scripts::decode(&f).unwrap();
        ckscript::conform::check(&f, &s, ckscript::Catalogue::builtin(), &mut c);
    }
    assert_eq!(c.nodes, 3885);
    assert_eq!(c.unknown, 0);
    // Most nodes match their declaration exactly; the rest differ by
    // variable pins/parameters or retyped generic parameters.
    assert!(c.exact > 2200, "{} exact", c.exact);
}

#[test]
fn slider_module_physics_parameters() {
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    let f = CkFile::open(dir.join("3D Entities/PH/P_Modul_34.nmo")).unwrap();
    let s = Scripts::decode(&f).unwrap();
    let d = Describer {
        file: &f,
        scripts: &s,
    };
    let root = s
        .roots
        .iter()
        .find(|(r, _)| f.objects[*r].name.as_deref() == Some("P_Modul_34_MF Script"))
        .unwrap()
        .0;
    let node = d.node(root).unwrap();
    let physicalize_nodes: Vec<_> = node
        .children
        .iter()
        .filter(|c| c.building_block.as_deref() == Some("Physicalize"))
        .collect();
    assert_eq!(physicalize_nodes.len(), 2);
    let get = |n: &ckscript::describe::Node, p: &str| {
        n.params_in
            .iter()
            .find(|x| x.name == p)
            .map(|x| x.value.split_whitespace().next().unwrap().to_owned())
            .unwrap()
    };
    let mut frictions: Vec<String> = physicalize_nodes
        .iter()
        .map(|n| get(n, "Friction"))
        .collect();
    frictions.sort();
    assert_eq!(frictions, vec!["0.5", "0.8"]);
    assert!(
        physicalize_nodes
            .iter()
            .all(|n| get(n, "Linear Speed Dampening") == "0.1")
    );
    assert!(node.links.iter().any(|l| l.from == "this.Start"));
}

#[test]
fn physics_tables_and_globals() {
    use ckscript::extract::{Extractor, Resolved, Value};
    use ckscript::{Cell, DataArray};
    let Some(dir) = install_dir() else {
        eprintln!("skipping: BALLANCE_DIR not set");
        return;
    };
    // Ball table.
    let f = CkFile::open(dir.join("3D Entities/Balls.nmo")).unwrap();
    let arr = f
        .objects
        .iter()
        .find(|o| o.name.as_deref() == Some("Physicalize_GameBall"))
        .unwrap();
    let a = DataArray::decode(arr.chunk.as_ref().unwrap()).unwrap();
    let force = a.column("Force").unwrap();
    let mass = a.column("Mass").unwrap();
    let wood = a.row_by_key("Ball_Wood").unwrap();
    assert_eq!(wood[force], Cell::Float(0.43));
    assert_eq!(wood[mass], Cell::Float(1.9));
    assert_eq!(a.row_by_key("Ball_Stone").unwrap()[mass], Cell::Float(10.0));

    // Gravity and the ball's force traced back to the table.
    let f = CkFile::open(dir.join("3D Entities/Gameplay.nmo")).unwrap();
    let s = Scripts::decode(&f).unwrap();
    let ex = Extractor::new(&f, &s);
    let nodes = ex.nodes(&|bb| bb.dll == "physics_RT");
    let globals = nodes
        .iter()
        .find(|n| {
            n.building_block == "Set Physics Globals" && n.path.iter().any(|p| p == "Init Ingame")
        })
        .unwrap();
    assert_eq!(
        globals.inputs[0].value,
        Resolved::Constant {
            value: Value::Vector(vec![0.0, -20.0, 0.0])
        }
    );
    let force_nodes: Vec<_> = nodes
        .iter()
        .filter(|n| {
            n.building_block == "Physics Force" && n.path.iter().any(|p| p == "Ball Navigation")
        })
        .collect();
    assert_eq!(force_nodes.len(), 5);
    let from_table = force_nodes
        .iter()
        .filter(|n| {
            n.inputs.iter().any(|i| {
                i.name == "Force Value"
                    && matches!(&i.value, Resolved::Dynamic { from, .. } if from.contains("Physicalize_GameBall"))
            })
        })
        .count();
    assert_eq!(from_table, 4);
}
