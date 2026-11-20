use ckfile::{Result, StateChunk};

use crate::missing;

const SECTION_LIGHT: u32 = 0x40_0000;

/// A light's own data (its placement is the [`crate::Entity3d`] part of the
/// same chunk; a light shines along its Z axis).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Light {
    /// 1 point, 2 spot, 3 directional.
    pub kind: u8,
    /// The type word's upper bits (not decoded).
    pub flags: u32,
    /// `0xAARRGGBB`.
    pub color: u32,
    /// Constant, linear and quadratic attenuation.
    pub attenuation: [f32; 3],
    pub range: f32,
}

impl Light {
    pub const POINT: u8 = 1;
    pub const SPOT: u8 = 2;
    pub const DIRECTIONAL: u8 = 3;

    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_LIGHT)?
            .ok_or_else(|| missing("Light", SECTION_LIGHT))?;
        let word = r.u32()?;
        let color = r.u32()?;
        let attenuation = [r.f32()?, r.f32()?, r.f32()?];
        let range = r.f32()?;
        Ok(Self {
            kind: (word & 0xFF) as u8,
            flags: word >> 8,
            color,
            attenuation,
            range,
        })
    }
}
