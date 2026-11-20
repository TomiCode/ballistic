//! The menus' screens, like the original's `Menu_*` scripts (research
//! findings 25): one at a time, drawn over the scene.
//!
//! - A **button menu** (main, level select, pause, end, game over,
//!   options): Up and Down move through the buttons (round, past locked
//!   ones); the active one is lit (`M_Button_Over`). Enter, or a click,
//!   picks it; Escape picks the menu's escape button. Pointing at a button
//!   makes it the active one. Picking sends `Menu_Click`.
//! - The **Yes/No** question: Left and Right choose (No at first), Enter or
//!   a click confirms, Escape says No.
//!
//! What a pick does is up to [`crate::game`]. The 4:3 layout is centred in
//! the window.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Gpu, Overlay, Quad, upload_ktx2};

use crate::hud::{TextStyle, text_quads};
use crate::play::Bundle;

const UP: u32 = 200;
const DOWN: u32 = 208;
const LEFT: u32 = 203;
const RIGHT: u32 = 205;
const ENTER: u32 = 28;
const ESCAPE: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuId {
    Main,
    Start,
    Pause,
    End,
    Dead,
    Options,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A message to the sound scripts.
    Message(&'static str),
    /// A button was picked.
    Picked(MenuId, usize),
    /// The question was answered.
    Answered(bool),
}

#[derive(Debug, Clone, PartialEq)]
enum Open {
    Buttons {
        id: MenuId,
        active: usize,
        locked: Vec<bool>,
    },
    YesNo {
        question: String,
        yes: bool,
    },
}

pub struct MenuScreens {
    data: bb::Menus,
    language: String,
    /// `None` in tests.
    overlay: Option<Overlay>,
    /// Texture key (`None`: white) → overlay texture.
    textures: HashMap<Option<String>, usize>,
    open: Option<Open>,
    events: Vec<Event>,
}

impl MenuScreens {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        data: &bb::Menus,
        format: wgpu::TextureFormat,
        language: &str,
    ) -> Result<Self> {
        let mut overlay = Overlay::new(gpu, format);
        let mut textures = HashMap::new();
        let menus = [
            &data.main,
            &data.start,
            &data.pause,
            &data.end,
            &data.dead,
            &data.options,
        ];
        let keys = menus
            .iter()
            .flat_map(|m| m.sprites.iter().chain(m.buttons.iter().map(|b| &b.sprite)))
            .chain(&data.yes_no.sprites)
            .chain([&data.yes_no.yes.sprite, &data.yes_no.no.sprite])
            .map(|s| s.texture.clone())
            .chain([
                data.button_up.0.clone(),
                data.button_over.0.clone(),
                data.button_inactive.0.clone(),
                None,
            ])
            .chain(data.fonts.values().map(|f| Some(f.texture.clone())))
            .collect::<Vec<_>>();
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

    pub(crate) fn with_textures(
        data: &bb::Menus,
        language: &str,
        textures: HashMap<Option<String>, usize>,
    ) -> Self {
        Self {
            data: data.clone(),
            language: language.to_owned(),
            overlay: None,
            textures,
            open: None,
            events: Vec::new(),
        }
    }

    fn menu(&self, id: MenuId) -> &bb::ButtonMenu {
        match id {
            MenuId::Main => &self.data.main,
            MenuId::Start => &self.data.start,
            MenuId::Pause => &self.data.pause,
            MenuId::End => &self.data.end,
            MenuId::Dead => &self.data.dead,
            MenuId::Options => &self.data.options,
        }
    }

    /// Opens menu `id`; `locked` buttons can't be picked (the level select's
    /// closed levels).
    pub fn open(&mut self, id: MenuId, locked: Vec<bool>) {
        let m = self.menu(id);
        let n = m.buttons.len();
        let mut locked = locked;
        locked.resize(n, false);
        let first = m.first.min(n.saturating_sub(1));
        let active = (0..n)
            .map(|i| (first + i) % n.max(1))
            .find(|&i| !locked[i])
            .unwrap_or(first);
        self.open = Some(Open::Buttons { id, active, locked });
    }

    /// Asks `question` (a `Menu_YesNo` question's name, `Exit Game?`, …).
    pub fn ask(&mut self, question: &str) {
        self.open = Some(Open::YesNo {
            question: question.to_owned(),
            yes: false,
        });
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// The open button menu.
    #[cfg(test)]
    pub fn showing(&self) -> Option<MenuId> {
        match &self.open {
            Some(Open::Buttons { id, .. }) => Some(*id),
            _ => None,
        }
    }

    pub fn active(&self) -> bool {
        self.open.is_some()
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// A key (`DirectInput` scan code) went down; whether a menu took it.
    pub fn key(&mut self, scan: u32) -> bool {
        let escape = match &self.open {
            Some(Open::Buttons { id, .. }) => self.menu(*id).escape,
            _ => None,
        };
        match self.open.as_mut() {
            Some(Open::Buttons { id, active, locked }) => {
                let n = locked.len();
                let step = |from: usize, forward: bool| {
                    (1..=n)
                        .map(|k| {
                            if forward {
                                (from + k) % n
                            } else {
                                (from + n * k - k) % n
                            }
                        })
                        .find(|&i| !locked[i])
                        .unwrap_or(from)
                };
                match scan {
                    UP if n > 0 => *active = step(*active, false),
                    DOWN if n > 0 => *active = step(*active, true),
                    ENTER => {
                        let (id, i) = (*id, *active);
                        self.pick(id, i);
                    }
                    ESCAPE => match escape {
                        Some(i) => {
                            let id = *id;
                            self.pick(id, i);
                        }
                        None => return false,
                    },
                    _ => return false,
                }
                true
            }
            Some(Open::YesNo { yes, .. }) => {
                match scan {
                    LEFT => *yes = true,
                    RIGHT => *yes = false,
                    ENTER => {
                        let answer = *yes;
                        self.answer(answer);
                    }
                    ESCAPE => self.answer(false),
                    _ => return false,
                }
                true
            }
            None => false,
        }
    }

    fn pick(&mut self, id: MenuId, i: usize) {
        if let Some(Open::Buttons { locked, .. }) = &self.open
            && locked.get(i).copied().unwrap_or(true)
        {
            return;
        }
        self.open = None;
        self.events.push(Event::Message("Menu_Click"));
        self.events.push(Event::Picked(id, i));
    }

    fn answer(&mut self, yes: bool) {
        self.open = None;
        self.events.push(Event::Message("Menu_Click"));
        self.events.push(Event::Answered(yes));
    }

    /// The button at `pos` (pixels in a `size` window): index, or Yes
    /// (`Ok(true)`) / No.
    fn hit(&self, pos: (f32, f32), size: (u32, u32)) -> Option<Result<usize, bool>> {
        let frame = Frame::new(size);
        let inside = |s: &bb::HudSprite| {
            let r = frame.px(s.rect);
            pos.0 >= r[0] && pos.0 < r[2] && pos.1 >= r[1] && pos.1 < r[3]
        };
        match self.open.as_ref()? {
            Open::Buttons { id, .. } => self
                .menu(*id)
                .buttons
                .iter()
                .position(|b| inside(&b.sprite))
                .map(Ok),
            Open::YesNo { .. } => {
                let y = &self.data.yes_no;
                if inside(&y.yes.sprite) {
                    Some(Err(true))
                } else if inside(&y.no.sprite) {
                    Some(Err(false))
                } else {
                    None
                }
            }
        }
    }

    /// The pointer moved: the button under it becomes the active one.
    pub fn point(&mut self, pos: (f32, f32), size: (u32, u32)) {
        let hit = self.hit(pos, size);
        match (self.open.as_mut(), hit) {
            (Some(Open::Buttons { active, locked, .. }), Some(Ok(i))) if !locked[i] => {
                *active = i;
            }
            (Some(Open::YesNo { yes, .. }), Some(Err(y))) => *yes = y,
            _ => {}
        }
    }

    /// A click; whether a button took it.
    pub fn click(&mut self, pos: (f32, f32), size: (u32, u32)) -> bool {
        match (self.open.clone(), self.hit(pos, size)) {
            (Some(Open::Buttons { id, locked, .. }), Some(Ok(i))) if !locked[i] => {
                self.pick(id, i);
                true
            }
            (Some(Open::YesNo { .. }), Some(Err(y))) => {
                self.answer(y);
                true
            }
            _ => false,
        }
    }

    fn localised(&self, text: &BTreeMap<String, String>) -> String {
        text.get(&self.language)
            .or_else(|| text.get("english"))
            .cloned()
            .unwrap_or_default()
    }

    /// The quads for a `size` window.
    pub fn quads(&self, size: (u32, u32)) -> Vec<Quad> {
        let frame = Frame::new(size);
        let mut quads = Vec::new();
        let mut texts: Vec<(&bb::MenuText, String, Option<&str>)> = Vec::new();
        let d = &self.data;
        if self.open.is_none() {
            return quads;
        }
        // The dark band: once a menu shows it, it stays behind the menus
        // that follow (the level select, the questions) until play.
        let band = self.band();
        if let Some(b) = band {
            self.sprite(&mut quads, &frame, b, None);
        }
        let others = |s: &&bb::HudSprite| band.is_none_or(|b| b.name != s.name);
        match &self.open {
            None => return quads,
            Some(Open::Buttons { id, active, locked }) => {
                let m = self.menu(*id);
                for s in m.sprites.iter().filter(others) {
                    self.sprite(&mut quads, &frame, s, None);
                }
                for (i, b) in m.buttons.iter().enumerate() {
                    let look = if locked[i] {
                        &d.button_inactive
                    } else if i == *active {
                        &d.button_over
                    } else {
                        &d.button_up
                    };
                    self.sprite(&mut quads, &frame, &b.sprite, Some(look));
                    let font = if locked[i] {
                        m.locked_font.as_deref()
                    } else {
                        None
                    };
                    texts.push((&b.text, self.localised(&b.text.text), font));
                }
                if let Some(t) = &m.title {
                    texts.push((t, self.localised(&t.text), None));
                }
            }
            Some(Open::YesNo { question, yes }) => {
                let y = &d.yes_no;
                for s in y.sprites.iter().filter(others) {
                    self.sprite(&mut quads, &frame, s, None);
                }
                for (b, lit) in [(&y.yes, *yes), (&y.no, !*yes)] {
                    let look = if lit {
                        &d.button_over
                    } else {
                        &d.button_inactive
                    };
                    self.sprite(&mut quads, &frame, &b.sprite, Some(look));
                    texts.push((&b.text, self.localised(&b.text.text), None));
                }
                let q = y.questions.get(question).map(|t| self.localised(t));
                texts.push((&y.text, q.unwrap_or_else(|| question.clone()), None));
            }
        }
        for (t, s, font) in texts {
            quads.extend(self.text(&frame, t, &s, font));
        }
        quads
    }

    /// `M_BlackScreen`, from the menus that show it.
    fn band(&self) -> Option<&bb::HudSprite> {
        [
            &self.data.main,
            &self.data.pause,
            &self.data.end,
            &self.data.dead,
        ]
        .into_iter()
        .flat_map(|m| &m.sprites)
        .find(|s| s.name.contains("BlackScreen"))
    }

    fn sprite(
        &self,
        quads: &mut Vec<Quad>,
        frame: &Frame,
        s: &bb::HudSprite,
        look: Option<&(Option<String>, [f32; 4])>,
    ) {
        let (texture, color) = look.map_or((&s.texture, s.color), |(t, c)| (t, *c));
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

    fn text(&self, frame: &Frame, t: &bb::MenuText, s: &str, font: Option<&str>) -> Vec<Quad> {
        let Some(font) = self.data.fonts.get(font.unwrap_or(&t.font)) else {
            return Vec::new();
        };
        let Some(&texture) = self.textures.get(&Some(font.texture.clone())) else {
            return Vec::new();
        };
        let style = TextStyle {
            align: t.align,
            margins: t.margins,
            scale: font.scale,
            color: font.color,
            top: t.top,
        };
        text_quads(
            font,
            texture,
            s,
            frame.px(t.rect),
            &style,
            (frame.fw, frame.fh),
        )
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

    fn menu(n: usize, first: usize, escape: Option<usize>) -> bb::ButtonMenu {
        bb::ButtonMenu {
            buttons: (0..n)
                .map(|i| {
                    #[expect(clippy::cast_precision_loss, reason = "a few buttons")]
                    let y = 0.1 * i as f32;
                    bb::MenuButton {
                        sprite: bb::HudSprite {
                            rect: [0.4, y, 0.6, y + 0.05],
                            ..bb::HudSprite::default()
                        },
                        text: bb::MenuText::default(),
                    }
                })
                .collect(),
            first,
            escape,
            ..bb::ButtonMenu::default()
        }
    }

    fn screens() -> MenuScreens {
        let data = bb::Menus {
            main: menu(5, 0, Some(4)),
            start: menu(4, 0, Some(3)),
            dead: menu(2, 0, None),
            ..bb::Menus::default()
        };
        MenuScreens::with_textures(&data, "english", HashMap::new())
    }

    #[test]
    fn keys_move_round_and_pick() {
        let mut s = screens();
        s.open(MenuId::Main, Vec::new());
        assert!(s.key(UP));
        assert!(s.key(ENTER));
        assert_eq!(
            s.take_events(),
            [Event::Message("Menu_Click"), Event::Picked(MenuId::Main, 4)]
        );
        assert!(!s.active());
        // Escape picks the escape button; the game over menu has none.
        s.open(MenuId::Main, Vec::new());
        assert!(s.key(ESCAPE));
        assert_eq!(
            s.take_events().last(),
            Some(&Event::Picked(MenuId::Main, 4))
        );
        s.open(MenuId::Dead, Vec::new());
        assert!(!s.key(ESCAPE));
        assert!(s.active());
    }

    #[test]
    fn locked_buttons_are_skipped_and_cant_be_picked() {
        let mut s = screens();
        s.open(MenuId::Start, vec![false, true, true, false]);
        assert!(s.key(DOWN));
        assert!(s.key(ENTER));
        assert_eq!(
            s.take_events().last(),
            Some(&Event::Picked(MenuId::Start, 3))
        );
        s.open(MenuId::Start, vec![false, true, true, false]);
        // A click on a locked one does nothing; on an open one it picks.
        let size = (800, 600);
        assert!(!s.click((400.0, 0.12 * 600.0), size));
        s.point((400.0, 0.02 * 600.0), size);
        assert!(s.click((400.0, 0.02 * 600.0), size));
        assert_eq!(
            s.take_events().last(),
            Some(&Event::Picked(MenuId::Start, 0))
        );
    }

    #[test]
    fn the_question_starts_on_no() {
        let mut s = screens();
        s.ask("Exit Game?");
        assert!(s.key(ENTER));
        assert_eq!(s.take_events().last(), Some(&Event::Answered(false)));
        s.ask("Exit Game?");
        assert!(s.key(LEFT));
        assert!(s.key(ENTER));
        assert_eq!(s.take_events().last(), Some(&Event::Answered(true)));
        s.ask("Exit Game?");
        assert!(s.key(ESCAPE));
        assert_eq!(s.take_events().last(), Some(&Event::Answered(false)));
    }
}
