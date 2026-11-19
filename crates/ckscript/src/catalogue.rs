//! The building-block catalogue: what each prototype GUID is, its pins and
//! parameter declarations, and every registered parameter type.
//!
//! Extracted from the game's DLLs (names, GUIDs and declarations only) in the
//! research repository and embedded here as `data/bb-catalogue.json`.

use std::collections::HashMap;
use std::sync::OnceLock;

use ckfile::{ClassId, Guid};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDecl {
    pub name: String,
    pub type_guid: Guid,
    pub default: Option<String>,
    /// Declared as a setting (settings share the local list).
    pub setting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildingBlock {
    pub guid: Guid,
    pub name: String,
    pub category: String,
    pub dll: String,
    /// Referenced by the game's files.
    pub used: bool,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub in_params: Vec<ParamDecl>,
    pub out_params: Vec<ParamDecl>,
    /// Locals and settings, in declaration order.
    pub locals: Vec<ParamDecl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    /// A plain or derived value type.
    Plain,
    /// Reference to an object of a class.
    Class(ClassId),
    Enum,
    Flags,
    Struct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamType {
    pub guid: Guid,
    pub name: String,
    pub kind: TypeKind,
    /// Enum / flag labels.
    pub values: Vec<(String, i32)>,
    pub derived_from: Option<Guid>,
    /// Struct members (name, type).
    pub members: Vec<(String, Guid)>,
}

#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    blocks: HashMap<Guid, BuildingBlock>,
    types: HashMap<Guid, ParamType>,
}

#[derive(Deserialize)]
struct RawCatalogue {
    building_blocks: Vec<RawBlock>,
    parameter_types: HashMap<String, RawType>,
}

#[derive(Deserialize)]
struct RawBlock {
    guid: String,
    name: String,
    category: Option<String>,
    dll: Option<String>,
    #[serde(default)]
    used: bool,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default)]
    in_params: Vec<RawParam>,
    #[serde(default)]
    out_params: Vec<RawParam>,
    #[serde(default)]
    locals: Vec<RawParam>,
}

#[derive(Deserialize)]
struct RawParam {
    name: Option<String>,
    #[serde(rename = "type")]
    ty: String,
    default: Option<String>,
    #[serde(default)]
    setting: bool,
}

#[derive(Deserialize)]
struct RawType {
    name: Option<String>,
    kind: String,
    values: Option<String>,
    derived_from: Option<String>,
    class_id: Option<u32>,
    members: Option<String>,
    member_types: Option<Vec<String>>,
}

/// Parses `"d1-d2"` (lowercase hex words).
pub fn parse_guid(s: &str) -> Option<Guid> {
    let (a, b) = s.split_once('-')?;
    Some(Guid(
        u32::from_str_radix(a, 16).ok()?,
        u32::from_str_radix(b, 16).ok()?,
    ))
}

fn parse_values(s: &str) -> Vec<(String, i32)> {
    s.split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().to_owned(), v.trim().parse().ok()?))
        })
        .collect()
}

