//! The save file in use (`ballistic-save`): loaded once, shared by each
//! level's play mode, written after each level.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use ballistic_bundle::Highscores;
use ballistic_save::{Highscore, Save};

pub type SharedSave = Rc<RefCell<SaveFile>>;

pub struct SaveFile {
    pub data: Save,
    /// `None`: kept in memory only.
    path: Option<PathBuf>,
}

impl SaveFile {
    /// `path`'s save, or a new one from the game's defaults if there's none.
    /// A file that can't be read is left alone (the save is then kept in
    /// memory only).
    pub fn open(path: Option<PathBuf>, defaults: &Highscores) -> Self {
        let fresh = new_save(defaults);
        let Some(path) = path else {
            return Self {
                data: fresh,
                path: None,
            };
        };
        match Save::load(&path) {
            Ok(Some(mut data)) => {
                data.complete(&fresh);
                eprintln!("save: {}", path.display());
                Self {
                    data,
                    path: Some(path),
                }
            }
            Ok(None) => {
                eprintln!("save: {} (new)", path.display());
                Self {
                    data: fresh,
                    path: Some(path),
                }
            }
            Err(e) => {
                eprintln!("warning: {e}; this session's progress won't be saved");
                Self {
                    data: fresh,
                    path: None,
                }
            }
        }
    }

    /// In memory only (headless runs, tests).
    pub fn in_memory(defaults: &Highscores) -> Self {
        Self::open(None, defaults)
    }

    pub fn shared(self) -> SharedSave {
        Rc::new(RefCell::new(self))
    }

    pub fn write(&self) {
        if let Some(p) = &self.path
            && let Err(e) = self.data.write(p)
        {
            eprintln!("warning: {e}");
        }
    }
}

/// A new player's save: the game's default tables, open levels and name.
fn new_save(d: &Highscores) -> Save {
    Save::new(
        &d.default_name,
        d.unlocked.clone(),
        d.defaults
            .iter()
            .map(|t| {
                t.iter()
                    .map(|r| Highscore {
                        name: r.name.clone(),
                        score: r.score,
                    })
                    .collect()
            })
            .collect(),
    )
}
