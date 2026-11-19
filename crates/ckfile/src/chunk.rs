//! `CKStateChunk`: Virtools' generic serialisation unit.
//!
//! A chunk is a buffer of 32-bit words. Inside that buffer, data is grouped into
//! **identifier sections** that form a singly linked list:
//!
//! ```text
//! data[p]     = identifier
//! data[p + 1] = index of the next section header (0 = this is the last one)
//! data[p + 2 ..next] = payload
//! ```
//!
//! Alongside the data the chunk may carry position lists that tell the engine
//! where object references, sub-chunks and manager-scoped values live. They are
//! kept here so higher layers can cross-check their decoding.

use std::fmt;

use crate::bytes::{Cursor, to_dwords};
use crate::class_id::ClassId;
use crate::error::{Error, Result};
use crate::guid::Guid;
use crate::text::decode_cp1252;

/// Bits of [`StateChunk::options`].
pub mod chunk_options {
    /// An object-reference position list follows the data.
    pub const IDS: u32 = 0x01;
    /// A manager-value position list follows the data.
    pub const MANAGERS: u32 = 0x02;
    /// A sub-chunk position list follows the data.
    pub const CHUNKS: u32 = 0x04;
    /// The chunk was written as part of a file. Object references are file indices.
    pub const FILE: u32 = 0x08;
}

/// The only chunk layout version found in Ballance files.
pub const SUPPORTED_CHUNK_VERSION: u32 = 7;

#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StateChunk {
    /// Class that wrote the chunk. Can differ from the object's own class: a
    /// `3dObject` stores `3dEntity` here. Sub-chunks without a class use
    /// `0xFFFF_FFFF`.
    pub class_id: ClassId,
    /// Class-specific data version.
    pub data_version: u32,
    /// Layout version of the chunk itself (always 7 here).
    pub chunk_version: u32,
    /// [`chunk_options`] flags (top-level chunks only, 0 for sub-chunks).
    pub options: u32,
    pub data: Vec<u32>,
    /// Positions in `data` that hold object references.
    pub object_positions: Vec<u32>,
    /// Positions in `data` where sub-chunks start.
    pub sub_chunk_positions: Vec<u32>,
    /// Positions in `data` of manager-scoped values (`guid` followed by value).
    pub manager_positions: Vec<u32>,
}

impl fmt::Debug for StateChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateChunk")
            .field("class_id", &self.class_id)
            .field("data_version", &self.data_version)
            .field("chunk_version", &self.chunk_version)
            .field("options", &format_args!("{:#x}", self.options))
            .field("data_len", &self.data.len())
            .field("objects", &self.object_positions.len())
            .field("sub_chunks", &self.sub_chunk_positions.len())
            .field("managers", &self.manager_positions.len())
            .finish()
    }
}

/// One identifier section of a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChunkSection {
    pub identifier: u32,
    /// Index of the first payload word in [`StateChunk::data`].
    pub start: usize,
    /// Payload length in words.
    pub len: usize,
}

