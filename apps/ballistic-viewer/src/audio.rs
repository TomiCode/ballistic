//! Sound output (kira, roadmap decision D4). Play mode queues sounds by bundle
//! key and volume; the window plays them here. Sounds are decoded from the
//! bundle on first use and kept.

use std::collections::HashMap;
use std::io::Cursor;

use kira::Tween;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend};

use crate::play::Bundle;

pub struct Audio {
    manager: AudioManager<DefaultBackend>,
    sounds: HashMap<String, Option<StaticSoundData>>,
    /// Looping sounds playing now, by key.
    loops: HashMap<String, StaticSoundHandle>,
}

impl Audio {
    /// The default output device; `None` (with a warning) if there's none.
    pub fn new() -> Option<Self> {
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => Some(Self {
                manager,
                sounds: HashMap::new(),
                loops: HashMap::new(),
            }),
            Err(e) => {
                eprintln!("warning: no audio output: {e}");
                None
            }
        }
    }

    /// Plays a bundle sound once at `volume` (0–1, linear) and `pitch` (a
    /// playback rate).
    pub fn play(&mut self, bundle: &mut Bundle, key: &str, volume: f32, pitch: f32) {
        let Some(data) = self.data(bundle, key) else {
            return;
        };
        let data = data
            .volume(decibels(volume))
            .playback_rate(f64::from(pitch.max(0.01)));
        if let Err(e) = self.manager.play(data) {
            eprintln!("warning: playing sound {key}: {e}");
        }
    }

    /// Keeps exactly these sounds looping (key, volume 0–1, pitch as a
    /// playback rate): starts new ones, updates the others, stops the rest.
    pub fn sync_loops(&mut self, bundle: &mut Bundle, wanted: &[(String, f32, f32)]) {
        self.loops.retain(|k, h| {
            let keep = wanted.iter().any(|(w, _, _)| w == k);
            if !keep {
                h.stop(Tween::default());
            }
            keep
        });
        for (key, volume, pitch) in wanted {
            let rate = f64::from(pitch.max(0.01));
            if let Some(h) = self.loops.get_mut(key) {
                h.set_volume(decibels(*volume), Tween::default());
                h.set_playback_rate(rate, Tween::default());
                continue;
            }
            let Some(data) = self.data(bundle, key) else {
                continue;
            };
            let data = data
                .loop_region(..)
                .volume(decibels(*volume))
                .playback_rate(rate);
            match self.manager.play(data) {
                Ok(h) => {
                    self.loops.insert(key.clone(), h);
                }
                Err(e) => eprintln!("warning: playing sound {key}: {e}"),
            }
        }
    }

    fn data(&mut self, bundle: &mut Bundle, key: &str) -> Option<StaticSoundData> {
        self.sounds
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
            .clone()
    }
}

/// A linear volume (0–1) in decibels.
fn decibels(volume: f32) -> Decibels {
    if volume > 0.0 {
        Decibels((20.0 * volume.log10()).max(Decibels::SILENCE.0))
    } else {
        Decibels::SILENCE
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
