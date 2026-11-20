//! The in-game HUD, like the original's `Gameplay_Energy` (research findings
//! 18): the points counter and the life balls, bottom left and bottom right.
//!
//! - **Points**: the font's glyphs, right-aligned and centred vertically in
//!   their rectangle, sized from the screen (*Screen Proportional*).
//! - **Lives**: one ball per life, leftwards from the ball sprite's own place,
//!   with the end cap left of the last. An extra life slides the cap one step
//!   left, then fades a new ball in; a lost life (2 s after *Ball Off*) fades
//!   the last ball out, then slides the cap back. One change at a time.
//! - **Glow**: an extra point shows the points' glow and fades it out.
//!
//! The original stretched its HUD with the window. Here it keeps the 4:3
//! proportions it was drawn for: a 4:3 frame as tall as the window (or as
//! wide, if narrower) whose left half anchors to the window's left edge and
//! right half to its right edge.

use std::collections::{HashMap, VecDeque};

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Gpu, Overlay, Quad, upload_ktx2};

use crate::play::Bundle;

/// The screen height the original's pixel margins were set for.
pub const REFERENCE_HEIGHT: f32 = 480.0;

/// The gap between characters, per unit of *Space* × a space's advance
/// (`Interface.dll` `0x25391bf0`).
const SPACING_FACTOR: f32 = 0.1;

enum Change {
    Add,
    Remove,
}

enum Anim {
    /// The cap slides to `to`.
    Slide {
        from: f32,
        to: f32,
        ms: f32,
    },
    FadeIn {
        ms: f32,
    },
    FadeOut {
        ms: f32,
    },
}

pub struct Hud {
    data: bb::Hud,
    /// `None` in tests.
    overlay: Option<Overlay>,
    textures: HashMap<String, usize>,
    /// The life balls' x (screen fraction) and opacity.
    balls: Vec<(f32, f32)>,
    end_x: f32,
    queue: VecDeque<Change>,
    anim: Option<Anim>,
    /// Lost lives waiting for their fade (ms left).
    lost: Vec<f32>,
    /// Time into the glow's fade.
    glow_ms: Option<f32>,
}

