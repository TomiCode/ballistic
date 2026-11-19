use ckfile::{ChunkReader, Guid, Result, StateChunk};

use crate::{ObjRef, bad, missing, obj_ref};

const SECTION_VALUE: u32 = 0x40;
const SECTION_DESTINATIONS: u32 = 0x20;
const SECTION_IN_SOURCE: u32 = 0x1000;
const SECTION_IN_SHARED: u32 = 0x800;
const SECTION_IN_DISABLED: u32 = 0x2000;
const SECTION_LOCAL_SETTING: u32 = 0x400;
const SECTION_LOCAL_MYSELF: u32 = 0x200;
const SECTION_OPERATION: u32 = 0x400;

/// How a parameter's value is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ParamValue {
    /// Raw bytes, interpreted according to the parameter type.
    Buffer(Vec<u8>),
    /// Words written by a type-specific serialiser (no size prefix), e.g. a
    /// GUID for "parameter type" parameters.
    Custom(Vec<u32>),
    /// Reference to another object (`None` = null reference).
    Object(Option<ObjRef>),
    /// A value serialised as a nested state chunk (curves, handles, …);
    /// `None` when the saved chunk is empty.
    Chunk(Option<StateChunk>),
    /// A value scoped to a manager (e.g. a message or attribute type index).
    Manager { manager: Guid, value: i32 },
    /// No value saved.
    None,
}

/// Reads the `[type GUID][value]` record shared by locals and outputs.
fn read_typed_value(chunk: &StateChunk, mut r: ChunkReader<'_>) -> Result<(Guid, ParamValue)> {
    let type_guid = r.guid()?;
    let pos = r.pos() as u32;
    // A manager-scoped value is flagged by the chunk's manager position list.
    if chunk.manager_positions.contains(&pos) {
        let manager = r.guid()?;
        return Ok((
            type_guid,
            ParamValue::Manager {
                manager,
                value: r.i32()?,
            },
        ));
    }
    let value = match r.u32()? {
        0 => {
            let mut peek = r.clone();
            if peek.u32()? == 0 {
                r.skip(1)?;
                ParamValue::Chunk(None)
            } else {
                ParamValue::Chunk(Some(r.sub_chunk()?))
            }
        }
        1 => {
            // Size-prefixed buffer, unless the size doesn't fit exactly: then
            // the type wrote its own representation.
            let mut peek = r.clone();
            let size = peek.u32()? as usize;
            if size.div_ceil(4) == peek.remaining() {
                ParamValue::Buffer(r.buffer()?)
            } else {
                ParamValue::Custom(r.dwords(r.remaining())?.to_vec())
            }
        }
        2 => ParamValue::Object(obj_ref(r.u32()?)),
        3 => ParamValue::None,
        other => return Err(bad(format!("parameter: unknown value kind {other}"))),
    };
    if !r.is_empty() {
        return Err(bad(format!("parameter: {} unread words", r.remaining())));
    }
    Ok((type_guid, value))
}

/// A constant or variable owned by a behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ParameterLocal {
    pub type_guid: Guid,
    pub value: ParamValue,
    /// Marked as a behaviour setting (section `0x400`; tentative).
    pub is_setting: bool,
    /// A "This" parameter: refers to the behaviour's owner object, no stored value.
    pub is_myself: bool,
}

impl ParameterLocal {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let sections = chunk.sections()?;
        let has = |id| sections.iter().any(|s| s.identifier == id);
        let is_myself = has(SECTION_LOCAL_MYSELF);
        let (type_guid, value) = match chunk.section(SECTION_VALUE)? {
            Some(r) => read_typed_value(chunk, r)?,
            // "This" parameters carry no value: they stand for the owner object.
            None if is_myself => (Guid::NULL, ParamValue::None),
            None => return Err(missing("ParameterLocal", SECTION_VALUE)),
        };
        Ok(Self {
            type_guid,
            value,
            is_setting: has(SECTION_LOCAL_SETTING),
            is_myself,
        })
    }
}

/// An output of a behaviour or operation; pushes its value to destinations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ParameterOut {
    pub type_guid: Guid,
    pub value: ParamValue,
    /// Locals/outputs this output also writes to.
    pub destinations: Vec<ObjRef>,
}

impl ParameterOut {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let r = chunk
            .section(SECTION_VALUE)?
            .ok_or_else(|| missing("ParameterOut", SECTION_VALUE))?;
        let (type_guid, value) = read_typed_value(chunk, r)?;
        let destinations = match chunk.section(SECTION_DESTINATIONS)? {
            Some(mut d) => {
                let n = d.u32()? as usize;
                (0..n)
                    .map(|_| d.u32().map(obj_ref))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .flatten()
                    .collect()
            }
            None => Vec::new(),
        };
        Ok(Self {
            type_guid,
            value,
            destinations,
        })
    }
}

/// An input of a behaviour or operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ParameterIn {
    pub type_guid: Guid,
    /// Where the value comes from: a local, an output, or nothing.
    pub source: Option<ObjRef>,
    /// Instead of a source, this input shares another input.
    pub shared: Option<ObjRef>,
    /// Section `0x2000` present: the input is disabled (optional pins; tentative).
    pub disabled: bool,
}

impl ParameterIn {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let disabled = chunk.section(SECTION_IN_DISABLED)?.is_some();
        if let Some(mut r) = chunk.section(SECTION_IN_SOURCE)? {
            return Ok(Self {
                type_guid: r.guid()?,
                source: obj_ref(r.u32()?),
                shared: None,
                disabled,
            });
        }
        let mut r = chunk
            .section(SECTION_IN_SHARED)?
            .ok_or_else(|| missing("ParameterIn", SECTION_IN_SOURCE))?;
        Ok(Self {
            type_guid: r.guid()?,
            source: None,
            shared: obj_ref(r.u32()?),
            disabled,
        })
    }
}

/// An inline expression node: `output = op(input_a, input_b)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ParameterOperation {
    pub operation: Guid,
    /// Raw word after the GUID (`3` throughout Ballance).
    pub mode: u32,
    pub input_a: Option<ObjRef>,
    pub input_b: Option<ObjRef>,
    pub output: Option<ObjRef>,
}

impl ParameterOperation {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_OPERATION)?
            .ok_or_else(|| missing("ParameterOperation", SECTION_OPERATION))?;
        Ok(Self {
            operation: r.guid()?,
            mode: r.u32()?,
            input_a: obj_ref(r.u32()?),
            input_b: obj_ref(r.u32()?),
            output: obj_ref(r.u32()?),
        })
    }
}
