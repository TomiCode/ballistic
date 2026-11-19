//! Extracts the moving elements' physics (`3D Entities/PH/P_Modul_*.nmo`)
//! from their prefab scripts: the Physicalize nodes (bodies), Set Physics
//! Hinge nodes, and the proximity test that wakes frozen bodies. Other
//! physics building blocks are listed as unsupported (research findings 09, 13).

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckfile::CkFile;
use ckscript::Scripts;
use ckscript::extract::{Extractor, NamedValue, NodeParams, Resolved, Value};

use crate::install::Install;

/// Building blocks a module script may use, and whether they're extracted.
const SUPPORTED: [&str; 10] = [
    "Box Box Intersection",
    "Physicalize",
    "Physics Hinge",
    "Set Physics Slider",
    "Set Physics Spring",
    "Physics Balljoint",
    "Physics Force",
    "Delayer",
    "Physics WakeUp",
    "TT Scaleable Proximity",
];
/// Every other `physics_RT` building block counts as unsupported.
const PHYSICS_DLL: &str = "physics_RT";

pub fn extract(install: &Install) -> Result<Vec<bb::ModuleSetup>> {
    let mut out = Vec::new();
    for (key, path) in install.under("3d entities/ph/") {
        let Some(stem) = key
            .strip_prefix("3d entities/ph/")
            .and_then(|k| k.strip_suffix(".nmo"))
        else {
            continue;
        };
        if !stem.starts_with("p_modul_") {
            continue;
        }
        let file = CkFile::open(path).with_context(|| format!("reading {key}"))?;
        let scripts =
            Scripts::decode(&file).with_context(|| format!("decoding scripts of {key}"))?;
        let ex = Extractor::new(&file, &scripts);
        let nodes = ex.nodes(&|b| {
            SUPPORTED.contains(&b.name.as_str()) || b.dll.eq_ignore_ascii_case(PHYSICS_DLL)
        });
        let group = file
            .objects
            .iter()
            .find_map(|o| {
                o.name
                    .as_deref()
                    .and_then(|n| n.strip_prefix("All_"))
                    .filter(|n| n.eq_ignore_ascii_case(stem))
                    .map(str::to_owned)
            })
            .with_context(|| format!("{key}: no All_<group> group"))?;
        out.push(setup(group, &nodes).with_context(|| format!("extracting {key}"))?);
    }
    Ok(out)
}

fn setup(group: String, nodes: &[NodeParams]) -> Result<bb::ModuleSetup> {
    let mut bodies = Vec::new();
    let mut hinges = Vec::new();
    let mut sliders = Vec::new();
    let mut springs = Vec::new();
    let mut ball_joints = Vec::new();
    let mut forces = Vec::new();
    let mut delays_ms = Vec::new();
    let mut proximities = Vec::new();
    let mut boxes = Vec::new();
    let mut wake = None;
    let wake_bodies: Vec<String> = nodes
        .iter()
        .filter(|n| n.building_block == "Physics WakeUp")
        .filter_map(target)
        .collect();
    let mut unsupported = BTreeSet::new();
    let has_wakeup = nodes.iter().any(|n| n.building_block == "Physics WakeUp");
    for n in nodes {
        match n.building_block.as_str() {
            "Physicalize" => {
                if let Some(b) = body(n)? {
                    bodies.push(b);
                } else {
                    unsupported.insert("Physicalize (computed target or shape)".to_owned());
                }
            }
            "Physics Hinge" => match hinge(n) {
                Some(h) => hinges.push(h),
                None => {
                    unsupported.insert("Physics Hinge (computed objects)".to_owned());
                }
            },
            "Set Physics Slider" => match slider(n) {
                Some(s) => sliders.push(s),
                None => {
                    unsupported.insert("Set Physics Slider (computed objects)".to_owned());
                }
            },
            "Physics Balljoint" => match ball_joint(n) {
                Some(j) => ball_joints.push(j),
                None => {
                    unsupported.insert("Physics Balljoint (computed objects)".to_owned());
                }
            },
            "Physics Force" => match force(n) {
                Some(f) => forces.push(f),
                None => {
                    unsupported.insert("Physics Force (computed values)".to_owned());
                }
            },
            "Delayer" => {
                if let Some(t) = float(input(n, "Time to Wait")) {
                    delays_ms.push(t);
                }
            }
            "Set Physics Spring" => match spring(n) {
                Some(s) => springs.push(s),
                None => {
                    unsupported.insert("Set Physics Spring (computed objects)".to_owned());
                }
            },
            "TT Scaleable Proximity" => {
                if has_wakeup && wake.is_none() {
                    wake = proximity(n, wake_bodies.clone());
                }
                proximities.extend(proximity(n, Vec::new()));
            }
            "Box Box Intersection" => boxes.extend(text(input(n, "Entity 2"))),
            "Physics WakeUp" => {}
            other => {
                unsupported.insert(other.to_owned());
            }
        }
    }
    Ok(bb::ModuleSetup {
        group,
        bodies,
        hinges,
        sliders,
        springs,
        ball_joints,
        forces,
        delays_ms,
        proximities,
        boxes,
        wake,
        unsupported: unsupported.into_iter().collect(),
    })
}

