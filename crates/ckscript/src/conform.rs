//! Cross-checks decoded building-block nodes against their catalogue
//! declarations: a strong test of both the graph decoder and the catalogue.

use std::collections::BTreeMap;

use ckfile::CkFile;

use crate::{Catalogue, ScriptObject, Scripts};

/// Outcome of comparing one aspect of a node with its declaration.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Conformance {
    pub nodes: usize,
    /// Prototype GUID not in the catalogue.
    pub unknown: usize,
    /// Pins and parameters match the declaration exactly (names and types).
    pub exact: usize,
    /// Mismatch counts per aspect (`"inputs"`, `"in_params"`, …), keyed by BB name.
    pub differences: BTreeMap<String, BTreeMap<&'static str, usize>>,
}

pub fn check(file: &CkFile, scripts: &Scripts, cat: &Catalogue, out: &mut Conformance) {
    let name = |i: usize| {
        file.objects
            .get(i)
            .and_then(|o| o.name.clone())
            .unwrap_or_default()
    };
    let in_type = |i: usize| match scripts.objects.get(i) {
        Some(Some(ScriptObject::In(p))) => Some(p.type_guid),
        _ => None,
    };
    let out_type = |i: usize| match scripts.objects.get(i) {
        Some(Some(ScriptObject::Out(p))) => Some(p.type_guid),
        _ => None,
    };
    for o in scripts.objects.iter().flatten() {
        let ScriptObject::Behavior(b) = o else {
            continue;
        };
        let Some(proto) = b.prototype else { continue };
        out.nodes += 1;
        let Some(decl) = cat.building_block(proto.guid) else {
            out.unknown += 1;
            continue;
        };
        let mut diffs: Vec<&'static str> = Vec::new();
        let names = |v: &[usize]| v.iter().map(|&i| name(i)).collect::<Vec<_>>();
        let pins_diff = |v: &[usize], d: &[String], what: [&'static str; 2]| {
            if v.len() != d.len() {
                Some(what[0])
            } else if names(v) != d {
                Some(what[1])
            } else {
                None
            }
        };
        diffs.extend(pins_diff(
            &b.inputs,
            &decl.inputs,
            ["inputs count", "inputs name"],
        ));
        diffs.extend(pins_diff(
            &b.outputs,
            &decl.outputs,
            ["outputs count", "outputs name"],
        ));
        // Classify parameter differences: count (variable parameters),
        // type (retyped by the author) or name (renamed).
        let params_diff = |v: &[usize],
                           d: &[crate::ParamDecl],
                           ty: &dyn Fn(usize) -> Option<ckfile::Guid>,
                           what: [&'static str; 3]| {
            if v.len() != d.len() {
                Some(what[0])
            } else if v.iter().zip(d).any(|(&i, d)| ty(i) != Some(d.type_guid)) {
                Some(what[1])
            } else if v.iter().zip(d).any(|(&i, d)| name(i) != d.name) {
                Some(what[2])
            } else {
                None
            }
        };
        diffs.extend(params_diff(
            &b.input_params,
            &decl.in_params,
            &in_type,
            ["in_params count", "in_params type", "in_params name"],
        ));
        diffs.extend(params_diff(
            &b.output_params,
            &decl.out_params,
            &out_type,
            ["out_params count", "out_params type", "out_params name"],
        ));
        if b.local_params.len() != decl.locals.len() {
            diffs.push("locals count");
        }
        if diffs.is_empty() {
            out.exact += 1;
        } else {
            let e = out.differences.entry(decl.name.clone()).or_default();
            for d in diffs {
                *e.entry(d).or_default() += 1;
            }
        }
    }
}
