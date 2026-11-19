use ckfile::{Result, StateChunk};

use crate::{is_hidden, missing};

const SECTION_FILES: u32 = 0x1_0000;
const SECTION_FORMAT: u32 = 0x8_0000;
const SECTION_OPTIONS: u32 = 0x2F_F000;

/// A texture. In Ballance every texture refers to an external image file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Texture {
    /// One file name per slot (Ballance only uses one slot). An empty name
    /// means a placeholder (e.g. a video target).
    pub files: Vec<String>,
    /// Image description saved with the file, when present.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits_per_pixel: Option<u32>,
    /// Extension of the original image (`"bmp"`, `"tga"`).
    pub source_format: Option<String>,
    /// Requested video-memory pixel format (raw: 1 for 32-bit with alpha, 6 for
    /// most BMPs; meaning tentative).
    pub video_format: Option<u32>,
    pub hidden: bool,
}

impl Texture {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let mut r = chunk
            .section(SECTION_FILES)?
            .ok_or_else(|| missing("Texture", SECTION_FILES))?;
        let slots = r.u32()? as usize;
        let files = (0..slots)
            .map(|_| r.string().map(Option::unwrap_or_default))
            .collect::<Result<Vec<_>>>()?;

        let (mut width, mut height, mut bpp, mut source_format) = (None, None, None, None);
        if let Some(mut f) = chunk.section(SECTION_FORMAT)? {
            // [props size][props size][reader guid ×2][extension (4 bytes)]
            // [desc size][flags][width][height][bytes per line][bits per pixel] …
            let w = f.dwords(f.remaining().min(11))?;
            if w.len() == 11 {
                source_format = Some(ckfile::decode_cp1252(&w[4].to_le_bytes()));
                width = Some(w[7]);
                height = Some(w[8]);
                bpp = Some(w[10]);
            }
        }
        let video_format = match chunk.section(SECTION_OPTIONS)? {
            Some(mut o) if o.remaining() >= 3 => {
                o.skip(2)?;
                Some(o.u32()?)
            }
            _ => None,
        };
        Ok(Self {
            files,
            width,
            height,
            bits_per_pixel: bpp,
            source_format,
            video_format,
            hidden: is_hidden(chunk)?,
        })
    }

    /// File name of the first slot, if non-empty.
    pub fn file(&self) -> Option<&str> {
        self.files
            .first()
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }
}
