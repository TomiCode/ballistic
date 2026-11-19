//! Little-endian byte cursor used by the container parser.

use crate::error::{Error, Result};

pub(crate) struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub(crate) fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::Truncated {
                what,
                offset: self.pos,
            });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads a `u32` length and converts it to `usize`.
    pub(crate) fn len(&mut self, what: &'static str) -> Result<usize> {
        Ok(self.u32(what)? as usize)
    }
}

/// Reinterprets a byte slice as little-endian `u32`s. The length must be a multiple of 4.
pub(crate) fn to_dwords(bytes: &[u8]) -> Vec<u32> {
    let (words, _rest) = bytes.as_chunks::<4>();
    words.iter().map(|&w| u32::from_le_bytes(w)).collect()
}
