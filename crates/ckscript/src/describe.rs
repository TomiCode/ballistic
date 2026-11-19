//! Resolved, human-oriented view of a script: a tree of nodes with named pins,
//! parameters with resolved sources and values, and links between named pins.

use std::fmt::Write as _;

use ckfile::{CkFile, Guid};

use crate::types::{format_value, object_label, type_name};
use crate::{Behavior, ObjRef, ParamValue, ScriptObject, Scripts};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Node {
    pub index: ObjRef,
    pub name: String,
    /// `None` for graphs.
    pub prototype: Option<Guid>,
    /// Building-block name and DLL from the catalogue.
    pub building_block: Option<String>,
    pub dll: Option<String>,
    pub target: Option<Param>,
    pub inputs: Vec<Pin>,
    pub outputs: Vec<Pin>,
    pub params_in: Vec<Param>,
    pub params_out: Vec<Param>,
    pub locals: Vec<Param>,
    pub operations: Vec<Operation>,
    pub links: Vec<Link>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Pin {
    pub index: ObjRef,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Param {
    pub index: ObjRef,
    pub name: String,
    pub type_name: String,
    /// For inputs: where the value comes from. For locals/outputs: the value.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Operation {
    pub index: ObjRef,
    pub name: String,
    pub guid: Guid,
    pub inputs: Vec<String>,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Link {
    pub from: String,
    pub to: String,
    pub delay: u16,
}

pub struct Describer<'a> {
    pub file: &'a CkFile,
    pub scripts: &'a Scripts,
}

impl Describer<'_> {
    fn name(&self, i: ObjRef) -> String {
        self.file
            .objects
            .get(i)
            .and_then(|o| o.name.clone())
            .unwrap_or_else(|| "<unnamed>".into())
    }

    /// `node.pin` label for a pin, relative to the graph being described.
    fn pin_label(&self, pin: ObjRef, graph: ObjRef) -> String {
        match self.scripts.owner.get(&pin) {
            Some(&o) if o == graph => format!("this.{}", self.name(pin)),
            Some(&o) => format!("[{o}] {}.{}", self.name(o), self.name(pin)),
            None => format!("?.{}", self.name(pin)),
        }
    }

    fn value_param(&self, i: ObjRef) -> Param {
        let (type_guid, value) = match self.scripts.objects.get(i) {
            Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) => {
                if l.is_myself {
                    (l.type_guid, "this (owner object)".into())
                } else {
                    (l.type_guid, format_value(self.file, l.type_guid, &l.value))
                }
            }
            Some(Some(ScriptObject::Out(o))) => {
                let mut v = format_value(self.file, o.type_guid, &o.value);
                if !o.destinations.is_empty() {
                    let d: Vec<String> = o.destinations.iter().map(|&d| self.name(d)).collect();
                    let _ = write!(v, " -> {}", d.join(", "));
                }
                (o.type_guid, v)
            }
            _ => (Guid::NULL, "?".into()),
        };
        Param {
            index: i,
            name: self.name(i),
            type_name: type_name(type_guid),
            value,
        }
    }

    /// Where an input's value comes from, as text.
    fn source(&self, i: ObjRef) -> String {
        let Some(Some(ScriptObject::In(p))) = self.scripts.objects.get(i) else {
            return "?".into();
        };
        if let Some(s) = p.shared {
            return format!("shares input [{}] {}", s, self.name(s));
        }
        let Some(src) = p.source else {
            return "(unconnected)".into();
        };
        match self.scripts.objects.get(src) {
            Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) => {
                let owner = self.scripts.owner.get(&src).copied();
                let v = if l.is_myself {
                    "this".into()
                } else {
                    format_value(self.file, l.type_guid, &l.value)
                };
                // A local owned by the same node is a constant; otherwise a shared variable.
                if owner == self.scripts.owner.get(&i).copied() {
                    v
                } else {
                    format!("{v}  (local {:?})", self.name(src))
                }
            }
            Some(Some(ScriptObject::Out(_))) => match self.scripts.owner.get(&src) {
                Some(&o) => format!("<- [{o}] {}.{}", self.name(o), self.name(src)),
                None => format!("<- {}", self.name(src)),
            },
            _ => object_label(self.file, src),
        }
    }

    fn param_in(&self, i: ObjRef) -> Param {
        let type_guid = match self.scripts.objects.get(i) {
            Some(Some(ScriptObject::In(p))) => p.type_guid,
            _ => Guid::NULL,
        };
        Param {
            index: i,
            name: self.name(i),
            type_name: type_name(type_guid),
            value: if matches!(self.scripts.objects.get(i), Some(Some(ScriptObject::In(p))) if p.disabled)
            {
                format!("{} (disabled)", self.source(i))
            } else {
                self.source(i)
            },
        }
    }

    pub fn node(&self, i: ObjRef) -> Option<Node> {
        let b: &Behavior = self.scripts.behavior(i)?;
        let pins = |v: &[ObjRef]| -> Vec<Pin> {
            v.iter()
                .map(|&p| Pin {
                    index: p,
                    name: self.name(p),
                })
                .collect()
        };
        let links = b
            .links
            .iter()
            .filter_map(|&l| match self.scripts.objects.get(l) {
                Some(Some(ScriptObject::Link(link))) => Some(Link {
                    from: link.source.map_or("?".into(), |s| self.pin_label(s, i)),
                    to: link.target.map_or("?".into(), |t| self.pin_label(t, i)),
                    delay: link.delay,
                }),
                _ => None,
            })
            .collect();
        let operations = b
            .operations
            .iter()
            .filter_map(|&o| match self.scripts.objects.get(o) {
                Some(Some(ScriptObject::Operation(op))) => Some(Operation {
                    index: o,
                    name: self.name(o),
                    guid: op.operation,
                    inputs: [op.input_a, op.input_b]
                        .into_iter()
                        .flatten()
                        .map(|p| format!("{} = {}", self.name(p), self.source(p)))
                        .collect(),
                    output: op.output.map_or("?".into(), |p| self.name(p)),
                }),
                _ => None,
            })
            .collect();
        let bb = b
            .prototype
            .and_then(|p| crate::Catalogue::builtin().building_block(p.guid));
        Some(Node {
            index: i,
            name: self.name(i),
            prototype: b.prototype.map(|p| p.guid),
            building_block: bb.map(|bb| bb.name.clone()),
            dll: bb.map(|bb| bb.dll.clone()),
            target: b.target_parameter.map(|t| self.param_in(t)),
            inputs: pins(&b.inputs),
            outputs: pins(&b.outputs),
            params_in: b.input_params.iter().map(|&p| self.param_in(p)).collect(),
            params_out: b
                .output_params
                .iter()
                .map(|&p| self.value_param(p))
                .collect(),
            locals: b
                .local_params
                .iter()
                .map(|&p| self.value_param(p))
                .collect(),
            operations,
            links,
            children: b
                .sub_behaviors
                .iter()
                .filter_map(|&c| self.node(c))
                .collect(),
        })
    }
}

