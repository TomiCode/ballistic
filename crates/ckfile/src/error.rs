use std::io;

/// Errors produced while reading a Virtools file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("not a Virtools file (bad signature)")]
    BadSignature,

    #[error("unsupported file version {0} (only version 8 is supported)")]
    UnsupportedVersion(u32),

    #[error("unexpected end of data while reading {what} at offset {offset:#x}")]
    Truncated { what: &'static str, offset: usize },

    #[error("zlib decompression of {what} failed: {source}")]
    Decompress {
        what: &'static str,
        #[source]
        source: io::Error,
    },

    #[error("{what}: expected {expected} bytes after decompression, got {actual}")]
    SizeMismatch {
        what: &'static str,
        expected: usize,
        actual: usize,
    },

    #[error("malformed state chunk: {0}")]
    BadChunk(String),

    #[error("unsupported feature: {0}")]
    Unsupported(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
