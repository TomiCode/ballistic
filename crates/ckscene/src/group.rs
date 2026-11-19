use ckfile::{Result, StateChunk};

use crate::{ObjRef, missing, obj_ref};

const SECTION_MEMBERS: u32 = 0xF_FFFF;

/// A named set of objects. Ballance encodes gameplay meaning in group names
/// (`Phys_Floors`, `PC_Checkpoints`, `Sector_01`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Group {
    pub members: Vec<ObjRef>,
}

impl Group {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_MEMBERS)?
            .ok_or_else(|| missing("Group", SECTION_MEMBERS))?;
        let n = r.u32()? as usize;
        let members = (0..n)
            .map(|_| r.u32().map(obj_ref))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(Self { members })
    }
}
