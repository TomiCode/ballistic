use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;

use crate::bytes::Cursor;
use crate::chunk::StateChunk;
use crate::class_id::ClassId;
use crate::error::{Error, Result};
use crate::guid::Guid;
use crate::header::{FileHeader, HEADER_SIZE};
use crate::text::decode_cp1252;

/// A parsed Virtools file.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CkFile {
    pub header: FileHeader,
    /// Objects in file order. Object references inside chunks are **indices
    /// into this vector** (`-1` meaning "none").
    pub objects: Vec<FileObject>,
    pub managers: Vec<ManagerData>,
    pub plugin_dependencies: Vec<PluginDependency>,
}

/// One entry of the object table plus its serialised state.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileObject {
    /// Object ID at save time. Only meaningful for diagnostics, since references
    /// use file indices.
    pub id: u32,
    pub class_id: ClassId,
    /// Offset of this object's data, as stored in the object table.
    pub file_offset: u32,
    pub name: Option<String>,
    /// `None` when the object was saved without data.
    pub chunk: Option<StateChunk>,
}

/// Per-manager data saved in the file (e.g. attribute or message manager state).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ManagerData {
    pub guid: Guid,
    pub chunk: Option<StateChunk>,
}

/// Plugin category of a [`PluginDependency`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PluginCategory {
    BitmapReader,
    SoundReader,
    ModelReader,
    Manager,
    Behavior,
    RenderEngine,
    MovieReader,
    Extension,
    Other(u32),
}

impl From<u32> for PluginCategory {
    fn from(v: u32) -> Self {
        match v {
            0 => Self::BitmapReader,
            1 => Self::SoundReader,
            2 => Self::ModelReader,
            3 => Self::Manager,
            4 => Self::Behavior,
            5 => Self::RenderEngine,
            6 => Self::MovieReader,
            7 => Self::Extension,
            other => Self::Other(other),
        }
    }
}

/// Plugins (by GUID) that the file needs in order to load.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PluginDependency {
    pub category: PluginCategory,
    pub guids: Vec<Guid>,
}

impl CkFile {
    /// Reads and parses a file from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::parse(&std::fs::read(path)?)
    }

    /// Parses a complete file image.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(raw);
        let header = FileHeader::parse(&mut c)?;
        debug_assert_eq!(c.pos(), HEADER_SIZE);

        let h1_packed = c.take(header.header1_packed_size as usize, "header1")?;
        let header1 =
            inflate_if_packed(h1_packed, header.header1_unpacked_size as usize, "header1")?;
        let data_packed = c.take(header.data_packed_size as usize, "data section")?;
        let data = inflate_if_packed(data_packed, header.data_unpacked_size as usize, "data")?;

        let (mut objects, plugin_dependencies) = parse_header1(&header, &header1)?;
        let managers = parse_data(&header, &data, &mut objects)?;

        Ok(Self {
            header,
            objects,
            managers,
            plugin_dependencies,
        })
    }

    /// Looks up an object by the file index used in chunk references.
    /// Negative values (the "null" reference) return `None`.
    pub fn object(&self, index: i32) -> Option<&FileObject> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.objects.get(i))
    }

    /// All behaviour-plugin GUIDs this file depends on.
    pub fn behavior_guids(&self) -> impl Iterator<Item = Guid> + '_ {
        self.plugin_dependencies
            .iter()
            .filter(|d| d.category == PluginCategory::Behavior)
            .flat_map(|d| d.guids.iter().copied())
    }
}

fn inflate_if_packed(packed: &[u8], unpacked_size: usize, what: &'static str) -> Result<Vec<u8>> {
    if packed.len() == unpacked_size {
        return Ok(packed.to_vec());
    }
    let mut out = Vec::with_capacity(unpacked_size);
    ZlibDecoder::new(packed)
        .read_to_end(&mut out)
        .map_err(|source| Error::Decompress { what, source })?;
    if out.len() != unpacked_size {
        return Err(Error::SizeMismatch {
            what,
            expected: unpacked_size,
            actual: out.len(),
        });
    }
    Ok(out)
}

fn parse_header1(
    header: &FileHeader,
    buf: &[u8],
) -> Result<(Vec<FileObject>, Vec<PluginDependency>)> {
    let mut c = Cursor::new(buf);

    let mut objects = Vec::with_capacity(header.object_count as usize);
    for _ in 0..header.object_count {
        let id = c.u32("object id")?;
        let class_id = ClassId(c.u32("object class id")?);
        let file_offset = c.u32("object file offset")?;
        let name_len = c.len("object name length")?;
        let name = if name_len > 0 {
            Some(decode_cp1252(c.take(name_len, "object name")?))
        } else {
            None
        };
        objects.push(FileObject {
            id,
            class_id,
            file_offset,
            name,
            chunk: None,
        });
    }

    let dep_count = c.len("plugin dependency count")?;
    let mut deps = Vec::with_capacity(dep_count);
    for _ in 0..dep_count {
        let category = PluginCategory::from(c.u32("plugin category")?);
        let n = c.len("plugin guid count")?;
        let guids = (0..n)
            .map(|_| Ok(Guid(c.u32("guid")?, c.u32("guid")?)))
            .collect::<Result<Vec<_>>>()?;
        deps.push(PluginDependency { category, guids });
    }

    // Version 8 appends the included-files table size and count.
    if c.remaining() >= 8 {
        let _included_size = c.u32("included files size")?;
        if c.u32("included files count")? != 0 {
            return Err(Error::Unsupported("files with embedded included files"));
        }
    }

    Ok((objects, deps))
}

fn parse_data(
    header: &FileHeader,
    buf: &[u8],
    objects: &mut [FileObject],
) -> Result<Vec<ManagerData>> {
    let mut c = Cursor::new(buf);

    let mut managers = Vec::with_capacity(header.manager_count as usize);
    for _ in 0..header.manager_count {
        let guid = Guid(c.u32("manager guid")?, c.u32("manager guid")?);
        let size = c.len("manager chunk size")?;
        let chunk = parse_chunk_bytes(c.take(size, "manager chunk")?)?;
        managers.push(ManagerData { guid, chunk });
    }

    for obj in objects.iter_mut() {
        let size = c.len("object chunk size")?;
        obj.chunk = parse_chunk_bytes(c.take(size, "object chunk")?)?;
    }

    if c.remaining() != 0 {
        return Err(Error::BadChunk(format!(
            "{} unexpected trailing bytes in data section",
            c.remaining()
        )));
    }
    Ok(managers)
}

fn parse_chunk_bytes(bytes: &[u8]) -> Result<Option<StateChunk>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    StateChunk::parse(bytes).map(Some)
}
