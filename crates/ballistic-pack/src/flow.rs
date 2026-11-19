//! Extracts the level flow and the event sounds (see the research repo,
//! findings 17):
//! - the `BallManager` (`Gameplay.nmo`): *New Ball*'s Delayer (the ball comes
//!   back that long after the start or a loss), *Deactivate Ball*'s Delayer
//!   (how long a lost ball lingers), the depth-test cubes group;
//! - `DepthTest`: how far below the lowest cube props are taken away, and which
//!   groups (`DepthTestGroups` in `Levelinit.nmo`);
//! - `Sound.nmo`'s *Switch On Message* blocks: each message's Wave Player and
//!   Play Sound Instance plays, followed through links, Nops, Delayers,
//!   Sequencers and sub-graphs (by message name: each file numbers its
//!   messages itself).

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckfile::CkFile;
use ckscript::extract::{Extractor, NodeParams};
use ckscript::{ObjRef, ScriptObject, Scripts};

use crate::install::Install;
use crate::physics::load;
use crate::transformers::{float, input, target, text};

pub fn extract(install: &Install) -> Result<bb::LevelFlow> {
    let g = load(install, "3d entities/gameplay.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let nodes = ex.nodes(&|_| true);
    let delay_in = |graph: &str| {
        nodes
            .iter()
            .find(|n| {
                n.building_block == "Delayer" && n.path.last().map(String::as_str) == Some(graph)
            })
            .and_then(|n| float(input(n, "Time to Wait")))
            .with_context(|| format!("`BallManager`: {graph}'s Delayer not found"))
    };
    let new_ball_ms = delay_in("New Ball")?;
    let lost_ms = delay_in("Deactivate Ball")?;
    // The cubes the ball's box is tested against (Init Ingame names the group).
    let depth_cubes = nodes
        .iter()
        .filter(|n| n.path.iter().any(|p| p == "Init Ingame"))
        .find_map(|n| text(input(n, "p1")).filter(|t| t.starts_with("DepthTestCubes")))
        .context("Init Ingame: DepthTestCubes group not found")?;
    let prop_depth_margin = nodes
        .iter()
        .filter(|n| {
            n.building_block == "Op" && n.path.last().map(String::as_str) == Some("DepthTest")
        })
        .find_map(|n| float(input(n, "p2")))
        .context("DepthTest: margin not found")?;
    let li = load(install, "3d entities/levelinit.nmo")?;
    let table = li.array("DepthTestGroups")?;
    let prop_depth_groups = table
        .rows
        .iter()
        .map(|r| li.row(table, r).string("Groupname"))
        .collect::<Result<_>>()?;
    Ok(bb::LevelFlow {
        new_ball_ms,
        lost_ms,
        depth_cubes,
        prop_depth_margin,
        prop_depth_groups,
        event_sounds: event_sounds(install)?,
    })
}

/// `Sound.nmo`: for each message a Switch On Message reacts to, the sounds it
/// plays.
fn event_sounds(install: &Install) -> Result<Vec<bb::EventSound>> {
    let g = load(install, "3d entities/sound.nmo")?;
    let (file, scripts) = (&g.file, &g.scripts);
    let names = ckscript::types::message_names(file);
    let ex = Extractor::new(file, scripts);
    let nodes = ex.nodes(&|_| true);
    let mut out: Vec<bb::EventSound> = Vec::new();
    for sw in nodes
        .iter()
        .filter(|n| n.building_block == "Switch On Message")
    {
        let Some(b) = scripts.behavior(sw.node) else {
            continue;
        };
        for (k, &param) in b.input_params.iter().enumerate() {
            let Some(message) = crate::transformers::message_index(file, scripts, param)
                .and_then(|i| names.get(i).cloned())
            else {
                continue;
            };
            let Some(&pin) = b.outputs.get(k) else {
                continue;
            };
            let mut cues = Vec::new();
            walk(file, scripts, &nodes, pin, 0, 0.0, 6, &mut cues);
            if cues.is_empty() {
                continue;
            }
            for c in &cues {
                anyhow::ensure!(
                    install.get(&format!("sounds/{}.wav", c.sound)).is_some(),
                    "sound {} not in Sounds/",
                    c.sound
                );
            }
            match out.iter_mut().find(|e| e.message == message) {
                Some(e) => e.cues.extend(cues),
                None => out.push(bb::EventSound { message, cues }),
            }
        }
    }
    Ok(out)
}

/// Follows the links from `pin` to the sounds they play.
#[allow(clippy::too_many_arguments)]
fn walk(
    file: &CkFile,
    scripts: &Scripts,
    nodes: &[NodeParams],
    pin: ObjRef,
    frames: u32,
    ms: f32,
    depth: u32,
    out: &mut Vec<bb::SoundCue>,
) {
    if depth == 0 {
        return;
    }
    let Some(&owner) = scripts.owner.get(&pin) else {
        return;
    };
    // Links from the pin: in the owner's parent graph, and (for a graph's own
    // input pin) inside the graph.
    let mut graphs = Vec::new();
    if let Some(&parent) = scripts.owner.get(&owner) {
        graphs.push(parent);
    }
    if scripts
        .behavior(owner)
        .is_some_and(|b| b.inputs.contains(&pin))
    {
        graphs.push(owner);
    }
    let pin_name = |p: ObjRef| {
        file.objects
            .get(p)
            .and_then(|o| o.name.clone())
            .unwrap_or_default()
    };
    let bb_name = |b: ObjRef| {
        scripts
            .behavior(b)
            .and_then(|x| x.prototype.as_ref())
            .and_then(|p| ckscript::Catalogue::builtin().building_block(p.guid))
            .map(|x| x.name.clone())
    };
    let node = |b: ObjRef| nodes.iter().find(|n| n.node == b);
    for graph in graphs {
        let Some(gb) = scripts.behavior(graph) else {
            continue;
        };
        for &l in &gb.links {
            let Some(Some(ScriptObject::Link(k))) = scripts.objects.get(l) else {
                continue;
            };
            if k.source != Some(pin) {
                continue;
            }
            let Some(t) = k.target else { continue };
            let Some(&tb) = scripts.owner.get(&t) else {
                continue;
            };
            let f = frames + u32::from(k.delay);
            let outputs = |b: ObjRef| {
                scripts
                    .behavior(b)
                    .map(|x| x.outputs.clone())
                    .unwrap_or_default()
            };
            match bb_name(tb).as_deref() {
                Some("Wave Player") if pin_name(t) == "Play" => {
                    if let Some(s) = node(tb).and_then(target) {
                        out.push(bb::SoundCue {
                            sound: s.to_lowercase(),
                            delay_frames: f,
                            delay_ms: ms,
                        });
                    }
                }
                Some("Play Sound Instance") => {
                    if let Some(s) = node(tb).and_then(target) {
                        out.push(bb::SoundCue {
                            sound: s.to_lowercase(),
                            delay_frames: f,
                            delay_ms: ms,
                        });
                    }
                }
                Some("Nop") => {
                    for o in outputs(tb) {
                        walk(file, scripts, nodes, o, f, ms, depth - 1, out);
                    }
                }
                Some("Delayer") => {
                    let wait = node(tb)
                        .and_then(|n| float(input(n, "Time to Wait")))
                        .unwrap_or(0.0);
                    for o in outputs(tb) {
                        walk(file, scripts, nodes, o, f, ms + wait, depth - 1, out);
                    }
                }
                // A Sequencer's first play is its "Out 1".
                Some("Sequencer") => {
                    if let Some(&o) = outputs(tb).get(1) {
                        walk(file, scripts, nodes, o, f, ms, depth - 1, out);
                    }
                }
                // A sub-graph: follow from its input inside it.
                None => walk(file, scripts, nodes, t, f, ms, depth - 1, out),
                _ => {}
            }
        }
    }
}
