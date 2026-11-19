use crate::bytes::Cursor;
use crate::error::{Error, Result};

pub(crate) const SIGNATURE: &[u8; 8] = b"Nemo Fi\0";
pub(crate) const HEADER_SIZE: usize = 64;

/// Bits of [`FileHeader::write_mode`].
pub mod write_mode {
    /// The file was saved for a runtime/viewer (editor data stripped).
    pub const FOR_VIEWER: u32 = 0x4;
    /// Header1 and data sections are zlib streams.
    pub const WHOLE_COMPRESSED: u32 = 0x8;
}

/// The fixed 64-byte file header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileHeader {
    pub crc: u32,
    pub ck_version: u32,
    pub file_version: u32,
    pub write_mode: u32,
    pub header1_packed_size: u32,
    pub data_packed_size: u32,
    pub data_unpacked_size: u32,
    pub manager_count: u32,
    pub object_count: u32,
    pub max_id_saved: u32,
    pub product_version: u32,
    pub product_build: u32,
    pub header1_unpacked_size: u32,
}

impl FileHeader {
    pub(crate) fn parse(c: &mut Cursor<'_>) -> Result<Self> {
        if c.take(8, "signature")? != SIGNATURE {
            return Err(Error::BadSignature);
        }
        let crc = c.u32("crc")?;
        let ck_version = c.u32("ck version")?;
        let file_version = c.u32("file version")?;
        if file_version != 8 {
            return Err(Error::UnsupportedVersion(file_version));
        }
        let _reserved = c.u32("reserved")?;
        let write_mode = c.u32("write mode")?;
        let header1_packed_size = c.u32("header1 packed size")?;
        Ok(Self {
            crc,
            ck_version,
            file_version,
            write_mode,
            header1_packed_size,
            data_packed_size: c.u32("data packed size")?,
            data_unpacked_size: c.u32("data unpacked size")?,
            manager_count: c.u32("manager count")?,
            object_count: c.u32("object count")?,
            max_id_saved: c.u32("max id")?,
            product_version: c.u32("product version")?,
            product_build: c.u32("product build")?,
            header1_unpacked_size: c.u32("header1 unpacked size")?,
        })
    }
}