impl Hud {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        data: &bb::Hud,
        format: wgpu::TextureFormat,
        lives: u32,
    ) -> Result<Self> {
        let mut overlay = Overlay::new(gpu, format);
        let mut textures = HashMap::new();
        let keys = data
            .sprites
            .iter()
            .filter_map(|s| s.texture.clone())
            .chain([data.font.texture.clone()]);
        for key in keys {
            if textures.contains_key(&key) {
                continue;
            }
            let bytes = bundle.texture(&key)?;
            let tex = upload_ktx2(gpu, &key, &bytes)?;
            textures.insert(key, overlay.add_texture(gpu, &tex));
        }
        let mut hud = Self {
            data: data.clone(),
            overlay: Some(overlay),
            textures,
            balls: Vec::new(),
            end_x: 0.0,
            queue: VecDeque::new(),
            anim: None,
            lost: Vec::new(),
            glow_ms: None,
        };
        hud.reset(lives);
        Ok(hud)
    }

    fn sprite(&self, name: &str) -> Option<&bb::HudSprite> {
        self.data.sprites.iter().find(|s| s.name == name)
    }

    /// The level start: `lives` balls, no changes pending.
    pub fn reset(&mut self, lives: u32) {
        let step = self.data.lives.step;
        let ball_x = self
            .sprite(&self.data.lives.ball)
            .map_or(0.0, |s| s.rect[0]);
        let end_x = self.sprite(&self.data.lives.end).map_or(0.0, |s| s.rect[0]);
        #[expect(clippy::cast_precision_loss, reason = "a few lives")]
        let n = lives as f32;
        self.balls = (1..=lives)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a few lives")]
                let i = i as f32;
                (ball_x - i * step, 1.0)
            })
            .collect();
        self.end_x = end_x - n * step;
        self.queue.clear();
        self.anim = None;
        self.lost.clear();
        self.glow_ms = None;
    }

    /// `Life_Up`.
    pub fn life_up(&mut self) {
        self.queue.push_back(Change::Add);
    }

    /// *Ball Off*: the last ball goes after the delay.
    pub fn ball_off(&mut self) {
        self.lost.push(self.data.lives.lost_delay_ms);
    }

    /// *Sub Life* (the score screen counting a life): the last ball goes now.
    pub fn sub_life(&mut self) {
        self.queue.push_back(Change::Remove);
    }

    /// *Extrapoint*.
    pub fn glow(&mut self) {
        self.glow_ms = Some(0.0);
    }

    /// One frame of real time.
    pub fn frame(&mut self, real_dt: f32) {
        let ms = real_dt * 1000.0;
        let mut due = 0;
        self.lost.retain_mut(|t| {
            *t -= ms;
            let waiting = *t > 0.0;
            if !waiting {
                due += 1;
            }
            waiting
        });
        for _ in 0..due {
            self.queue.push_back(Change::Remove);
        }
        if let Some(g) = self.glow_ms.as_mut() {
            *g += ms;
            if self.data.glow.fade.done(*g) {
                self.glow_ms = None;
            }
        }
        self.advance(ms);
    }

    fn advance(&mut self, ms: f32) {
        let l = &self.data.lives;
        if self.anim.is_none() {
            let step = l.step;
            match self.queue.pop_front() {
                Some(Change::Add) => {
                    let x = self.end_x;
                    // The new ball's place: a step left of the last.
                    let ball_x = self
                        .balls
                        .last()
                        .map_or_else(|| self.sprite(&l.ball).map_or(0.0, |s| s.rect[0]), |b| b.0)
                        - step;
                    self.balls.push((ball_x, 0.0));
                    self.anim = Some(Anim::Slide {
                        from: x,
                        to: x - step,
                        ms: 0.0,
                    });
                }
                Some(Change::Remove) if !self.balls.is_empty() => {
                    self.anim = Some(Anim::FadeOut { ms: 0.0 });
                }
                _ => return,
            }
        }
        let l = &self.data.lives;
        let next = match self.anim.as_mut() {
            Some(Anim::Slide { from, to, ms: t }) => {
                *t += ms;
                self.end_x = *from + (*to - *from) * l.slide.value(*t);
                if !l.slide.done(*t) {
                    return;
                }
                // After sliding left comes the new ball's fade-in.
                (*to < *from).then_some(Anim::FadeIn { ms: 0.0 })
            }
            Some(Anim::FadeIn { ms: t }) => {
                *t += ms;
                if let Some(b) = self.balls.last_mut() {
                    b.1 = l.fade_in.value(*t);
                }
                if !l.fade_in.done(*t) {
                    return;
                }
                None
            }
            Some(Anim::FadeOut { ms: t }) => {
                *t += ms;
                if let Some(b) = self.balls.last_mut() {
                    b.1 = 1.0 - l.fade_out.value(*t);
                }
                if !l.fade_out.done(*t) {
                    return;
                }
                self.balls.pop();
                let x = self.end_x;
                Some(Anim::Slide {
                    from: x,
                    to: x + l.step,
                    ms: 0.0,
                })
            }
            None => None,
        };
        self.anim = next;
    }

    /// The quads for a `size` window showing `points`.
    pub fn quads(&self, size: (u32, u32), points: i32) -> Vec<Quad> {
        #[expect(clippy::cast_precision_loss, reason = "window sizes")]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let fw = w.min(h * 4.0 / 3.0);
        let fh = fw * 0.75;
        // Screen fractions to pixels: left half from the left edge, right
        // half from the right, bottom-aligned.
        let px = |x: f32, anchor_right: bool| {
            if anchor_right {
                w - (1.0 - x) * fw
            } else {
                x * fw
            }
        };
        let py = |y: f32| h - (1.0 - y) * fh;
        let place = |rect: [f32; 4], x: f32| {
            let right = f32::midpoint(rect[0], rect[2]) > 0.5;
            let dx = x - rect[0];
            [
                px(rect[0] + dx, right),
                py(rect[1]),
                px(rect[2] + dx, right),
                py(rect[3]),
            ]
        };
        let mut sprites: Vec<(i32, Quad)> = Vec::new();
        let mut add = |s: &bb::HudSprite, x: f32, alpha: f32| {
            let Some(&texture) = s.texture.as_ref().and_then(|k| self.textures.get(k)) else {
                return;
            };
            let mut color = s.color;
            color[3] *= alpha;
            sprites.push((
                s.z_order,
                Quad {
                    texture,
                    rect: place(s.rect, x),
                    uv: s.source,
                    color,
                },
            ));
        };
        let l = &self.data.lives;
        for s in &self.data.sprites {
            if s.name == l.ball {
                for &(x, a) in &self.balls {
                    add(s, x, a);
                }
            } else if s.name == l.end {
                add(s, self.end_x, 1.0);
            } else if s.name == self.data.glow.sprite {
                if let Some(t) = self.glow_ms {
                    add(s, s.rect[0], self.data.glow.fade.value(t));
                }
            } else if s.shown {
                add(s, s.rect[0], 1.0);
            }
        }
        // Stable: equal z-orders keep the bundle's order.
        sprites.sort_by_key(|(z, _)| *z);
        let mut quads: Vec<Quad> = sprites.into_iter().map(|(_, q)| q).collect();
        if let Some(s) = self.sprite(&self.data.points.sprite) {
            quads.extend(self.text(&points.to_string(), place(s.rect, s.rect[0]), fw, fh));
        }
        quads
    }

    /// The points' *2D Text* in `rect` (pixels).
    fn text(&self, s: &str, rect: [f32; 4], fw: f32, fh: f32) -> Vec<Quad> {
        let f = &self.data.font;
        let Some(&texture) = self.textures.get(&f.texture) else {
            return Vec::new();
        };
        let style = TextStyle {
            align: self.data.points.align,
            margins: self.data.points.margins,
            scale: f.scale,
            color: f.color,
            top: false,
        };
        text_quads(f, texture, s, rect, &style, (fw, fh))
    }

    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        size: (u32, u32),
        points: i32,
    ) {
        let quads = self.quads(size, points);
        if let Some(o) = self.overlay.as_mut() {
            o.render(gpu, encoder, color, size, &quads);
        }
    }
}

