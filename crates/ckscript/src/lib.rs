//! Decoders for Virtools behaviour graphs.
//!
//! A script is a tree of [`Behavior`]s. A *graph* behaviour holds
//! sub-behaviours, [`BehaviorLink`]s (control flow between [`BehaviorIo`] pins)
//! and [`ParameterOperation`]s. A *building block* is a native node identified
//! by its prototype GUID. Data flows through parameters: each
//! [`ParameterIn`] reads from a [`ParameterLocal`] (a constant) or a
//! [`ParameterOut`], or shares another input.
//!
//! As in `ckscene`, object references are **file indices** ([`ObjRef`]).
//! The layouts were derived from the game's own files; see the research
//! repository (`findings/08-script-graphs.md`).

mod array;
mod behavior;
pub mod catalogue;
pub mod conform;
pub mod describe;
pub mod extract;
mod graph;
mod param;
pub mod types;

pub use array::{Cell, Column, ColumnType, DataArray};
pub use behavior::{Behavior, BehaviorIo, BehaviorLink, Prototype, behavior_flags, list_flags};
pub use catalogue::{BuildingBlock, Catalogue, ParamDecl, ParamType, TypeKind};
pub use graph::{ScriptObject, Scripts, decode_script_object, script_lists};
pub use param::{ParamValue, ParameterIn, ParameterLocal, ParameterOperation, ParameterOut};

/// Object reference: index into [`ckfile::CkFile::objects`].
pub type ObjRef = usize;

pub(crate) fn obj_ref(raw: u32) -> Option<ObjRef> {
    (raw != u32::MAX).then_some(raw as usize)
}

pub(crate) fn missing(class: &str, id: u32) -> ckfile::Error {
    ckfile::Error::BadChunk(format!("{class}: required section {id:#x} missing"))
}

pub(crate) fn bad(msg: impl Into<String>) -> ckfile::Error {
    ckfile::Error::BadChunk(msg.into())
}

#[cfg(test)]
mod tests;
