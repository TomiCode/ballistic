use ckfile::{Result, StateChunk};

use crate::{ObjRef, missing, obj_ref};

const SECTION_GEOMETRY: u32 = 0x10_F000;
const SECTION_MATERIAL: u32 = 0x20_0000;

/// Flag bit: a z-order word follows the two rectangles.
const FLAG_Z_ORDER: u32 = 0x2_0000;

/// A 2D entity: a screen rectangle drawn with (part of) a material's texture.
/// Ballance's HUD and menus are made of these.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Entity2d {
    pub flags: u32,
    /// Left, top, right, bottom. Ballance's are fractions of the screen.
    pub rect: [f32; 4],
    /// The part of the texture drawn: left, top, right, bottom in UV.
    pub source: [f32; 4],
    /// Drawing order (higher on top); 0 when not stored.
    pub z_order: i32,
    pub material: Option<ObjRef>,
    pub hidden: bool,
}

impl Entity2d {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_GEOMETRY)?
            .ok_or_else(|| missing("2dEntity", SECTION_GEOMETRY))?;
        let flags = r.u32()?;
        let mut rect = [0.0; 4];
        for v in &mut rect {
            *v = r.f32()?;
        }
        let mut source = [0.0; 4];
        for v in &mut source {
            *v = r.f32()?;
        }
        let z_order = if flags & FLAG_Z_ORDER != 0 {
            r.i32()?
        } else {
            0
        };
        let material = match chunk.section(SECTION_MATERIAL)? {
            Some(mut m) => obj_ref(m.u32()?),
            None => None,
        };
        Ok(Self {
            flags,
            rect,
            source,
            z_order,
            material,
            hidden: crate::is_hidden(chunk)?,
        })
    }
}
