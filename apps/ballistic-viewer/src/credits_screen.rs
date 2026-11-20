//! The credits, like the original's `Menu_Credits` (research findings 28):
//! the two logos fade in, stay and fade out in turn; a second later the
//! pages follow, each a title and its text fading in, staying (longer for
//! longer texts) and fading out; then the logos again, round and round.
//! Escape, Enter or *Back* leave. Typing the cheat word opens every level.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Gpu, Overlay, Quad, upload_ktx2};

use crate::hud::{TextStyle, text_block, text_quads};
use crate::play::Bundle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Message(&'static str),
    /// The screen was left.
    Left,
    /// The cheat word was typed.
    Cheat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Logo { i: usize, ms: f32 },
    Pause { ms: f32 },
    Page { i: usize, ms: f32 },
}

type Look = (Option<String>, [f32; 4]);

pub struct CreditsScreen {
    data: bb::CreditsScreen,
    fonts: BTreeMap<String, bb::HudFont>,
    band: Option<bb::HudSprite>,
    /// The button: up, active.
    looks: [Look; 2],
    /// `None` in tests.
    overlay: Option<Overlay>,
    textures: HashMap<Option<String>, usize>,
    stage: Option<Stage>,
    /// The pointer is on *Back*.
    hover: bool,
    typed: String,
    events: Vec<Event>,
}

