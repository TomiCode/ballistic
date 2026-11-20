//! The highscore name entry and table after a level, like the original's
//! `Menu_HighscoreEntry` and `Menu_Highscore` (research findings 24).
//!
//! - **Name entry** (a new highscore): over a dark band, *New highscore
//!   entry!*, the score, and the name, starting as the last one entered.
//!   Typing adds characters (up to 9), Backspace takes one off, Enter
//!   confirms.
//! - **Table**: *Highscore Level N* with its ten places (number, name,
//!   score). Left and Right go through the levels; Enter or Escape leaves
//!   (the button says *Next*).
//!
//! Keys only for now: the buttons can't be clicked yet (that comes with the
//! menus). The 4:3 layout is centred in the window.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Gpu, Overlay, Quad, upload_ktx2};
use ballistic_save::Save;

use crate::hud::{REFERENCE_HEIGHT, TextStyle, text_block, text_quads};
use crate::play::Bundle;

/// Scan code of Backspace (the original's `TT InputString` edits with it).
pub const BACKSPACE: u32 = 14;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A message to the sound scripts (`Menu_Highscore`, `Menu_Click`).
    Message(&'static str),
    /// The name was confirmed.
    Named(String),
    /// The table was left.
    Left,
}

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Entry {
        score: i32,
        name: String,
    },
    Table {
        /// 0-based.
        level: usize,
        levels: usize,
        /// Arrow keys held: previous, next.
        pressed: [bool; 2],
        /// Opened after a level (the button says *Next*), else from a menu
        /// (*Back*).
        after_level: bool,
    },
}

pub struct HighscoreScreens {
    data: bb::Highscores,
    /// The menu fonts, by name.
    fonts: BTreeMap<String, bb::HudFont>,
    language: String,
    /// `None` in tests.
    overlay: Option<Overlay>,
    /// Texture key (`None`: white) → overlay texture.
    textures: HashMap<Option<String>, usize>,
    screen: Option<Screen>,
    events: Vec<Event>,
}