impl StateChunk {
    /// Parses a top-level chunk as stored in a file's data section.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if !bytes.len().is_multiple_of(4) {
            return Err(Error::BadChunk(format!(
                "length {} is not a multiple of 4",
                bytes.len()
            )));
        }
        let mut c = Cursor::new(bytes);
        let head = c.u32("chunk header")?;
        let data_version = head & 0xFF;
        let class_id = ClassId((head >> 8) & 0xFF);
        let chunk_version = (head >> 16) & 0xFF;
        let options = head >> 24;
        if chunk_version != SUPPORTED_CHUNK_VERSION {
            return Err(Error::BadChunk(format!(
                "unsupported chunk version {chunk_version}"
            )));
        }
        let data_len = c.len("chunk data size")?;
        let data = to_dwords(c.take(data_len * 4, "chunk data")?);

        let mut read_list = |flag: u32, what: &'static str| -> Result<Vec<u32>> {
            if options & flag == 0 {
                return Ok(Vec::new());
            }
            let n = c.len(what)?;
            Ok(to_dwords(c.take(n * 4, what)?))
        };
        // Order: objects, sub-chunks, managers. Only CHUNKS and MANAGERS (never
        // together) occur in Ballance, so the relative order is unverified.
        let object_positions = read_list(chunk_options::IDS, "chunk object list")?;
        let sub_chunk_positions = read_list(chunk_options::CHUNKS, "chunk sub-chunk list")?;
        let manager_positions = read_list(chunk_options::MANAGERS, "chunk manager list")?;

        if c.remaining() != 0 {
            return Err(Error::BadChunk(format!(
                "{} trailing bytes after chunk lists",
                c.remaining()
            )));
        }

        Ok(Self {
            class_id,
            data_version,
            chunk_version,
            options,
            data,
            object_positions,
            sub_chunk_positions,
            manager_positions,
        })
    }

    /// Walks the identifier list.
    pub fn sections(&self) -> Result<Vec<ChunkSection>> {
        let mut out = Vec::new();
        if self.data.is_empty() {
            return Ok(out);
        }
        let len = self.data.len();
        let mut pos = 0usize;
        loop {
            if pos + 2 > len {
                return Err(Error::BadChunk(format!(
                    "identifier header at {pos} past end ({len})"
                )));
            }
            let identifier = self.data[pos];
            let next = self.data[pos + 1] as usize;
            let end = if next == 0 { len } else { next };
            if end < pos + 2 || end > len {
                return Err(Error::BadChunk(format!(
                    "identifier {identifier:#x} at {pos} links to invalid position {next}"
                )));
            }
            out.push(ChunkSection {
                identifier,
                start: pos + 2,
                len: end - pos - 2,
            });
            if next == 0 {
                return Ok(out);
            }
            pos = next;
        }
    }

    /// Reader over the payload of the first section with this identifier.
    pub fn section(&self, identifier: u32) -> Result<Option<ChunkReader<'_>>> {
        Ok(self
            .sections()?
            .into_iter()
            .find(|s| s.identifier == identifier)
            .map(|s| ChunkReader {
                data: &self.data,
                pos: s.start,
                end: s.start + s.len,
            }))
    }

    /// Reader over the whole data buffer (identifier headers included).
    pub fn reader(&self) -> ChunkReader<'_> {
        ChunkReader {
            data: &self.data,
            pos: 0,
            end: self.data.len(),
        }
    }

    /// Parses every sub-chunk listed in [`Self::sub_chunk_positions`].
    pub fn sub_chunks(&self) -> Result<Vec<StateChunk>> {
        self.sub_chunk_positions
            .iter()
            .map(|&p| {
                let mut r = ChunkReader {
                    data: &self.data,
                    pos: p as usize,
                    end: self.data.len(),
                };
                r.sub_chunk()
            })
            .collect()
    }
}

/// Sequential reader over a range of a chunk's data words.
#[derive(Debug, Clone)]
pub struct ChunkReader<'a> {
    data: &'a [u32],
    pos: usize,
    end: usize,
}

impl<'a> ChunkReader<'a> {
    fn eof(&self, what: &str) -> Error {
        Error::BadChunk(format!(
            "read past end of section while reading {what} at word {}",
            self.pos
        ))
    }

    /// Current word index in the chunk data.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Words left in this section.
    pub fn remaining(&self) -> usize {
        self.end - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn dwords(&mut self, n: usize) -> Result<&'a [u32]> {
        if self.remaining() < n {
            return Err(self.eof("dwords"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.dwords(n).map(|_| ())
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(self.dwords(1)?[0])
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()?.cast_signed())
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u32()? != 0)
    }

    pub fn guid(&mut self) -> Result<Guid> {
        Ok(Guid(self.u32()?, self.u32()?))
    }

    /// An object reference: an index into [`crate::CkFile::objects`], or `-1`.
    pub fn object_ref(&mut self) -> Result<i32> {
        self.i32()
    }

    /// Raw bytes spanning `byte_len` bytes, padded to a whole number of words.
    pub fn raw_bytes(&mut self, byte_len: usize) -> Result<Vec<u8>> {
        let words = self.dwords(byte_len.div_ceil(4))?;
        let mut out: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        out.truncate(byte_len);
        Ok(out)
    }

    /// Length-prefixed (in bytes) buffer.
    pub fn buffer(&mut self) -> Result<Vec<u8>> {
        let n = self.u32()? as usize;
        self.raw_bytes(n)
    }

    /// Length-prefixed (in bytes, NUL included) string. `None` if the length is 0.
    pub fn string(&mut self) -> Result<Option<String>> {
        let n = self.u32()? as usize;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(decode_cp1252(&self.raw_bytes(n)?)))
    }

