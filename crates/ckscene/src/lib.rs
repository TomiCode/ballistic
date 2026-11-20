//! Decoders for the scene classes stored in Virtools files: [`Mesh`],
//! [`Material`], [`Texture`], [`Entity3d`] (3D objects/entities), [`Entity2d`]
//! (screen rectangles) and [`Group`].
//!
//! Every decoder takes a [`ckfile::StateChunk`] and returns plain data.
//! Object references stay as **file indices** (`ObjRef`), and resolving them is
//! up to the caller (see [`ckfile::CkFile::object`]).
//!
//! Coordinate convention: Virtools is **left-handed, Y up**, and transforms
//! row vectors (`v' = v · M`, translation in the last row).
//!
//! The layouts were derived from the game's own files. See the research
//! repository (`findings/07-scene-classes.md`) for evidence and confidence.

mod decoded;
mod entity;
mod entity2d;
mod group;
mod light;
mod material;
mod mesh;
mod sprite;
mod texture;

pub use decoded::{Decoded, decode_object};
pub use entity::Entity3d;
pub use entity2d::Entity2d;
pub use group::Group;
pub use light::Light;
pub use material::{
    AddressMode, BlendFactor, CompareFunc, FillMode, FilterMode, Material, ShadeMode, TextureBlend,
};
pub use mesh::{Face, Mesh};
pub use sprite::Sprite3d;
pub use texture::Texture;

/// Object reference: index into [`ckfile::CkFile::objects`].
pub type ObjRef = usize;

/// Converts a raw stored reference (`-1` = none) into an [`ObjRef`].
pub(crate) fn obj_ref(raw: u32) -> Option<ObjRef> {
    (raw != u32::MAX).then_some(raw as usize)
}

/// Section identifier written by the object base class when the object is hidden.
pub const SECTION_HIDDEN: u32 = 0x4;

/// `true` if the chunk carries the "hidden" marker section.
pub fn is_hidden(chunk: &ckfile::StateChunk) -> ckfile::Result<bool> {
    Ok(chunk
        .sections()?
        .iter()
        .any(|s| s.identifier == SECTION_HIDDEN))
}

/// Unpacks `0xAARRGGBB` into `[r, g, b, a]` in `0.0..=1.0`.
pub fn argb_to_rgba(c: u32) -> [f32; 4] {
    let ch = |shift: u32| f32::from(((c >> shift) & 0xFF) as u8) / 255.0;
    [ch(16), ch(8), ch(0), ch(24)]
}

pub(crate) fn missing(class: &str, id: u32) -> ckfile::Error {
    ckfile::Error::BadChunk(format!("{class}: required section {id:#x} missing"))
}

#[cfg(test)]
mod tests;
