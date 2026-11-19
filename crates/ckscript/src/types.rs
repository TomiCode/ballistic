//! Parameter types: names and value formatting.
//!
//! The table below lists types whose meaning is evident from the game's data
//! (value sizes, contents and parameter names). Each entry notes its evidence.
//! It's meant to be extended or replaced by the building-block catalogue
//! extracted from the DLLs.

use ckfile::{CkFile, Guid};

use crate::ParamValue;
use crate::catalogue::{Catalogue, TypeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Float,
    Int,
    Bool,
    String,
    Vector2,
    Vector3,
    /// RGBA floats.
    Color,
    Matrix,
    /// Time in milliseconds stored as float.
    TimeMs,
    /// Rectangle as four floats.
    Rect,
    /// Integer enumeration whose labels aren't known yet.
    Enum,
    /// Bit flags.
    Flags,
    /// Object reference of some class.
    Object,
    Other,
}

pub struct TypeInfo {
    pub guid: Guid,
    pub name: &'static str,
    pub kind: ValueKind,
}

/// Evidence-based type table (see module docs).
pub const KNOWN_TYPES: &[TypeInfo] = &[
    // 4-byte values like 0x3f800000 (1.0), 0x3dcccccd (0.1); "Friction", "Mass".
    TypeInfo {
        guid: Guid(0x4788_4c3f, 0x432c_2c20),
        name: "Float",
        kind: ValueKind::Float,
    },
    // "Row Index", "Column Index", "Line Count": small integers.
    TypeInfo {
        guid: Guid(0x5a57_16fd, 0x44e2_76d7),
        name: "Integer",
        kind: ValueKind::Int,
    },
    // 0/1 only; "Hierarchy", "Key State", "Awake Object".
    TypeInfo {
        guid: Guid(0x1ad5_2a8e, 0x5e74_1920),
        name: "Boolean",
        kind: ValueKind::Bool,
    },
    // NUL-terminated text such as mesh names.
    TypeInfo {
        guid: Guid(0x6bd0_10e2, 0x1156_17ea),
        name: "String",
        kind: ValueKind::String,
    },
    // 12 bytes; "Position", "Direction", "Shift Mass Center".
    TypeInfo {
        guid: Guid(0x4882_4eae, 0x2fe4_7960),
        name: "Vector",
        kind: ValueKind::Vector3,
    },
    // 8 bytes; "Offset", "Shadow Size".
    TypeInfo {
        guid: Guid(0x4efc_b34a, 0x6079_e42f),
        name: "Vector 2D",
        kind: ValueKind::Vector2,
    },
    // 16 bytes of floats, usually 1.0; "C", "Variance".
    TypeInfo {
        guid: Guid(0x57d4_2fee, 0x7cbb_3b91),
        name: "Color",
        kind: ValueKind::Color,
    },
    // 64 bytes, identity by default; "Matrix", "BallMatrix".
    TypeInfo {
        guid: Guid(0x643f_046e, 0x6521_1b71),
        name: "Matrix",
        kind: ValueKind::Matrix,
    },
    // Float values such as 352.0 / 1000.0; "Duration", "Time to Wait".
    TypeInfo {
        guid: Guid(0x54b4_422b, 0x730f_0f4f),
        name: "Time",
        kind: ValueKind::TimeMs,
    },
    // 16 bytes; "Margins", "Text Extents".
    TypeInfo {
        guid: Guid(0x7ab2_0d20, 0x6930_44a9),
        name: "Rectangle",
        kind: ValueKind::Rect,
    },
    // Object references, named by their parameter names ("Target (3D Entity)", …).
    TypeInfo {
        guid: Guid(0x5b8a_05d5, 0x31ea_28d4),
        name: "3D Entity",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x024d_52f1, 0x6782_23b2),
        name: "Array",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x55ab_12cd, 0x22ae_6a8b),
        name: "Material",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x71d8_0779, 0x402f_42f3),
        name: "Behavioral Object",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x1816_71e3, 0x1bdc_1503),
        name: "2D Entity",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x7ea4_176d, 0x1b40_5d30),
        name: "Script",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x5c0f_151b, 0x6f05_47fd),
        name: "Group",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x4bf7_4e5e, 0x45f4_09ef),
        name: "Wave Sound",
        kind: ValueKind::Object,
    },
    TypeInfo {
        guid: Guid(0x155b_2870, 0x1836_79f8),
        name: "Texture",
        kind: ValueKind::Object,
    },
    // "Message", "Message 0": manager-scoped message type.
    TypeInfo {
        guid: Guid(0x0388_1e12, 0x5ba3_4e2b),
        name: "Message",
        kind: ValueKind::Enum,
    },
    // "Source Blend", "Destination Blend": values 2, 5 …
    TypeInfo {
        guid: Guid(0x089e_28d4, 0x2ef0_31d2),
        name: "Blend Factor",
        kind: ValueKind::Enum,
    },
    // "Class", "Filter Class": values like 23 (Group).
    TypeInfo {
        guid: Guid(0x1964_4e43, 0x4d6f_6123),
        name: "Registered Class",
        kind: ValueKind::Enum,
    },
    // "Key Waited", "Pin 0": key codes.
    TypeInfo {
        guid: Guid(0xfa6e_1bdd, 0x62d2_abd7),
        name: "Keyboard Key",
        kind: ValueKind::Enum,
    },
    // "Progression", "Following Speed": 0..1 floats.
    TypeInfo {
        guid: Guid(0xf3c8_4b4e, 0x0ffa_cc34),
        name: "Percentage",
        kind: ValueKind::Float,
    },
    // "Yaw Variance", "Roll": radians.
    TypeInfo {
        guid: Guid(0x1126_2cf5, 0x30b0_233a),
        name: "Angle",
        kind: ValueKind::Float,
    },
];

