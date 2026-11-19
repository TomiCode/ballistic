//! `ckinspect`: developer tool for looking inside Virtools `.nmo` / `.cmo` files.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ckfile::{CkFile, ClassId, StateChunk};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Header, class histogram and plugin dependencies.
    Info { file: PathBuf },
    /// List the object table.
    Objects {
        file: PathBuf,
        /// Only show objects of this class (name like `Mesh` or numeric id).
        #[arg(long)]
        class: Option<String>,
        /// Also list each object's identifier sections.
        #[arg(long)]
        sections: bool,
    },
    /// Dump one object's chunk (by file index, or by exact name).
    Chunk {
        file: PathBuf,
        object: String,
        /// Maximum words to print per section.
        #[arg(long, default_value_t = 64)]
        max_words: usize,
    },
    /// Dump the whole file as JSON.
    Json { file: PathBuf },
    /// Parse every .nmo/.cmo under a directory and report problems.
    Scan { dir: PathBuf },
    /// Histogram of identifiers per class across a directory.
    Identifiers {
        dir: PathBuf,
        #[arg(long)]
        class: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { file } => info(&file),
        Command::Objects {
            file,
            class,
            sections,
        } => objects(&file, class.as_deref(), sections),
        Command::Chunk {
            file,
            object,
            max_words,
        } => chunk(&file, &object, max_words),
        Command::Json { file } => {
            let f = open(&file)?;
            println!("{}", serde_json::to_string_pretty(&f)?);
            Ok(())
        }
        Command::Scan { dir } => scan(&dir),
        Command::Identifiers { dir, class } => identifiers(&dir, class.as_deref()),
    }
}

fn open(path: &Path) -> Result<CkFile> {
    CkFile::open(path).with_context(|| format!("reading {}", path.display()))
}

fn parse_class(s: &str) -> Result<ClassId> {
    if let Ok(n) = s.parse::<u32>() {
        return Ok(ClassId(n));
    }
    (0..256)
        .map(ClassId)
        .find(|c| c.name().is_some_and(|n| n.eq_ignore_ascii_case(s)))
        .with_context(|| format!("unknown class {s:?}"))
}

fn info(path: &Path) -> Result<()> {
    let f = open(path)?;
    let h = &f.header;
    println!("file:            {}", path.display());
    println!("ck version:      {:#010x}", h.ck_version);
    println!("file version:    {}", h.file_version);
    println!("write mode:      {:#x}", h.write_mode);
    println!("product build:   {:#010x}", h.product_build);
    println!("objects:         {}", f.objects.len());
    println!("managers:        {}", f.managers.len());
    for m in &f.managers {
        let words = m.chunk.as_ref().map_or(0, |c| c.data.len());
        println!("  {}  ({words} words)", m.guid);
    }
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    for o in &f.objects {
        *hist.entry(o.class_id.to_string()).or_default() += 1;
    }
    let mut hist: Vec<_> = hist.into_iter().collect();
    hist.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    println!("classes:");
    for (name, n) in hist {
        println!("  {n:>6}  {name}");
    }
    println!("plugin dependencies:");
    for d in &f.plugin_dependencies {
        println!("  {:?}: {} guid(s)", d.category, d.guids.len());
    }
    Ok(())
}

fn objects(path: &Path, class: Option<&str>, with_sections: bool) -> Result<()> {
    let f = open(path)?;
    let filter = class.map(parse_class).transpose()?;
    for (i, o) in f.objects.iter().enumerate() {
        if filter.is_some_and(|c| c != o.class_id) {
            continue;
        }
        let words = o.chunk.as_ref().map_or(0, |c| c.data.len());
        println!(
            "{i:>6}  id={:<6} {:<18} {:>6}w  {}",
            o.id,
            o.class_id.to_string(),
            words,
            o.name.as_deref().unwrap_or("<unnamed>")
        );
        if with_sections && let Some(c) = &o.chunk {
            let s: Vec<String> = c
                .sections()?
                .iter()
                .map(|s| format!("{:#x}[{}]", s.identifier, s.len))
                .collect();
            println!("        sections: {}", s.join(" "));
        }
    }
    Ok(())
}

fn find_object(f: &CkFile, key: &str) -> Result<usize> {
    if let Ok(i) = key.parse::<usize>() {
        if i < f.objects.len() {
            return Ok(i);
        }
        bail!("index {i} out of range ({} objects)", f.objects.len());
    }
    f.objects
        .iter()
        .position(|o| o.name.as_deref() == Some(key))
        .with_context(|| format!("no object named {key:?}"))
}

fn chunk(path: &Path, key: &str, max_words: usize) -> Result<()> {
    let f = open(path)?;
    let i = find_object(&f, key)?;
    let o = &f.objects[i];
    println!(
        "object #{i} id={} class={} name={:?}",
        o.id, o.class_id, o.name
    );
    match &o.chunk {
        None => println!("  (no chunk)"),
        Some(c) => dump_chunk(&f, c, 1, max_words)?,
    }
    Ok(())
}

