//! Ballistic's save file: which levels are open, each level's highscore
//! table, and the name last entered. Its own JSON format (the original's
//! `Database.tdb` isn't read: decision D9 in the research repo's roadmap).
//!
//! ```json
//! {
//!   "format": "ballistic-save",
//!   "version": 1,
//!   "player": "Tom",
//!   "unlocked": [true, true, false, …],
//!   "highscores": [[{ "name": "Tom", "score": 4210 }, …], …],
//!   "settings": { "volume": 0.8, "keys": ["ArrowUp", …], "invert_rotation": false,
//!     "resolution": null, "vsync": true, "clouds": true }
//! }
//! ```
//!
//! - `unlocked` and `highscores` are in level order (level 1 first);
//!   `highscores` best first, as many places as the defaults had.
//! - Unknown fields are ignored and missing ones take their defaults, so
//!   later versions can add fields (settings, personal bests) without
//!   breaking older files; `version` changes only when a field changes
//!   meaning.
//! - Written whole to a temporary file next to it, then renamed over it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "ballistic-save";
pub const VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: not a save file: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{path}: not a Ballistic save (format {format:?} v{version})")]
    Format {
        path: PathBuf,
        format: String,
        version: u32,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// One place in a highscore table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highscore {
    pub name: String,
    pub score: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Save {
    pub format: String,
    pub version: u32,
    /// The name last entered.
    pub player: String,
    /// By level, level 1 first.
    pub unlocked: Vec<bool>,
    /// By level: best first.
    pub highscores: Vec<Vec<Highscore>>,
    pub settings: Settings,
}

/// The options (`Menu_Opt_*`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 0–1.
    pub volume: f32,
    /// Forward, backward, left, right, camera rotation, camera lift (the
    /// window's key names, e.g. `ArrowUp`); empty: the game's defaults.
    pub keys: Vec<String>,
    /// Shift + Left turns the camera the other way.
    pub invert_rotation: bool,
    /// The window's size; `None`: as it opens.
    pub resolution: Option<[u32; 2]>,
    pub vsync: bool,
    /// The cloud layer (level 12).
    pub clouds: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            keys: Vec::new(),
            invert_rotation: false,
            resolution: None,
            vsync: true,
            clouds: true,
        }
    }
}

impl Default for Save {
    fn default() -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            player: String::new(),
            unlocked: Vec::new(),
            highscores: Vec::new(),
            settings: Settings::default(),
        }
    }
}

impl Save {
    /// A new player's save.
    pub fn new(player: &str, unlocked: Vec<bool>, highscores: Vec<Vec<Highscore>>) -> Self {
        Self {
            player: player.to_owned(),
            unlocked,
            highscores,
            ..Self::default()
        }
    }

    /// Reads `path`; `Ok(None)` if there's no file.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let save: Self = serde_json::from_slice(&bytes).map_err(|source| Error::Json {
            path: path.to_owned(),
            source,
        })?;
        if save.format != FORMAT || save.version > VERSION {
            return Err(Error::Format {
                path: path.to_owned(),
                format: save.format,
                version: save.version,
            });
        }
        Ok(Some(save))
    }

    /// Writes `path` (and its folder), replacing it whole.
    pub fn write(&self, path: &Path) -> Result<()> {
        let io = |source| Error::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|source| Error::Json {
            path: path.to_owned(),
            source,
        })?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    /// Fills in what a save lacks from `defaults` (levels added since, say).
    pub fn complete(&mut self, defaults: &Self) {
        for (i, &open) in defaults.unlocked.iter().enumerate() {
            match self.unlocked.get_mut(i) {
                Some(u) => *u |= open,
                None => self.unlocked.push(open),
            }
        }
        for (i, table) in defaults.highscores.iter().enumerate() {
            if self.highscores.get(i).is_none_or(Vec::is_empty) {
                if i < self.highscores.len() {
                    self.highscores[i].clone_from(table);
                } else {
                    self.highscores.push(table.clone());
                }
            }
        }
    }

    /// Where `score` would go in `level`'s table (0-based level, 0-based
    /// place): above every lower score, below equal ones; `None` if it
    /// doesn't beat the last place (a full table's).
    pub fn place(&self, level: usize, score: i32) -> Option<usize> {
        let table = self.highscores.get(level)?;
        if table.last().is_some_and(|last| score <= last.score) {
            return None;
        }
        Some(
            table
                .iter()
                .position(|h| h.score < score)
                .unwrap_or(table.len()),
        )
    }

    /// Puts `name` with `score` in `level`'s table, dropping the last place;
    /// the place it took.
    pub fn insert(&mut self, level: usize, name: &str, score: i32) -> Option<usize> {
        let place = self.place(level, score)?;
        let table = &mut self.highscores[level];
        let places = table.len().max(1);
        table.insert(
            place,
            Highscore {
                name: name.to_owned(),
                score,
            },
        );
        table.truncate(places);
        Some(place)
    }

    /// Opens `level` (0-based).
    pub fn unlock(&mut self, level: usize) {
        if let Some(u) = self.unlocked.get_mut(level) {
            *u = true;
        }
    }

    pub fn is_unlocked(&self, level: usize) -> bool {
        self.unlocked.get(level).copied().unwrap_or(false)
    }
}

