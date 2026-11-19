//! `ckinspect`: developer tool for looking inside Virtools `.nmo` / `.cmo` files.

mod render;

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
    /// Decode every mesh/material/texture/entity/group under a directory.
    SceneCheck { dir: PathBuf },
    /// Decode and validate every script object under a directory.
    ScriptCheck { dir: PathBuf },
    /// Extract physics parameters from every script under a directory
    /// (physics building blocks plus the `Physicalize_*` tables).
    Physics {
        dir: PathBuf,
        /// Write the full result as JSON here (derived from game data: keep it out of the repos).
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Print a file's data arrays (script tables) with resolved cells.
    Arrays {
        file: PathBuf,
        /// Only arrays whose name contains this text.
        #[arg(long)]
        filter: Option<String>,
    },
    /// Print a file's scripts as an indented tree (or JSON / Graphviz DOT).
    Scripts {
        file: PathBuf,
        /// Only scripts whose name contains this text.
        #[arg(long)]
        filter: Option<String>,
        #[arg(long, value_enum, default_value_t = ScriptFormat::Tree)]
        format: ScriptFormat,
    },
    /// Print decoded scene objects of one file as JSON.
    Scene {
        file: PathBuf,
        #[arg(long)]
        class: Option<String>,
    },
    /// Software-render a file's visible geometry to PNG (dev-only, output is
    /// derived game data: keep it out of the repo).
    Render {
        file: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Directory holding the texture images (e.g. `<install>/Textures`).
        #[arg(long)]
        textures: Option<PathBuf>,
        #[arg(long, default_value_t = 0.0)]
        azimuth: f32,
        #[arg(long, default_value_t = 90.0)]
        elevation: f32,
        #[arg(long, default_value_t = 2048)]
        size: u32,
        /// Also draw objects saved as hidden.
        #[arg(long)]
        include_hidden: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ScriptFormat {
    Tree,
    Json,
    Dot,
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
        Command::SceneCheck { dir } => scene_check(&dir),
        Command::ScriptCheck { dir } => script_check(&dir),
        Command::Scripts {
            file,
            filter,
            format,
        } => scripts(&file, filter.as_deref(), format),
        Command::Arrays { file, filter } => arrays(&file, filter.as_deref()),
        Command::Physics { dir, json } => physics(&dir, json.as_deref()),
        Command::Scene { file, class } => scene(&file, class.as_deref()),
        Command::Render {
            file,
            out,
            textures,
            azimuth,
            elevation,
            size,
            include_hidden,
        } => render::render(
            &open(&file)?,
            textures.as_deref(),
            &render::View {
                azimuth,
                elevation,
                size,
                include_hidden,
            },
            &out,
        ),
    }
}

fn scene_check(dir: &Path) -> Result<()> {
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failures = 0;
    for p in walk(dir)? {
        let f = open(&p)?;
        for (i, o) in f.objects.iter().enumerate() {
            let Some(res) = ckscene::decode_object(o) else {
                continue;
            };
            let e = counts.entry(o.class_id.to_string()).or_default();
            match res {
                Ok(_) => e.0 += 1,
                Err(err) => {
                    e.1 += 1;
                    failures += 1;
                    println!("FAIL {} #{i} {:?}: {err}", p.display(), o.name);
                }
            }
        }
    }
    for (class, (ok, bad)) in &counts {
        println!("{class:<14} {ok:>6} ok {bad:>4} failed");
    }
    if failures > 0 {
        bail!("{failures} object(s) failed to decode");
    }
    Ok(())
}

fn physics(dir: &Path, json: Option<&Path>) -> Result<()> {
    let mut all_nodes = Vec::new();
    let mut tables = serde_json::Map::new();
    for p in walk(dir)? {
        let f = open(&p)?;
        let s = ckscript::Scripts::decode(&f)?;
        let rel = p
            .strip_prefix(dir)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        for (i, o) in s.objects.iter().enumerate() {
            let Some(ckscript::ScriptObject::Array(a)) = o else {
                continue;
            };
            let name = f.objects[i].name.clone().unwrap_or_default();
            if !name.starts_with("Physicalize") {
                continue;
            }
            println!("== table {name} ({rel})");
            let rows: Vec<serde_json::Value> = a
                .rows
                .iter()
                .map(|row| {
                    let mut m = serde_json::Map::new();
                    for (c, cell) in a.columns.iter().zip(row) {
                        m.insert(
                            c.name.clone(),
                            serde_json::Value::String(ckscript::types::format_cell(&f, &s, cell)),
                        );
                    }
                    serde_json::Value::Object(m)
                })
                .collect();
            for r in &rows {
                println!("   {r}");
            }
            tables.insert(name, serde_json::Value::Array(rows));
        }
        let ex = ckscript::extract::Extractor::new(&f, &s);
        for n in ex.nodes(&|bb| bb.dll == "physics_RT") {
            all_nodes.push((rel.clone(), n));
        }
    }
    println!();
    let mut last = String::new();
    for (file, n) in &all_nodes {
        if *file != last {
            println!("== {file}");
            last.clone_from(file);
        }
        let target = n
            .target
            .as_ref()
            .map_or(String::new(), |t| format!(" on {}", t.value));
        println!(
            "  {} [{}]{target}  ({})",
            n.building_block,
            n.path.join(" > "),
            n.owner.as_deref().unwrap_or("?")
        );
        for v in &n.inputs {
            println!("      {:<32} {}", v.name, v.value);
        }
    }
    println!("{} physics nodes, {} tables", all_nodes.len(), tables.len());
    if let Some(out) = json {
        let nodes: Vec<serde_json::Value> = all_nodes
            .iter()
            .map(|(file, n)| serde_json::json!({ "file": file, "node": n }))
            .collect();
        let doc = serde_json::json!({ "tables": tables, "nodes": nodes });
        std::fs::write(out, serde_json::to_string_pretty(&doc)?)?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

fn arrays(path: &Path, filter: Option<&str>) -> Result<()> {
    let f = open(path)?;
    let s = ckscript::Scripts::decode(&f)?;
    for (i, o) in s.objects.iter().enumerate() {
        let Some(ckscript::ScriptObject::Array(a)) = o else {
            continue;
        };
        let name = f.objects[i].name.clone().unwrap_or_default();
        if filter.is_some_and(|q| !name.contains(q)) {
            continue;
        }
        println!("== {name} (#{i}, {} rows)", a.rows.len());
        let header: Vec<String> = a
            .columns
            .iter()
            .map(|c| match c.ty {
                ckscript::ColumnType::Parameter(g) => {
                    format!("{} <{}>", c.name, ckscript::types::type_name(g))
                }
                t => format!("{} <{t:?}>", c.name),
            })
            .collect();
        println!("   {}", header.join(" | "));
        for row in &a.rows {
            let cells: Vec<String> = row
                .iter()
                .map(|c| ckscript::types::format_cell(&f, &s, c))
                .collect();
            println!("   {}", cells.join(" | "));
        }
        println!();
    }
    Ok(())
}

fn scripts(path: &Path, filter: Option<&str>, format: ScriptFormat) -> Result<()> {
    let f = open(path)?;
    let s = ckscript::Scripts::decode(&f)?;
    let d = ckscript::describe::Describer {
        file: &f,
        scripts: &s,
    };
    let mut nodes = Vec::new();
    for &(root, owner) in &s.roots {
        let Some(node) = d.node(root) else { continue };
        if filter.is_some_and(|q| !node.name.contains(q)) {
            continue;
        }
        nodes.push((owner, node));
    }
    match format {
        ScriptFormat::Tree => {
            for (owner, node) in &nodes {
                let on = owner.map_or("(unattached)".into(), |o| {
                    ckscript::types::object_label(&f, o)
                });
                let mut out = format!(
                    "== script on {on}
"
                );
                ckscript::describe::write_tree(&mut out, node, 0);
                println!("{out}");
            }
        }
        ScriptFormat::Json => {
            let v: Vec<_> = nodes
                .iter()
                .map(|(owner, n)| serde_json::json!({ "owner": owner, "script": n }))
                .collect();
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        ScriptFormat::Dot => {
            for (_, n) in &nodes {
                println!("{}", ckscript::describe::to_dot(n));
            }
        }
    }
    Ok(())
}

fn script_check(dir: &Path) -> Result<()> {
    let (mut files, mut objects, mut roots, mut attached, mut problems) = (0, 0, 0, 0, 0);
    let mut conformance = ckscript::conform::Conformance::default();
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    for p in walk(dir)? {
        let f = open(&p)?;
        let s =
            ckscript::Scripts::decode(&f).with_context(|| format!("decoding {}", p.display()))?;
        files += 1;
        roots += s.roots.len();
        attached += s.roots.iter().filter(|(_, owner)| owner.is_some()).count();
        for o in s.objects.iter().flatten() {
            objects += 1;
            let k = match o {
                ckscript::ScriptObject::Behavior(b) if b.is_graph() => "graph",
                ckscript::ScriptObject::Behavior(_) => "building block",
                ckscript::ScriptObject::Io(_) => "pin",
                ckscript::ScriptObject::Link(_) => "link",
                ckscript::ScriptObject::In(_) => "param in",
                ckscript::ScriptObject::Out(_) => "param out",
                ckscript::ScriptObject::Local(_) => "param local",
                ckscript::ScriptObject::Parameter(_) => "parameter",
                ckscript::ScriptObject::Operation(_) => "operation",
                ckscript::ScriptObject::Array(_) => "data array",
            };
            *kinds.entry(k).or_default() += 1;
        }
        ckscript::conform::check(&f, &s, ckscript::Catalogue::builtin(), &mut conformance);
        for msg in s.validate(&f) {
            problems += 1;
            if problems <= 30 {
                println!("{}: {msg}", p.display());
            }
        }
    }
    println!(
        "{files} files, {objects} script objects, {roots} root behaviours ({attached} attached to scene objects)"
    );
    for (k, n) in &kinds {
        println!("  {n:>6}  {k}");
    }
    println!("{problems} structural problem(s)");
    let c = &conformance;
    println!(
        "catalogue: {} building-block nodes, {} match their declaration exactly, {} unknown prototypes",
        c.nodes, c.exact, c.unknown
    );
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    for (bb, d) in &c.differences {
        println!("  differs: {bb:<32} {d:?}");
        for (k, n) in d {
            *totals.entry(k).or_default() += n;
        }
    }
    println!("difference totals: {totals:?}");
    if problems > 0 {
        bail!("validation failed");
    }
    Ok(())
}

fn scene(path: &Path, class: Option<&str>) -> Result<()> {
    let f = open(path)?;
    let filter = class.map(parse_class).transpose()?;
    let mut out = Vec::new();
    for (i, o) in f.objects.iter().enumerate() {
        if filter.is_some_and(|c| c != o.class_id) {
            continue;
        }
        if let Some(d) = ckscene::decode_object(o) {
            out.push(serde_json::json!({ "index": i, "name": o.name, "value": d? }));
        }
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
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