fn dump_chunk(f: &CkFile, c: &StateChunk, depth: usize, max_words: usize) -> Result<()> {
    let ind = "  ".repeat(depth);
    println!(
        "{ind}chunk class={} data_version={} options={:#x} words={}",
        c.class_id,
        c.data_version,
        c.options,
        c.data.len()
    );
    if !c.object_positions.is_empty() {
        println!("{ind}object refs at {:?}", c.object_positions);
    }
    if !c.manager_positions.is_empty() {
        println!("{ind}manager values at {:?}", c.manager_positions);
    }
    for s in c.sections()? {
        println!(
            "{ind}section {:#x} ({} words @ {})",
            s.identifier, s.len, s.start
        );
        let words = &c.data[s.start..s.start + s.len];
        for (row, ws) in words
            .iter()
            .take(max_words)
            .collect::<Vec<_>>()
            .chunks(8)
            .enumerate()
        {
            let mut line = String::new();
            for w in ws {
                write!(line, "{w:08x} ")?;
            }
            let pretty: Vec<String> = ws.iter().map(|&&w| describe_word(f, w)).collect();
            println!(
                "{ind}  {:>4}: {line:<72}| {}",
                s.start + row * 8,
                pretty.join(" ")
            );
        }
        if s.len > max_words {
            println!("{ind}  … {} more words", s.len - max_words);
        }
    }
    for (n, sub) in c.sub_chunks()?.iter().enumerate() {
        println!("{ind}sub-chunk {n} at word {}:", c.sub_chunk_positions[n]);
        dump_chunk(f, sub, depth + 1, max_words)?;
    }
    Ok(())
}

/// Best-effort interpretation of a word for human eyes: small ints, floats, text.
fn describe_word(_f: &CkFile, w: u32) -> String {
    let fl = f32::from_bits(w);
    let bytes = w.to_le_bytes();
    if w == u32::MAX {
        "-1".into()
    } else if w < 0x10000 {
        w.to_string()
    } else if bytes.iter().all(|&b| b == 0 || (0x20..0x7F).contains(&b)) && bytes[0] != 0 {
        format!(
            "{:?}",
            String::from_utf8_lossy(&bytes).trim_end_matches('\0')
        )
    } else if fl.is_finite() && fl.abs() > 1e-6 && fl.abs() < 1e7 {
        format!("{fl}f")
    } else {
        format!("{w:#x}")
    }
}

fn walk(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let p = e?.path();
        if p.is_dir() {
            out.extend(walk(&p)?);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("nmo") || e.eq_ignore_ascii_case("cmo"))
        {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

fn scan(dir: &Path) -> Result<()> {
    let files = walk(dir)?;
    let (mut ok, mut bad, mut objects, mut chunks, mut subs) = (0, 0, 0usize, 0usize, 0usize);
    for p in &files {
        let res = (|| -> Result<()> {
            let f = open(p)?;
            objects += f.objects.len();
            for c in f
                .objects
                .iter()
                .filter_map(|o| o.chunk.as_ref())
                .chain(f.managers.iter().filter_map(|m| m.chunk.as_ref()))
            {
                c.sections()?;
                let s = c.sub_chunks()?;
                for sub in &s {
                    sub.sections()?;
                }
                subs += s.len();
                chunks += 1;
            }
            Ok(())
        })();
        match res {
            Ok(()) => ok += 1,
            Err(e) => {
                bad += 1;
                println!("FAIL {}: {e:#}", p.display());
            }
        }
    }
    println!(
        "{} files: {ok} ok, {bad} failed; {objects} objects, {chunks} chunks, {subs} sub-chunks",
        files.len()
    );
    if bad > 0 {
        bail!("{bad} file(s) failed to parse");
    }
    Ok(())
}

fn identifiers(dir: &Path, class: Option<&str>) -> Result<()> {
    let filter = class.map(parse_class).transpose()?;
    // class -> identifier -> (occurrences, min len, max len)
    let mut map: BTreeMap<String, BTreeMap<u32, (usize, usize, usize)>> = BTreeMap::new();
    for p in walk(dir)? {
        let f = open(&p)?;
        for o in &f.objects {
            if filter.is_some_and(|c| c != o.class_id) {
                continue;
            }
            let Some(c) = &o.chunk else { continue };
            let entry = map.entry(o.class_id.to_string()).or_default();
            for s in c.sections()? {
                let e = entry.entry(s.identifier).or_insert((0, usize::MAX, 0));
                e.0 += 1;
                e.1 = e.1.min(s.len);
                e.2 = e.2.max(s.len);
            }
        }
    }
    for (class, ids) in map {
        println!("{class}:");
        for (id, (n, lo, hi)) in ids {
            println!("  {id:#010x}  x{n:<6} len {lo}..={hi}");
        }
    }
    Ok(())
}
