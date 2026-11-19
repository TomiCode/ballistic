use ckfile::{Result, StateChunk};

use crate::{ObjRef, missing, obj_ref};

const SECTION_MATERIAL: u32 = 0x1000;
const SECTION_EFFECT: u32 = 0x1_0000;

macro_rules! raw_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident = $v:literal,)* }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        pub enum $name {
            $($var,)*
            Unknown(u8),
        }
        impl From<u8> for $name {
            fn from(v: u8) -> Self {
                match v {
                    $($v => Self::$var,)*
                    other => Self::Unknown(other),
                }
            }
        }
    };
}

raw_enum!(
    /// How the texture colour combines with the lit vertex colour.
    TextureBlend { Decal = 1, Modulate = 2, DecalAlpha = 3, ModulateAlpha = 4,
                   DecalMask = 5, ModulateMask = 6, Copy = 7, Add = 8, }
);
raw_enum!(FilterMode { Nearest = 1, Linear = 2, MipNearest = 3, MipLinear = 4,
                       LinearMipNearest = 5, LinearMipLinear = 6, Anisotropic = 7, });
raw_enum!(BlendFactor { Zero = 1, One = 2, SrcColor = 3, InvSrcColor = 4, SrcAlpha = 5,
                        InvSrcAlpha = 6, DestAlpha = 7, InvDestAlpha = 8, DestColor = 9,
                        InvDestColor = 10, SrcAlphaSat = 11, });
raw_enum!(ShadeMode { Flat = 1, Gouraud = 2, Phong = 3, });
raw_enum!(FillMode { Point = 1, Wireframe = 2, Solid = 3, });
raw_enum!(AddressMode { Wrap = 1, Mirror = 2, Clamp = 3, Border = 4, MirrorOnce = 5, });
raw_enum!(CompareFunc { Never = 1, Less = 2, Equal = 3, LessEqual = 4, Greater = 5,
                        NotEqual = 6, GreaterEqual = 7, Always = 8, });

/// A fixed-function material.
///
/// Colours are raw `0xAARRGGBB` (use [`crate::argb_to_rgba`]).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Material {
    pub diffuse: u32,
    pub ambient: u32,
    pub specular: u32,
    pub emissive: u32,
    pub specular_power: f32,
    pub texture: Option<ObjRef>,
    pub texture_border_color: u32,
    pub texture_blend: TextureBlend,
    pub min_filter: FilterMode,
    pub mag_filter: FilterMode,
    pub src_blend: BlendFactor,
    pub dest_blend: BlendFactor,
    pub shade_mode: ShadeMode,
    pub fill_mode: FillMode,
    pub address_mode: AddressMode,
    /// Raw flag byte. Interpreted (tentatively) by the accessors below.
    pub flags: u8,
    pub z_func: CompareFunc,
    pub alpha_func: CompareFunc,
    pub alpha_ref: u8,
    /// Optional material effect parameter (e.g. `TexGen Type` for environment mapping).
    pub effect: Option<ObjRef>,
}

impl Material {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_MATERIAL)?
            .ok_or_else(|| missing("Material", SECTION_MATERIAL))?;
        let diffuse = r.u32()?;
        let ambient = r.u32()?;
        let specular = r.u32()?;
        let emissive = r.u32()?;
        let specular_power = r.f32()?;
        let texture = obj_ref(r.u32()?);
        let texture_border_color = r.u32()?;
        let modes = r.u32()?;
        let misc = r.u32()?;
        let nib = |i: u32| ((modes >> (i * 4)) & 0xF) as u8;
        let byte = |i: u32| ((misc >> (i * 8)) & 0xFF) as u8;

        let effect = match chunk.section(SECTION_EFFECT)? {
            Some(mut e) => obj_ref(e.u32()?),
            None => None,
        };

        Ok(Self {
            diffuse,
            ambient,
            specular,
            emissive,
            specular_power,
            texture,
            texture_border_color,
            texture_blend: nib(0).into(),
            min_filter: nib(1).into(),
            mag_filter: nib(2).into(),
            src_blend: nib(3).into(),
            dest_blend: nib(4).into(),
            shade_mode: nib(5).into(),
            fill_mode: nib(6).into(),
            address_mode: nib(7).into(),
            flags: byte(0),
            z_func: byte(1).into(),
            alpha_func: byte(2).into(),
            alpha_ref: byte(3),
            effect,
        })
    }

    /// Render both faces. Tentative: only set on light-cone gradients.
    pub fn two_sided(&self) -> bool {
        self.flags & 0x01 != 0
    }

    pub fn z_write(&self) -> bool {
        self.flags & 0x02 != 0
    }

    pub fn alpha_blend(&self) -> bool {
        self.flags & 0x08 != 0
    }

    pub fn alpha_test(&self) -> bool {
        self.flags & 0x10 != 0
    }
}
