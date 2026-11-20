//! The score screen after a level, like the original's `Menu_Score`
//! (research findings 23).
//!
//! - The field, highlight and line fade in (200 ms) with the four labels and
//!   values (all 0), the highlight on the first row.
//! - A second later the **level bonus** shows (*Dong*). A second after that
//!   the **points** are counted over: each frame a step goes from the points
//!   left (the HUD counts down) to the row, 1 at a time to 80, then 5 to 500,
//!   then 25, the counter sound every other frame, higher at each speed. Then
//!   a *Dong*.
//! - A second later each **life** left is counted (one every 610 ms, the HUD
//!   taking a ball each time), the last with a *Dong*. A second later the
//!   highlight goes to the **total** (*Dong*).
//! - The total stays 4 s, or until Space, Enter or Escape (*Click*); then the
//!   screen fades out.
//! - **Fast scoring**: one of those keys while counting shows every value at
//!   once and takes the lives left one every 700 ms; the wait starts.
//!
//! The 4:3 layout is centred in the window. Counting runs at 60 frames a
//! second (the original counted per frame).

use std::collections::HashMap;

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Gpu, Overlay, Quad, upload_ktx2};

use crate::hud::{REFERENCE_HEIGHT, TextStyle, text_quads};
use crate::play::Bundle;

