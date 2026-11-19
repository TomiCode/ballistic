//! Whole-file view: decodes every script object, finds the root scripts and
//! who owns what, and checks the graph's structural invariants.

use std::collections::{BTreeMap, HashMap};

use ckfile::{CkFile, ClassId, FileObject, Result};

use crate::{
    Behavior, BehaviorIo, BehaviorLink, ObjRef, ParameterIn, ParameterLocal, ParameterOperation,
    ParameterOut, obj_ref,
};

/// Section on scene objects (entities, groups, levels…) listing attached scripts.
const SECTION_SCRIPTS: u32 = 0x800;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ScriptObject {
    Behavior(Behavior),
    Io(BehaviorIo),
    Link(BehaviorLink),
    In(ParameterIn),
    Out(ParameterOut),
    Local(ParameterLocal),
    /// A free-standing `Parameter` (same storage as a local).
    Parameter(ParameterLocal),
    Operation(ParameterOperation),
    Array(crate::DataArray),
}

/// Decodes `obj` if it is one of the script classes.
pub fn decode_script_object(obj: &FileObject) -> Option<Result<ScriptObject>> {
    let chunk = obj.chunk.as_ref()?;
    Some(match obj.class_id {
        ClassId::BEHAVIOR => Behavior::decode(chunk).map(ScriptObject::Behavior),
        ClassId::BEHAVIOR_IO => BehaviorIo::decode(chunk).map(ScriptObject::Io),
        ClassId::BEHAVIOR_LINK => BehaviorLink::decode(chunk).map(ScriptObject::Link),
        ClassId::PARAMETER_IN => ParameterIn::decode(chunk).map(ScriptObject::In),
        ClassId::PARAMETER_OUT => ParameterOut::decode(chunk).map(ScriptObject::Out),
        ClassId::PARAMETER_LOCAL => ParameterLocal::decode(chunk).map(ScriptObject::Local),
        ClassId::PARAMETER => ParameterLocal::decode(chunk).map(ScriptObject::Parameter),
        ClassId::PARAMETER_OPERATION => {
            ParameterOperation::decode(chunk).map(ScriptObject::Operation)
        }
        ClassId::DATA_ARRAY => crate::DataArray::decode(chunk).map(ScriptObject::Array),
        _ => return None,
    })
}

fn is_script_class(c: ClassId) -> bool {
    matches!(
        c,
        ClassId::BEHAVIOR
            | ClassId::BEHAVIOR_IO
            | ClassId::BEHAVIOR_LINK
            | ClassId::PARAMETER_IN
            | ClassId::PARAMETER_OUT
            | ClassId::PARAMETER_LOCAL
            | ClassId::PARAMETER
            | ClassId::PARAMETER_OPERATION
    )
}

/// Scripts attached to scene objects: `(owner, scripts)`.
pub fn script_lists(file: &CkFile) -> Result<Vec<(ObjRef, Vec<ObjRef>)>> {
    let mut out = Vec::new();
    for (i, o) in file.objects.iter().enumerate() {
        if is_script_class(o.class_id) {
            continue;
        }
        let Some(chunk) = &o.chunk else { continue };
        let Some(mut r) = chunk.section(SECTION_SCRIPTS)? else {
            continue;
        };
        let n = r.u32()? as usize;
        if n == 0 || r.remaining() != n {
            continue;
        }
        let refs: Vec<ObjRef> = r.dwords(n)?.iter().filter_map(|&w| obj_ref(w)).collect();
        if refs.iter().all(|&s| {
            file.objects
                .get(s)
                .is_some_and(|b| b.class_id == ClassId::BEHAVIOR)
        }) {
            out.push((i, refs));
        }
    }
    Ok(out)
}

/// All script objects of a file plus derived structure.
#[derive(Debug, Clone)]
pub struct Scripts {
    /// Indexed like `CkFile::objects`; `None` for non-script objects.
    pub objects: Vec<Option<ScriptObject>>,
    /// Top-level behaviours (not a sub-behaviour of any graph), in file order,
    /// with the scene object they're attached to, if any.
    pub roots: Vec<(ObjRef, Option<ObjRef>)>,
    /// Owning behaviour (or operation, for its parameters) of each pin,
    /// parameter, link, operation and sub-behaviour.
    pub owner: HashMap<ObjRef, ObjRef>,
}

