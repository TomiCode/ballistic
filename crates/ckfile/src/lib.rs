//! Reader for Virtools composition files (`.nmo` / `.cmo`), as used by *Ballance*.
//!
//! Only **file version 8** (Virtools 2.1 era) is supported. That is the version
//! every file in a retail Ballance install uses.
//!
//! The crate is purely structural. It decodes the container (header, object
//! table, plugin dependencies, manager/object data) and the generic
//! [`StateChunk`] serialisation unit each object is stored in. Decoding what a
//! chunk *means* for a given class (mesh, material, behaviour, …) lives in
//! higher layers.
//!
//! ```no_run
//! let file = ckfile::CkFile::open("Level_01.NMO")?;
//! for obj in &file.objects {
//!     println!("{} {} {:?}", obj.id, obj.class_id, obj.name);
//! }
//! # Ok::<(), ckfile::Error>(())
//! ```

mod bytes;
mod chunk;
mod class_id;
mod error;
mod file;
mod guid;
mod header;
mod text;

pub use chunk::{ChunkReader, ChunkSection, StateChunk, chunk_options};
pub use class_id::ClassId;
pub use error::{Error, Result};
pub use file::{CkFile, FileObject, ManagerData, PluginCategory, PluginDependency};
pub use guid::Guid;
pub use header::{FileHeader, write_mode};
pub use text::decode_cp1252;
