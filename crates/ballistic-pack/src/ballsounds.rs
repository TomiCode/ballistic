//! Extracts the ball's hit and roll sounds (see the research repo, findings
//! 15):
//! - `Sound.nmo`'s `Sound_Manager`:
//!   - *Hit Sounds*: one `PhysicsCollDetection` per hit ID, each driving a
//!     Tuner that plays a `BallSound` column;
//!   - the roll graphs: `PhysicsContinuousContact`'s outputs per roll ID, each
//!     starting a Wave Player of a `BallSound` column (the paper ball's graph,
//!     chosen by a Test on the ball's name, uses one column for all);
//!   - `MultiRollSoundControl`: the roll sounds' volume and pitch from the
//!     ball's speed;
//!   - the `BallSound` table: each ball type's sound names.
//! - `Levelinit.nmo`'s *set Coll- and Rollsound*: the level groups whose
//!   members get a hit or roll ID (the number at the end of the group's name).
//! - The prefabs' objects' *Coll Detection ID* and *Continuous Contact ID*
//!   attributes.

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckfile::CkFile;
use ckscript::attributes::{attribute_names, object_attributes};
use ckscript::extract::{Extractor, NodeParams, Value};
use ckscript::{ObjRef, ScriptObject, Scripts};

use crate::install::Install;
use crate::physics::load;
use crate::transformers::{follow_to, input, text};

/// The attributes the physics plug-in reads (`physics_RT.dll` looks them up
/// by these names).
const HIT_ATTRIBUTE: &str = "Coll Detection ID";
const ROLL_ATTRIBUTE: &str = "Continuous Contact ID";