    /// An embedded sub-chunk:
    ///
    /// ```text
    /// size, class_id, version (chunk << 16 | data), data_len, has_file,
    /// object_count, sub_chunk_count, manager_count,
    /// data[data_len], objects[..], sub_chunks[..], managers[..]
    /// ```
    ///
    /// `size` counts the words that follow it.
    pub fn sub_chunk(&mut self) -> Result<StateChunk> {
        let size = self.u32()? as usize;
        let start = self.pos;
        let class_id = ClassId(self.u32()?);
        let version = self.u32()?;
        let data_len = self.u32()? as usize;
        let _has_file = self.u32()?;
        let n_objects = self.u32()? as usize;
        let n_chunks = self.u32()? as usize;
        let n_managers = self.u32()? as usize;
        let data = self.dwords(data_len)?.to_vec();
        let object_positions = self.dwords(n_objects)?.to_vec();
        let sub_chunk_positions = self.dwords(n_chunks)?.to_vec();
        let manager_positions = self.dwords(n_managers)?.to_vec();
        let consumed = self.pos - start;
        if consumed != size {
            return Err(Error::BadChunk(format!(
                "sub-chunk declared {size} words but contained {consumed}"
            )));
        }
        let chunk_version = version >> 16;
        if chunk_version != SUPPORTED_CHUNK_VERSION {
            return Err(Error::BadChunk(format!(
                "unsupported sub-chunk version {chunk_version}"
            )));
        }
        Ok(StateChunk {
            class_id,
            data_version: version & 0xFFFF,
            chunk_version,
            options: 0,
            data,
            object_positions,
            sub_chunk_positions,
            manager_positions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// Two sections: 0x10 holding a string and a float, 0x20 holding a group-like list.
    fn sample() -> Vec<u32> {
        vec![
            // section 0x10, next section at word 6
            0x10,
            6,
            6, // string length in bytes, NUL included: "Brick\0"
            u32::from_le_bytes(*b"Bric"),
            u32::from_le_bytes([b'k', 0, 0, 0]),
            1.5f32.to_bits(),
            // section 0x20, last
            0x20,
            0,
            2,
            1,
            3,
        ]
    }

    #[test]
    fn parses_top_level_chunk_and_sections() {
        let data = sample();
        let mut words = vec![0x0807_170A, data.len() as u32];
        words.extend(&data);
        let chunk = StateChunk::parse(&bytes(&words)).unwrap();
        assert_eq!(chunk.class_id, ClassId::GROUP);
        assert_eq!(chunk.data_version, 10);
        assert_eq!(chunk.options, chunk_options::FILE);

        let sections = chunk.sections().unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(
            sections[0],
            ChunkSection {
                identifier: 0x10,
                start: 2,
                len: 4
            }
        );

        let mut r = chunk.section(0x10).unwrap().unwrap();
        assert_eq!(r.string().unwrap().as_deref(), Some("Brick"));
        assert!((r.f32().unwrap() - 1.5).abs() < f32::EPSILON);
        assert!(r.is_empty());

        let mut r = chunk.section(0x20).unwrap().unwrap();
        assert_eq!(r.u32().unwrap(), 2);
        assert_eq!((r.object_ref().unwrap(), r.object_ref().unwrap()), (1, 3));
        assert!(r.u32().is_err());
    }

    #[test]
    fn parses_sub_chunks_from_position_list() {
        let inner = [0x1, 0, 0x73];
        // class, version, data_len, has_file, n_objects, n_chunks, n_managers, data.., objects..
        let sub = [
            u32::MAX,
            0x7_0000,
            3,
            0,
            1,
            0,
            0,
            inner[0],
            inner[1],
            inner[2],
            2,
        ];
        let mut data = vec![0x10, 0, sub.len() as u32];
        data.extend_from_slice(&sub);
        let mut words = vec![0x0C07_080A, data.len() as u32];
        words.extend(&data);
        words.extend([1, 2]); // sub-chunk list: one entry at word 2
        let chunk = StateChunk::parse(&bytes(&words)).unwrap();
        assert_eq!(chunk.sub_chunk_positions, vec![2]);
        let subs = chunk.sub_chunks().unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].data, inner);
        assert_eq!(subs[0].object_positions, vec![2]);
        assert_eq!(subs[0].class_id, ClassId(u32::MAX));
    }

    #[test]
    fn rejects_broken_links() {
        let words = [0x0807_170A, 3, 0x10, 99, 0];
        let chunk = StateChunk::parse(&bytes(&words)).unwrap();
        assert!(chunk.sections().is_err());
    }

    #[test]
    fn rejects_trailing_garbage() {
        let words = [0x0807_170A, 2, 0x10, 0, 0xDEAD];
        assert!(StateChunk::parse(&bytes(&words)).is_err());
    }
}
