use ckfile::{Result, StateChunk};

use crate::{ObjRef, missing, obj_ref};

const SECTION_SPRITE: u32 = 0x40_0000;

/// A 3D sprite's own data (its placement is the [`crate::Entity3d`] part of
/// the same chunk): a material on a 1 × 1 quad scaled by the entity, facing
/// the camera.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Sprite3d {
    /// 0: a billboard facing the camera (the only mode in Ballance).
    pub mode: u32,
    /// The quad's pivot within it (0.5, 0.5: centred).
    pub offset: [f32; 2],
    /// Not decoded (0, 0 in every sprite).
    pub unknown: [f32; 2],
    /// The part of the texture drawn: left, top, right, bottom in UV.
    pub uv: [f32; 4],
    pub material: Option<ObjRef>,
}

impl Sprite3d {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_SPRITE)?
            .ok_or_else(|| missing("Sprite3D", SECTION_SPRITE))?;
        let mode = r.u32()?;
        let offset = [r.f32()?, r.f32()?];
        let unknown = [r.f32()?, r.f32()?];
        let uv = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let material = obj_ref(r.u32()?);
        Ok(Self {
            mode,
            offset,
            unknown,
            uv,
            material,
        })
    }
}
