//! Extracts physics data from the game's scripts and tables into the bundle.
//!
//! Sources (see the research repo, findings 09–11):
//! - `Balls.nmo` table `Physicalize_GameBall`: player ball types
//! - `Levelinit.nmo` tables `Physicalize_Floors`, `Physicalize_Convex`, `Physicalize_Balls`
//! - `Gameplay.nmo`: `Set Physics Globals` in `Init Ingame` (gravity, time factor),
//!   `physicalize new Ball` (which ball is a sphere, with what radius), and the
//!   `Trafo Manager` (ball transformers, in [`crate::transformers`])

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckfile::CkFile;
use ckscript::extract::{Extractor, Resolved, Value};
use ckscript::{Cell, DataArray, ScriptObject, Scripts};

use crate::install::Install;

pub(crate) struct Loaded {
    pub(crate) file: CkFile,
    pub(crate) scripts: Scripts,
}

pub(crate) fn load(install: &Install, key: &str) -> Result<Loaded> {
    let path = install.get(key).with_context(|| format!("{key} missing"))?;
    let file = CkFile::open(path).with_context(|| format!("reading {key}"))?;
    let scripts = Scripts::decode(&file).with_context(|| format!("decoding scripts of {key}"))?;
    Ok(Loaded { file, scripts })
}

impl Loaded {
    pub(crate) fn array(&self, name: &str) -> Result<&DataArray> {
        for (i, o) in self.scripts.objects.iter().enumerate() {
            if let Some(ScriptObject::Array(a)) = o
                && self.file.objects[i].name.as_deref() == Some(name)
            {
                return Ok(a);
            }
        }
        bail!("table {name} not found")
    }

    /// Typed view of one row.
    pub(crate) fn row<'a>(&'a self, a: &'a DataArray, r: &'a [Cell]) -> Row<'a> {
        Row {
            owner: self,
            array: a,
            cells: r,
        }
    }
}

pub(crate) struct Row<'a> {
    owner: &'a Loaded,
    array: &'a DataArray,
    cells: &'a [Cell],
}

impl Row<'_> {
    fn cell(&self, col: &str) -> Result<&Cell> {
        let i = self
            .array
            .column(col)
            .with_context(|| format!("column {col:?} missing"))?;
        Ok(&self.cells[i])
    }

    pub(crate) fn f32(&self, col: &str) -> Result<f32> {
        match self.cell(col)? {
            Cell::Float(v) => Ok(*v),
            Cell::Int(v) => Ok(*v as f32),
            // A Time (milliseconds as a float) or similar typed cell.
            Cell::Parameter(Some(p)) => match self.owner.scripts.objects.get(*p) {
                Some(Some(ScriptObject::Out(o))) => buffer_f32(&o.value),
                Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) => {
                    buffer_f32(&l.value)
                }
                _ => None,
            }
            .with_context(|| format!("column {col:?}: parameter cell not decodable")),
            c => bail!("column {col:?}: expected number, got {c:?}"),
        }
    }

    /// A colour or vector cell's components.
    pub(crate) fn floats(&self, col: &str) -> Result<Vec<f32>> {
        match self.cell(col)? {
            Cell::Float(v) => Ok(vec![*v]),
            Cell::Int(v) => Ok(vec![*v as f32]),
            Cell::Parameter(Some(p)) => match self.owner.scripts.objects.get(*p) {
                Some(Some(ScriptObject::Out(o))) => buffer_floats(&o.value),
                Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) => {
                    buffer_floats(&l.value)
                }
                _ => None,
            }
            .with_context(|| format!("column {col:?}: parameter cell not decodable")),
            c => bail!("column {col:?}: expected numbers, got {c:?}"),
        }
    }

    pub(crate) fn string(&self, col: &str) -> Result<String> {
        match self.cell(col)? {
            Cell::String(s) => Ok(s.clone()),
            c => bail!("column {col:?}: expected string, got {c:?}"),
        }
    }

    fn bool(&self, col: &str) -> Result<bool> {
        match self.cell(col)? {
            Cell::Int(v) => Ok(*v != 0),
            Cell::Parameter(Some(p)) => match self.owner.scripts.objects.get(*p) {
                Some(Some(ScriptObject::Out(o))) => Ok(
                    matches!(&o.value, ckscript::ParamValue::Buffer(b) if b.first().is_some_and(|&x| x != 0)),
                ),
                Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) => Ok(
                    matches!(&l.value, ckscript::ParamValue::Buffer(b) if b.first().is_some_and(|&x| x != 0)),
                ),
                _ => bail!("column {col:?}: parameter cell not decodable"),
            },
            c => bail!("column {col:?}: expected boolean, got {c:?}"),
        }
    }
}