/// How a *2D Text* is laid out and coloured.
pub struct TextStyle {
    pub align: bb::TextAlign,
    /// Left, top, right, bottom, in pixels at 480 lines.
    pub margins: [f32; 4],
    /// Glyph size: texture-coordinate size × frame size × this.
    pub scale: [f32; 2],
    pub color: [f32; 4],
    /// At the box's top (within the margins) instead of centred.
    pub top: bool,
}

/// How wide `s` is laid out in `font` (pixels), and its line height.
pub fn text_size(font: &bb::HudFont, s: &str, scale: [f32; 2], frame: (f32, f32)) -> (f32, f32) {
    let (fw, fh) = frame;
    let glyph = |c: char| {
        u32::from(c)
            .try_into()
            .ok()
            .and_then(|i: usize| font.glyphs.get(i))
    };
    let (xf, yf) = (fw * scale[0], fh * scale[1]);
    let advance = |g: &bb::Glyph| g.before + g.width + g.after;
    let spacing = glyph(' ').map_or(0.0, |g| advance(g) * font.space[0] * xf * SPACING_FACTOR);
    let width = s
        .chars()
        .filter_map(glyph)
        .map(|g| advance(g) * xf + spacing)
        .sum();
    (width, font.glyphs.first().map_or(0.0, |g| g.height) * yf)
}