impl HighscoreScreens {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        data: &bb::Highscores,
        fonts: &BTreeMap<String, bb::HudFont>,
        format: wgpu::TextureFormat,
        language: &str,
    ) -> Result<Self> {
        let mut overlay = Overlay::new(gpu, format);
        let mut textures = HashMap::new();
        let keys = data
            .entry
            .sprites
            .iter()
            .chain(&data.table.sprites)
            .map(|s| s.texture.clone())
            .chain([data.table.arrow_pressed.0.clone(), None])
            .chain(fonts.values().map(|f| Some(f.texture.clone())));
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
        let mut s = Self::with_textures(data, fonts, language, textures);
        s.overlay = Some(overlay);
        Ok(s)
    }

    pub(crate) fn with_textures(
        data: &bb::Highscores,
        fonts: &BTreeMap<String, bb::HudFont>,
        language: &str,
        textures: HashMap<Option<String>, usize>,
    ) -> Self {
        Self {
            data: data.clone(),
            fonts: fonts.clone(),
            language: language.to_owned(),
            overlay: None,
            textures,
            screen: None,
            events: Vec::new(),
        }
    }

    /// The name entry for `score`, starting with `name`.
    pub fn start_entry(&mut self, score: i32, name: &str) {
        let max = self.data.entry.max_len as usize;
        self.screen = Some(Screen::Entry {
            score,
            name: name.chars().take(max).collect(),
        });
        self.events.push(Event::Message("Menu_Highscore"));
    }

    /// The table, at `level` (0-based) of `levels`, after a level or from a
    /// menu.
    pub fn start_table(&mut self, level: usize, levels: usize, after_level: bool) {
        self.screen = Some(Screen::Table {
            level,
            levels: levels.max(1),
            pressed: [false; 2],
            after_level,
        });
    }

    pub fn active(&self) -> bool {
        self.screen.is_some()
    }

    /// The name entry is on (it takes typed text).
    pub fn typing(&self) -> bool {
        matches!(self.screen, Some(Screen::Entry { .. }))
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Typed text, for the name.
    pub fn text(&mut self, s: &str) {
        let max = self.data.entry.max_len as usize;
        let font = self.fonts.get(&self.data.entry.name.font);
        let Some(Screen::Entry { name, .. }) = self.screen.as_mut() else {
            return;
        };
        for c in s.chars() {
            // Characters the font has (it's indexed by code).
            let known = font.is_some_and(|f| (c as usize) < f.glyphs.len());
            if !c.is_control() && known && name.chars().count() < max {
                name.push(c);
            }
        }
    }

    /// A key (`DirectInput` scan code) went down; whether a screen took it.
    pub fn key(&mut self, scan: u32) -> bool {
        let t = &self.data.table;
        match self.screen.as_mut() {
            Some(Screen::Entry { name, .. }) => {
                if scan == BACKSPACE {
                    name.pop();
                    true
                } else if self.data.entry.keys.contains(&scan) {
                    let name = std::mem::take(name);
                    self.screen = None;
                    self.events.push(Event::Message("Menu_Click"));
                    self.events.push(Event::Named(name));
                    true
                } else {
                    false
                }
            }
            Some(Screen::Table {
                level,
                levels,
                pressed,
                ..
            }) => {
                // Through the levels, round (`TT_Counter`, 1 to the last).
                if t.previous_keys.contains(&scan) {
                    *level = (*level + *levels - 1) % *levels;
                    pressed[0] = true;
                } else if t.next_keys.contains(&scan) {
                    *level = (*level + 1) % *levels;
                    pressed[1] = true;
                } else if t.leave_keys.contains(&scan) {
                    self.screen = None;
                    self.events.push(Event::Message("Menu_Click"));
                    self.events.push(Event::Left);
                    return true;
                } else {
                    return false;
                }
                self.events.push(Event::Message("Menu_Click"));
                true
            }
            None => false,
        }
    }

    /// A key went up (the arrows show pressed while held).
    pub fn key_up(&mut self, scan: u32) {
        let t = &self.data.table;
        if let Some(Screen::Table { pressed, .. }) = self.screen.as_mut() {
            if t.previous_keys.contains(&scan) {
                pressed[0] = false;
            }
            if t.next_keys.contains(&scan) {
                pressed[1] = false;
            }
        }
    }

    fn localised(&self, text: &bb::MenuText) -> String {
        self.localised_map(&text.text)
    }

    fn localised_map(&self, text: &BTreeMap<String, String>) -> String {
        text.get(&self.language)
            .or_else(|| text.get("english"))
            .cloned()
            .unwrap_or_default()
    }

    /// The quads for a `size` window; the table shows `save`'s scores.
    pub fn quads(&self, size: (u32, u32), save: &Save) -> Vec<Quad> {
        let Some(screen) = &self.screen else {
            return Vec::new();
        };
        #[expect(clippy::cast_precision_loss, reason = "window sizes")]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let fw = w.min(h * 4.0 / 3.0);
        let fh = fw * 0.75;
        let frame = Frame {
            x0: (w - fw) * 0.5,
            y0: (h - fh) * 0.5,
            fw,
            fh,
        };
        let mut quads = Vec::new();
        let mut texts = Vec::new();
        match screen {
            Screen::Entry { score, name } => {
                let e = &self.data.entry;
                for s in &e.sprites {
                    self.sprite(&mut quads, &frame, s, None);
                }
                texts.push((&e.title, self.localised(&e.title), 0.0));
                let points = format!("{score} {}", self.localised(&e.score));
                texts.push((&e.score, points, 0.0));
                texts.push((&e.ok, self.localised(&e.ok), 0.0));
                let name_quads = self.text_in(&frame, &e.name, name, 0.0);
                // The caret after the name: `caret_size` × the line high.
                let line = name_quads.first().map_or_else(
                    || {
                        let r = frame.px(e.name.rect);
                        let hgt = (r[3] - r[1]) * 0.6;
                        let x = r[0] + e.name.margins[0] * fh / REFERENCE_HEIGHT;
                        [x, (r[1] + r[3] - hgt) * 0.5, x, (r[1] + r[3] + hgt) * 0.5]
                    },
                    |q| {
                        let last = name_quads.last().map_or(q.rect[2], |l| l.rect[2]);
                        [last, q.rect[1], last, q.rect[3]]
                    },
                );
                if let Some(&white) = self.textures.get(&None) {
                    let width = ((line[3] - line[1]) * e.caret_size).max(1.0);
                    quads.push(Quad {
                        texture: white,
                        rect: [line[0], line[1], line[0] + width, line[3]],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: [1.0; 4],
                    });
                }
                quads.extend(name_quads);
            }
            Screen::Table {
                level,
                pressed,
                after_level,
                ..
            } => {
                let t = &self.data.table;
                for s in &t.sprites {
                    let alt = t
                        .arrows
                        .iter()
                        .zip(pressed)
                        .find(|(a, p)| **a == s.name && **p)
                        .map(|_| &t.arrow_pressed);
                    self.sprite(&mut quads, &frame, s, alt);
                }
                texts.push((
                    &t.title,
                    format!("{}{}", self.localised(&t.title), level + 1),
                    0.0,
                ));
                let back = if *after_level {
                    self.localised(&t.back)
                } else {
                    self.localised_map(&t.back_text)
                };
                texts.push((&t.back, back, 0.0));
                let table = save.highscores.get(*level).map_or(&[][..], Vec::as_slice);
                for (i, (n, s)) in t.names.iter().zip(&t.scores).enumerate() {
                    let Some(row) = table.get(i) else {
                        continue;
                    };
                    texts.push((n, row.name.clone(), t.name_offset * fw));
                    texts.push((s, row.score.to_string(), 0.0));
                }
            }
        }
        for (t, s, dx) in texts {
            quads.extend(self.text_in(&frame, t, &s, dx));
        }
        quads
    }

    fn sprite(
        &self,
        quads: &mut Vec<Quad>,
        frame: &Frame,
        s: &bb::HudSprite,
        material: Option<&(Option<String>, [f32; 4])>,
    ) {
        let (texture, color) = material.map_or((&s.texture, s.color), |(t, c)| (t, *c));
        let Some(&texture) = self.textures.get(texture) else {
            return;
        };
        quads.push(Quad {
            texture,
            rect: frame.px(s.rect),
            uv: s.source,
            color,
        });
    }

    /// `s` in `t`'s box, moved right by `dx` pixels.
    fn text_in(&self, frame: &Frame, t: &bb::MenuText, s: &str, dx: f32) -> Vec<Quad> {
        let Some(font) = self.fonts.get(&t.font) else {
            return Vec::new();
        };
        let Some(&texture) = self.textures.get(&Some(font.texture.clone())) else {
            return Vec::new();
        };
        let mut r = frame.px(t.rect);
        r[0] += dx;
        let style = TextStyle {
            align: t.align,
            margins: t.margins,
            scale: font.scale,
            color: font.color,
            top: t.top,
        };
        let size = (frame.fw, frame.fh);
        if t.wrap {
            // *WordWrap*: lines broken at spaces to fit.
            return text_block(font, texture, s, r, &style, size, true);
        }
        text_quads(font, texture, s, r, &style, size)
    }

    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        size: (u32, u32),
        save: &Save,
    ) {
        let quads = self.quads(size, save);
        if let Some(o) = self.overlay.as_mut() {
            o.render(gpu, encoder, color, size, &quads);
        }
    }
}