/// The usual place for the save file: the platform's data folder
/// (`%APPDATA%\Ballistic`, `~/Library/Application Support/Ballistic`,
/// `$XDG_DATA_HOME/ballistic` or `~/.local/share/ballistic`).
pub fn default_path() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let dir = if cfg!(windows) {
        env("APPDATA")?.join("Ballistic")
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support/Ballistic")
    } else {
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".local/share")))?
            .join("ballistic")
    };
    Some(dir.join("save.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(scores: &[i32]) -> Vec<Highscore> {
        scores
            .iter()
            .map(|&score| Highscore {
                name: "Default".into(),
                score,
            })
            .collect()
    }

    fn save() -> Save {
        Save::new(
            "name",
            vec![true, false, false],
            vec![table(&[400, 300, 200]), table(&[400, 300, 200])],
        )
    }

    #[test]
    fn a_score_goes_below_equal_ones_and_pushes_the_last_out() {
        let mut s = save();
        assert_eq!(s.place(0, 200), None);
        assert_eq!(s.place(0, 201), Some(2));
        assert_eq!(s.insert(0, "Tom", 300), Some(2));
        let t: Vec<(&str, i32)> = s.highscores[0]
            .iter()
            .map(|h| (h.name.as_str(), h.score))
            .collect();
        assert_eq!(t, [("Default", 400), ("Default", 300), ("Tom", 300)]);
        assert_eq!(s.insert(0, "Ann", 500), Some(0));
        assert_eq!(s.highscores[0].len(), 3);
        assert_eq!(s.highscores[0][0].name, "Ann");
        // Unknown levels have no table.
        assert_eq!(s.place(5, 9999), None);
    }

    #[test]
    fn unlocking_and_completing_from_the_defaults() {
        let mut s = save();
        s.unlock(1);
        s.unlock(9);
        assert!(s.is_unlocked(1) && !s.is_unlocked(2) && !s.is_unlocked(9));
        let mut defaults = save();
        defaults.unlocked.push(false);
        defaults.highscores.push(table(&[10]));
        s.highscores[1].clear();
        s.complete(&defaults);
        assert_eq!(s.unlocked, [true, true, false, false]);
        assert_eq!(s.highscores.len(), 3);
        assert_eq!(s.highscores[1].len(), 3);
    }

    #[test]
    fn round_trips_and_refuses_other_files() {
        let dir = std::env::temp_dir().join(format!("ballistic-save-test-{}", std::process::id()));
        let path = dir.join("sub").join("save.json");
        assert_eq!(Save::load(&path).unwrap(), None);
        let mut s = save();
        s.insert(1, "Tom", 350);
        s.write(&path).unwrap();
        assert_eq!(Save::load(&path).unwrap(), Some(s));
        // Missing fields take their defaults; unknown ones are ignored.
        std::fs::write(
            &path,
            r#"{"format":"ballistic-save","version":1,"player":"A","extra":1}"#,
        )
        .unwrap();
        let s = Save::load(&path).unwrap().unwrap();
        assert_eq!(s.player, "A");
        assert_eq!(s.settings, Settings::default());
        assert_eq!(s.highscores, Vec::<Vec<Highscore>>::new());
        std::fs::write(&path, r#"{"format":"something-else","version":1}"#).unwrap();
        assert!(matches!(Save::load(&path), Err(Error::Format { .. })));
        std::fs::write(&path, "not json").unwrap();
        assert!(matches!(Save::load(&path), Err(Error::Json { .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