/// Indented text rendering of a node tree.
pub fn write_tree(out: &mut String, n: &Node, depth: usize) {
    let ind = "  ".repeat(depth);
    let kind = match (&n.building_block, n.prototype) {
        (Some(bb), _) if *bb == n.name => n.dll.clone().unwrap_or_else(|| "BB".into()),
        (Some(bb), _) => format!("BB {bb} · {}", n.dll.as_deref().unwrap_or("?")),
        (None, Some(g)) => format!("unknown BB {g}"),
        (None, None) => "graph".into(),
    };
    let _ = writeln!(out, "{ind}[{}] {}  ({kind})", n.index, n.name);
    let pins = |p: &[Pin]| {
        p.iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !n.inputs.is_empty() || !n.outputs.is_empty() {
        let _ = writeln!(
            out,
            "{ind}    pins: in [{}]  out [{}]",
            pins(&n.inputs),
            pins(&n.outputs)
        );
    }
    if let Some(t) = &n.target {
        let _ = writeln!(out, "{ind}    target: {} = {}", t.type_name, t.value);
    }
    for (label, list) in [
        ("in", &n.params_in),
        ("out", &n.params_out),
        ("local", &n.locals),
    ] {
        for p in list {
            let _ = writeln!(
                out,
                "{ind}    {label:<5} {:<28} {:<14} {}",
                p.name, p.type_name, p.value
            );
        }
    }
    for op in &n.operations {
        let _ = writeln!(
            out,
            "{ind}    op    {} -> {}  [{}]",
            op.name,
            op.output,
            op.inputs.join("; ")
        );
    }
    for c in &n.children {
        write_tree(out, c, depth + 1);
    }
    for l in &n.links {
        let d = if l.delay > 0 {
            format!("  (delay {})", l.delay)
        } else {
            String::new()
        };
        let _ = writeln!(out, "{ind}    link  {} -> {}{d}", l.from, l.to);
    }
}

fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Graphviz rendering of one graph level (children as boxes, links as edges).
pub fn to_dot(n: &Node) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "digraph \"{}\" {{", dot_escape(&n.name));
    let _ = writeln!(
        s,
        "  rankdir=LR; node [shape=box, fontname=\"Helvetica\", fontsize=10];"
    );
    let _ = writeln!(
        s,
        "  this [label=\"{}\", shape=ellipse];",
        dot_escape(&n.name)
    );
    for c in &n.children {
        let mut label = match &c.building_block {
            Some(bb) if *bb != c.name => format!("{}\n<{}>", dot_escape(&c.name), dot_escape(bb)),
            _ => dot_escape(&c.name),
        };
        for p in &c.params_in {
            let _ = write!(
                label,
                "\\n{} = {}",
                dot_escape(&p.name),
                dot_escape(&truncate(&p.value, 40))
            );
        }
        let shape = if c.prototype.is_some() {
            "box"
        } else {
            "box3d"
        };
        let _ = writeln!(s, "  n{} [label=\"{label}\", shape={shape}];", c.index);
    }
    for l in &n.links {
        let (a, ap) = dot_end(&l.from);
        let (b, bp) = dot_end(&l.to);
        let mut attrs = format!(
            "taillabel=\"{}\", headlabel=\"{}\"",
            dot_escape(&ap),
            dot_escape(&bp)
        );
        if l.delay > 0 {
            let _ = write!(attrs, ", label=\"+{}\"", l.delay);
        }
        let _ = writeln!(s, "  {a} -> {b} [{attrs}, fontsize=8];");
    }
    s.push_str("}\n");
    s
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_owned()
    }
}

/// Splits a pin label (`[12] Name.pin` or `this.pin`) into a DOT node id and pin name.
fn dot_end(label: &str) -> (String, String) {
    if let Some(rest) = label.strip_prefix("this.") {
        return ("this".into(), rest.into());
    }
    if let Some(rest) = label.strip_prefix('[')
        && let Some((idx, tail)) = rest.split_once(']')
    {
        let pin = tail.rsplit_once('.').map_or("", |(_, p)| p);
        return (format!("n{idx}"), pin.into());
    }
    ("unknown".into(), label.into())
}

/// Convenience: is this value a plain constant (not an object, chunk or manager value)?
pub fn is_constant(v: &ParamValue) -> bool {
    matches!(v, ParamValue::Buffer(_) | ParamValue::Custom(_))
}
