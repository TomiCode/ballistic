//! The options' screens, like the original's `Menu_Opt_Graphics`, `_Keys`
//! and `_Sound` (research findings 27).
//!
//! - Up and Down move between the rows and *OK* (round); the active row's
//!   field is lit.
//! - Left and Right change the active row: the resolution and the volume
//!   step through their values, a yes/no row picks Yes (left) or No
//!   (right).
//! - Enter on a key row waits for the new key (Escape keeps the old one); a
//!   key bound elsewhere swaps with it. Enter on *OK*, or Escape, leaves.
//! - Pointing at a row makes it active; clicks work on the arrows, Yes and
//!   No, a key row and *OK*.
//!
//! Every change is kept in [`OptionsScreens::settings`] and reported as
//! [`Event::Changed`] so it takes effect at once; the caller saves on
//! [`Event::Done`].

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Gpu, Overlay, Quad, upload_ktx2};
use ballistic_save::Settings;

use crate::hud::{TextStyle, text_quads};
use crate::play::Bundle;

const UP: u32 = 200;
const DOWN: u32 = 208;
const LEFT: u32 = 203;
const RIGHT: u32 = 205;
const ENTER: u32 = 28;
const ESCAPE: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Graphics,
    Controls,
    Sound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A message to the sound scripts (`Menu_Click`, `Menu_Dong`).
    Message(&'static str),
    /// A setting changed.
    Changed,
    /// The screen was left.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Open {
    which: Which,
    /// A row, or the rows' count for *OK*.
    active: usize,
    /// A key row waits for its key.
    waiting: bool,
}

type Look = (Option<String>, [f32; 4]);

pub struct OptionsScreens {
    data: bb::OptionsScreens,
    fonts: BTreeMap<String, bb::HudFont>,
    /// Buttons: up, active, locked.
    looks: [Look; 3],
    /// The dark band behind the menus.
    band: Option<bb::HudSprite>,
    language: String,
    /// `None` in tests.
    overlay: Option<Overlay>,
    textures: HashMap<Option<String>, usize>,
    open: Option<Open>,
    /// The settings being edited.
    pub settings: Settings,
    events: Vec<Event>,
}