/// The 4:3 frame in the window.
struct Frame {
    x0: f32,
    y0: f32,
    fw: f32,
    fh: f32,
}

impl Frame {
    fn px(&self, r: [f32; 4]) -> [f32; 4] {
        [
            self.x0 + r[0] * self.fw,
            self.y0 + r[1] * self.fh,
            self.x0 + r[2] * self.fw,
            self.y0 + r[3] * self.fh,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> bb::Highscores {
        let mut d = bb::Highscores::default();
        d.entry.max_len = 9;
        d.entry.keys = vec![28];
        d.table.previous_keys = vec![203];
        d.table.next_keys = vec![205];
        d.table.leave_keys = vec![1, 28];
        d.entry.name.font = "f".into();
        d
    }

    fn fonts() -> BTreeMap<String, bb::HudFont> {
        BTreeMap::from([(
            "f".into(),
            bb::HudFont {
                glyphs: vec![bb::Glyph::default(); 255],
                ..bb::HudFont::default()
            },
        )])
    }

    #[test]
    fn the_name_is_typed_edited_and_confirmed() {
        let mut s = HighscoreScreens::with_textures(&data(), &fonts(), "english", HashMap::new());
        s.start_entry(1234, "name");
        assert!(s.typing());
        assert_eq!(s.take_events(), [Event::Message("Menu_Highscore")]);
        for _ in 0..4 {
            assert!(s.key(BACKSPACE));
        }
        s.text("Tomasz Kr\u{f3}l!\u{8}");
        // Space is a character here, not a key.
        assert!(!s.key(57));
        assert!(s.key(28));
        assert!(!s.active());
        assert_eq!(
            s.take_events(),
            [
                Event::Message("Menu_Click"),
                Event::Named("Tomasz Kr".into())
            ]
        );
    }

    #[test]
    fn words_wrap_at_spaces() {
        #[expect(clippy::cast_precision_loss, reason = "short strings")]
        let width = |s: &str| s.len() as f32;
        assert_eq!(
            crate::hud::wrap_words("Highscore Level 1", 15.0, width),
            ["Highscore Level", "1"]
        );
        assert_eq!(
            crate::hud::wrap_words("Highscore Level 1", 9.0, width),
            ["Highscore", "Level 1"]
        );
        assert_eq!(crate::hud::wrap_words("Ok", 1.0, width), ["Ok"]);
    }

    #[test]
    fn the_table_goes_round_the_levels_and_leaves() {
        let mut s = HighscoreScreens::with_textures(&data(), &fonts(), "english", HashMap::new());
        s.start_table(0, 12, true);
        assert!(s.key(203));
        assert_eq!(
            s.screen,
            Some(Screen::Table {
                level: 11,
                levels: 12,
                pressed: [true, false],
                after_level: true,
            })
        );
        s.key_up(203);
        assert!(s.key(205));
        assert!(s.key(205));
        assert!(matches!(s.screen, Some(Screen::Table { level: 1, .. })));
        assert!(!s.key(57));
        assert!(s.key(1));
        assert!(!s.active());
        assert_eq!(s.take_events().last(), Some(&Event::Left));
    }
}