impl Scripts {
    pub fn decode(file: &CkFile) -> Result<Self> {
        let objects = file
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| {
                decode_script_object(o)
                    .transpose()
                    .map_err(|e| ckfile::Error::BadChunk(format!("object #{i} {:?}: {e}", o.name)))
            })
            .collect::<Result<Vec<_>>>()?;

        // Graphs may re-export a descendant's parameters as their own, so a
        // parameter can be listed twice. The innermost (non-graph) owner wins:
        // graphs are registered first and building blocks overwrite them.
        let mut owner = HashMap::new();
        let mut order: Vec<usize> = (0..objects.len()).collect();
        order.sort_by_key(|&i| match &objects[i] {
            Some(ScriptObject::Behavior(b)) if b.is_graph() => 0,
            _ => 1,
        });
        for i in order {
            match &objects[i] {
                Some(ScriptObject::Behavior(b)) => {
                    for &c in b
                        .sub_behaviors
                        .iter()
                        .chain(&b.links)
                        .chain(&b.operations)
                        .chain(&b.input_params)
                        .chain(&b.output_params)
                        .chain(&b.local_params)
                        .chain(&b.inputs)
                        .chain(&b.outputs)
                        .chain(b.target_parameter.iter())
                    {
                        owner.insert(c, i);
                    }
                }
                Some(ScriptObject::Operation(op)) => {
                    for c in [op.input_a, op.input_b, op.output].into_iter().flatten() {
                        owner.insert(c, i);
                    }
                }
                _ => {}
            }
        }

        let attached: BTreeMap<ObjRef, ObjRef> = script_lists(file)?
            .into_iter()
            .flat_map(|(o, scripts)| scripts.into_iter().map(move |s| (s, o)))
            .collect();
        let roots = objects
            .iter()
            .enumerate()
            .filter(|(i, o)| matches!(o, Some(ScriptObject::Behavior(_))) && !owner.contains_key(i))
            .map(|(i, _)| (i, attached.get(&i).copied()))
            .collect();

