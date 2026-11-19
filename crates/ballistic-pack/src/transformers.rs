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
    })
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

fn input<'a>(n: &'a NodeParams, name: &str) -> Option<&'a NamedValue> {
    n.inputs.iter().find(|i| i.name == name)
}

fn float(v: Option<&NamedValue>) -> Option<f32> {
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