/// The counting's frame time.
const COUNT_FRAME_S: f32 = 1.0 / 60.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Play a bundle sound at this pitch (playback rate).
    Sound { key: String, pitch: f32 },
    /// A message to the sound scripts (`Menu_Dong`, `Menu_Click`).
    Message(&'static str),
    /// A life counted: the HUD takes a ball.
    SubLife,
    /// The points left to count (the HUD shows them).
    Points(i32),
    /// Gone; the level's score.
    Done(i32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// Not started.
    Idle,
    FadeIn {
        ms: f32,
    },
    /// Before `next` (1–4: level bonus, points, lives, total).
    Pause {
        ms: f32,
        next: u8,
    },
    Points {
        row: usize,
    },
    Lives {
        ms: f32,
    },
    /// Counting stopped by fast scoring before it got here.
    Stalled,
    Wait {
        ms: f32,
    },
    FadeOut {
        ms: f32,
    },
    /// Faded out; leaves once the lives are taken.
    Exit,
    Done,
}

pub struct ScoreScreen {
    data: bb::ScoreScreen,
    language: String,
    /// `None` in tests.
    overlay: Option<Overlay>,
    /// Texture key (`None`: white) → overlay texture.
    textures: HashMap<Option<String>, usize>,
    stage: Stage,
    /// What the rows end at: level bonus, points, lives × bonus.
    target: [i32; 3],
    shown: [i32; 3],
    points_left: i32,
    lives_left: i32,
    life_bonus: i32,
    highlight: [f32; 2],
    /// Sprites' and texts' opacity (the fades).
    alpha: f32,
    /// `Weiter?`: not fast scoring.
    counting: bool,
    /// Fast scoring is possible (from the fade-in to the total).
    keys_on: bool,
    /// The lives are all taken (the screen may leave).
    exit_possible: bool,
    /// Fast scoring: the next life goes in this long.
    fast_lives_ms: Option<f32>,
    /// Counting frames: time over, and the counter sound's sequencer step.
    count_s: f32,
    sequencer: u8,
    /// The counter's pitch: from, to, ms into the ramp.
    pitch: ([f32; 2], f32),
    events: Vec<Event>,
}

impl ScoreScreen {
    /// Labels in `language` (a `Language.nmo` column; `english` if
    /// missing). Hidden until [`Self::start`].
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        data: &bb::ScoreScreen,
        format: wgpu::TextureFormat,
        language: &str,
    ) -> Result<Self> {
        let mut overlay = Overlay::new(gpu, format);
        let mut textures = HashMap::new();
        let keys = data
            .sprites
            .iter()
            .map(|s| s.texture.clone())
            .chain([Some(data.font.texture.clone())]);
        for key in keys {
            if textures.contains_key(&key) {
                continue;
            }
            let tex = match &key {
                Some(k) => upload_ktx2(gpu, k, &bundle.texture(k)?)?,
                None => ballistic_render::white(gpu),
            };
            textures.insert(key, overlay.add_texture(gpu, &tex));
        }
        let mut s = Self::with_textures(data, language, textures);
        s.overlay = Some(overlay);
        Ok(s)
    }

    fn with_textures(
        data: &bb::ScoreScreen,
        language: &str,
        textures: HashMap<Option<String>, usize>,
    ) -> Self {
        Self {
            data: data.clone(),
            language: language.to_owned(),
            overlay: None,
            textures,
            stage: Stage::Idle,
            target: [0; 3],
            shown: [0; 3],
            points_left: 0,
            lives_left: 0,
            life_bonus: 0,
            highlight: data.highlight_rows.first().copied().unwrap_or_default(),
            alpha: 0.0,
            counting: true,
            keys_on: false,
            exit_possible: false,
            fast_lives_ms: None,
            count_s: 0.0,
            sequencer: 0,
            pitch: ([1.0, 1.0], 0.0),
            events: Vec::new(),
        }
    }

    /// `Menu_Score` starts: `score` from the level's energy, with `lives`
    /// left.
    pub fn start(&mut self, score: crate::energy::Score, lives: i32) {
        let lives = lives.max(0);
        *self = Self {
            data: std::mem::take(&mut self.data),
            language: std::mem::take(&mut self.language),
            overlay: self.overlay.take(),
            textures: std::mem::take(&mut self.textures),
            ..Self::with_textures(&bb::ScoreScreen::default(), "", HashMap::new())
        };
        self.stage = Stage::FadeIn { ms: 0.0 };
        self.target = [score.level_bonus, score.points, score.life_bonus];
        self.points_left = score.points;
        self.lives_left = lives;
        self.life_bonus = if lives > 0 {
            score.life_bonus / lives
        } else {
            0
        };
        self.highlight = self
            .data
            .highlight_rows
            .first()
            .copied()
            .unwrap_or_default();
    }

    /// Started and not yet gone.
    pub fn active(&self) -> bool {
        !matches!(self.stage, Stage::Idle | Stage::Done)
    }

    pub fn total(&self) -> i32 {
        self.shown.iter().sum()
    }

    #[cfg(test)]
    pub fn done(&self) -> bool {
        self.stage == Stage::Done
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// A key (`DirectInput` scan code) went down; whether the screen took it.
    pub fn key(&mut self, scan: u32) -> bool {
        if !self.data.keys.contains(&scan) {
            return false;
        }
        match self.stage {
            Stage::Wait { .. } => {
                self.events.push(Event::Message("Menu_Click"));
                self.stage = Stage::FadeOut { ms: 0.0 };
                true
            }
            _ if self.keys_on && self.counting => {
                self.fast_scoring();
                true
            }
            _ => false,
        }
    }

    /// *Fast Scoring*: every value at once; the lives left go one by one.
    fn fast_scoring(&mut self) {
        self.events.push(Event::Message("Menu_Dong"));
        self.counting = false;
        self.keys_on = false;
        self.highlight = self.data.highlight_fast;
        self.shown = self.target;
        self.points_left = 0;
        self.events.push(Event::Points(0));
        self.fast_lives_ms = Some(0.0);
        self.stage = Stage::Wait {
            ms: self.data.wait_ms,
        };
    }

    pub fn frame(&mut self, real_dt: f32) {
        let ms = real_dt * 1000.0;
        if let Some(t) = self.fast_lives_ms.as_mut() {
            *t -= ms;
            if *t <= 0.0 {
                if self.lives_left > 0 {
                    self.lives_left -= 1;
                    self.events.push(Event::SubLife);
                    *t += self.data.fast_life_ms;
                } else {
                    self.exit_possible = true;
                    self.fast_lives_ms = None;
                }
            }
        }
        self.pitch.1 += ms;
        let fade = self.data.fade_ms.max(1.0);
        self.stage = match self.stage {
            Stage::FadeIn { ms: t } => {
                let t = t + ms;
                self.alpha = (t / fade).min(1.0);
                if t >= fade {
                    self.keys_on = true;
                    Stage::Pause {
                        ms: self.data.step_ms,
                        next: 1,
                    }
                } else {
                    Stage::FadeIn { ms: t }
                }
            }
            Stage::Pause { ms: t, next } => {
                let t = t - ms;
                if t > 0.0 {
                    Stage::Pause { ms: t, next }
                } else if self.counting {
                    self.step(next)
                } else {
                    Stage::Stalled
                }
            }
            Stage::Points { row } => {
                self.count_s += real_dt;
                let mut stage = Stage::Points { row };
                while self.count_s >= COUNT_FRAME_S {
                    self.count_s -= COUNT_FRAME_S;
                    let Stage::Points { row } = stage else {
                        break;
                    };
                    stage = self.count_points(row);
                }
                stage
            }
            Stage::Lives { ms: t } => {
                let t = t - ms;
                if t > 0.0 {
                    Stage::Lives { ms: t }
                } else {
                    self.count_life()
                }
            }
            Stage::Wait { ms: t } => {
                let t = t - ms;
                if t > 0.0 {
                    Stage::Wait { ms: t }
                } else {
                    Stage::FadeOut { ms: 0.0 }
                }
            }
            Stage::FadeOut { ms: t } => {
                let t = t + ms;
                self.alpha = (1.0 - t / fade).max(0.0);
                if t >= fade {
                    Stage::Exit
                } else {
                    Stage::FadeOut { ms: t }
                }
            }
            Stage::Exit if self.exit_possible => {
                self.events.push(Event::Done(self.total()));
                Stage::Done
            }
            s => s,
        };
    }

    /// Score 1 to 4.
    fn step(&mut self, n: u8) -> Stage {
        let row = usize::from(n - 1);
        if let Some(&p) = self.data.highlight_rows.get(row) {
            self.highlight = p;
        }
        match n {
            1 => {
                self.events.push(Event::Message("Menu_Dong"));
                self.shown[0] = self.target[0];
                Stage::Pause {
                    ms: self.data.step_ms,
                    next: 2,
                }
            }
            2 => {
                self.set_pitch(0);
                Stage::Points { row: 0 }
            }
            3 => Stage::Lives { ms: 0.0 },
            _ => {
                self.events.push(Event::Message("Menu_Dong"));
                self.keys_on = false;
                Stage::Wait {
                    ms: self.data.wait_ms,
                }
            }
        }
    }

    fn set_pitch(&mut self, row: usize) {
        let p = self.data.count_pitch.get(row).copied().unwrap_or([1.0; 2]);
        self.pitch = (p, 0.0);
    }

    fn current_pitch(&self) -> f32 {
        let ([a, b], t) = self.pitch;
        let k = (t / self.data.pitch_ramp_ms.max(1.0)).clamp(0.0, 1.0);
        a + (b - a) * k
    }

    /// One counting frame of *Score 2*.
    fn count_points(&mut self, mut row: usize) -> Stage {
        loop {
            let Some(&[limit, step]) = self.data.count_speed.get(row) else {
                return self.points_counted();
            };
            if self.shown[1] >= limit {
                row += 1;
                self.set_pitch(row);
                continue;
            }
            // The counter sound: its sequencer plays every other frame.
            self.sequencer = (self.sequencer + 1) % 4;
            if self.sequencer.is_multiple_of(2) {
                self.events.push(Event::Sound {
                    key: self.data.counter_sound.clone(),
                    pitch: self.current_pitch(),
                });
            }
            if self.points_left >= step && step > 0 {
                self.points_left -= step;
                self.shown[1] += step;
                self.events.push(Event::Points(self.points_left));
                return Stage::Points { row };
            }
            return self.points_counted();
        }
    }

    fn points_counted(&mut self) -> Stage {
        self.shown[1] = self.target[1];
        self.points_left = 0;
        self.events.push(Event::Points(0));
        self.events.push(Event::Message("Menu_Dong"));
        Stage::Pause {
            ms: self.data.step_ms,
            next: 3,
        }
    }

    /// One life of *Score 3*.
    fn count_life(&mut self) -> Stage {
        if !self.counting {
            return Stage::Stalled;
        }
        if self.lives_left > 0 {
            self.lives_left -= 1;
            self.shown[2] += self.life_bonus;
            if self.lives_left > 0 {
                self.events.push(Event::Sound {
                    key: self.data.counter_sound.clone(),
                    pitch: 1.0,
                });
            } else {
                self.events.push(Event::Message("Menu_Dong"));
            }
            self.events.push(Event::SubLife);
            return Stage::Lives {
                ms: self.data.life_ms,
            };
        }
        self.shown[2] = self.target[2];
        self.exit_possible = true;
        Stage::Pause {
            ms: self.data.step_ms,
            next: 4,
        }
    }

    /// The quads for a `size` window.
    pub fn quads(&self, size: (u32, u32)) -> Vec<Quad> {
        if self.alpha <= 0.0 || !self.active() {
            return Vec::new();
        }
        #[expect(clippy::cast_precision_loss, reason = "window sizes")]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let fw = w.min(h * 4.0 / 3.0);
        let fh = fw * 0.75;
        let (x0, y0) = ((w - fw) * 0.5, (h - fh) * 0.5);
        let px = |r: [f32; 4]| {
            [
                x0 + r[0] * fw,
                y0 + r[1] * fh,
                x0 + r[2] * fw,
                y0 + r[3] * fh,
            ]
        };
        let mut quads = Vec::new();
        for s in &self.data.sprites {
            let Some(&texture) = self.textures.get(&s.texture) else {
                continue;
            };
            let mut rect = s.rect;
            if s.name == self.data.highlight {
                let [x, y] = self.highlight;
                rect = [x, y, x + rect[2] - rect[0], y + rect[3] - rect[1]];
            }
            let mut color = s.color;
            color[3] *= self.alpha;
            quads.push(Quad {
                texture,
                rect: px(rect),
                uv: s.source,
                color,
            });
        }
        let f = &self.data.font;
        let Some(&texture) = self.textures.get(&Some(f.texture.clone())) else {
            return quads;
        };
        let values = [self.shown[0], self.shown[1], self.shown[2], self.total()];
        let sh = &self.data.shadow;
        let fade = |mut c: [f32; 4]| {
            c[3] *= self.alpha;
            c
        };
        let text_style = TextStyle {
            align: self.data.align,
            margins: self.data.margins,
            scale: f.scale,
            color: fade(f.color),
        };
        let shadow_style = TextStyle {
            scale: sh.size,
            color: fade(sh.color),
            ..text_style
        };
        // The shadow's offset: *Shadow Angle* and *Distance* (pixels at 480
        // lines; screen y down).
        let d = sh.distance * fh / REFERENCE_HEIGHT;
        let (dx, dy) = (sh.angle.cos() * d, sh.angle.sin() * d);
        let mut shadows = Vec::new();
        let mut texts = Vec::new();
        for (row, value) in self.data.rows.iter().zip(values) {
            let label = row
                .label
                .get(&self.language)
                .or_else(|| row.label.get("english"))
                .cloned()
                .unwrap_or_default();
            for (s, rect) in [(label, row.label_rect), (value.to_string(), row.value_rect)] {
                let r = px(rect);
                texts.extend(text_quads(f, texture, &s, r, &text_style, (fw, fh)));
                let r = [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy];
                shadows.extend(text_quads(f, texture, &s, r, &shadow_style, (fw, fh)));
            }
        }
        quads.extend(shadows);
        quads.extend(texts);
        quads
    }

    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        let quads = self.quads(size);
        if let Some(o) = self.overlay.as_mut() {
            o.render(gpu, encoder, color, size, &quads);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::energy::Score;

    fn data() -> bb::ScoreScreen {
        bb::ScoreScreen {
            fade_ms: 200.0,
            highlight_rows: vec![[0.3, 0.3], [0.3, 0.36], [0.3, 0.42], [0.3, 0.5]],
            highlight_fast: [0.32, 0.5],
            step_ms: 1000.0,
            count_speed: vec![[80, 1], [500, 5], [9999, 25]],
            count_pitch: vec![[1.0, 1.0], [1.0, 1.1], [1.1, 1.2]],
            pitch_ramp_ms: 100.0,
            counter_sound: "menu_counter".into(),
            life_ms: 610.0,
            fast_life_ms: 700.0,
            wait_ms: 4000.0,
            keys: vec![57, 28, 1],
            ..bb::ScoreScreen::default()
        }
    }

    fn score() -> Score {
        Score {
            level_bonus: 300,
            points: 700,
            life_bonus: 400,
            total: 1400,
        }
    }

    fn screen() -> ScoreScreen {
        let mut s = ScoreScreen::with_textures(&data(), "english", HashMap::new());
        assert!(!s.active());
        s.start(score(), 2);
        assert!(s.active());
        s
    }

    /// Runs until `done` or `seconds`; all events.
    fn run(s: &mut ScoreScreen, seconds: f32) -> Vec<Event> {
        let mut out = Vec::new();
        let dt = 1.0 / 60.0;
        #[expect(clippy::cast_sign_loss, reason = "positive durations")]
        for _ in 0..(seconds / dt).round() as u32 {
            s.frame(dt);
            out.extend(s.take_events());
        }
        out
    }

    fn dongs(e: &[Event]) -> usize {
        e.iter()
            .filter(|e| **e == Event::Message("Menu_Dong"))
            .count()
    }

    #[test]
    fn rows_count_up_in_turn_then_the_screen_fades() {
        let mut s = screen();
        // Fade in, then the level bonus a second later.
        let e = run(&mut s, 1.1);
        assert_eq!(s.shown, [0; 3]);
        assert_eq!(dongs(&e), 0);
        let e = run(&mut s, 0.2);
        assert_eq!(s.shown[0], 300);
        assert_eq!(dongs(&e), 1);
        // A second later the points: 80 frames of 1, 84 of 5, 12 of 25.
        run(&mut s, 1.0);
        let e = run(&mut s, 2.0);
        assert!(s.shown[1] > 80 && s.shown[1] < 700, "{:?}", s.shown);
        assert!(
            e.iter()
                .any(|e| matches!(e, Event::Sound { pitch, .. } if *pitch > 1.0))
        );
        let e = run(&mut s, 1.5);
        assert_eq!(s.shown[1], 700);
        assert!(e.contains(&Event::Points(0)));
        // The lives, one every 610 ms, the HUD taking each.
        let e = run(&mut s, 2.5);
        assert_eq!(s.shown[2], 400);
        assert_eq!(e.iter().filter(|e| **e == Event::SubLife).count(), 2);
        assert_eq!(s.total(), 1400);
        // The total, the wait, the fade out.
        let e = run(&mut s, 6.0);
        assert!(s.done());
        assert_eq!(e.last(), Some(&Event::Done(1400)));
    }

    #[test]
    fn fast_scoring_shows_everything_and_takes_the_lives() {
        let mut s = screen();
        // Not during the fade in.
        run(&mut s, 0.1);
        assert!(!s.key(57));
        run(&mut s, 1.5);
        assert!(s.key(57));
        assert_eq!(s.total(), 1400);
        let e = run(&mut s, 0.1);
        assert_eq!(e.iter().filter(|e| **e == Event::SubLife).count(), 1);
        // The wait can be cut short, but the screen leaves once the lives
        // are taken.
        assert!(s.key(28));
        let e = run(&mut s, 0.7);
        assert!(!s.done());
        assert_eq!(e.iter().filter(|e| **e == Event::SubLife).count(), 1);
        run(&mut s, 1.0);
        assert!(s.done());
        // Other keys aren't taken.
        assert!(!s.key(30));
    }
}
