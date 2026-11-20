//! The Ballistic asset bundle (`.bpk`): the only data the engine reads.
//!
//! See `docs/bundle-format.md` for the specification. This crate holds the
//! shared types plus a [`BundleReader`] / [`BundleWriter`] over the zip container.
//! It knows nothing about the original game's formats.

use flate2 as _;

mod archive;
mod manifest;
mod physics;
mod scene;

pub use archive::{BundleReader, BundleWriter};
pub use manifest::{
    AudioEncoding, Manifest, PackOptions, SceneEntry, SoundEntry, SourceInfo, TextureEncoding,
    TextureEntry, TextureFormat,
};
pub use physics::{
    BallJointParams, BallParams, BallPieces, BallShape, BallSoundSet, BallSounds, CloudLayer,
    CurvePoint, EmitterShape, EnergyParams, Environment, EventSound, ExtraLife, ExtraPoint,
    ForceParams, Glyph, HingeParams, HitTest, Hud, HudFont, HudGlow, HudLives, HudSprite, HudText,
    LevelEnvironment, LevelFlow, ModuleBody, ModuleSetup, ObjectSoundIds, ParticleEmitter,
    ParticleRender, PhysicsData, PieceSounds, Progression, PropParams, RingPart, SkyParams,
    SliderParams, SoundCue, SoundGroup, SpringAnchor, SpringParams, SurfaceParams, TextAlign,
    TransformerAnimation, TransformerParams, TransformerType, Varied, WakeParams, curve_y,
};
pub use scene::{
    AddressMode, BlendFactor, CompareFunc, Group, Material, Mesh, ObjectKind, Scene, SceneObject,
    Submesh, TextureBlend, Vertex,
};

/// Value of [`Manifest::format`].
pub const FORMAT_NAME: &str = "ballistic-bundle";
/// Version of the bundle layout. Bump on any incompatible change.
/// v2: adds `data/physics.json`.
pub const FORMAT_VERSION: u32 = 16;
/// Version of the `.bscn` scene encoding.
pub const SCENE_VERSION: u32 = 2;
/// Magic at the start of every `.bscn` entry.
pub const SCENE_MAGIC: &[u8; 8] = b"BLSCENE\0";

pub const MANIFEST_PATH: &str = "manifest.json";
pub const PHYSICS_PATH: &str = "data/physics.json";

/// Zip entry path of a scene.
pub fn scene_path(key: &str) -> String {
    format!("scenes/{key}.bscn")
}

/// Zip entry path of a texture.
pub fn texture_path(key: &str) -> String {
    format!("textures/{key}.ktx2")
}

/// Zip entry path of a sound, by encoding.
pub fn sound_path(key: &str, encoding: AudioEncoding) -> String {
    match encoding {
        AudioEncoding::Vorbis => format!("sounds/{key}.ogg"),
        AudioEncoding::Wav => format!("sounds/{key}.wav"),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("manifest error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("scene encoding error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error(
        "unsupported bundle (format {found_name:?} v{found_version}, expected {FORMAT_NAME:?} v{FORMAT_VERSION}); re-run ballistic-pack"
    )]
    Unsupported {
        found_name: String,
        found_version: u32,
    },
    #[error("corrupt entry {0}: {1}")]
    Corrupt(String, &'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests;