/// *2D Text* in `rect` (pixels) in `font` (on overlay texture `texture`),
/// glyphs sized from the `frame` (width, height; *Screen Proportional*),
/// centred vertically.
pub fn text_quads(
    font: &bb::HudFont,
    texture: usize,
    s: &str,
    rect: [f32; 4],
    style: &TextStyle,
    frame: (f32, f32),
) -> Vec<Quad> {
    let (fw, fh) = frame;
    let glyph = |c: char| {
        u32::from(c)
            .try_into()
            .ok()
            .and_then(|i: usize| font.glyphs.get(i))
    };
    let xf = fw * style.scale[0];
    let advance = |g: &bb::Glyph| g.before + g.width + g.after;
    let spacing = glyph(' ').map_or(0.0, |g| advance(g) * font.space[0] * xf * SPACING_FACTOR);
    let (width, height) = text_size(font, s, style.scale, frame);
    let m = style.margins.map(|v| v * fh / REFERENCE_HEIGHT);
    let (left, top, right, bottom) = (
        rect[0] + m[0],
        rect[1] + m[1],
        rect[2] - m[2],
        rect[3] - m[3],
    );
    let mut x = match style.align {
        bb::TextAlign::Left => left,
        bb::TextAlign::Right => right - width,
        bb::TextAlign::Center => (left + right - width) * 0.5,
    };
    let y = if style.top {
        top
    } else {
        (top + bottom - height) * 0.5
    };
    let mut quads = Vec::new();
    for g in s.chars().filter_map(glyph) {
        let x1 = x + g.before * xf;
        let x2 = x1 + g.width * xf;
        quads.push(Quad {
            texture,
            rect: [x1, y, x2, y + height],
            uv: [g.u, g.v, g.u + g.width, g.v + g.height],
            color: style.color,
        });
        x = x2 + g.after * xf + spacing;
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(ms: f32) -> bb::Progression {
        let p = |x: f32, y: f32| bb::CurvePoint {
            position: [x, y],
            in_tangent: [1.0, 1.0],
            out_tangent: [1.0, 1.0],
        };
        bb::Progression {
            duration_ms: ms,
            curve: vec![p(0.0, 0.0), p(1.0, 1.0)],
        }
    }

    fn sprite(name: &str, rect: [f32; 4], z_order: i32, shown: bool) -> bb::HudSprite {
        bb::HudSprite {
            name: name.into(),
            texture: Some("atlas".into()),
            rect,
            source: [0.0, 0.0, 1.0, 1.0],
            z_order,
            color: [1.0; 4],
            shown,
        }
    }

    fn hud(lives: u32) -> Hud {
        let glyph = |width: f32| bb::Glyph {
            u: 0.0,
            v: 0.0,
            width,
            before: 0.01,
            after: 0.01,
            height: 0.0625,
        };
        let mut glyphs = vec![glyph(0.02); 128];
        glyphs[usize::from(b'1')] = glyph(0.01);
        let data = bb::Hud {
            sprites: vec![
                sprite("ball", [0.95, 0.885, 0.99, 0.94], 9, false),
                sprite("end", [0.93, 0.88, 0.95, 0.94], 10, true),
                sprite("glow", [0.015, 0.87, 0.235, 0.98], 9, false),
                sprite("digits", [0.05, 0.88, 0.2, 0.93], 15, true),
            ],
            font: bb::HudFont {
                texture: "font".into(),
                glyphs,
                scale: [0.8, 0.9],
                space: [1.5, 1.0],
                color: [1.0; 4],
            },
            points: bb::HudText {
                sprite: "digits".into(),
                align: bb::TextAlign::Right,
                margins: [2.0; 4],
            },
            lives: bb::HudLives {
                ball: "ball".into(),
                end: "end".into(),
                step: 0.04,
                fade_in: linear(300.0),
                fade_out: linear(300.0),
                slide: linear(300.0),
                lost_delay_ms: 2000.0,
            },
            glow: bb::HudGlow {
                sprite: "glow".into(),
                fade: linear(500.0),
            },
        };
        let mut h = Hud {
            data,
            overlay: None,
            textures: [("atlas".to_owned(), 0), ("font".to_owned(), 1)].into(),
            balls: Vec::new(),
            end_x: 0.0,
            queue: VecDeque::new(),
            anim: None,
            lost: Vec::new(),
            glow_ms: None,
        };
        h.reset(lives);
        h
    }

    fn run(h: &mut Hud, seconds: f32) {
        let mut t = 0.0;
        while t < seconds - 1e-4 {
            h.frame(1.0 / 60.0);
            t += 1.0 / 60.0;
        }
    }

    #[test]
    fn lives_start_left_of_the_ball_sprite() {
        let h = hud(3);
        let xs: Vec<f32> = h.balls.iter().map(|b| b.0).collect();
        assert!(
            (xs[0] - 0.91).abs() < 1e-5 && (xs[2] - 0.83).abs() < 1e-5,
            "{xs:?}"
        );
        assert!((h.end_x - 0.81).abs() < 1e-5);
    }

    #[test]
    fn a_lost_life_fades_after_the_delay_then_the_end_slides_back() {
        let mut h = hud(3);
        h.ball_off();
        run(&mut h, 1.9);
        assert_eq!(h.balls.len(), 3);
        assert!((h.balls[2].1 - 1.0).abs() < 1e-6);
        run(&mut h, 0.25);
        assert!(h.balls[2].1 > 0.2 && h.balls[2].1 < 0.8, "{:?}", h.balls[2]);
        run(&mut h, 0.6);
        assert_eq!(h.balls.len(), 2);
        assert!((h.end_x - 0.85).abs() < 1e-4, "{}", h.end_x);
    }

    #[test]
    fn an_extra_life_slides_the_end_then_fades_a_ball_in() {
        let mut h = hud(3);
        h.life_up();
        run(&mut h, 0.15);
        assert_eq!(h.balls.len(), 4);
        assert!(h.balls[3].1 < 1e-6, "hidden while the end slides");
        assert!(h.end_x < 0.81 && h.end_x > 0.77, "{}", h.end_x);
        run(&mut h, 0.6);
        assert!((h.balls[3].1 - 1.0).abs() < 1e-6);
        assert!((h.balls[3].0 - 0.79).abs() < 1e-5);
        assert!((h.end_x - 0.77).abs() < 1e-4);
    }

    #[test]
    fn the_glow_shows_while_it_fades() {
        let mut h = hud(3);
        let glows = |h: &Hud| h.quads((640, 480), 1000).len();
        let without = glows(&h);
        h.glow();
        run(&mut h, 0.25);
        assert_eq!(glows(&h), without + 1);
        run(&mut h, 0.3);
        assert_eq!(glows(&h), without);
    }

    #[test]
    fn points_are_right_aligned_in_their_box() {
        let h = hud(3);
        let quads = h.quads((640, 480), 1000);
        let text: Vec<&Quad> = quads.iter().filter(|q| q.texture == 1).collect();
        assert_eq!(text.len(), 4);
        // Advance after the last glyph, then the margin: the box's right edge.
        let xf = 640.0 * 0.8;
        let spacing = 0.04 * 1.5 * xf * 0.1;
        let end = text[3].rect[2] + 0.01 * xf + spacing + 2.0;
        assert!((end - 0.2 * 640.0).abs() < 1e-3, "{end}");
        // 28.8 px tall, centred in the box.
        let height = text[0].rect[3] - text[0].rect[1];
        assert!((height - 480.0 * 0.9 * 0.0625).abs() < 1e-3);
        let mid = f32::midpoint(text[0].rect[1], text[0].rect[3]);
        assert!((mid - 0.905 * 480.0).abs() < 1e-3, "{mid}");
    }

    #[test]
    fn a_wide_window_keeps_the_hud_proportions() {
        let h = hud(3);
        let q = h.quads((1920, 1080), 1000);
        // The end cap: anchored right, 4:3 frame 1440 wide.
        let end = q
            .iter()
            .find(|q| q.texture == 0 && (q.rect[0] - (1920.0 - 0.19 * 1440.0)).abs() < 1e-2);
        assert!(end.is_some(), "{q:?}");
    }
}
