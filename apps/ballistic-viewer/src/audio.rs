//! Sound output (kira, roadmap decision D4). Play mode queues sounds by bundle
//! key and volume; the window plays them here. Sounds are decoded from the
//! bundle on first use and kept.

use std::collections::HashMap;
use std::io::Cursor;

use kira::sound::static_sound::StaticSoundData;
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend};

use crate::play::Bundle;

pub struct Audio {
    manager: AudioManager<DefaultBackend>,
    sounds: HashMap<String, Option<StaticSoundData>>,
}

impl Audio {
    /// The default output device; `None` (with a warning) if there's none.
    pub fn new() -> Option<Self> {
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => Some(Self {
                manager,
                sounds: HashMap::new(),
            }),
            Err(e) => {
                eprintln!("warning: no audio output: {e}");
                None
            }
        }
    }

    /// Plays a bundle sound once at `volume` (0–1, linear).
    pub fn play(&mut self, bundle: &mut Bundle, key: &str, volume: f32) {
        let data = self
            .sounds
            .entry(key.to_owned())
            .or_insert_with(|| {
                let bytes = bundle
                    .sound(key)
                    .map_err(|e| eprintln!("warning: sound {key}: {e:#}"))
                    .ok()?;
                StaticSoundData::from_cursor(Cursor::new(bytes))
                    .map_err(|e| eprintln!("warning: decoding sound {key}: {e}"))
                    .ok()
            })
            .clone();
        let Some(data) = data else { return };
        let db = if volume > 0.0 {
            (20.0 * volume.log10()).max(Decibels::SILENCE.0)
        } else {
            Decibels::SILENCE.0
        };
        if let Err(e) = self.manager.play(data.volume(Decibels(db))) {
            eprintln!("warning: playing sound {key}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundle's Ogg Vorbis sounds decode (no output device needed).
    #[test]
    fn decodes_bundle_sounds() {
        let Some(path) = std::env::var_os("BALLISTIC_BUNDLE") else {
            eprintln!("skipping: BALLISTIC_BUNDLE not set");
            return;
        };
        let mut b = ballistic_bundle::BundleReader::open(path).unwrap();
        for key in ["misc_trafo", "pieces_wood", "pieces_stone", "pieces_paper"] {
            let data = StaticSoundData::from_cursor(Cursor::new(b.sound(key).unwrap())).unwrap();
            assert!(data.duration().as_secs_f64() > 0.05, "{key}");
        }
    }
}