pub fn type_info(g: Guid) -> Option<&'static TypeInfo> {
    KNOWN_TYPES.iter().find(|t| t.guid == g)
}

/// Type name: from the catalogue, else the built-in table, else the GUID.
pub fn type_name(g: Guid) -> String {
    if let Some(t) = Catalogue::builtin().param_type(g)
        && !t.name.is_empty()
    {
        return t.name.clone();
    }
    type_info(g).map_or_else(|| format!("type {g}"), |t| t.name.to_owned())
}

/// How to decode a type's bytes: the built-in table for the type itself,
/// else for its root type in the catalogue's derivation chain.
pub fn value_kind(g: Guid) -> Option<ValueKind> {
    let cat = Catalogue::builtin();
    match cat.param_type(g).map(|t| t.kind) {
        Some(TypeKind::Enum) => return Some(ValueKind::Enum),
        Some(TypeKind::Flags) => return Some(ValueKind::Flags),
        Some(TypeKind::Class(_)) => return Some(ValueKind::Object),
        _ => {}
    }
    type_info(g)
        .or_else(|| type_info(cat.root_type(g)))
        .map(|t| t.kind)
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|&c| f32::from_le_bytes(c))
        .collect()
}

fn fmt_f(v: f32) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.to_owned()
    }
}

/// Human-readable value, using the type table where possible.
pub fn format_value(file: &CkFile, type_guid: Guid, v: &ParamValue) -> String {
    let kind = value_kind(type_guid);
    match v {
        ParamValue::None => "(no value)".into(),
        ParamValue::Object(None) => "null".into(),
        ParamValue::Object(Some(r)) => object_label(file, *r),
        ParamValue::Manager { manager, value } => format!("{value} (manager {manager})"),
        ParamValue::Chunk(None) => "(empty chunk)".into(),
        ParamValue::Chunk(Some(c)) => format!("(chunk, {} words)", c.data.len()),
        ParamValue::Custom(w) => {
            if w.len() == 2 {
                let g = Guid(w[0], w[1]);
                format!("{} [{g}]", type_name(g))
            } else {
                format!("custom {w:x?}")
            }
        }
        ParamValue::Buffer(b) => match (kind, b.len()) {
            (Some(ValueKind::Float | ValueKind::TimeMs), 4) => fmt_f(f32s(b)[0]),
            (Some(ValueKind::Int | ValueKind::Enum | ValueKind::Flags), 4) => {
                let n = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                match Catalogue::builtin().value_label(type_guid, n) {
                    Some(label) => format!("{label} ({n})"),
                    None => n.to_string(),
                }
            }
            (Some(ValueKind::Bool), 4) => (b[0] != 0).to_string(),
            (Some(ValueKind::String), _) => format!("{:?}", ckfile::decode_cp1252(b)),
            (
                Some(ValueKind::Vector2 | ValueKind::Vector3 | ValueKind::Color | ValueKind::Rect),
                _,
            )
            | (Some(ValueKind::Matrix), 64) => {
                let v: Vec<String> = f32s(b).into_iter().map(fmt_f).collect();
                format!("({})", v.join(", "))
            }
            (_, 4) => {
                let i = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let f = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if (-100_000..=100_000).contains(&i) {
                    i.to_string()
                } else if f.is_finite() && f.abs() > 1e-6 && f.abs() < 1e7 {
                    format!("{}f", fmt_f(f))
                } else {
                    format!("{i:#x}")
                }
            }
            (_, n)
                if n > 0 && b.iter().all(|&c| c == 0 || (0x20..0x7F).contains(&c)) && b[0] != 0 =>
            {
                format!("{:?}", ckfile::decode_cp1252(b))
            }
            (_, n) => format!("{n} bytes {:02x?}", &b[..n.min(16)]),
        },
    }
}

pub fn object_label(file: &CkFile, r: usize) -> String {
    match file.objects.get(r) {
        Some(o) => format!(
            "@{} ({} #{r})",
            o.name.as_deref().unwrap_or("<unnamed>"),
            o.class_id
        ),
        None => format!("@#{r}?"),
    }
}

/// Human-readable data-array cell; parameter cells show their parameter's value.
pub fn format_cell(file: &CkFile, scripts: &crate::Scripts, c: &crate::Cell) -> String {
    match c {
        crate::Cell::Int(v) => v.to_string(),
        crate::Cell::Float(v) => fmt_f(*v),
        crate::Cell::String(v) => format!("{v:?}"),
        crate::Cell::Object(None) | crate::Cell::Parameter(None) => "null".into(),
        crate::Cell::Object(Some(r)) => object_label(file, *r),
        crate::Cell::Parameter(Some(r)) => match scripts.objects.get(*r) {
            Some(Some(crate::ScriptObject::Parameter(p) | crate::ScriptObject::Local(p))) => {
                format_value(file, p.type_guid, &p.value)
            }
            Some(Some(crate::ScriptObject::Out(p))) => format_value(file, p.type_guid, &p.value),
            _ => object_label(file, *r),
        },
    }
}
