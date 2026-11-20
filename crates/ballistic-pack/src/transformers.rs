//! Extracts the ball transformers from `Gameplay.nmo`'s `Trafo Manager` (see
//! the research repo, findings 14):
//! - the reach: the `Test` on the nearest transformer's distance;
//! - the pull into the transformer: `TT Set Dynamic Position` in `dephysic Ball`,
//!   and its Delayer;
//! - the two Delayers after it: the one fed by `dephysic Ball` (the ball hangs
//!   in the transformer), and the one fed by `start Explosion` (until the new
//!   ball appears).
//!
//! Each `P_Trafo_<X>` prefab carries the transformer attribute with the value
//! the script maps to `Ball_<X>`, so a transformer's ball follows its name.
//!
//! Also:
//! - the transformer's animation from `AnimTrafo.nmo` (`AnimTrafo_MF Script`);
//! - each ball type's pieces from `Balls.nmo` (`Ball_Explosion_<Type>`,
//!   `Ball_ResetPieces_<Type>`);
//! - when they fade: the `Fadeout Manager`'s Timer in the Trafo Manager;
//! - the sounds: the messages the Trafo Manager sends to the sound group,
//!   followed in `Sound.nmo`'s `Simple Sound Messages` to a Wave Player (by
//!   message name: each file numbers its messages itself), and each type's
//!   pieces' collision sounds in `Ball_CollSound_Pieces`.

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckfile::CkFile;
use ckscript::extract::{Extractor, NamedValue, NodeParams, Resolved, Value};
use ckscript::{ObjRef, ScriptObject, Scripts};

use crate::install::Install;

const MANAGER: &str = "Trafo Manager";

pub fn extract(
    install: &Install,
    file: &CkFile,
    scripts: &Scripts,
    balls: &[bb::BallParams],
) -> Result<bb::TransformerParams> {
    let ex = Extractor::new(file, scripts);
    let nodes: Vec<NodeParams> = ex
        .nodes(&|_| true)
        .into_iter()
        .filter(|n| n.path.iter().any(|p| p == MANAGER))
        .collect();
    let at_top = |n: &&NodeParams| n.path.last().map(String::as_str) == Some(MANAGER);

    let reach = nodes
        .iter()
        .filter(at_top)
        .find(|n| n.building_block == "Test")
        .and_then(|n| float(input(n, "B")))
        .context("Trafo Manager: distance test not found")?;

    let pull = nodes
        .iter()
        .find(|n| {
            n.building_block == "TT Set Dynamic Position"
                && n.path.last().map(String::as_str) == Some("dephysic Ball")
        })
        .context("Trafo Manager: TT Set Dynamic Position not found")?;
    let axes = |prefix: &str| -> Result<[f32; 3]> {
        let f = |a: &str| {
            float(input(pull, &format!("{prefix} {a}")))
                .with_context(|| format!("Trafo Manager: {prefix} {a}"))
        };
        Ok([f("X")?, f("Y")?, f("Z")?])
    };
    let pull_force = axes("Force")?;
    let pull_damping = axes("Damping")?;
    let o = axes("Offset")?;
    // Mirror Z into bundle space (adding 0.0 turns -0.0 into 0.0).
    let pull_offset = [o[0], o[1], 0.0 - o[2]];

    let delay = |n: &NodeParams| float(input(n, "Time to Wait"));
    let pull_ms = nodes
        .iter()
        .find(|n| {
            n.building_block == "Delayer"
                && n.path.last().map(String::as_str) == Some("dephysic Ball")
        })
        .and_then(delay)
        .context("Trafo Manager: pull Delayer not found")?;
    let after = |source: &str| {
        nodes
            .iter()
            .filter(at_top)
            .filter(|n| n.building_block == "Delayer")
            .find(|n| fed_by(file, scripts, n.node).iter().any(|s| s == source))
            .and_then(&delay)
            .with_context(|| format!("Trafo Manager: Delayer after {source} not found"))
    };
    let hold_ms = after("dephysic Ball")?;
    let swap_ms = after("start Explosion")?;

    let pieces_fade_after_ms = nodes
        .iter()
        .find(|n| n.building_block == "Timer" && n.path.iter().any(|p| p == "Fadeout Manager"))
        .and_then(|n| float(input(n, "Duration")))
        .context("Trafo Manager: Fadeout Manager Timer not found")?;

    let mut types = Vec::new();
    for b in balls {
        let Some(kind) = b.name.strip_prefix("Ball_") else {
            continue;
        };
        let group = format!("P_Trafo_{kind}");
        if install
            .get(&format!("3d entities/ph/{}.nmo", group.to_lowercase()))
            .is_some()
        {
            types.push(bb::TransformerType {
                group,
                ball: b.name.clone(),
            });
        }
    }

    Ok(bb::TransformerParams {
        types,
        reach,
        pull_force,
        pull_damping,
        pull_offset,
        pull_ms,
        hold_ms,
        swap_ms,
        animation: animation(install)?,
        pieces: pieces(install, balls)?,
        pieces_fade_after_ms,
        sound: trafo_sound(install, file, scripts, &nodes)?,
    })
}

