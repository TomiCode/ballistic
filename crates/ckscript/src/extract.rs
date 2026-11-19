//! Extracts the effective parameters of building-block nodes: constants where
//! the scripts hard-code them, and the runtime source where they don't
//! ("written by `Get Row(Physicalize_GameBall).Friction`").

use std::collections::{HashMap, HashSet};

use ckfile::{CkFile, Guid};

use crate::types::{ValueKind, value_kind};
use crate::{BuildingBlock, Catalogue, ObjRef, ParamValue, ScriptObject, Scripts};

/// A typed constant.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Vector(Vec<f64>),
    Text(String),
    /// Object reference by name (`None` = null).
    Object(Option<String>),
    /// Anything without a typed decoding.
    Other(String),
}

/// What a parameter evaluates to.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum Resolved {
    /// Fixed in the file.
    Constant {
        value: Value,
    },
    /// Computed at runtime. `saved` is the value stored in the file (often a placeholder).
    Dynamic {
        from: String,
        saved: Option<Value>,
    },
    Unset,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NamedValue {
    pub name: String,
    pub type_name: String,
    pub value: Resolved,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NodeParams {
    pub node: ObjRef,
    /// Script names from the root down to the node's parent graph.
    pub path: Vec<String>,
    pub name: String,
    pub building_block: String,
    pub dll: String,
    /// The object the script is attached to.
    pub owner: Option<String>,
    pub target: Option<NamedValue>,
    pub inputs: Vec<NamedValue>,
    pub locals: Vec<NamedValue>,
}

/// Decodes a stored value using its parameter type.
pub fn typed_value(file: &CkFile, type_guid: Guid, v: &ParamValue) -> Value {
    let obj_name = |r: usize| file.objects.get(r).and_then(|o| o.name.clone());
    match v {
        ParamValue::Object(r) => Value::Object(r.and_then(obj_name)),
        ParamValue::Buffer(b) => {
            let f32s = || -> Vec<f64> {
                b.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&c| f64::from(f32::from_le_bytes(c)))
                    .collect()
            };
            let i = || {
                b.first_chunk::<4>()
                    .map(|&c| i64::from(i32::from_le_bytes(c)))
            };
            match value_kind(type_guid) {
                Some(ValueKind::Float | ValueKind::TimeMs) if b.len() == 4 => {
                    Value::Float(round(f32s()[0]))
                }
                Some(ValueKind::Bool) if b.len() == 4 => Value::Bool(b[0] != 0),
                Some(ValueKind::Int | ValueKind::Enum | ValueKind::Flags) if b.len() == 4 => {
                    Value::Int(i().unwrap_or_default())
                }
                Some(ValueKind::String) => Value::Text(ckfile::decode_cp1252(b)),
                Some(
                    ValueKind::Vector2
                    | ValueKind::Vector3
                    | ValueKind::Color
                    | ValueKind::Rect
                    | ValueKind::Matrix,
                ) => Value::Vector(f32s().into_iter().map(round).collect()),
                _ => Value::Other(crate::types::format_value(file, type_guid, v)),
            }
        }
        _ => Value::Other(crate::types::format_value(file, type_guid, v)),
    }
}

/// Rounds an f32-derived value to 6 significant decimals for readability.
fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

pub struct Extractor<'a> {
    file: &'a CkFile,
    scripts: &'a Scripts,
    /// Local/output → outputs that write into it at runtime.
    writers: HashMap<ObjRef, Vec<ObjRef>>,
}

impl<'a> Extractor<'a> {
    pub fn new(file: &'a CkFile, scripts: &'a Scripts) -> Self {
        let mut writers: HashMap<ObjRef, Vec<ObjRef>> = HashMap::new();
        for (i, o) in scripts.objects.iter().enumerate() {
            if let Some(ScriptObject::Out(p)) = o {
                for &d in &p.destinations {
                    writers.entry(d).or_default().push(i);
                }
            }
        }
        Self {
            file,
            scripts,
            writers,
        }
    }

    fn name(&self, i: ObjRef) -> String {
        self.file
            .objects
            .get(i)
            .and_then(|o| o.name.clone())
            .unwrap_or_else(|| "<unnamed>".into())
    }

    /// `Get Row(Physicalize_GameBall).Friction`-style label for an output.
    fn output_label(&self, out: ObjRef) -> String {
        let Some(&node) = self.scripts.owner.get(&out) else {
            return self.name(out);
        };
        let target = self
            .scripts
            .behavior(node)
            .and_then(|b| b.target_parameter)
            .and_then(|t| match self.resolve_input(t, &mut HashSet::new()) {
                Resolved::Constant {
                    value: Value::Object(Some(n)),
                } => Some(n),
                // Not fixed in the file: name the variable it's read from
                // (e.g. "Physicalize_GameBall Array").
                _ => match self.scripts.objects.get(t) {
                    Some(Some(ScriptObject::In(p))) => p.source.map(|s| self.name(s)),
                    _ => None,
                },
            });
        let bb = self
            .scripts
            .behavior(node)
            .and_then(|b| b.prototype)
            .and_then(|p| Catalogue::builtin().building_block(p.guid))
            .map_or_else(|| self.name(node), |b| b.name.clone());
        match target {
            Some(t) => format!("{bb}({t}).{}", self.name(out)),
            None => format!("{bb}.{}", self.name(out)),
        }
    }