fn constant(r: &Resolved) -> Option<&Value> {
    match r {
        Resolved::Constant { value } => Some(value),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
pub fn extract(install: &Install) -> Result<bb::PhysicsData> {
    let balls_nmo = load(install, "3d entities/balls.nmo")?;
    let levelinit = load(install, "3d entities/levelinit.nmo")?;
    let gameplay = load(install, "3d entities/gameplay.nmo")?;

    // World settings.
    let ex = Extractor::new(&gameplay.file, &gameplay.scripts);
    let nodes = ex.nodes(&|bb| bb.name == "Set Physics Globals");
    let globals = nodes
        .iter()
        .find(|n| n.path.iter().any(|p| p == "Init Ingame"))
        .context("Set Physics Globals in Init Ingame not found")?;
    let input = |name: &str| {
        globals
            .inputs
            .iter()
            .find(|i| i.name == name)
            .and_then(|i| constant(&i.value))
            .cloned()
    };
    let gravity = match input("Gravity") {
        // Mirror Z into bundle space (adding 0.0 turns -0.0 into 0.0).
        Some(Value::Vector(v)) if v.len() == 3 => [v[0] as f32, v[1] as f32, 0.0 - v[2] as f32],
        other => bail!("unexpected gravity {other:?}"),
    };
    let time_factor = match input("Physic Time Factor") {
        Some(Value::Float(f)) => f as f32,
        other => bail!("unexpected time factor {other:?}"),
    };

    // Ball shapes: which Physicalize variant each ball name is routed to.
    let shapes = ball_shapes(&gameplay, &ex)?;

    let mut balls = Vec::new();
    let table = balls_nmo.array("Physicalize_GameBall")?;
    for cells in &table.rows {
        let row = balls_nmo.row(table, cells);
        let name = row.string("Ballname")?;
        let shape = match shapes.iter().find(|(n, _)| *n == name).map(|(_, r)| *r) {
            Some(Some(radius)) => bb::BallShape::Sphere { radius },
            Some(None) => bb::BallShape::Convex {
                radius: mesh_radius(&balls_nmo.file, &name)?,
            },
            None => bail!("no Physicalize variant found for ball {name}"),
        };
        balls.push(bb::BallParams {
            friction: row.f32("Friction")?,
            elasticity: row.f32("Elasticity")?,
            mass: row.f32("Mass")?,
            linear_damping: row.f32("Linear Damp")?,
            rotational_damping: row.f32("Rot Damp")?,
            force: row.f32("Force")?,
            collision_group: row.string("CollGroup")?,
            shape,
            name,
        });
    }

    let mut surfaces = Vec::new();
    let table = levelinit.array("Physicalize_Floors")?;
    for cells in &table.rows {
        let row = levelinit.row(table, cells);
        surfaces.push(bb::SurfaceParams {
            group: row.string("Group_Name")?,
            friction: row.f32("Friction")?,
            elasticity: row.f32("Elasticity")?,
            collision_group: row.string("Col Group")?,
            collision: row.bool("Enable Col?")?,
        });
    }

    let mut props = Vec::new();
    for (table_name, spheres) in [("Physicalize_Convex", false), ("Physicalize_Balls", true)] {
        let table = levelinit.array(table_name)?;
        for cells in &table.rows {
            let row = levelinit.row(table, cells);
            props.push(bb::PropParams {
                group: row.string("Group Name")?,
                fixed: row.bool("Fixed?")?,
                friction: row.f32("Friction")?,
                elasticity: row.f32("Elasticity")?,
                mass: row.f32("Mass")?,
                collision_group: row.string("Col Group")?,
                start_frozen: row.bool("Frozen?")?,
                collision: row.bool("Enable Col?")?,
                auto_mass_center: row.bool("autm. Mass Center?")?,
                linear_damping: row.f32("Linear Damp")?,
                rotational_damping: row.f32("Rot Damp")?,
                sphere_radius: if spheres {
                    Some(row.f32("Radius")?)
                } else {
                    None
                },
            });
        }
    }

    Ok(bb::PhysicsData {
        gravity,
        time_factor,
        surfaces,
        props,
        modules: crate::modules::extract(install)?,
        transformers: crate::transformers::extract(
            install,
            &gameplay.file,
            &gameplay.scripts,
            &balls,
        )?,
        sounds: crate::ballsounds::extract(install, &balls)?,
        energy: crate::energy::extract(install)?,
        flow: crate::flow::extract(install)?,
        hud: crate::hud::extract(install)?,
        environment: crate::environment::extract(install)?,
        particles: crate::particles::extract(install)?,
        level_end: crate::score::level_end(install)?,
        score_screen: crate::score::score_screen(install)?,
        highscores: crate::highscores::extract(install)?,
        menus: crate::menus::extract(install)?,
        balls,
    })
}

/// `(ball name, Some(radius) for a sphere | None for convex)` from the
/// `Switch On Parameter` in `physicalize new Ball` and the Physicalize nodes it
/// routes to.
fn ball_shapes(g: &Loaded, ex: &Extractor<'_>) -> Result<Vec<(String, Option<f32>)>> {
    let s = &g.scripts;
    let name = |i: usize| g.file.objects[i].name.clone().unwrap_or_default();
    let bb_name = |i: usize| {
        s.behavior(i)
            .and_then(|b| b.prototype)
            .and_then(|p| ckscript::Catalogue::builtin().building_block(p.guid))
            .map(|b| b.name.clone())
    };
    let graph = (0..s.objects.len())
        .find(|&i| {
            s.behavior(i).is_some_and(ckscript::Behavior::is_graph)
                && name(i) == "physicalize new Ball"
        })
        .context("graph 'physicalize new Ball' not found")?;
    let gb = s.behavior(graph).unwrap();
    let switch = gb
        .sub_behaviors
        .iter()
        .copied()
        .find(|&c| bb_name(c).as_deref() == Some("Switch On Parameter"))
        .context("Switch On Parameter not found")?;
    let sw = s.behavior(switch).unwrap();

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    // Input params after "Test" are "Pin 1..n"; output pins 1..n correspond (0 = None).
    for (k, &pin_param) in sw.input_params.iter().enumerate().skip(1) {
        let Some(Value::Text(ball)) =
            constant(&ex.resolve_input(pin_param, &mut HashSet::new())).cloned()
        else {
            continue;
        };
        let Some(&out_pin) = sw.outputs.get(k) else {
            continue;
        };
        // Follow the link from this output to a Physicalize node.
        let target = gb.links.iter().find_map(|&l| match s.objects.get(l) {
            Some(Some(ScriptObject::Link(link))) if link.source == Some(out_pin) => {
                link.target.and_then(|t| s.owner.get(&t).copied())
            }
            _ => None,
        });
        let Some(node) = target.filter(|&n| bb_name(n).as_deref() == Some("Physicalize")) else {
            continue;
        };
        let radius = s
            .behavior(node)
            .unwrap()
            .input_params
            .iter()
            .find_map(|&p| {
                (name(p) == "ball radius 1")
                    .then(|| constant(&ex.resolve_input(p, &mut HashSet::new())).cloned())
                    .flatten()
                    .and_then(|v| match v {
                        Value::Float(f) => Some(f as f32),
                        _ => None,
                    })
            });
        if seen.insert(ball.clone()) {
            out.push((ball, radius));
        }
    }
    Ok(out)
}

/// Bounding radius (from the object's origin) of a ball's mesh.
fn mesh_radius(file: &CkFile, object: &str) -> Result<f32> {
    let obj = file
        .objects
        .iter()
        .find(|o| o.name.as_deref() == Some(object) && o.class_id == ckfile::ClassId::OBJECT_3D)
        .with_context(|| format!("ball object {object} not found"))?;
    let e = ckscene::Entity3d::decode(obj.chunk.as_ref().context("no chunk")?)?;
    let m = e.current_mesh.context("ball has no mesh")?;
    let mesh = ckscene::Mesh::decode(file.objects[m].chunk.as_ref().context("no mesh chunk")?)?;
    Ok(mesh
        .positions
        .iter()
        .map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt())
        .fold(0.0, f32::max))
}

fn buffer_floats(v: &ckscript::ParamValue) -> Option<Vec<f32>> {
    match v {
        ckscript::ParamValue::Buffer(b) => Some(
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect(),
        ),
        _ => None,
    }
}

fn buffer_f32(v: &ckscript::ParamValue) -> Option<f32> {
    match v {
        ckscript::ParamValue::Buffer(b) => b.first_chunk::<4>().map(|c| f32::from_le_bytes(*c)),
        _ => None,
    }
}
