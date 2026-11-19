//! Locating, indexing and verifying a user's Ballance installation.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

/// Known-good hashes of the files the packer reads, per distribution.
const KNOWN: &[(&str, &str)] = &[("gog", include_str!("../data/gog-1918420377.sha256"))];

/// Files that must exist for a usable install (lowercase, `/`-separated).
const REQUIRED: &[&str] = &[
    "base.cmo",
    "3d entities/balls.nmo",
    "3d entities/gameplay.nmo",
    "3d entities/menu.nmo",
    "3d entities/level/level_01.nmo",
    "3d entities/level/level_12.nmo",
];

/// An indexed installation. Lookups are case-insensitive, because the game
/// itself is (Windows), while copies on Linux/macOS may not preserve case.
pub struct Install {
    pub root: PathBuf,
    /// Lowercase relative path (`/` separators) → real path.
    files: BTreeMap<String, PathBuf>,
}

impl Install {
    /// Finds the install: explicit path first, then `BALLANCE_DIR`, then
    /// common locations.
    pub fn locate(explicit: Option<&Path>) -> Result<Self> {
        if let Some(p) = explicit {
            // Accept the game root or its `Bin` folder.
            for candidate in [p, p.parent().unwrap_or(p)] {
                if is_install(candidate) {
                    return Self::index(candidate);
                }
            }
            bail!(
                "{} does not look like a Ballance installation (no base.cmo found)",
                p.display()
            );
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(env) = std::env::var_os("BALLANCE_DIR") {
            candidates.push(env.into());
        }
        candidates.extend(default_locations());
        for c in &candidates {
            if is_install(c) {
                return Self::index(c);
            }
        }
        bail!(
            "could not find a Ballance installation; pass --from <dir> (the folder containing base.cmo)"
        )
    }

    pub fn index(root: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        walk(root, root, &mut files).with_context(|| format!("indexing {}", root.display()))?;
        let missing: Vec<&str> = REQUIRED
            .iter()
            .copied()
            .filter(|r| !files.contains_key(*r))
            .collect();
        if !missing.is_empty() {
            bail!(
                "installation is incomplete, missing: {}",
                missing.join(", ")
            );
        }
        Ok(Self {
            root: root.to_path_buf(),
            files,
        })
    }

    pub fn get(&self, key: &str) -> Option<&Path> {
        self.files.get(key).map(PathBuf::as_path)
    }

    /// Files under a directory prefix (lowercase, ending in `/`), sorted.
    pub fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a str, &'a Path)> + 'a {
        self.files
            .range(prefix.to_owned()..)
            .take_while(move |(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.as_str(), v.as_path()))
    }
}

fn is_install(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("base.cmo")
        })
    })
}

fn default_locations() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for base in [
            "C:/GOG Games",
            "C:/Program Files (x86)/GOG Galaxy/Games",
            "C:/Games",
        ] {
            v.push(Path::new(base).join("Ballance"));
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for sub in [
            ".wine/drive_c/GOG Games/Ballance",
            ".wine/drive_c/Program Files (x86)/GOG Galaxy/Games/Ballance",
            "Games/Ballance",
            "GOG Games/Ballance",
        ] {
            v.push(home.join(sub));
        }
    }
    v
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(root, &p, out)?;
        } else if let Ok(rel) = p.strip_prefix(root) {
            let key = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                .collect::<Vec<_>>()
                .join("/");
            out.insert(key, p);
        }
    }
    Ok(())
}

/// Result of checking the used files against known distributions.
#[derive(Debug, Default)]
pub struct Verification {
    /// Best-matching distribution, or `"unknown"`.
    pub variant: String,
    pub verified: Vec<String>,
    pub modified: Vec<String>,
    pub unknown: Vec<String>,
}

/// Hashes `keys` and compares them against the known distributions.
pub fn verify(install: &Install, keys: &[&str]) -> Result<Verification> {
    let hashes: Vec<(String, String)> = keys
        .par_iter()
        .map(|k| {
            let path = install
                .get(k)
                .with_context(|| format!("{k} not in install"))?;
            Ok(((*k).to_owned(), sha256_file(path)?))
        })
        .collect::<Result<_>>()?;

    let mut best = Verification {
        variant: "unknown".into(),
        unknown: hashes.iter().map(|(k, _)| k.clone()).collect(),
        ..Default::default()
    };
    for (variant, table) in KNOWN {
        let known = parse_table(table);
        let mut v = Verification {
            variant: (*variant).to_owned(),
            ..Default::default()
        };
        for (k, h) in &hashes {
            match known.get(k.as_str()) {
                Some(expected) if expected == h => v.verified.push(k.clone()),
                Some(_) => v.modified.push(k.clone()),
                None => v.unknown.push(k.clone()),
            }
        }
        if v.verified.len() > best.verified.len() {
            best = v;
        }
    }
    Ok(best)
}

fn parse_table(table: &str) -> BTreeMap<String, &str> {
    table
        .lines()
        .filter_map(|l| {
            let (hash, path) = l.split_once("  ")?;
            Some((path.trim().to_lowercase(), hash.trim()))
        })
        .collect()
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_table_parses() {
        let t = parse_table(KNOWN[0].1);
        assert!(t.len() > 200);
        assert!(t.contains_key("3d entities/level/level_01.nmo"));
        assert!(t.values().all(|h| h.len() == 64));
    }
}