impl OptionsScreens {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        menus: &bb::Menus,
        format: wgpu::TextureFormat,
        language: &str,
    ) -> Result<Self> {
        let mut overlay = Overlay::new(gpu, format);
        let mut s = Self::with_textures(menus, language, HashMap::new());
        let d = &menus.settings;
        let mut keys: Vec<Option<String>> = vec![
            d.field_active.0.clone(),
            d.field_inactive.0.clone(),
            d.key_waiting.0.clone(),
            None,
        ];
        keys.extend(s.looks.iter().map(|l| l.0.clone()));
        keys.extend(s.band.iter().map(|b| b.texture.clone()));
        keys.extend(menus.fonts.values().map(|f| Some(f.texture.clone())));
        for screen in [&d.graphics, &d.controls, &d.sound] {
            keys.push(screen.ok.sprite.texture.clone());
            for r in &screen.rows {
                keys.push(r.field.texture.clone());
                match &r.kind {
                    bb::OptionKind::Choice { left, right, .. } => {
                        keys.push(left.texture.clone());
                        keys.push(right.texture.clone());
                    }
                    bb::OptionKind::YesNo { yes, no } => {
                        keys.push(yes.sprite.texture.clone());
                        keys.push(no.sprite.texture.clone());
                    }
                    _ => {}
                }
            }
        }
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
        language: &str,
        textures: HashMap<Option<String>, usize>,
    ) -> Self {
        let band = [&menus.main, &menus.pause, &menus.end, &menus.dead]
            .into_iter()
            .flat_map(|m| &m.sprites)
            .find(|s| s.name.contains("BlackScreen"))
            .cloned();
        Self {
            data: menus.settings.clone(),
            fonts: menus.fonts.clone(),
            looks: [
                menus.button_up.clone(),
                menus.button_over.clone(),
                menus.button_inactive.clone(),
            ],
            band,
            language: language.to_owned(),
            overlay: None,
            textures,
            open: None,
            settings: Settings::default(),
            events: Vec::new(),
        }
    }

    fn screen(&self, which: Which) -> &bb::OptionsScreen {
        match which {
            Which::Graphics => &self.data.graphics,
            Which::Controls => &self.data.controls,
            Which::Sound => &self.data.sound,
        }
    }

    /// Opens screen `which` on `settings`.
    pub fn open(&mut self, which: Which, settings: &Settings) {
        self.settings = settings.clone();
        // The six keys, the game's defaults where unset.
        let bound = crate::keys::bindings(&self.settings.keys, &self.data.default_keys);
        self.settings.keys = bound
            .iter()
            .map(|&k| crate::keys::name(k).unwrap_or_default().to_owned())
            .collect();
        self.open = Some(Open {
            which,
            active: 0,
            waiting: false,
        });
    }

    pub fn active(&self) -> bool {
        self.open.is_some()
    }

    /// A key row waits for its key.
    pub fn waiting(&self) -> bool {
        self.open.is_some_and(|o| o.waiting)
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The new key for the waiting row (a settings key name).
    pub fn bind(&mut self, name: &str) {
        let Some(o) = self.open.as_mut().filter(|o| o.waiting) else {
            return;
        };
        o.waiting = false;
        let (which, active) = (o.which, o.active);
        let Some(i) = self.key_index(which, active) else {
            return;
        };
        // Bound elsewhere: they swap.
        if let Some(j) = self.settings.keys.iter().position(|k| k == name)
            && j != i
        {
            self.settings.keys.swap(i, j);
        } else {
            name.clone_into(&mut self.settings.keys[i]);
        }
        self.events.push(Event::Message("Menu_Click"));
        self.events.push(Event::Changed);
    }

    /// Which binding a key row sets (0 forward … 5 lift).
    fn key_index(&self, which: Which, row: usize) -> Option<usize> {
        let setting = &self.screen(which).rows.get(row)?.setting;
        [
            "key_forward",
            "key_backward",
            "key_left",
            "key_right",
            "key_rotate",
            "key_lift",
        ]
        .iter()
        .position(|s| s == setting)
    }

    /// A key (`DirectInput` scan code) went down; whether the screen took
    /// it.
    pub fn key(&mut self, scan: u32) -> bool {
        let Some(o) = self.open else { return false };
        let rows = self.screen(o.which).rows.len();
        if o.waiting {
            // Escape keeps the old key; other keys come through `bind`.
            if scan == ESCAPE {
                self.set(Open {
                    waiting: false,
                    ..o
                });
            }
            return true;
        }
        match scan {
            UP => self.set(Open {
                active: (o.active + rows) % (rows + 1),
                ..o
            }),
            DOWN => self.set(Open {
                active: (o.active + 1) % (rows + 1),
                ..o
            }),
            LEFT => self.change(o, -1),
            RIGHT => self.change(o, 1),
            ENTER if o.active == rows => self.done(),
            ENTER => {
                if self.key_index(o.which, o.active).is_some() {
                    self.events.push(Event::Message("Menu_Click"));
                    self.set(Open { waiting: true, ..o });
                }
            }
            ESCAPE => self.done(),
            _ => return false,
        }
        true
    }

    fn set(&mut self, o: Open) {
        self.open = Some(o);
    }

    fn done(&mut self) {
        self.open = None;
        self.events.push(Event::Message("Menu_Click"));
        self.events.push(Event::Done);
    }

    /// Left (`-1`) or Right (`1`) on the active row.
    fn change(&mut self, o: Open, dir: i32) {
        let Some(row) = self.screen(o.which).rows.get(o.active) else {
            return;
        };
        let setting = row.setting.clone();
        let s = &mut self.settings;
        match setting.as_str() {
            "resolution" => {
                // None (as the window opened), then the sizes.
                let n = self.data.resolutions.len() + 1;
                let at = s
                    .resolution
                    .and_then(|r| self.data.resolutions.iter().position(|&x| x == r))
                    .map_or(0, |i| i + 1);
                #[expect(clippy::cast_possible_wrap, reason = "a few sizes")]
                let next = (at as i32 + dir).rem_euclid(n as i32) as usize;
                s.resolution = next.checked_sub(1).map(|i| self.data.resolutions[i]);
            }
            "volume" => {
                #[expect(clippy::cast_precision_loss, reason = "-1 or 1")]
                let v = s.volume + dir as f32 * self.data.volume_step;
                s.volume = (v * 100.0).round().clamp(0.0, 100.0) / 100.0;
                self.events.push(Event::Changed);
                // The gong at the new volume.
                self.events.push(Event::Message("Menu_Dong"));
                return;
            }
            "vsync" => s.vsync = dir < 0,
            "clouds" => s.clouds = dir < 0,
            "invert_rotation" => s.invert_rotation = dir < 0,
            _ => return,
        }
        self.events.push(Event::Message("Menu_Click"));
        self.events.push(Event::Changed);
    }

    /// The row (or *OK*) at `pos`, and what part of it.
    fn hit(&self, pos: (f32, f32), size: (u32, u32)) -> Option<(usize, Part)> {
        let o = self.open?;
        let frame = Frame::new(size);
        let inside = |s: &bb::HudSprite| {
            let r = frame.px(s.rect);
            pos.0 >= r[0] && pos.0 < r[2] && pos.1 >= r[1] && pos.1 < r[3]
        };
        let screen = self.screen(o.which);
        if inside(&screen.ok.sprite) {
            return Some((screen.rows.len(), Part::Field));
        }
        screen.rows.iter().enumerate().find_map(|(i, r)| {
            let part = match &r.kind {
                bb::OptionKind::Choice { left, .. } if inside(left) => Part::Left,
                bb::OptionKind::Choice { right, .. } if inside(right) => Part::Right,
                bb::OptionKind::YesNo { yes, .. } if inside(&yes.sprite) => Part::Left,
                bb::OptionKind::YesNo { no, .. } if inside(&no.sprite) => Part::Right,
                _ if inside(&r.field) => Part::Field,
                _ => return None,
            };
            Some((i, part))
        })
    }

    /// The pointer moved: the row under it becomes active.
    pub fn point(&mut self, pos: (f32, f32), size: (u32, u32)) {
        if let (Some(o), Some((row, _))) = (self.open, self.hit(pos, size))
            && !o.waiting
        {
            self.set(Open { active: row, ..o });
        }
    }

    /// A click; whether the screen took it.
    pub fn click(&mut self, pos: (f32, f32), size: (u32, u32)) -> bool {
        let (Some(o), Some((row, part))) = (self.open, self.hit(pos, size)) else {
            return false;
        };
        if o.waiting {
            return true;
        }
        let o = Open { active: row, ..o };
        self.set(o);
        match part {
            Part::Left => self.change(o, -1),
            Part::Right => self.change(o, 1),
            Part::Field => {
                self.key(ENTER);
            }
        }
        true
    }

    fn localised(&self, text: &BTreeMap<String, String>) -> String {
        text.get(&self.language)
            .or_else(|| text.get("english"))
            .cloned()
            .unwrap_or_default()
    }

    /// The value a choice row shows.
    fn value(&self, setting: &str) -> String {
        match setting {
            "resolution" => self
                .settings
                .resolution
                .map_or_else(|| "Window".to_owned(), |[w, h]| format!("{w} x {h}")),
            "volume" => format!("{:.0}%", self.settings.volume * 100.0),
            _ => String::new(),
        }
    }

    fn yes(&self, setting: &str) -> bool {
        match setting {
            "vsync" => self.settings.vsync,
            "clouds" => self.settings.clouds,
            "invert_rotation" => self.settings.invert_rotation,
            _ => false,
        }
    }

    /// The quads for a `size` window.
    pub fn quads(&self, size: (u32, u32)) -> Vec<Quad> {
        let Some(o) = self.open else {
            return Vec::new();
        };
        let frame = Frame::new(size);
        let screen = self.screen(o.which);
        let [up, over, locked] = &self.looks;
        let mut quads = Vec::new();
        let mut texts: Vec<(&bb::MenuText, String)> = Vec::new();
        if let Some(b) = &self.band {
            self.sprite(&mut quads, &frame, b, None);
        }
        texts.push((&screen.title, self.localised(&screen.title.text)));
        for (i, r) in screen.rows.iter().enumerate() {
            let lit = i == o.active;
            let field = match &r.kind {
                bb::OptionKind::YesNo { .. } if lit => &self.data.field_active,
                bb::OptionKind::YesNo { .. } => &self.data.field_inactive,
                _ if lit => over,
                _ => up,
            };
            self.sprite(&mut quads, &frame, &r.field, Some(field));
            texts.push((&r.label, self.localised(&r.label.text)));
            match &r.kind {
                bb::OptionKind::Choice { value, left, right } => {
                    self.sprite(&mut quads, &frame, left, None);
                    self.sprite(&mut quads, &frame, right, None);
                    texts.push((value, self.value(&r.setting)));
                }
                bb::OptionKind::YesNo { yes, no } => {
                    let y = self.yes(&r.setting);
                    self.sprite(
                        &mut quads,
                        &frame,
                        &yes.sprite,
                        Some(if y { up } else { locked }),
                    );
                    self.sprite(
                        &mut quads,
                        &frame,
                        &no.sprite,
                        Some(if y { locked } else { up }),
                    );
                    texts.push((&yes.text, self.localised(&yes.text.text)));
                    texts.push((&no.text, self.localised(&no.text.text)));
                }
                bb::OptionKind::Key { key } => {
                    let shown = match self.key_index(o.which, i) {
                        _ if lit && o.waiting => "?".to_owned(),
                        Some(k) => self
                            .settings
                            .keys
                            .get(k)
                            .map_or_else(String::new, |n| crate::keys::display(n).to_owned()),
                        None => String::new(),
                    };
                    texts.push((key, shown));
                }
                bb::OptionKind::None => {}
            }
        }
        let ok_look = if o.active == screen.rows.len() {
            over
        } else {
            up
        };
        self.sprite(&mut quads, &frame, &screen.ok.sprite, Some(ok_look));
        texts.push((&screen.ok.text, self.localised(&screen.ok.text.text)));
        for (t, s) in texts {
            quads.extend(self.text(&frame, t, &s));
        }
        quads
    }

    fn sprite(&self, quads: &mut Vec<Quad>, frame: &Frame, s: &bb::HudSprite, look: Option<&Look>) {
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

    fn text(&self, frame: &Frame, t: &bb::MenuText, s: &str) -> Vec<Quad> {
        let Some(font) = self.fonts.get(&t.font) else {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Field,
    Left,
    Right,
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

    fn row(setting: &str, kind: bb::OptionKind) -> bb::OptionRow {
        bb::OptionRow {
            setting: setting.into(),
            kind,
            ..bb::OptionRow::default()
        }
    }

    fn screens() -> OptionsScreens {
        let choice = || bb::OptionKind::Choice {
            value: bb::MenuText::default(),
            left: bb::HudSprite::default(),
            right: bb::HudSprite::default(),
        };
        let yes_no = || bb::OptionKind::YesNo {
            yes: bb::MenuButton::default(),
            no: bb::MenuButton::default(),
        };
        let key = || bb::OptionKind::Key {
            key: bb::MenuText::default(),
        };
        let menus = bb::Menus {
            settings: bb::OptionsScreens {
                graphics: bb::OptionsScreen {
                    rows: vec![
                        row("resolution", choice()),
                        row("vsync", yes_no()),
                        row("clouds", yes_no()),
                    ],
                    ..bb::OptionsScreen::default()
                },
                controls: bb::OptionsScreen {
                    rows: [
                        "key_forward",
                        "key_backward",
                        "key_left",
                        "key_right",
                        "key_rotate",
                        "key_lift",
                    ]
                    .iter()
                    .map(|s| row(s, key()))
                    .chain([row("invert_rotation", yes_no())])
                    .collect(),
                    ..bb::OptionsScreen::default()
                },
                sound: bb::OptionsScreen {
                    rows: vec![row("volume", choice())],
                    ..bb::OptionsScreen::default()
                },
                resolutions: vec![[640, 480], [800, 600]],
                default_keys: vec![200, 208, 203, 205, 42, 57],
                volume_step: 0.1,
                ..bb::OptionsScreens::default()
            },
            ..bb::Menus::default()
        };
        OptionsScreens::with_textures(&menus, "english", HashMap::new())
    }

    #[test]
    fn choices_step_and_yes_no_switch() {
        let mut s = screens();
        s.open(Which::Graphics, &Settings::default());
        assert!(s.key(RIGHT));
        assert_eq!(s.settings.resolution, Some([640, 480]));
        assert!(s.key(LEFT) && s.key(LEFT));
        assert_eq!(s.settings.resolution, Some([800, 600]), "round");
        assert!(s.key(DOWN) && s.key(RIGHT));
        assert!(!s.settings.vsync);
        assert!(s.take_events().contains(&Event::Changed));
        // Up from the top goes round to OK.
        assert!(s.key(UP) && s.key(UP));
        assert!(s.key(ENTER));
        assert!(!s.active());
        assert_eq!(s.take_events().last(), Some(&Event::Done));

        s.open(Which::Sound, &Settings::default());
        assert!(s.key(LEFT) && s.key(LEFT));
        assert!((s.settings.volume - 0.8).abs() < 1e-6);
        assert!(s.take_events().contains(&Event::Message("Menu_Dong")));
    }

    #[test]
    fn keys_are_bound_and_swapped() {
        let mut s = screens();
        s.open(Which::Controls, &Settings::default());
        assert_eq!(s.settings.keys[0], "ArrowUp");
        assert!(s.key(ENTER));
        assert!(s.waiting());
        s.bind("KeyW");
        assert_eq!(s.settings.keys[0], "KeyW");
        // Down to backward; binding forward's key swaps them.
        assert!(s.key(DOWN) && s.key(ENTER));
        s.bind("KeyW");
        assert_eq!(s.settings.keys[0], "ArrowDown");
        assert_eq!(s.settings.keys[1], "KeyW");
        // Escape while waiting keeps the key.
        assert!(s.key(ENTER) && s.key(ESCAPE));
        assert!(!s.waiting() && s.active());
        assert_eq!(s.settings.keys[1], "KeyW");
    }
}