fn constant(v: &NamedValue) -> Option<&Value> {
    match &v.value {
        Resolved::Constant { value } => Some(value),
        _ => None,
    }
}

fn input<'a>(n: &'a NodeParams, name: &str) -> Option<&'a NamedValue> {
    n.inputs.iter().find(|i| i.name == name)
}

fn local<'a>(n: &'a NodeParams, name: &str) -> Option<&'a NamedValue> {
    n.locals.iter().find(|i| i.name == name)
}

fn text(v: Option<&NamedValue>) -> Option<String> {
    match v.and_then(constant)? {
        Value::Text(s) | Value::Object(Some(s)) => Some(s.clone()),
        _ => None,
    }
}

fn float(v: Option<&NamedValue>) -> Option<f32> {
    match v.and_then(constant)? {
        Value::Float(f) => Some(*f as f32),
        Value::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn int(v: Option<&NamedValue>) -> Option<i32> {
    match v.and_then(constant)? {
        Value::Int(i) => i32::try_from(*i).ok(),
        Value::Float(f) => Some(*f as i32),
        _ => None,
    }
}

fn boolean(v: Option<&NamedValue>) -> Option<bool> {
    match v.and_then(constant)? {
        Value::Bool(b) => Some(*b),
        Value::Int(i) => Some(*i != 0),
        _ => None,
    }
}

fn target(n: &NodeParams) -> Option<String> {
    n.target.as_ref().and_then(|t| text(Some(t)))
}

/// A Physicalize node with a constant target and constant convex meshes.
fn body(n: &NodeParams) -> Result<Option<bb::ModuleBody>> {
    let Some(object) = target(n) else {
        return Ok(None);
    };
    let convex: Vec<String> = n
        .inputs
        .iter()
        .filter(|i| i.name.to_lowercase().starts_with("convex"))
        .map(|i| text(Some(i)))
        .collect::<Option<_>>()
        .unwrap_or_default();
    if convex.is_empty() {
        return Ok(None);
    }
    let f = |name: &str| float(input(n, name)).with_context(|| format!("{object}: {name}"));
    let b = |name: &str| boolean(input(n, name)).with_context(|| format!("{object}: {name}"));
    let shift = match local(n, "Shift Mass Center").and_then(constant) {
        // Mirror Z into bundle space (adding 0.0 turns -0.0 into 0.0).
        Some(Value::Vector(v)) if v.len() == 3 => [v[0] as f32, v[1] as f32, 0.0 - v[2] as f32],
        _ => [0.0; 3],
    };
    Ok(Some(bb::ModuleBody {
        fixed: b("Fixed ?")?,
        friction: f("Friction")?,
        elasticity: f("Elasticity")?,
        mass: f("Mass")?,
        collision_group: text(input(n, "Collision Group")).unwrap_or_default(),
        start_frozen: b("Start Frozen")?,
        collision: b("Enable Collision")?,
        auto_mass_center: b("Automatic Calculate Mass Center")?,
        shift_mass_center: shift,
        linear_damping: f("Linear Speed Dampening")?,
        rotational_damping: f("Rot Speed Dampening")?,
        convex,
        object,
    }))
}

/// Object2 of a joint: a computed `FixCube` (`variable "FixCube Object"`) is
/// the world. `None` if it's computed some other way.
#[allow(
    clippy::option_option,
    reason = "outer: decodable; inner: None is the world"
)]
fn other_body(n: &NodeParams) -> Option<Option<String>> {
    let p = input(n, "Object2")?;
    match &p.value {
        Resolved::Dynamic { from, .. } if from.contains("FixCube") => Some(None),
        _ => Some(Some(text(Some(p))?)),
    }
}

