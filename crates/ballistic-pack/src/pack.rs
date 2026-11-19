//! The build pipeline: install → verify → convert (in parallel) → bundle.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use ballistic_bundle::{self as bb, AudioEncoding, BundleWriter, PackOptions, TextureEncoding};
use rayon::prelude::*;

use crate::install::{self, Install};
use crate::{audio, scenes, textures};

pub struct Options {
    pub from: Option<PathBuf>,
    pub out: PathBuf,
    pub texture_encoding: TextureEncoding,
    pub audio_encoding: AudioEncoding,
    pub audio_quality: f32,
}

pub struct Report {
    pub install: PathBuf,
    pub manifest: bb::Manifest,
    pub warnings: Vec<String>,
    pub bundle_bytes: u64,
    pub seconds: f32,
}

/// Inputs selected from an install, as `(key, source path)` sorted by key.
pub struct Inputs<'a> {
    pub scenes: Vec<(String, &'a Path)>,
    pub textures: Vec<(String, &'a Path)>,
    pub sounds: Vec<(String, &'a Path)>,
}

impl Inputs<'_> {
    /// Install-relative keys of every file the packer reads.
    pub fn source_keys(&self, install: &Install) -> Vec<String> {
        let root = &install.root;
        self.scenes
            .iter()
            .chain(&self.textures)
            .chain(&self.sounds)
            .map(|(_, p)| rel_key(root, p))
            .collect()
    }
}

/// Install keys are lowercase, so plain `ends_with` extension checks are correct.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn select(install: &Install) -> Inputs<'_> {
    let strip_ext = |s: &str| s.rsplit_once('.').map_or(s, |(a, _)| a).to_owned();

    let mut scenes: Vec<(String, &Path)> = install
        .under("3d entities/")
        .filter(|(k, _)| k.ends_with(".nmo") || k.ends_with(".cmo"))
        .map(|(k, p)| (strip_ext(&k["3d entities/".len()..]), p))
        .collect();
    if let Some(p) = install.get("base.cmo") {
        scenes.push(("base".into(), p));
    }
    scenes.sort();

    let textures = install
        .under("textures/")
        .filter(|(k, _)| {
            (k.ends_with(".bmp") || k.ends_with(".tga")) && !k.starts_with("textures/sky_low/")
        })
        .map(|(k, p)| (strip_ext(&k["textures/".len()..]), p))
        .collect();

    let sounds = install
        .under("sounds/")
        .filter(|(k, _)| k.ends_with(".wav") && !k["sounds/".len()..].contains('/'))
        .map(|(k, p)| (strip_ext(&k["sounds/".len()..]), p))
        .collect();

    Inputs {
        scenes,
        textures,
        sounds,
    }
}

fn rel_key(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

/// Maps material texture file names to texture keys.
struct Resolver {
    /// lowercase file name (with extension) → key; top-level images win.
    by_file: BTreeMap<String, String>,
}

impl Resolver {
    fn new(textures: &[(String, &Path)]) -> Self {
        let mut by_file = BTreeMap::new();
        // Deeper paths first, so top-level entries overwrite them.
        let mut sorted: Vec<_> = textures.iter().collect();
        sorted.sort_by_key(|(k, _)| std::cmp::Reverse(k.matches('/').count()));
        for (key, path) in sorted {
            if let Some(name) = path.file_name() {
                by_file.insert(name.to_string_lossy().to_lowercase(), key.clone());
            }
        }
        Self { by_file }
    }
}

impl scenes::TextureResolver for Resolver {
    fn resolve(&self, file_name: &str) -> Option<String> {
        let name = file_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(file_name)
            .to_lowercase();
        self.by_file.get(&name).cloned()
    }
}

pub fn build(opts: &Options) -> Result<Report> {
    let t0 = Instant::now();
    let install = Install::locate(opts.from.as_deref())?;
    let inputs = select(&install);
    let keys = inputs.source_keys(&install);
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let verification = install::verify(&install, &key_refs)?;

    let mut warnings = Vec::new();
    for m in &verification.modified {
        warnings.push(format!(
            "{m}: differs from the known {} release",
            verification.variant
        ));
    }

    // Textures.
    let encoded_textures: Vec<textures::EncodedTexture> = inputs
        .textures
        .par_iter()
        .map(|(key, path)| textures::encode_file(path, key, opts.texture_encoding))
        .collect::<Result<_>>()?;

    // Scenes.
    let resolver = Resolver::new(&inputs.textures);
    let converted: Vec<Option<(String, scenes::Converted)>> = inputs
        .scenes
        .par_iter()
        .map(|(key, path)| {
            let file = ckfile::CkFile::open(path)
                .with_context(|| format!("reading {}", path.display()))?;
            if !scenes::has_geometry(&file) {
                return Ok(None);
            }
            let c = scenes::convert(&file, &resolver)
                .with_context(|| format!("converting {}", path.display()))?;
            Ok(Some((key.clone(), c)))
        })
        .collect::<Result<_>>()?;

    // Sounds.
    let encoded_sounds: Vec<audio::EncodedSound> = inputs
        .sounds
        .par_iter()
        .map(|(key, path)| audio::encode_file(path, key, opts.audio_encoding, opts.audio_quality))
        .collect::<Result<_>>()?;

    // Write, in key order, to a temp file renamed at the end.
    let tmp = opts.out.with_extension("bpk.partial");
    let mut writer =
        BundleWriter::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    let mut scene_entries = Vec::new();
    for (key, c) in converted.into_iter().flatten() {
        for w in c.warnings {
            warnings.push(format!("scene {key}: {w}"));
        }
        writer.add_scene(&key, &c.scene)?;
        scene_entries.push(bb::SceneEntry {
            objects: c.scene.objects.len() as u32,
            meshes: c.scene.meshes.len() as u32,
            materials: c.scene.materials.len() as u32,
            key,
        });
    }
    let mut texture_entries = Vec::new();
    for t in encoded_textures {
        writer.add(&bb::texture_path(&t.entry.key), &t.ktx2, true)?;
        texture_entries.push(t.entry);
    }
    let mut sound_entries = Vec::new();
    for s in encoded_sounds {
        let compress = opts.audio_encoding == AudioEncoding::Wav;
        writer.add(
            &bb::sound_path(&s.entry.key, opts.audio_encoding),
            &s.bytes,
            compress,
        )?;
        sound_entries.push(s.entry);
    }

    let manifest = bb::Manifest {
        format: bb::FORMAT_NAME.into(),
        format_version: bb::FORMAT_VERSION,
        packer_version: env!("CARGO_PKG_VERSION").into(),
        source: bb::SourceInfo {
            variant: verification.variant.clone(),
            verified_files: verification.verified.len() as u32,
            modified_files: verification.modified.len() as u32,
            unknown_files: verification.unknown.len() as u32,
        },
        options: PackOptions {
            texture_encoding: opts.texture_encoding,
            audio_encoding: opts.audio_encoding,
            audio_quality: opts.audio_quality,
        },
        scenes: scene_entries,
        textures: texture_entries,
        sounds: sound_entries,
    };
    writer.finish(&manifest)?;
    std::fs::rename(&tmp, &opts.out)
        .with_context(|| format!("moving bundle to {}", opts.out.display()))?;

    Ok(Report {
        install: install.root.clone(),
        bundle_bytes: std::fs::metadata(&opts.out)?.len(),
        manifest,
        warnings,
        seconds: t0.elapsed().as_secs_f32(),
    })
}