/// The Wave Player that a message of the Trafo Manager's (to the sound group)
/// starts in `Sound.nmo`.
fn trafo_sound(
    install: &Install,
    file: &CkFile,
    scripts: &Scripts,
    nodes: &[NodeParams],
) -> Result<Option<bb::SoundCue>> {
    let names = ckscript::types::message_names(file);
    let sent: Vec<String> = nodes
        .iter()
        .filter(|n| {
            n.building_block == "Send Message" && n.path.last().map(String::as_str) == Some(MANAGER)
        })
        .filter_map(|n| message(file, scripts, n.node, "Message"))
        .filter_map(|i| names.get(i).cloned())
        .collect();
    let g = load(install, "3d entities/sound.nmo")?;
    let sound_names = ckscript::types::message_names(&g.file);
    let ex = Extractor::new(&g.file, &g.scripts);
    let Some(switch) = ex
        .nodes(&|b| b.name == "Switch On Message")
        .into_iter()
        .find(|n| n.path.last().map(String::as_str) == Some("Simple Sound Messages"))
    else {
        return Ok(None);
    };
    let Some(b) = g.scripts.behavior(switch.node) else {
        return Ok(None);
    };
    // Switch On Message: input "Message k" → output k.
    for (k, &param) in b.input_params.iter().enumerate() {
        let Some(name) = message_index(&g.file, &g.scripts, param).and_then(|i| sound_names.get(i))
        else {
            continue;
        };
        if !sent.contains(name) {
            continue;
        }
        let Some(&out) = b.outputs.get(k) else {
            continue;
        };
        if let Some((player, delay)) = follow_to(&g.file, &g.scripts, out, "Wave Player", 0, 4) {
            let target = ex
                .nodes(&|bb| bb.name == "Wave Player")
                .into_iter()
                .find(|n| n.node == player)
                .and_then(|n| target(&n));
            if let Some(t) = target {
                let key = t.to_lowercase();
                anyhow::ensure!(
                    install.get(&format!("sounds/{key}.wav")).is_some(),
                    "sound {t} not in Sounds/"
                );
                return Ok(Some(bb::SoundCue {
                    sound: key,
                    delay_frames: delay,
                    delay_ms: 0.0,
                }));
            }
        }
    }
    Ok(None)
}

/// The behaviour of type `bb` reached from pin `from` through links and Nops
/// (and the total link delay), within `depth` hops.
pub(crate) fn follow_to(
    file: &CkFile,
    scripts: &Scripts,
    from: ObjRef,
    bb: &str,
    delay: u32,
    depth: u32,
) -> Option<(ObjRef, u32)> {
    if depth == 0 {
        return None;
    }
    let owner = *scripts.owner.get(&from)?;
    let graph = *scripts.owner.get(&owner)?;
    let links = &scripts.behavior(graph)?.links;
    let proto = |b: ObjRef| -> Option<String> {
        let beh = scripts.behavior(b)?;
        let p = beh.prototype.as_ref()?;
        ckscript::Catalogue::builtin()
            .building_block(p.guid)
            .map(|x| x.name.clone())
    };
    for &l in links {
        let Some(Some(ScriptObject::Link(k))) = scripts.objects.get(l) else {
            continue;
        };
        if k.source != Some(from) {
            continue;
        }
        let Some(t) = k.target else { continue };
        let Some(&tb) = scripts.owner.get(&t) else {
            continue;
        };
        let d = delay + u32::from(k.delay);
        let name = proto(tb).unwrap_or_default();
        // Only a Wave Player's Play input starts it.
        if name == bb && file.objects.get(t).and_then(|o| o.name.as_deref()) == Some("Play") {
            return Some((tb, d));
        }
        if name == "Nop" {
            for &o in &scripts.behavior(tb)?.outputs {
                if let Some(r) = follow_to(file, scripts, o, bb, d, depth - 1) {
                    return Some(r);
                }
            }
        }
    }
    None
}