impl CreditsScreen {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        menus: &bb::Menus,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let mut overlay = Overlay::new(gpu, format);
        let mut s = Self::with_textures(menus, HashMap::new());
        let keys: Vec<Option<String>> = s
            .data
            .logos
            .iter()
            .map(|l| l.texture.clone())
            .chain(s.band.iter().map(|b| b.texture.clone()))
            .chain(s.looks.iter().map(|l| l.0.clone()))
            .chain(s.fonts.values().map(|f| Some(f.texture.clone())))
            .collect();
        for key in keys {
            if s.textures.contains_key(&key) {
                continue;
            }
            let tex = match &key {
                Some(k) => upload_ktx2(gpu, k, &bundle.texture(k)?)?,
                None => ballistic_render::white(gpu),
            };
            s.textures.insert(key, overlay.add_texture(gpu, &tex));
        }
        s.overlay = Some(overlay);
        Ok(s)
    }

    pub(crate) fn with_textures(
        menus: &bb::Menus,
        textures: HashMap<Option<String>, usize>,
    ) -> Self {
        let band = [&menus.main, &menus.pause, &menus.end, &menus.dead]
            .into_iter()
            .flat_map(|m| &m.sprites)
            .find(|s| s.name.contains("BlackScreen"))
            .cloned();
        Self {
            data: menus.credits.clone(),
            fonts: menus.fonts.clone(),
            band,
            looks: [menus.button_up.clone(), menus.button_over.clone()],
            overlay: None,
            textures,
            stage: None,
            hover: false,
            typed: String::new(),
            events: Vec::new(),
        }
    }

    pub fn open(&mut self) {
        self.stage = Some(Stage::Logo { i: 0, ms: 0.0 });
        self.typed.clear();
    }

    pub fn active(&self) -> bool {
        self.stage.is_some()
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn leave(&mut self) {
        self.stage = None;
        self.events.push(Event::Message("Menu_Click"));
        self.events.push(Event::Left);
    }

    /// A key (`DirectInput` scan code) went down; whether the screen took it.
    pub fn key(&mut self, scan: u32) -> bool {
        if self.stage.is_none() || !self.data.keys.contains(&scan) {
            return false;
        }
        self.leave();
        true
    }

    /// Typed text: the cheat word, its last letters.
    pub fn text(&mut self, s: &str) {
        let cheat = &self.data.cheat;
        if self.stage.is_none() || cheat.is_empty() {
            return;
        }
        for c in s.chars() {
            self.typed.push(c.to_ascii_lowercase());
            let n = self.typed.chars().count();
            if n > cheat.chars().count() {
                self.typed = self.typed.chars().skip(n - cheat.chars().count()).collect();
            }
            if self.typed == *cheat {
                self.typed.clear();
                self.events.push(Event::Cheat);
            }
        }
    }

    fn on_back(&self, pos: (f32, f32), size: (u32, u32)) -> bool {
        let r = Frame::new(size).px(self.data.back.sprite.rect);
        pos.0 >= r[0] && pos.0 < r[2] && pos.1 >= r[1] && pos.1 < r[3]
    }

    pub fn point(&mut self, pos: (f32, f32), size: (u32, u32)) {
        self.hover = self.stage.is_some() && self.on_back(pos, size);
    }

    pub fn click(&mut self, pos: (f32, f32), size: (u32, u32)) -> bool {
        if self.stage.is_none() || !self.on_back(pos, size) {
            return false;
        }
        self.leave();
        true
    }

    /// How long page `i` stays (ms).
    fn hold(&self, i: usize) -> f32 {
        let chars = self.data.pages.get(i).map_or(0, |p| p[1].chars().count());
        #[expect(clippy::cast_precision_loss, reason = "text lengths")]
        let hold = chars as f32 * self.data.hold_per_char_ms + self.data.hold_ms;
        if i == 0 {
            (hold - self.data.first_shorter_ms).max(0.0)
        } else {
            hold
        }
    }

    pub fn frame(&mut self, real_dt: f32) {
        let Some(stage) = self.stage else { return };
        let ms = real_dt * 1000.0;
        let d = &self.data;
        self.stage = Some(match stage {
            Stage::Logo { i, ms: t } => {
                let t = t + ms;
                let total: f32 = d.logo_fades_ms.get(i).map_or(0.0, |f| f.iter().sum());
                if t < total {
                    Stage::Logo { i, ms: t }
                } else if i + 1 < d.logos.len() {
                    Stage::Logo { i: i + 1, ms: 0.0 }
                } else {
                    Stage::Pause { ms: d.pause_ms }
                }
            }
            Stage::Pause { ms: t } if t - ms > 0.0 => Stage::Pause { ms: t - ms },
            Stage::Pause { .. } => Stage::Page { i: 0, ms: 0.0 },
            Stage::Page { i, ms: t } => {
                let t = t + ms;
                if t < d.fade_ms * 2.0 + self.hold(i) {
                    Stage::Page { i, ms: t }
                } else if i + 1 < d.pages.len() {
                    Stage::Page { i: i + 1, ms: 0.0 }
                } else {
                    Stage::Logo { i: 0, ms: 0.0 }
                }
            }
        });
    }

    /// Opacity over a fade in, a hold and a fade out.
    fn alpha(t: f32, [fade_in, hold, fade_out]: [f32; 3]) -> f32 {
        if t < fade_in {
            t / fade_in.max(1.0)
        } else if t < fade_in + hold {
            1.0
        } else {
            (1.0 - (t - fade_in - hold) / fade_out.max(1.0)).max(0.0)
        }
    }

    pub fn quads(&self, size: (u32, u32)) -> Vec<Quad> {
        let Some(stage) = self.stage else {
            return Vec::new();
        };
        let frame = Frame::new(size);
        let mut quads = Vec::new();
        let d = &self.data;
        if let Some(b) = &self.band {
            self.sprite(&mut quads, &frame, b, None, 1.0);
        }
        match stage {
            Stage::Logo { i, ms } => {
                if let (Some(logo), Some(f)) = (d.logos.get(i), d.logo_fades_ms.get(i)) {
                    self.sprite(&mut quads, &frame, logo, None, Self::alpha(ms, *f));
                }
            }
            Stage::Pause { .. } => {}
            Stage::Page { i, ms } => {
                if let Some([title, text]) = d.pages.get(i) {
                    let a = Self::alpha(ms, [d.fade_ms, self.hold(i), d.fade_ms]);
                    quads.extend(self.page_text(&frame, title, &d.title_font, a));
                    quads.extend(self.page_text(&frame, text, &d.text_font, a));
                }
            }
        }
        let look = &self.looks[usize::from(self.hover)];
        self.sprite(&mut quads, &frame, &d.back.sprite, Some(look), 1.0);
        if let (Some(font), Some(text)) = (
            self.fonts.get(&d.back.text.font),
            d.back.text.text.get("english"),
        ) && let Some(&texture) = self.textures.get(&Some(font.texture.clone()))
        {
            let t = &d.back.text;
            let style = TextStyle {
                align: t.align,
                margins: t.margins,
                scale: font.scale,
                color: font.color,
                top: t.top,
            };
            quads.extend(text_quads(
                font,
                texture,
                text,
                frame.px(t.rect),
                &style,
                (frame.fw, frame.fh),
            ));
        }
        quads
    }

    /// A page's title or text in its font, faded to `alpha`.
    fn page_text(&self, frame: &Frame, s: &str, font: &str, alpha: f32) -> Vec<Quad> {
        let Some(font) = self.fonts.get(font) else {
            return Vec::new();
        };
        let Some(&texture) = self.textures.get(&Some(font.texture.clone())) else {
            return Vec::new();
        };
        let t = &self.data.text;
        let mut color = font.color;
        color[3] *= alpha;
        let style = TextStyle {
            align: t.align,
            margins: t.margins,
            scale: font.scale,
            color,
            top: t.top,
        };
        text_block(
            font,
            texture,
            s,
            frame.px(t.rect),
            &style,
            (frame.fw, frame.fh),
            t.wrap,
        )
    }

    fn sprite(
        &self,
        quads: &mut Vec<Quad>,
        frame: &Frame,
        s: &bb::HudSprite,
        look: Option<&Look>,
        alpha: f32,
    ) {
        let (texture, mut color) = look.map_or((&s.texture, s.color), |(t, c)| (t, *c));
        let Some(&texture) = self.textures.get(texture) else {
            return;
        };
        color[3] *= alpha;
        quads.push(Quad {
            texture,
            rect: frame.px(s.rect),
            uv: s.source,
            color,
        });
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

/// The 4:3 frame centred in the window.
struct Frame {
    x0: f32,
    y0: f32,
    fw: f32,
    fh: f32,
}

impl Frame {
    fn new(size: (u32, u32)) -> Self {
        #[expect(clippy::cast_precision_loss, reason = "window sizes")]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let fw = w.min(h * 4.0 / 3.0);
        let fh = fw * 0.75;
        Self {
            x0: (w - fw) * 0.5,
            y0: (h - fh) * 0.5,
            fw,
            fh,
        }
    }

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

    fn screen() -> CreditsScreen {
        let menus = bb::Menus {
            credits: bb::CreditsScreen {
                logos: vec![bb::HudSprite::default(); 2],
                logo_fades_ms: vec![[500.0, 4000.0, 500.0], [500.0, 4000.0, 2000.0]],
                pause_ms: 1000.0,
                pages: vec![["a".into(), "x".repeat(100)], ["b".into(), "y".repeat(10)]],
                fade_ms: 500.0,
                hold_per_char_ms: 50.0,
                hold_ms: 1500.0,
                first_shorter_ms: 2000.0,
                cheat: "gravity".into(),
                keys: vec![1, 28],
                ..bb::CreditsScreen::default()
            },
            ..bb::Menus::default()
        };
        CreditsScreen::with_textures(&menus, HashMap::new())
    }

    fn run(s: &mut CreditsScreen, seconds: f32) {
        #[expect(clippy::cast_sign_loss, reason = "positive durations")]
        for _ in 0..(seconds * 60.0) as u32 {
            s.frame(1.0 / 60.0);
        }
    }

    #[test]
    fn logos_then_pages_round_and_round() {
        let mut s = screen();
        s.open();
        // Logo 1: 5 s; logo 2: 6.5 s; a 1 s pause.
        run(&mut s, 12.6);
        assert!(
            matches!(s.stage, Some(Stage::Page { i: 0, .. })),
            "{:?}",
            s.stage
        );
        // Page 1: 500 + (100 × 50 + 1500 − 2000) + 500 = 5.5 s.
        assert!((s.hold(0) - 4500.0).abs() < 1e-3);
        run(&mut s, 5.6);
        assert!(
            matches!(s.stage, Some(Stage::Page { i: 1, .. })),
            "{:?}",
            s.stage
        );
        // Page 2: 500 + 2000 + 500 = 3 s, then the logos again.
        run(&mut s, 3.1);
        assert!(
            matches!(s.stage, Some(Stage::Logo { i: 0, .. })),
            "{:?}",
            s.stage
        );
    }

    #[test]
    fn keys_leave_and_the_cheat_word_is_heard() {
        let mut s = screen();
        s.open();
        s.text("xxGRAV");
        s.text("ity");
        assert_eq!(s.take_events(), [Event::Cheat]);
        assert!(!s.key(57));
        assert!(s.key(1));
        assert!(!s.active());
        assert_eq!(s.take_events().last(), Some(&Event::Left));
    }
}
