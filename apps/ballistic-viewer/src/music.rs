//! The music during a level, like the original's `Music_Manager` (research
//! findings 26), and the last level's storm (`Gameplay_Blitz`).
//!
//! - *Start Music* (the level start) fades the music in over 1 s.
//! - **Ambience**: after a random wait (up to 15 s) one of the three ambient
//!   pieces plays; when it ends, another wait. Likewise the level's three
//!   **themes**, from 7 s into the level, with waits up to 50 s. (A piece
//!   due while the music is off waits until it's on.)
//! - At the **last checkpoint** the themes stop and the last-stage loop
//!   sounds, fading out over 10 s. Within 200 of the level end it fades in
//!   over 13 s (and out over 1.5 s on leaving). At the **finish** it fades
//!   out over 0.4 s and the finish music plays (another on the last level).
//! - A new highscore plays its tune.
//! - **Storm** (the last level): 4 s in, then every 10 to 90 s, a flash of
//!   light (0.2 s) and, 0.15 s later, thunder.
//!
//! The player emits [`Cmd`]s; the window carries them out. Random waits use
//! a generator seeded per level, so headless runs repeat.

use std::collections::HashMap;

use ballistic_bundle as bb;
use ballistic_physics::Vec3;

/// What the audio is to do with the music.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// Start `key` as voice `id`, fading in over `fade_in_ms`.
    Play {
        id: u32,
        key: String,
        fade_in_ms: f32,
        looped: bool,
    },
    /// Stop voice `id`, fading out over `fade_out_ms`.
    Stop { id: u32, fade_out_ms: f32 },
    /// The music's volume (0–1), reached over `ms`.
    Volume { volume: f32, ms: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Slot {
    Off,
    Waiting { ms: f32 },
    Playing { id: u32, ms: f32 },
}

pub struct MusicPlayer {
    data: bb::Music,
    /// Each sound's length (ms).
    lengths: HashMap<String, f32>,
    themes: Vec<String>,
    last_level: bool,
    level_end: Option<Vec3>,
    rng: u64,
    on: bool,
    atmo: Slot,
    theme: Slot,
    /// The last stage's voice once the last checkpoint is reached, and
    /// whether the ball is near the end.
    last_stage: Option<(u32, bool)>,
    finished: bool,
    next_id: u32,
    cmds: Vec<Cmd>,
    storm: Option<StormState>,
}

#[derive(Debug, Clone, Copy)]
struct StormState {
    /// Until the next flash (ms).
    next: f32,
    /// Into the flash (ms).
    flash: Option<f32>,
    /// Until the thunder (ms).
    thunder: Option<f32>,
}

impl MusicPlayer {
    /// For level `level` (0-based; `None`: no themes), `last_level` if it's
    /// the last; `level_end`: where the level ends; `lengths`: sound lengths
    /// (ms) by key.
    pub fn new(
        data: &bb::Music,
        lengths: HashMap<String, f32>,
        level: Option<usize>,
        last_level: bool,
        level_end: Option<Vec3>,
    ) -> Self {
        let mut m = Self {
            data: data.clone(),
            lengths,
            themes: level
                .and_then(|l| data.themes.get(l))
                .cloned()
                .unwrap_or_default(),
            last_level,
            level_end,
            rng: 0x9E37_79B9_7F4A_7C15 ^ (level.unwrap_or(99) as u64 + 1).wrapping_mul(0x2545_F491),
            on: false,
            atmo: Slot::Off,
            theme: Slot::Off,
            last_stage: None,
            finished: false,
            next_id: 1,
            cmds: Vec::new(),
            storm: last_level.then_some(StormState {
                next: data.storm.first_ms,
                flash: None,
                thunder: None,
            }),
        };
        let atmo_wait = m.random(m.data.atmo_gap_ms);
        m.atmo = Slot::Waiting { ms: atmo_wait };
        if !m.themes.is_empty() {
            m.theme = Slot::Waiting {
                ms: m.data.theme_start_ms,
            };
        }
        m
    }

    /// A uniform random value in `0..max`.
    fn random(&mut self, max: f32) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let r = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        #[expect(clippy::cast_precision_loss, reason = "24 random bits")]
        let unit = r as f32 / (1u64 << 24) as f32;
        unit * max
    }

    fn voice(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    pub fn take_cmds(&mut self) -> Vec<Cmd> {
        std::mem::take(&mut self.cmds)
    }

    /// *Start Music*.
    pub fn start_music(&mut self) {
        self.on = true;
        self.cmds.push(Cmd::Volume {
            volume: 1.0,
            ms: self.data.fade_ms,
        });
    }

    /// *last Checkpoint reached*: the themes stop, the last stage sounds and
    /// fades out.
    pub fn last_checkpoint(&mut self) {
        if let Slot::Playing { id, .. } = self.theme {
            self.cmds.push(Cmd::Stop {
                id,
                fade_out_ms: 0.0,
            });
        }
        self.theme = Slot::Off;
        if self.last_stage.is_some() || self.data.last_stage.is_empty() {
            return;
        }
        let id = self.voice();
        self.cmds.push(Cmd::Play {
            id,
            key: self.data.last_stage.clone(),
            fade_in_ms: 0.0,
            looped: true,
        });
        self.cmds.push(Cmd::Stop {
            id,
            fade_out_ms: self.data.last_stage_fade_out_ms,
        });
        self.last_stage = Some((id, false));
    }

    /// `Level_Finish`: the last stage fades out; the finish music plays.
    pub fn level_finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some((id, _)) = self.last_stage {
            self.cmds.push(Cmd::Stop {
                id,
                fade_out_ms: self.data.finish_fade_out_ms,
            });
        }
        let key = if self.last_level {
            self.data.last_finish.clone()
        } else {
            self.data.finish.clone()
        };
        let id = self.voice();
        self.cmds.push(Cmd::Play {
            id,
            key,
            fade_in_ms: 0.0,
            looped: false,
        });
    }

    /// `Menu_Highscore`.
    pub fn highscore(&mut self) {
        let id = self.voice();
        self.cmds.push(Cmd::Play {
            id,
            key: self.data.highscore.clone(),
            fade_in_ms: 0.0,
            looped: false,
        });
    }

    /// One frame (real time); `ball` where the ball is, `end` the level end
    /// (when it has moved). Returns one-shot sounds (the thunder).
    pub fn frame(&mut self, real_dt: f32, ball: Option<Vec3>, end: Option<Vec3>) -> Vec<String> {
        if end.is_some() {
            self.level_end = end;
        }
        let ms = real_dt * 1000.0;
        let atmo = self.data.atmo.clone();
        self.atmo = self.slot(self.atmo, ms, &atmo, self.data.atmo_gap_ms);
        let themes = self.themes.clone();
        self.theme = self.slot(self.theme, ms, &themes, self.data.theme_gap_ms);
        // Near the level end after the last checkpoint.
        if let (Some((id, near)), Some(end), Some(ball)) = (self.last_stage, self.level_end, ball)
            && !self.finished
        {
            let inside = ball.distance(end) < self.data.near_end_distance;
            let [fade_in, fade_out] = self.data.near_end_fades_ms;
            if inside && !near {
                self.cmds.push(Cmd::Play {
                    id,
                    key: self.data.last_stage.clone(),
                    fade_in_ms: fade_in,
                    looped: true,
                });
            } else if !inside && near {
                self.cmds.push(Cmd::Stop {
                    id,
                    fade_out_ms: fade_out,
                });
            }
            self.last_stage = Some((id, inside));
        }
        self.storm_frame(ms)
    }

    fn slot(&mut self, slot: Slot, ms: f32, choices: &[String], gap_ms: f32) -> Slot {
        match slot {
            Slot::Off => Slot::Off,
            Slot::Waiting { ms: left } if left - ms > 0.0 => Slot::Waiting { ms: left - ms },
            // Due: plays once the music is on.
            Slot::Waiting { .. } if !self.on || choices.is_empty() => Slot::Waiting { ms: 0.0 },
            Slot::Waiting { .. } => {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "an index among three"
                )]
                let pick = (self.random(choices.len() as f32) as usize).min(choices.len() - 1);
                let key = choices[pick].clone();
                let length = self.lengths.get(&key).copied().unwrap_or(0.0);
                let id = self.voice();
                self.cmds.push(Cmd::Play {
                    id,
                    key,
                    fade_in_ms: 0.0,
                    looped: false,
                });
                Slot::Playing { id, ms: length }
            }
            Slot::Playing { id, ms: left } if left - ms > 0.0 => {
                Slot::Playing { id, ms: left - ms }
            }
            Slot::Playing { .. } => Slot::Waiting {
                ms: self.random(gap_ms),
            },
        }
    }

    fn storm_frame(&mut self, ms: f32) -> Vec<String> {
        let Some(mut s) = self.storm else {
            return Vec::new();
        };
        let mut out = Vec::new();
        s.next -= ms;
        if s.next <= 0.0 {
            let [lo, hi] = self.data.storm.gap_ms;
            s.next = lo + self.random(hi - lo);
            s.flash = Some(0.0);
            s.thunder = Some(self.data.storm.thunder_delay_ms);
        }
        if let Some(t) = s.flash.as_mut() {
            *t += ms;
            if *t > self.data.storm.flash.duration_ms {
                s.flash = None;
            }
        }
        if let Some(t) = s.thunder.as_mut() {
            *t -= ms;
            if *t <= 0.0 {
                s.thunder = None;
                out.push(self.data.thunder.clone());
            }
        }
        self.storm = Some(s);
        out
    }

    /// The storm's light now (added to the scene's light), if flashing.
    pub fn flash(&self) -> Option<[f32; 4]> {
        let t = self.storm?.flash?;
        let k = self.data.storm.flash.value(t);
        Some(self.data.storm.color.map(|c| c * k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> bb::Music {
        bb::Music {
            fade_ms: 1000.0,
            atmo: vec!["a1".into(), "a2".into(), "a3".into()],
            atmo_gap_ms: 15000.0,
            themes: vec![vec!["t1".into(), "t2".into(), "t3".into()]],
            theme_gap_ms: 50000.0,
            theme_start_ms: 7000.0,
            last_stage: "last".into(),
            last_stage_fade_out_ms: 10000.0,
            near_end_distance: 200.0,
            near_end_fades_ms: [13000.0, 1500.0],
            finish_fade_out_ms: 400.0,
            finish: "final".into(),
            last_finish: "lastfinal".into(),
            highscore: "hs".into(),
            thunder: "thunder".into(),
            storm: bb::Storm {
                first_ms: 4000.0,
                gap_ms: [10000.0, 90000.0],
                flash: bb::Progression {
                    duration_ms: 200.0,
                    curve: Vec::new(),
                },
                color: [1.0; 4],
                thunder_delay_ms: 150.0,
            },
        }
    }

    fn lengths() -> HashMap<String, f32> {
        ["a1", "a2", "a3", "t1", "t2", "t3"]
            .iter()
            .map(|k| ((*k).to_owned(), 30000.0))
            .collect()
    }

    fn played(cmds: &[Cmd]) -> Vec<String> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Play { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    /// Runs `seconds` at 60 frames a second; every command.
    fn run(m: &mut MusicPlayer, seconds: f32, ball: Option<Vec3>) -> Vec<Cmd> {
        let mut out = Vec::new();
        #[expect(clippy::cast_sign_loss, reason = "positive durations")]
        for _ in 0..(seconds * 60.0) as u32 {
            m.frame(1.0 / 60.0, ball, None);
            out.extend(m.take_cmds());
        }
        out
    }

    #[test]
    fn nothing_plays_until_the_music_starts_then_ambience_and_themes() {
        let mut m = MusicPlayer::new(&data(), lengths(), Some(0), false, None);
        assert_eq!(played(&run(&mut m, 20.0, None)), Vec::<String>::new());
        m.start_music();
        let cmds = run(&mut m, 1.0, None);
        assert_eq!(
            cmds.first(),
            Some(&Cmd::Volume {
                volume: 1.0,
                ms: 1000.0
            })
        );
        // Both were due: an ambient piece and a theme now.
        let keys = played(&cmds);
        assert!(keys.iter().any(|k| k.starts_with('a')), "{keys:?}");
        assert!(keys.iter().any(|k| k.starts_with('t')), "{keys:?}");
        // Nothing more while they play (30 s).
        assert_eq!(played(&run(&mut m, 25.0, None)), Vec::<String>::new());
    }

    #[test]
    fn the_last_stage_stops_the_themes_and_comes_back_near_the_end() {
        let end = Vec3::new(0.0, 0.0, 0.0);
        let mut m = MusicPlayer::new(&data(), lengths(), Some(0), false, Some(end));
        m.start_music();
        run(&mut m, 8.0, Some(Vec3::new(1000.0, 0.0, 0.0)));
        m.last_checkpoint();
        let cmds = m.take_cmds();
        // The theme playing stops at once.
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Stop { fade_out_ms, .. } if *fade_out_ms == 0.0))
        );
        assert!(
            matches!(cmds.last(), Some(Cmd::Stop { fade_out_ms, .. }) if (*fade_out_ms - 10000.0).abs() < 1e-3)
        );
        // Near the end: the loop fades in; the finish fades it out.
        let cmds = run(&mut m, 0.1, Some(Vec3::new(50.0, 0.0, 0.0)));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Play { key, fade_in_ms, looped: true, .. } if key == "last" && (*fade_in_ms - 13000.0).abs() < 1e-3)));
        m.level_finish();
        let cmds = m.take_cmds();
        assert!(cmds.iter().any(
            |c| matches!(c, Cmd::Stop { fade_out_ms, .. } if (*fade_out_ms - 400.0).abs() < 1e-3)
        ));
        assert_eq!(played(&cmds), ["final"]);
        // No themes after the last checkpoint.
        assert!(
            !played(&run(&mut m, 120.0, None))
                .iter()
                .any(|k| k.starts_with('t'))
        );
    }

    #[test]
    fn the_last_level_storms() {
        let mut m = MusicPlayer::new(&data(), lengths(), Some(0), true, None);
        let mut thunder = 0;
        let mut flashed = false;
        for _ in 0..(120 * 60) {
            thunder += m.frame(1.0 / 60.0, None, None).len();
            flashed |= m.flash().is_some();
        }
        // The first at 4 s, then every 10 to 90 s.
        assert!(flashed);
        assert!((2..=13).contains(&thunder), "{thunder}");
        let mut calm = MusicPlayer::new(&data(), lengths(), Some(0), false, None);
        assert!((0..600).all(|_| calm.frame(1.0 / 60.0, None, None).is_empty()));
    }
}