/// The message index a node's input parameter `pin` holds.
fn message(file: &CkFile, scripts: &Scripts, node: ObjRef, pin: &str) -> Option<usize> {
    let b = scripts.behavior(node)?;
    let param = b
        .input_params
        .iter()
        .copied()
        .find(|&p| file.objects.get(p).and_then(|o| o.name.as_deref()) == Some(pin))?;
    message_index(file, scripts, param)
}

/// The message index an input parameter holds (through its source local).
pub(crate) fn message_index(file: &CkFile, scripts: &Scripts, param: ObjRef) -> Option<usize> {
    let Some(Some(ScriptObject::In(i))) = scripts.objects.get(param) else {
        return None;
    };
    let value = match scripts.objects.get(i.source?)? {
        Some(ScriptObject::Local(l)) => &l.value,
        Some(ScriptObject::Out(o)) => &o.value,
        _ => return None,
    };
    let _ = file;
    match value {
        ckscript::ParamValue::Manager { value, .. } => usize::try_from(*value).ok(),
        _ => None,
    }
}

struct Loaded {
    file: CkFile,
    scripts: Scripts,
}

fn load(install: &Install, key: &str) -> Result<Loaded> {
    let path = install.get(key).with_context(|| format!("{key} missing"))?;
    let file = CkFile::open(path).with_context(|| format!("reading {key}"))?;
    let scripts = Scripts::decode(&file).with_context(|| format!("decoding scripts of {key}"))?;
    Ok(Loaded { file, scripts })
}