/// A vector input, mirrored into bundle space.
fn vector(v: Option<&NamedValue>) -> Option<[f32; 3]> {
    match v.and_then(constant)? {
        Value::Vector(v) if v.len() == 3 => Some([v[0] as f32, v[1] as f32, 0.0 - v[2] as f32]),
        _ => None,
    }
}

fn slider(n: &NodeParams) -> Option<bb::SliderParams> {
    let limited = boolean(input(n, "Limitations (meter)")).unwrap_or(false);
    Some(bb::SliderParams {
        body: target(n)?,
        other: other_body(n)?,
        axis_from: text(input(n, "Axis first Point"))?,
        axis_to: text(input(n, "Axis second Point"))?,
        limits: limited
            .then(|| {
                Some([
                    float(input(n, "Lower Limit"))?,
                    float(input(n, "Upper Limit"))?,
                ])
            })
            .flatten(),
    })
}

fn spring(n: &NodeParams) -> Option<bb::SpringParams> {
    let anchor = |k: u8| bb::SpringAnchor {
        frame: text(input(n, &format!("Referential {k}"))),
        position: vector(input(n, &format!("Position {k}"))).unwrap_or([0.0; 3]),
    };
    Some(bb::SpringParams {
        body: target(n)?,
        other: other_body(n)?,
        anchor: anchor(1),
        other_anchor: anchor(2),
        length: float(input(n, "Length"))?,
        constant: float(input(n, "Constant"))?,
        linear_damping: float(input(n, "Linear Dampening"))?,
        global_damping: float(input(n, "Global Dampening"))?,
    })
}

fn ball_joint(n: &NodeParams) -> Option<bb::BallJointParams> {
    Some(bb::BallJointParams {
        body: target(n)?,
        other: other_body(n)?,
        pivot: bb::SpringAnchor {
            frame: text(input(n, "Referential 1")),
            position: vector(input(n, "Position 1")).unwrap_or([0.0; 3]),
        },
    })
}

/// A force; a computed target (the active ball) is `body: None`.
fn force(n: &NodeParams) -> Option<bb::ForceParams> {
    Some(bb::ForceParams {
        body: target(n),
        position: bb::SpringAnchor {
            frame: text(input(n, "Pos Referential")),
            position: vector(input(n, "Position")).unwrap_or([0.0; 3]),
        },
        direction: vector(input(n, "Direction"))?,
        direction_frame: text(input(n, "Direction Ref")),
        value: float(input(n, "Force Value"))?,
    })
}

/// A hinge with constant objects (Object2 `FixCube` = the world).
fn hinge(n: &NodeParams) -> Option<bb::HingeParams> {
    let body = target(n)?;
    let other = other_body(n)?;
    let frame = text(input(n, "Joint Referential"))?;
    let limited = boolean(input(n, "Limitations (-180 to 180 degree)")).unwrap_or(false);
    let limits = limited
        .then(|| {
            Some([
                float(input(n, "Lower Limit"))?,
                float(input(n, "Upper Limit"))?,
            ])
        })
        .flatten();
    Some(bb::HingeParams {
        body,
        other,
        frame,
        limits,
    })
}

fn proximity(n: &NodeParams, bodies: Vec<String>) -> Option<bb::WakeParams> {
    let axes = int(local(n, "Check Axis:")).unwrap_or(7);
    Some(bb::WakeParams {
        object: text(input(n, "ObjectB"))?,
        distance: float(input(n, "Distance"))?,
        exactness: [
            float(input(n, "Exactness min. Distance"))?,
            float(input(n, "Exactness max. Distance"))?,
        ],
        frame_delay: [
            int(input(n, "Minimum Framedelay"))?,
            int(input(n, "Maximum Framedelay"))?,
        ],
        // Bit 1 is Y.
        xz: axes & 2 == 0,
        bodies,
    })
}
