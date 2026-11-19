use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::{
    Error, FORMAT_NAME, FORMAT_VERSION, MANIFEST_PATH, Manifest, Result, SCENE_MAGIC,
    SCENE_VERSION, Scene,
};

/// Writes a bundle. Entries are written in the order given; for reproducible
/// output, callers should add them in a deterministic order.
pub struct BundleWriter<W: Write + Seek> {
    zip: ZipWriter<W>,
}

impl BundleWriter<BufWriter<File>> {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::new(BufWriter::new(File::create(path)?)))
    }
}

impl<W: Write + Seek> BundleWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            zip: ZipWriter::new(inner),
        }
    }

    fn options(compress: bool) -> SimpleFileOptions {
        SimpleFileOptions::default()
            .compression_method(if compress {
                CompressionMethod::Deflated
            } else {
                CompressionMethod::Stored
            })
            .last_modified_time(DateTime::default())
            .unix_permissions(0o644)
    }

    /// Adds a raw entry. `compress = false` for already-compressed payloads.
    pub fn add(&mut self, path: &str, bytes: &[u8], compress: bool) -> Result<()> {
        self.zip.start_file(path, Self::options(compress))?;
        self.zip.write_all(bytes)?;
        Ok(())
    }

    pub fn add_scene(&mut self, key: &str, scene: &Scene) -> Result<()> {
        let mut bytes = Vec::with_capacity(1 << 16);
        bytes.extend_from_slice(SCENE_MAGIC);
        bytes.extend_from_slice(&SCENE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&postcard::to_stdvec(scene)?);
        self.add(&crate::scene_path(key), &bytes, true)
    }

    pub fn add_physics(&mut self, physics: &crate::PhysicsData) -> Result<()> {
        let json = serde_json::to_vec_pretty(physics)?;
        self.add(crate::PHYSICS_PATH, &json, true)
    }

    /// Writes the manifest and finishes the archive.
    pub fn finish(mut self, manifest: &Manifest) -> Result<W> {
        let json = serde_json::to_vec_pretty(manifest)?;
        self.add(MANIFEST_PATH, &json, true)?;
        Ok(self.zip.finish()?)
    }
}

/// Reads a bundle, checking its format and version on open.
pub struct BundleReader<R: Read + Seek> {
    zip: ZipArchive<R>,
    manifest: Manifest,
}

impl BundleReader<BufReader<File>> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::new(BufReader::new(File::open(path)?))
    }
}

impl<R: Read + Seek> BundleReader<R> {
    pub fn new(inner: R) -> Result<Self> {
        let mut zip = ZipArchive::new(inner)?;
        let mut json = Vec::new();
        zip.by_name(MANIFEST_PATH)?.read_to_end(&mut json)?;
        // Check identity before full parsing, so future layouts give a clear error.
        let head: serde_json::Value = serde_json::from_slice(&json)?;
        let name = head["format"].as_str().unwrap_or_default().to_owned();
        let version = head["format_version"].as_u64().unwrap_or(0) as u32;
        if name != FORMAT_NAME || version != FORMAT_VERSION {
            return Err(Error::Unsupported {
                found_name: name,
                found_version: version,
            });
        }
        let manifest = serde_json::from_slice(&json)?;
        Ok(Self { zip, manifest })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let mut f = self.zip.by_name(path)?;
        let mut out = Vec::with_capacity(usize::try_from(f.size()).unwrap_or(0));
        f.read_to_end(&mut out)?;
        Ok(out)
    }

    pub fn scene(&mut self, key: &str) -> Result<Scene> {
        let path = crate::scene_path(key);
        let bytes = self.read(&path)?;
        if bytes.len() < 12 || &bytes[..8] != SCENE_MAGIC {
            return Err(Error::Corrupt(path, "bad scene magic"));
        }
        if bytes[8..12] != SCENE_VERSION.to_le_bytes() {
            return Err(Error::Corrupt(path, "unsupported scene version"));
        }
        Ok(postcard::from_bytes(&bytes[12..])?)
    }

    pub fn physics(&mut self) -> Result<crate::PhysicsData> {
        let bytes = self.read(crate::PHYSICS_PATH)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// KTX2 file bytes of a texture.
    pub fn texture(&mut self, key: &str) -> Result<Vec<u8>> {
        self.read(&crate::texture_path(key))
    }

    /// Encoded sound file bytes (`.ogg` or `.wav`, per the manifest options).
    pub fn sound(&mut self, key: &str) -> Result<Vec<u8>> {
        let enc = self.manifest.options.audio_encoding;
        self.read(&crate::sound_path(key, enc))
    }

    pub fn entry_names(&self) -> Result<Vec<String>> {
        Ok(self
            .zip
            .file_names()
            .map(|n| n.map(std::borrow::Cow::into_owned))
            .collect::<Result<_, _>>()?)
    }
}