#[allow(clippy::too_many_lines, reason = "one pass over the sound scripts")]
pub fn extract(install: &Install, balls: &[bb::BallParams]) -> Result<bb::BallSounds> {
    let g = load(install, "3d entities/sound.nmo")?;
    let (file, scripts) = (&g.file, &g.scripts);
    let ex = Extractor::new(file, scripts);
    let nodes = ex.nodes(&|_| true);
    let name_of = |i: ObjRef| file.objects.get(i).and_then(|o| o.name.clone());

    // Hit Sounds: each Tuner's Volume comes from one detector; its Sound is a
    // local named after a BallSound column.
    let mut hit_columns = Vec::new(); // (id, column)
    let mut hits = Vec::new();
    for (i, o) in scripts.objects.iter().enumerate() {
        let Some(ScriptObject::Behavior(b)) = o else {
            continue;
        };
        if name_of(i).as_deref() != Some("Tuner") {
            continue;
        }
        let mut detector = None;
        let mut column = None;
        for &p in &b.input_params {
            let Some(src) = source(scripts, p) else {
                continue;
            };
            match name_of(p).as_deref() {
                Some("Volume") => detector = scripts.owner.get(&src).copied(),
                Some("Sound") => column = name_of(src),
                _ => {}
            }
        }
        let (Some(d), Some(column)) = (detector, column) else {
            continue;
        };
        let Some(n) = nodes
            .iter()
            .find(|n| n.node == d && n.building_block == "PhysicsCollDetection")
        else {
            continue;
        };
        let f = |pin: &str| {
            crate::transformers::float(input(n, pin)).with_context(|| format!("Hit Sounds: {pin}"))
        };
        let id =
            u8::try_from(f("Collision ID")?.round() as i64).context("Hit Sounds: Collision ID")?;
        hits.push(bb::HitTest {
            id,
            min_speed: f("Min Speed m/s")?,
            max_speed: f("Max Speed m/s")?,
            sleep_s: f("Sleep afterwards")?,
        });
        hit_columns.push((id, column));
    }
    hits.sort_by_key(|h| h.id);
    hit_columns.sort();
    if hits.is_empty() {
        bail!("Sound.nmo: no hit sound tests found");
    }

    // Roll graphs: per roll ID (output pair), the column its Wave Player plays.
    let roll_graph = |graph: &str| -> Result<(Vec<String>, [f32; 2])> {
        let n = nodes
            .iter()
            .find(|n| {
                n.building_block == "PhysicsContinuousContact"
                    && n.path.last().map(String::as_str) == Some(graph)
            })
            .with_context(|| format!("{graph}: PhysicsContinuousContact not found"))?;
        let b = scripts.behavior(n.node).context("contact block")?;
        let mut columns = Vec::new();
        // Outputs: contact on 1, contact off 1, contact on 2, …
        for &on in b.outputs.iter().step_by(2) {
            let Some((player, _)) = follow_to(file, scripts, on, "Wave Player", 0, 3) else {
                break;
            };
            let column = scripts
                .behavior(player)
                .and_then(|p| p.target_parameter)
                .and_then(|t| source(scripts, t))
                .and_then(name_of)
                .with_context(|| format!("{graph}: Wave Player target"))?;
            columns.push(column);
        }
        let f = |pin: &str| {
            crate::transformers::float(input(n, pin)).with_context(|| format!("{graph}: {pin}"))
        };
        Ok((columns, [f("Time Delay Start")?, f("Time Delay End")?]))
    };
    let (paper_columns, delays) = roll_graph("Roll Paper")?;
    let (columns, delays2) = roll_graph("Roll Wood/Stone")?;
    anyhow::ensure!(
        delays
            .iter()
            .zip(delays2)
            .all(|(a, b)| (a - b).abs() < 1e-6),
        "roll graphs with different delays"
    );
    // The Test that picks the paper graph: the ball's name against a constant.
    let single = nodes
        .iter()
        .filter(|n| {
            n.building_block == "Test" && n.path.last().map(String::as_str) == Some("Sound_Manager")
        })
        .find_map(|n| text(input(n, "B")))
        .context("Sound_Manager: ball Test not found")?;

    // MultiRollSoundControl: pitch from its Calculator ("A+(a*B)"), volume
    // from the speed times the SoundVolumeFactor.
    let expr = nodes
        .iter()
        .find(|n| {
            n.building_block == "Calculator"
                && n.path.last().map(String::as_str) == Some("MultiRollSoundControl")
        })
        .and_then(|n| text(input(n, "expression")))
        .context("MultiRollSoundControl: Calculator not found")?;
    let roll_pitch =
        parse_linear(&expr).with_context(|| format!("unexpected pitch expression {expr:?}"))?;
    let roll_volume = scripts
        .objects
        .iter()
        .enumerate()
        .find_map(|(i, o)| match o {
            Some(ScriptObject::Local(l)) if name_of(i).as_deref() == Some("SoundVolumeFactor") => {
                match ckscript::extract::typed_value(file, l.type_guid, &l.value) {
                    Value::Float(f) => Some(f as f32),
                    _ => None,
                }
            }
            _ => None,
        })
        .context("Sound_Manager: SoundVolumeFactor not found")?;

    // BallSound: each ball's sound names.
    let table = g.array("BallSound")?;
    let mut sets = Vec::new();
    for cells in &table.rows {
        let row = g.row(table, cells);
        let ball = row.string("Active Ball")?;
        if !balls.iter().any(|b| b.name == ball) {
            continue;
        }
        let key = |col: &str| -> Result<String> {
            let name = row.string(col)?;
            let key = name.to_lowercase();
            anyhow::ensure!(
                install.get(&format!("sounds/{key}.wav")).is_some(),
                "sound {name} not in Sounds/"
            );
            Ok(key)
        };
        let cols = if ball == single {
            &paper_columns
        } else {
            &columns
        };
        sets.push(bb::BallSoundSet {
            roll: cols.iter().map(|c| key(c)).collect::<Result<_>>()?,
            hit: hit_columns
                .iter()
                .map(|(_, c)| key(c))
                .collect::<Result<_>>()?,
            ball,
        });
    }

    let (hit_groups, roll_groups) = sound_groups(install)?;
    Ok(bb::BallSounds {
        sets,
        hits,
        roll_delay_start_s: delays[0],
        roll_delay_end_s: delays[1],
        roll_volume,
        roll_pitch,
        hit_groups,
        roll_groups,
        objects: object_ids(install)?,
    })
}