        Ok(Self {
            objects,
            roots,
            owner,
        })
    }

    pub fn behavior(&self, i: ObjRef) -> Option<&Behavior> {
        match self.objects.get(i) {
            Some(Some(ScriptObject::Behavior(b))) => Some(b),
            _ => None,
        }
    }

    /// Checks structural invariants; returns human-readable problems.
    #[allow(clippy::too_many_lines)] // one pass per object kind, kept together
    pub fn validate(&self, file: &CkFile) -> Vec<String> {
        use ClassId as C;
        let mut problems = Vec::new();
        let class = |i: ObjRef| file.objects.get(i).map(|o| o.class_id);
        let name = |i: ObjRef| {
            file.objects
                .get(i)
                .and_then(|o| o.name.clone())
                .unwrap_or_default()
        };
        // object -> listing owners, with whether each owner is a graph
        let mut uses: HashMap<ObjRef, Vec<(ObjRef, bool)>> = HashMap::new();

        let expect =
            |problems: &mut Vec<String>, what: &str, owner: ObjRef, r: ObjRef, ok: &[ClassId]| {
                if !class(r).is_some_and(|c| ok.contains(&c)) {
                    problems.push(format!(
                        "#{owner} {:?}: {what} #{r} has class {:?}",
                        name(owner),
                        class(r)
                    ));
                }
            };

        let params_out = [C::PARAMETER_OUT];
        for (i, o) in self.objects.iter().enumerate() {
            match o {
                Some(ScriptObject::Behavior(b)) => {
                    let lists: [(&str, &Vec<ObjRef>, &[ClassId]); 8] = [
                        ("sub-behavior", &b.sub_behaviors, &[C::BEHAVIOR]),
                        ("link", &b.links, &[C::BEHAVIOR_LINK]),
                        ("operation", &b.operations, &[C::PARAMETER_OPERATION]),
                        ("input param", &b.input_params, &[C::PARAMETER_IN]),
                        ("output param", &b.output_params, &params_out),
                        (
                            "local param",
                            &b.local_params,
                            &[C::PARAMETER_LOCAL, C::PARAMETER],
                        ),
                        ("input", &b.inputs, &[C::BEHAVIOR_IO]),
                        ("output", &b.outputs, &[C::BEHAVIOR_IO]),
                    ];
                    for (what, refs, ok) in lists {
                        for &r in refs {
                            expect(&mut problems, what, i, r, ok);
                            uses.entry(r).or_default().push((i, b.is_graph()));
                        }
                    }
                    for &r in &b.inputs {
                        if let Some(Some(ScriptObject::Io(io))) = self.objects.get(r)
                            && !io.is_input()
                        {
                            problems.push(format!("#{i}: input pin #{r} not flagged as input"));
                        }
                    }
                    for &r in &b.outputs {
                        if let Some(Some(ScriptObject::Io(io))) = self.objects.get(r)
                            && !io.is_output()
                        {
                            problems.push(format!("#{i}: output pin #{r} not flagged as output"));
                        }
                    }
                    // Links connect pins of this graph or of its direct children.
                    for &l in &b.links {
                        let Some(Some(ScriptObject::Link(link))) = self.objects.get(l) else {
                            continue;
                        };
                        for end in [link.source, link.target].into_iter().flatten() {
                            let pin_owner = self.owner.get(&end).copied();
                            let local = pin_owner == Some(i)
                                || pin_owner.is_some_and(|p| b.sub_behaviors.contains(&p));
                            if !local {
                                problems.push(format!(
                                    "#{i} {:?}: link #{l} touches pin #{end} owned by {pin_owner:?}",
                                    name(i)
                                ));
                            }
                        }
                    }
                }
                Some(ScriptObject::In(p)) => {
                    if let Some(s) = p.source {
                        expect(
                            &mut problems,
                            "source",
                            i,
                            s,
                            &[C::PARAMETER_LOCAL, C::PARAMETER_OUT, C::PARAMETER],
                        );
                    }
                    if let Some(s) = p.shared {
                        expect(&mut problems, "shared input", i, s, &[C::PARAMETER_IN]);
                    }
                }
                Some(ScriptObject::Operation(op)) => {
                    for r in [op.input_a, op.input_b].into_iter().flatten() {
                        expect(&mut problems, "operation input", i, r, &[C::PARAMETER_IN]);
                        uses.entry(r).or_default().push((i, false));
                    }
                    if let Some(r) = op.output {
                        expect(&mut problems, "operation output", i, r, &params_out);
                        uses.entry(r).or_default().push((i, false));
                    }
                }
                _ => {}
            }
        }
        // Everything except root behaviours and free-standing parameters needs an owner.
        for (i, o) in self.objects.iter().enumerate() {
            let needs_owner = matches!(
                o,
                Some(
                    ScriptObject::Io(_)
                        | ScriptObject::Link(_)
                        | ScriptObject::In(_)
                        | ScriptObject::Operation(_)
                )
            );
            if needs_owner && !self.owner.contains_key(&i) {
                problems.push(format!("#{i} {:?} ({:?}) has no owner", name(i), class(i)));
            }
        }
        for (&r, owners) in &uses {
            let shareable = matches!(class(r), Some(C::PARAMETER_LOCAL | C::PARAMETER));
            let non_graph = owners.iter().filter(|(_, g)| !g).count();
            // Allowed: one inner owner plus any number of graphs re-exporting it.
            let exported =
                matches!(class(r), Some(C::PARAMETER_IN | C::PARAMETER_OUT)) && non_graph <= 1;
            if owners.len() > 1 && !shareable && !exported {
                problems.push(format!(
                    "#{r} {:?} is listed by {} owners: {owners:?}",
                    name(r),
                    owners.len()
                ));
            }
        }
        problems
    }
}
