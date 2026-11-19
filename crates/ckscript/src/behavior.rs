use ckfile::{ChunkReader, ClassId, Guid, Result, StateChunk};

use crate::{ObjRef, bad, missing, obj_ref};

const SECTION_BEHAVIOR: u32 = 0x20;
const SECTION_SCRIPT: u32 = 0x20_0000;
const SECTION_IO: u32 = 0x8;
const SECTION_LINK: u32 = 0x20;

/// Bits of [`Behavior::flags`] that change the record layout.
pub mod behavior_flags {
    /// A priority word follows.
    pub const PRIORITY: u32 = 0x4;
    /// The node is a building block: prototype GUID and version follow.
    pub const BUILDING_BLOCK: u32 = 0x8;
    /// A compatible class id follows.
    pub const COMPATIBLE_CLASS: u32 = 0x10;
    /// A target parameter reference follows.
    pub const TARGET_PARAMETER: u32 = 0x4_0000;
}

/// Bits of [`Behavior::list_flags`]: which reference lists follow, in this order.
pub mod list_flags {
    /// Sub-behaviours, then links (graphs).
    pub const GRAPH: u32 = 0x100;
    /// Parameter operations (graphs).
    pub const OPERATIONS: u32 = 0x4000;
    pub const INPUT_PARAMS: u32 = 0x200;
    pub const OUTPUT_PARAMS: u32 = 0x400;
    pub const LOCAL_PARAMS: u32 = 0x2_0000;
    pub const INPUTS: u32 = 0x800;
    pub const OUTPUTS: u32 = 0x1000;
    /// Bits seen in the data that add no fields.
    pub const NO_DATA: u32 = 0x8_0000;
    pub const KNOWN: u32 = GRAPH
        | OPERATIONS
        | INPUT_PARAMS
        | OUTPUT_PARAMS
        | LOCAL_PARAMS
        | INPUTS
        | OUTPUTS
        | NO_DATA;
}

/// Prototype of a building block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Prototype {
    pub guid: Guid,
    pub version: u32,
}

/// A behaviour: either a graph or a building-block node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Behavior {
    /// Raw behaviour flags (see [`behavior_flags`]).
    pub flags: u32,
    /// Raw list flags (see [`list_flags`]).
    pub list_flags: u32,
    pub prototype: Option<Prototype>,
    pub priority: Option<i32>,
    pub compatible_class: Option<ClassId>,
    pub target_parameter: Option<ObjRef>,
    pub sub_behaviors: Vec<ObjRef>,
    pub links: Vec<ObjRef>,
    pub operations: Vec<ObjRef>,
    pub input_params: Vec<ObjRef>,
    pub output_params: Vec<ObjRef>,
    pub local_params: Vec<ObjRef>,
    pub inputs: Vec<ObjRef>,
    pub outputs: Vec<ObjRef>,
    /// Present on top-level scripts (raw value, `0x50` throughout Ballance).
    pub script_flags: Option<u32>,
}

fn list(r: &mut ChunkReader<'_>, present: bool) -> Result<Vec<ObjRef>> {
    if !present {
        return Ok(Vec::new());
    }
    let n = r.u32()? as usize;
    if n > r.remaining() {
        return Err(bad(format!("Behavior: list of {n} exceeds section")));
    }
    Ok(r.dwords(n)?.iter().filter_map(|&w| obj_ref(w)).collect())
}

impl Behavior {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_BEHAVIOR)?
            .ok_or_else(|| missing("Behavior", SECTION_BEHAVIOR))?;
        let flags = r.u32()?;
        let prototype = if flags & behavior_flags::BUILDING_BLOCK != 0 {
            Some(Prototype {
                guid: r.guid()?,
                version: r.u32()?,
            })
        } else {
            None
        };
        let priority = if flags & behavior_flags::PRIORITY != 0 {
            Some(r.i32()?)
        } else {
            None
        };
        let compatible_class = if flags & behavior_flags::COMPATIBLE_CLASS != 0 {
            Some(ClassId(r.u32()?))
        } else {
            None
        };
        let target_parameter = if flags & behavior_flags::TARGET_PARAMETER != 0 {
            obj_ref(r.u32()?)
        } else {
            None
        };
        let lf = r.u32()?;
        if lf & !list_flags::KNOWN != 0 {
            return Err(bad(format!(
                "Behavior: unknown list flags {:#x}",
                lf & !list_flags::KNOWN
            )));
        }
        let graph = lf & list_flags::GRAPH != 0;
        let sub_behaviors = list(&mut r, graph)?;
        let links = list(&mut r, graph)?;
        let operations = list(&mut r, lf & list_flags::OPERATIONS != 0)?;
        let input_params = list(&mut r, lf & list_flags::INPUT_PARAMS != 0)?;
        let output_params = list(&mut r, lf & list_flags::OUTPUT_PARAMS != 0)?;
        let local_params = list(&mut r, lf & list_flags::LOCAL_PARAMS != 0)?;
        let inputs = list(&mut r, lf & list_flags::INPUTS != 0)?;
        let outputs = list(&mut r, lf & list_flags::OUTPUTS != 0)?;
        if !r.is_empty() {
            return Err(bad(format!("Behavior: {} unread words", r.remaining())));
        }
        let script_flags = match chunk.section(SECTION_SCRIPT)? {
            Some(mut s) => Some(s.u32()?),
            None => None,
        };
        Ok(Self {
            flags,
            list_flags: lf,
            prototype,
            priority,
            compatible_class,
            target_parameter,
            sub_behaviors,
            links,
            operations,
            input_params,
            output_params,
            local_params,
            inputs,
            outputs,
            script_flags,
        })
    }

    pub fn is_graph(&self) -> bool {
        self.prototype.is_none()
    }
}

/// A control-flow pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BehaviorIo {
    /// Raw flags: `1` input, `2` output, `0x100` set on some script entry pins.
    pub flags: u32,
}

impl BehaviorIo {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_IO)?
            .ok_or_else(|| missing("BehaviorIO", SECTION_IO))?;
        Ok(Self { flags: r.u32()? })
    }

    pub fn is_input(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn is_output(&self) -> bool {
        self.flags & 2 != 0
    }
}

/// A control-flow edge between two pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BehaviorLink {
    /// Frames to wait before activating the target (low half of the delay word).
    pub delay: u16,
    /// High half of the delay word, equal to `delay` throughout Ballance
    /// (presumably the initial delay).
    pub initial_delay: u16,
    pub source: Option<ObjRef>,
    pub target: Option<ObjRef>,
}

impl BehaviorLink {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_LINK)?
            .ok_or_else(|| missing("BehaviorLink", SECTION_LINK))?;
        let d = r.u32()?;
        Ok(Self {
            delay: d as u16,
            initial_delay: (d >> 16) as u16,
            source: obj_ref(r.u32()?),
            target: obj_ref(r.u32()?),
        })
    }
}