/// `Levelinit.nmo`'s *set Coll- and Rollsound*: the group names, and the
/// number its `Get SubString` takes from each.
fn sound_groups(install: &Install) -> Result<(Vec<bb::SoundGroup>, Vec<bb::SoundGroup>)> {
    let g = load(install, "3d entities/levelinit.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let nodes: Vec<NodeParams> = ex
        .nodes(&|_| true)
        .into_iter()
        .filter(|n| n.path.last().map(String::as_str) == Some("set Coll- and Rollsound"))
        .collect();
    let mut hit = Vec::new();
    let mut roll = Vec::new();
    for n in nodes
        .iter()
        .filter(|n| n.building_block == "Parameter Selector")
    {
        for i in n.inputs.iter().filter(|i| i.name.starts_with("pIn")) {
            let Some(name) = text(Some(i)) else { continue };
            let digits: String = name
                .chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect();
            let id: u8 = digits
                .chars()
                .rev()
                .collect::<String>()
                .parse()
                .with_context(|| format!("group {name}"))?;
            let group = bb::SoundGroup {
                group: name.clone(),
                id,
            };
            if name.contains("HitID") {
                hit.push(group);
            } else if name.contains("RollID") {
                roll.push(group);
            }
        }
    }
    anyhow::ensure!(
        !hit.is_empty() && !roll.is_empty(),
        "Levelinit: sound groups not found"
    );
    Ok((hit, roll))
}

/// The prefabs' objects with a hit or roll ID attribute.
fn object_ids(install: &Install) -> Result<Vec<bb::ObjectSoundIds>> {
    let mut out = Vec::new();
    let mut prefabs: Vec<(&str, &std::path::Path)> = install
        .under("3d entities/ph/")
        .filter(|(k, _)| {
            std::path::Path::new(k)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("nmo"))
        })
        .collect();
    prefabs.sort_by(|a, b| a.0.cmp(b.0));
    for (key, path) in prefabs {
        let file = CkFile::open(path).with_context(|| format!("reading {key}"))?;
        let scripts = Scripts::decode(&file).with_context(|| format!("decoding {key}"))?;
        let names = attribute_names(&file);
        let scene = format!(
            "ph/{}",
            &key["3d entities/ph/".len()..key.len() - ".nmo".len()]
        );
        for (i, o) in file.objects.iter().enumerate() {
            let attrs = object_attributes(&file, &scripts, &names, i);
            let int = |name: &str| {
                attrs
                    .iter()
                    .find(|(n, _)| n == name)
                    .and_then(|(_, v)| match v {
                        Value::Int(x) => u8::try_from(*x).ok(),
                        _ => None,
                    })
            };
            let (hit, roll) = (int(HIT_ATTRIBUTE), int(ROLL_ATTRIBUTE));
            if hit.is_none() && roll.is_none() {
                continue;
            }
            out.push(bb::ObjectSoundIds {
                scene: scene.clone(),
                object: o.name.clone().unwrap_or_default(),
                hit: hit.unwrap_or(0),
                roll: roll.unwrap_or(0),
            });
        }
    }
    Ok(out)
}

/// `A+(a*B)` → `[A, B]`.
fn parse_linear(expr: &str) -> Option<[f32; 2]> {
    let e: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    let (a, rest) = e.split_once("+(a*")?;
    let b = rest.strip_suffix(')')?;
    Some([a.parse().ok()?, b.parse().ok()?])
}

/// Where an input parameter's value comes from (a local or an output).
fn source(scripts: &Scripts, param: ObjRef) -> Option<ObjRef> {
    match scripts.objects.get(param)? {
        Some(ScriptObject::In(i)) => i.source.or(i.shared.and_then(|s| source(scripts, s))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_the_pitch_expression() {
        assert_eq!(super::parse_linear("0.5+(a*0.01)"), Some([0.5, 0.01]));
        assert_eq!(super::parse_linear("a*2"), None);
    }
}