/// `AnimTrafo_MF Script`'s graphs.
fn animation(install: &Install) -> Result<bb::TransformerAnimation> {
    let g = load(install, "3d entities/animtrafo.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let nodes = ex.nodes(&|_| true);
    let graph = |name: &str| -> Vec<&NodeParams> {
        nodes
            .iter()
            .filter(|n| n.path.last().map(String::as_str) == Some(name))
            .collect()
    };
    let one = |graph: &[&NodeParams], bb: &str| -> Result<NodeParams> {
        graph
            .iter()
            .find(|n| n.building_block == bb)
            .map(|n| (*n).clone())
            .with_context(|| format!("AnimTrafo: {bb} not found"))
    };
    let open = graph("Ring_Open");
    let close = graph("Ring_Close");
    let lift = graph("Up ´n Down");
    let intro = graph("lntro");
    let flash = graph("FlashAnim");
    let colors = graph("Get Colors");

    let ring_parts = open
        .iter()
        .filter(|n| n.building_block == "Set Position")
        .map(|n| {
            Ok(bb::RingPart {
                object: target(n).context("AnimTrafo: ring part")?,
                frame: text(input(n, "Referential")).context("AnimTrafo: ring frame")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let interp = |graph: &[&NodeParams], len: usize, pin: &str| -> Result<Vec<f32>> {
        graph
            .iter()
            .filter(|n| n.building_block == "Interpolator")
            .find_map(|n| vector(input(n, pin)).filter(|v| v.len() == len))
            .with_context(|| format!("AnimTrafo: Interpolator {pin} ({len} values)"))
    };
    let mirror = |v: Vec<f32>| [v[0], v[1], 0.0 - v[2]];
    let colors_of = |graph: &[&NodeParams]| -> Result<[[f32; 4]; 2]> {
        let a = interp(graph, 4, "A")?;
        let b = interp(graph, 4, "B")?;
        Ok([[a[0], a[1], a[2], a[3]], [b[0], b[1], b[2], b[3]]])
    };
    let scroll = one(&flash, "Parameter Selector")?;
    let v2 = |name: &str| -> Result<[f32; 2]> {
        let v = vector(input(&scroll, name)).with_context(|| format!("FlashAnim {name}"))?;
        Ok([v[0], v[1]])
    };
    Ok(bb::TransformerAnimation {
        root: target(&one(&intro, "Set World Matrix")?).context("AnimTrafo: root")?,
        bars: target(&one(&lift, "Set Position")?).context("AnimTrafo: bars")?,
        ring_parts,
        ring_offset: mirror(interp(&open, 3, "B")?),
        ring_open: progression(&one(&open, "Bezier Progression")?)?,
        ring_close: progression(&one(&close, "Bezier Progression")?)?,
        bars_offset: mirror(interp(&lift, 3, "B")?),
        lift: progression(&one(&lift, "Bezier Progression")?)?,
        shadow_colors: colors_of(&lift)?,
        flash: target(&one(&lift, "Show")?).context("AnimTrafo: flash field")?,
        flash_scroll: [v2("pIn 0")?, v2("pIn 1")?],
        flash_ms: float(input(&one(&flash, "Delayer")?, "Time to Wait"))
            .context("FlashAnim: Delayer")?,
        ring_material: target(&one(&colors, "Set Diffuse")?).context("AnimTrafo: ring material")?,
    })
}

/// Each `Ball_Explosion_<Type>` and its `Ball_ResetPieces_<Type>`.
fn pieces(install: &Install, balls: &[bb::BallParams]) -> Result<Vec<bb::BallPieces>> {
    let g = load(install, "3d entities/balls.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let nodes = ex.nodes(&|_| true);
    let mut out = Vec::new();
    for b in balls {
        let Some(kind) = b.name.strip_prefix("Ball_") else {
            continue;
        };
        let explosion = format!("Ball_Explosion_{kind}");
        let reset = format!("Ball_ResetPieces_{kind}");
        let under = |root: &str, bb: &str| {
            nodes
                .iter()
                .find(|n| {
                    n.building_block == bb && n.path.first().map(String::as_str) == Some(root)
                })
                .with_context(|| format!("{root}: {bb} not found"))
        };
        let phys = under(&explosion, "Physicalize")?;
        let impulse = under(&explosion, "Physics Impulse")?;
        // A value from a constant, or the range of the Random that writes it.
        let range = |n: &NodeParams, pin: &str| -> Result<[f32; 2]> {
            if let Some(v) = float(input(n, pin)) {
                return Ok([v, v]);
            }
            let r = writer_of(&g.file, &g.scripts, n.node, pin)
                .and_then(|w| {
                    nodes
                        .iter()
                        .find(|r| r.node == w && r.building_block == "Random")
                })
                .with_context(|| format!("{explosion}: {pin} is neither constant nor Random"))?;
            Ok([
                float(input(r, "Min")).context("Random Min")?,
                float(input(r, "Max")).context("Random Max")?,
            ])
        };
        let constant = |n: &NodeParams, pin: &str| {
            float(input(n, pin)).with_context(|| format!("{explosion}: {pin}"))
        };
        let v3 = |n: &NodeParams, pin: &str| -> Result<[f32; 3]> {
            let v = vector(input(n, pin)).with_context(|| format!("{explosion}: {pin}"))?;
            Ok([v[0], v[1], 0.0 - v[2]])
        };
        let iter = under(&explosion, "Group Iterator")?;
        let fade_colors = {
            let i = under(&reset, "Interpolator")?;
            let c = |pin: &str| -> Result<[f32; 4]> {
                let v = vector(input(i, pin)).with_context(|| format!("{reset}: {pin}"))?;
                Ok([v[0], v[1], v[2], v[3]])
            };
            [c("A")?, c("B")?]
        };
        out.push(bb::BallPieces {
            ball: b.name.clone(),
            group: text(input(iter, "Group")).with_context(|| format!("{explosion}: group"))?,
            frame: target(under(&explosion, "Set Position")?)
                .with_context(|| format!("{explosion}: frame"))?,
            materials: nodes
                .iter()
                .filter(|n| {
                    n.building_block == "Set Diffuse"
                        && n.path.first().map(String::as_str) == Some(reset.as_str())
                })
                .filter_map(target)
                .collect(),
            mass: range(phys, "Mass")?,
            friction: range(phys, "Friction")?,
            elasticity: constant(phys, "Elasticity")?,
            linear_damping: constant(phys, "Linear Speed Dampening")?,
            rotational_damping: constant(phys, "Rot Speed Dampening")?,
            collision_group: text(input(phys, "Collision Group")).unwrap_or_default(),
            impulse: range(impulse, "Impulse")?,
            impulse_position: v3(impulse, "Position")?,
            impulse_direction: v3(impulse, "Direction")?,
            fade: progression(under(&reset, "Bezier Progression")?)?,
            fade_colors,
            sounds: piece_sounds(install, &nodes, kind)?,
        });
    }
    Ok(out)
}

/// `Ball_CollSound_Pieces`: the `<Kind>-Collision Sound` graph's collision
/// tests, or (without one, the paper) a single play at the burst. The sound is
/// the one `Balls_Init` loads by name, `Pieces_<Kind>`.
fn piece_sounds(install: &Install, nodes: &[NodeParams], kind: &str) -> Result<bb::PieceSounds> {
    let name = format!("Pieces_{kind}");
    anyhow::ensure!(
        nodes
            .iter()
            .any(|n| n.path.first().map(String::as_str) == Some("Balls_Init")
                && text(input(n, "p1")).as_deref() == Some(name.as_str())),
        "Balls_Init doesn't load {name}"
    );
    let key = name.to_lowercase();
    anyhow::ensure!(
        install.get(&format!("sounds/{key}.wav")).is_some(),
        "sound {name} not in Sounds/"
    );
    let graph = format!("{kind}-Collision Sound");
    let in_graph = |bb: &str| {
        nodes
            .iter()
            .filter(|n| n.building_block == bb && n.path.last() == Some(&graph))
            .collect::<Vec<_>>()
    };
    let tests = in_graph("PhysicsCollDetection");
    let Some(first) = tests.first() else {
        return Ok(bb::PieceSounds {
            sound: key,
            colliders: Vec::new(),
            min_speed: 0.0,
            max_speed: 0.0,
            sleep_s: 0.0,
            listen_ms: 0.0,
        });
    };
    let f =
        |n: &NodeParams, pin: &str| float(input(n, pin)).with_context(|| format!("{graph}: {pin}"));
    Ok(bb::PieceSounds {
        sound: key,
        colliders: tests.iter().filter_map(|n| target(n)).collect(),
        min_speed: f(first, "Min Speed m/s")?,
        max_speed: f(first, "Max Speed m/s")?,
        sleep_s: f(first, "Sleep afterwards")?,
        listen_ms: in_graph("Delayer")
            .first()
            .and_then(|n| float(input(n, "Time to Wait")))
            .with_context(|| format!("{graph}: Delayer"))?,
    })
}

pub(crate) fn progression(n: &NodeParams) -> Result<bb::Progression> {
    let duration_ms = float(input(n, "Duration")).context("Bezier Progression: Duration")?;
    let curve = match input(n, "Progression Curve").map(|i| &i.value) {
        Some(Resolved::Constant {
            value: Value::Curve(points),
        }) => points
            .iter()
            .map(|p| bb::CurvePoint {
                position: [p.x, p.y],
                in_tangent: p.in_tangent,
                out_tangent: p.out_tangent,
            })
            .collect(),
        other => anyhow::bail!("Bezier Progression: no curve ({other:?})"),
    };
    Ok(bb::Progression { duration_ms, curve })
}

/// The behaviour whose output feeds `node`'s input `pin`.
pub(crate) fn writer_of(
    file: &CkFile,
    scripts: &Scripts,
    node: ObjRef,
    pin: &str,
) -> Option<ObjRef> {
    let b = scripts.behavior(node)?;
    let param = b
        .input_params
        .iter()
        .copied()
        .find(|&p| file.objects.get(p).and_then(|o| o.name.as_deref()) == Some(pin))?;
    let Some(Some(ScriptObject::In(i))) = scripts.objects.get(param) else {
        return None;
    };
    scripts.owner.get(&i.source?).copied()
}

pub(crate) fn target(n: &NodeParams) -> Option<String> {
    text(n.target.as_ref())
}

pub(crate) fn text(v: Option<&NamedValue>) -> Option<String> {
    match &v?.value {
        Resolved::Constant {
            value: Value::Text(s) | Value::Object(Some(s)),
        } => Some(s.clone()),
        _ => None,
    }
}

pub(crate) fn vector(v: Option<&NamedValue>) -> Option<Vec<f32>> {
    match &v?.value {
        Resolved::Constant {
            value: Value::Vector(v),
        } => Some(v.iter().map(|&x| x as f32).collect()),
        _ => None,
    }
}

/// Names of the behaviours whose outputs are linked to `node`'s inputs in its
/// parent graph.
fn fed_by(file: &CkFile, scripts: &Scripts, node: ObjRef) -> Vec<String> {
    let (Some(&parent), Some(b)) = (scripts.owner.get(&node), scripts.behavior(node)) else {
        return Vec::new();
    };
    let Some(pb) = scripts.behavior(parent) else {
        return Vec::new();
    };
    pb.links
        .iter()
        .filter_map(|&l| match scripts.objects.get(l) {
            Some(Some(ScriptObject::Link(k))) => Some(k),
            _ => None,
        })
        .filter(|k| k.target.is_some_and(|t| b.inputs.contains(&t)))
        .filter_map(|k| scripts.owner.get(&k.source?))
        .filter_map(|&o| file.objects.get(o).and_then(|o| o.name.clone()))
        .collect()
}

pub(crate) fn input<'a>(n: &'a NodeParams, name: &str) -> Option<&'a NamedValue> {
    n.inputs.iter().find(|i| i.name == name)
}

pub(crate) fn float(v: Option<&NamedValue>) -> Option<f32> {
    match &v?.value {
        Resolved::Constant {
            value: Value::Float(f),
        } => Some(*f as f32),
        Resolved::Constant {
            value: Value::Int(i),
        } => Some(*i as f32),
        _ => None,
    }
}