    fn local_value(&self, local: ObjRef) -> Option<Value> {
        match self.scripts.objects.get(local) {
            Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) if !l.is_myself => {
                Some(typed_value(self.file, l.type_guid, &l.value))
            }
            Some(Some(ScriptObject::Out(o))) => Some(typed_value(self.file, o.type_guid, &o.value)),
            _ => None,
        }
    }

    /// Resolves a `ParameterIn` to a constant or its runtime source.
    pub fn resolve_input(&self, input: ObjRef, seen: &mut HashSet<ObjRef>) -> Resolved {
        if !seen.insert(input) {
            return Resolved::Dynamic {
                from: "cycle".into(),
                saved: None,
            };
        }
        let Some(Some(ScriptObject::In(p))) = self.scripts.objects.get(input) else {
            return Resolved::Unset;
        };
        if let Some(shared) = p.shared {
            return self.resolve_input(shared, seen);
        }
        let Some(src) = p.source else {
            return Resolved::Unset;
        };
        match self.scripts.objects.get(src) {
            Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) => {
                if l.is_myself {
                    return Resolved::Dynamic {
                        from: "this (owner object)".into(),
                        saved: None,
                    };
                }
                let saved = self.local_value(src);
                match self.writers.get(&src) {
                    Some(w) if !w.is_empty() => Resolved::Dynamic {
                        from: format!(
                            "variable {:?} written by {}",
                            self.name(src),
                            w.iter()
                                .map(|&o| self.output_label(o))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        saved,
                    },
                    _ => Resolved::Constant {
                        value: saved.unwrap_or(Value::Other("?".into())),
                    },
                }
            }
            Some(Some(ScriptObject::Out(_))) => Resolved::Dynamic {
                from: self.output_label(src),
                saved: self.local_value(src),
            },
            _ => Resolved::Unset,
        }
    }

    fn named_input(&self, i: ObjRef) -> NamedValue {
        let type_guid = match self.scripts.objects.get(i) {
            Some(Some(ScriptObject::In(p))) => p.type_guid,
            _ => Guid::NULL,
        };
        NamedValue {
            name: self.name(i),
            type_name: crate::types::type_name(type_guid),
            value: self.resolve_input(i, &mut HashSet::new()),
        }
    }

    fn named_local(&self, i: ObjRef) -> NamedValue {
        let type_guid = match self.scripts.objects.get(i) {
            Some(Some(ScriptObject::Local(l) | ScriptObject::Parameter(l))) => l.type_guid,
            _ => Guid::NULL,
        };
        NamedValue {
            name: self.name(i),
            type_name: crate::types::type_name(type_guid),
            value: self
                .local_value(i)
                .map_or(Resolved::Unset, |value| Resolved::Constant { value }),
        }
    }

    /// Every building-block node for which `select` returns true.
    pub fn nodes(&self, select: &dyn Fn(&BuildingBlock) -> bool) -> Vec<NodeParams> {
        let mut out = Vec::new();
        for &(root, owner) in &self.scripts.roots {
            let owner = owner.map(|o| self.name(o));
            self.walk(root, &mut Vec::new(), owner.as_deref(), select, &mut out);
        }
        out
    }

    fn walk(
        &self,
        node: ObjRef,
        path: &mut Vec<String>,
        owner: Option<&str>,
        select: &dyn Fn(&BuildingBlock) -> bool,
        out: &mut Vec<NodeParams>,
    ) {
        let Some(b) = self.scripts.behavior(node) else {
            return;
        };
        if let Some(proto) = b.prototype {
            if let Some(bb) = Catalogue::builtin().building_block(proto.guid)
                && select(bb)
            {
                out.push(NodeParams {
                    node,
                    path: path.clone(),
                    name: self.name(node),
                    building_block: bb.name.clone(),
                    dll: bb.dll.clone(),
                    owner: owner.map(str::to_owned),
                    target: b.target_parameter.map(|t| self.named_input(t)),
                    inputs: b
                        .input_params
                        .iter()
                        .map(|&p| self.named_input(p))
                        .collect(),
                    locals: b
                        .local_params
                        .iter()
                        .map(|&p| self.named_local(p))
                        .collect(),
                });
            }
            return;
        }
        path.push(self.name(node));
        for &c in &b.sub_behaviors {
            self.walk(c, path, owner, select, out);
        }
        path.pop();
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Vector(v) => {
                let s: Vec<String> = v.iter().map(ToString::to_string).collect();
                write!(f, "({})", s.join(", "))
            }
            Value::Text(t) => write!(f, "{t:?}"),
            Value::Object(Some(n)) => write!(f, "@{n}"),
            Value::Object(None) => f.write_str("null"),
            Value::Other(s) => f.write_str(s),
        }
    }
}

impl std::fmt::Display for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Resolved::Constant { value } => write!(f, "{value}"),
            Resolved::Dynamic { from, .. } => write!(f, "<{from}>"),
            Resolved::Unset => f.write_str("(unset)"),
        }
    }
}