impl Catalogue {
    /// The catalogue built into this crate.
    ///
    /// # Panics
    ///
    /// Never in practice: the embedded data is checked by this crate's tests.
    pub fn builtin() -> &'static Catalogue {
        static CAT: OnceLock<Catalogue> = OnceLock::new();
        CAT.get_or_init(|| {
            Catalogue::from_json(include_str!("../data/bb-catalogue.json"))
                .expect("embedded catalogue is valid")
        })
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let raw: RawCatalogue = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let guid = |s: &str| parse_guid(s).ok_or_else(|| format!("bad GUID {s:?}"));
        let params = |v: Vec<RawParam>| -> Result<Vec<ParamDecl>, String> {
            v.into_iter()
                .map(|p| {
                    Ok(ParamDecl {
                        type_guid: guid(&p.ty)?,
                        name: p.name.unwrap_or_default(),
                        default: p.default,
                        setting: p.setting,
                    })
                })
                .collect()
        };
        let mut blocks = HashMap::new();
        for b in raw.building_blocks {
            let g = guid(&b.guid)?;
            blocks.insert(
                g,
                BuildingBlock {
                    guid: g,
                    name: b.name,
                    category: b.category.unwrap_or_default(),
                    dll: b.dll.unwrap_or_default(),
                    used: b.used,
                    inputs: b.inputs,
                    outputs: b.outputs,
                    in_params: params(b.in_params)?,
                    out_params: params(b.out_params)?,
                    locals: params(b.locals)?,
                },
            );
        }
        let mut types = HashMap::new();
        for (g, t) in raw.parameter_types {
            let g = guid(&g)?;
            let kind = match (t.kind.as_str(), t.class_id) {
                ("class", Some(c)) => TypeKind::Class(ClassId(c)),
                ("enum", _) => TypeKind::Enum,
                ("flags", _) => TypeKind::Flags,
                ("struct", _) => TypeKind::Struct,
                _ => TypeKind::Plain,
            };
            let members = match (t.members, t.member_types) {
                (Some(names), Some(tys)) => names
                    .split(',')
                    .zip(tys)
                    .filter_map(|(n, ty)| Some((n.trim().to_owned(), parse_guid(&ty)?)))
                    .collect(),
                _ => Vec::new(),
            };
            types.insert(
                g,
                ParamType {
                    guid: g,
                    name: t.name.unwrap_or_default(),
                    kind,
                    values: t.values.as_deref().map(parse_values).unwrap_or_default(),
                    derived_from: t
                        .derived_from
                        .as_deref()
                        .and_then(parse_guid)
                        .filter(|d| *d != Guid::NULL),
                    members,
                },
            );
        }
        Ok(Self { blocks, types })
    }

    pub fn building_block(&self, g: Guid) -> Option<&BuildingBlock> {
        self.blocks.get(&g)
    }

    pub fn building_blocks(&self) -> impl Iterator<Item = &BuildingBlock> {
        self.blocks.values()
    }

    pub fn param_type(&self, g: Guid) -> Option<&ParamType> {
        self.types.get(&g)
    }

    /// Follows `derived_from` to the root type (e.g. Angle → Float).
    pub fn root_type(&self, mut g: Guid) -> Guid {
        for _ in 0..16 {
            match self.types.get(&g).and_then(|t| t.derived_from) {
                Some(parent) => g = parent,
                None => break,
            }
        }
        g
    }

    /// Label for an enum value, or `A | B` for flags.
    pub fn value_label(&self, g: Guid, v: i32) -> Option<String> {
        let t = self.types.get(&g)?;
        match t.kind {
            TypeKind::Enum => t
                .values
                .iter()
                .find(|(_, x)| *x == v)
                .map(|(n, _)| n.clone()),
            TypeKind::Flags => {
                if v == 0 {
                    return Some("none".into());
                }
                let names: Vec<&str> = t
                    .values
                    .iter()
                    .filter(|(_, bit)| *bit != 0 && v & bit == *bit)
                    .map(|(n, _)| n.as_str())
                    .collect();
                (!names.is_empty()).then(|| names.join(" | "))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalogue_loads() {
        let c = Catalogue::builtin();
        let phys = c
            .building_block(Guid(0x7522_370e, 0x37ec_15ec))
            .expect("Physicalize");
        assert_eq!(phys.name, "Physicalize");
        assert_eq!(phys.inputs, vec!["Physicalize", "Unphysicalize"]);
        assert_eq!(phys.in_params[1].name, "Friction");
        assert_eq!(phys.in_params[1].default.as_deref(), Some("0.7"));
        assert!(c.building_blocks().filter(|b| b.used).count() >= 184);
    }

    #[test]
    fn type_derivation_and_labels() {
        let c = Catalogue::builtin();
        let float = Guid(0x4788_4c3f, 0x432c_2c20);
        let angle = Guid(0x1126_2cf5, 0x30b0_233a);
        assert_eq!(c.root_type(angle), float);
        let blend = Guid(0x089e_28d4, 0x2ef0_31d2);
        assert_eq!(c.value_label(blend, 5).as_deref(), Some("Source Alpha"));
        assert!(matches!(
            c.param_type(Guid(0x024d_52f1, 0x6782_23b2)).unwrap().kind,
            TypeKind::Class(ClassId::DATA_ARRAY)
        ));
    }
}
