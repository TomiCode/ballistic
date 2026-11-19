//! `ballistic-pack`: builds a Ballistic asset bundle from your own copy of Ballance.

use std::path::PathBuf;

use anyhow::Result;
use ballistic_bundle::{AudioEncoding, BundleReader, TextureEncoding};
use ballistic_pack::{install, pack};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a bundle from a Ballance installation.
    Build {
        /// Game folder (the one containing base.cmo). Searched for if omitted.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Output bundle path.
        #[arg(long, default_value = "ballance.bpk")]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = TexArg::Bc)]
        texture_encoding: TexArg,
        #[arg(long, value_enum, default_value_t = AudioArg::Vorbis)]
        audio_encoding: AudioArg,
        /// Vorbis quality, -0.1 … 1.0.
        #[arg(long, default_value_t = 0.6)]
        audio_quality: f32,
        /// Number of worker threads (default: all cores).
        #[arg(long)]
        jobs: Option<usize>,
        /// Print every warning instead of a summary.
        #[arg(long, short)]
        verbose: bool,
    },
    /// Check an installation against known releases without building.
    Verify {
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Summarise an existing bundle.
    Inspect { bundle: PathBuf },
}

#[derive(Clone, Copy, ValueEnum)]
enum TexArg {
    Bc,
    Rgba8,
}

#[derive(Clone, Copy, ValueEnum)]
enum AudioArg {
    Vorbis,
    Wav,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Build {
            from,
            out,
            texture_encoding,
            audio_encoding,
            audio_quality,
            jobs,
            verbose,
        } => {
            if let Some(n) = jobs {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .build_global()?;
            }
            anyhow::ensure!(
                (-0.1..=1.0).contains(&audio_quality),
                "--audio-quality must be between -0.1 and 1.0"
            );
            let report = pack::build(&pack::Options {
                from,
                out: out.clone(),
                texture_encoding: match texture_encoding {
                    TexArg::Bc => TextureEncoding::Bc,
                    TexArg::Rgba8 => TextureEncoding::Rgba8,
                },
                audio_encoding: match audio_encoding {
                    AudioArg::Vorbis => AudioEncoding::Vorbis,
                    AudioArg::Wav => AudioEncoding::Wav,
                },
                audio_quality,
            })?;
            print_report(&report, &out, verbose);
            Ok(())
        }
        Command::Verify { from } => {
            let install = install::Install::locate(from.as_deref())?;
            let inputs = pack::select(&install);
            let keys = inputs.source_keys(&install);
            let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            let v = install::verify(&install, &refs)?;
            println!("install:  {}", install.root.display());
            println!("release:  {}", v.variant);
            println!(
                "files:    {} verified, {} modified, {} unknown",
                v.verified.len(),
                v.modified.len(),
                v.unknown.len()
            );
            for m in &v.modified {
                println!("  modified: {m}");
            }
            for u in &v.unknown {
                println!("  unknown:  {u}");
            }
            Ok(())
        }
        Command::Inspect { bundle } => inspect(&bundle),
    }
}

fn print_report(r: &pack::Report, out: &std::path::Path, verbose: bool) {
    let m = &r.manifest;
    println!("install:   {}", r.install.display());
    println!(
        "release:   {} ({} verified, {} modified, {} unknown files)",
        m.source.variant, m.source.verified_files, m.source.modified_files, m.source.unknown_files
    );
    println!(
        "packed:    {} scenes, {} textures, {} sounds",
        m.scenes.len(),
        m.textures.len(),
        m.sounds.len()
    );
    println!(
        "bundle:    {} ({:.1} MiB) in {:.1}s",
        out.display(),
        r.bundle_bytes as f64 / (1024.0 * 1024.0),
        r.seconds
    );
    if !r.warnings.is_empty() {
        println!("warnings:  {}", r.warnings.len());
        let shown = if verbose { r.warnings.len() } else { 10 };
        for w in r.warnings.iter().take(shown) {
            println!("  {w}");
        }
        if r.warnings.len() > shown {
            println!("  … {} more (use --verbose)", r.warnings.len() - shown);
        }
    }
}

fn inspect(path: &std::path::Path) -> Result<()> {
    let r = BundleReader::open(path)?;
    let m = r.manifest();
    println!(
        "{} v{} (packer {}), source: {}",
        m.format, m.format_version, m.packer_version, m.source.variant
    );
    println!("options: {:?}", m.options);
    let (objects, meshes): (u32, u32) = m
        .scenes
        .iter()
        .fold((0, 0), |(o, me), s| (o + s.objects, me + s.meshes));
    println!(
        "scenes:   {:>4} ({objects} objects, {meshes} meshes)",
        m.scenes.len()
    );
    let mut by_fmt = std::collections::BTreeMap::new();
    for t in &m.textures {
        *by_fmt.entry(format!("{:?}", t.format)).or_insert(0) += 1;
    }
    println!("textures: {:>4} {by_fmt:?}", m.textures.len());
    let secs: f64 = m
        .sounds
        .iter()
        .map(|s| s.frames as f64 / f64::from(s.sample_rate))
        .sum();
    println!("sounds:   {:>4} ({secs:.0} s of audio)", m.sounds.len());
    Ok(())
}
