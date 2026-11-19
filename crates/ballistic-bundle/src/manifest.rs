use serde::{Deserialize, Serialize};

/// `manifest.json`: what the bundle contains and how it was built.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub packer_version: String,
    pub source: SourceInfo,
    pub options: PackOptions,
    pub scenes: Vec<SceneEntry>,
    pub textures: Vec<TextureEntry>,
    pub sounds: Vec<SoundEntry>,
}

/// Which installation the bundle was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Recognised distribution (`"gog"`) or `"unknown"`.
    pub variant: String,
    /// Files whose hash matched the known distribution.
    pub verified_files: u32,
    /// Known files whose hash differs (modded or patched install).
    pub modified_files: u32,
    /// Used files with no known hash.
    pub unknown_files: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PackOptions {
    pub texture_encoding: TextureEncoding,
    pub audio_encoding: AudioEncoding,
    pub audio_quality: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextureEncoding {
    /// BC1 for opaque images, BC3 for images with alpha.
    Bc,
    /// Uncompressed RGBA8.
    Rgba8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioEncoding {
    Vorbis,
    Wav,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneEntry {
    pub key: String,
    pub objects: u32,
    pub meshes: u32,
    pub materials: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextureFormat {
    Bc1Srgb,
    Bc3Srgb,
    Rgba8Srgb,
}

impl TextureFormat {
    /// Vulkan format number stored in the KTX2 header.
    pub fn vk_format(self) -> u32 {
        match self {
            Self::Bc1Srgb => 132,
            Self::Bc3Srgb => 138,
            Self::Rgba8Srgb => 43,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureEntry {
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub mip_levels: u32,
    pub has_alpha: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundEntry {
    pub key: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Length in sample frames.
    pub frames: u64,
    pub format: AudioEncoding,
}
