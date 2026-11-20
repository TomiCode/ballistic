//! Level 1's tutorial, like the original's `Gameplay_Tutorial` (research
//! findings 29): ten chapters, each a text in a dark box at the bottom of
//! the screen and something to do.
//!
//! - **Intro**: the physics is stopped and the ball doesn't come yet.
//!   Return goes on (the ball comes); Q, or 25 s without either, ends the
//!   tutorial.
//! - **View rotation**, **overview**: arrows round the ball (they follow it,
//!   turned to the camera); Return goes on.
//! - **Navigation**: the physics runs; the four move arrows grow while their
//!   keys are held. Done once all four were pressed, or the ball reaches the
//!   track's end marker, or is lost.
//! - **Life extra**, **transformer**, **wood transformer**, **point extra**,
//!   **checkpoint**: each waits for the ball to come near its place, then
//!   stops the physics (with a sound) and points an arrow at the place;
//!   Return lets the ball roll on, and the chapter ends when the ball gets
//!   there. A wooden ball skips the wood transformer's chapter.
//! - **Hints**: 4 s after the checkpoint's chapter; the physics stops until
//!   Return.
//!
//! Between chapters: the text goes 510 ms after the chapter's end. Texts
//! and the box fade in and out over 200 ms, arrows over 500 ms.

use std::collections::HashMap;

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Camera, Gpu, Overlay, Quad, RenderOptions, SceneRenderer, upload_ktx2};
use glam::{Mat3, Mat4, Vec3};

use crate::hud::{TextStyle, text_block};
use crate::play::{Bundle, PlayKey};

/// What the tutorial asks of the level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `Tutorial Ready`: the ball may come.
    Ready,
    /// A level message (`Menu_Click`).
    Message(&'static str),
    /// A sound (bundle key).
    Sound(String),
}

/// What the tutorial reads from the level each frame.
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub real_dt: f32,
    /// The ball's place, and its type's name.
    pub ball: Vec3,
    pub ball_type: &'a str,
    /// Held keys: forward, backward, left, right, rotation.
    pub held: [bool; 5],
    /// The ball was lost this frame (`Ball Off`).
    pub ball_off: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// Before the first text.
    Start {
        ms: f32,
    },
    /// The chapter's wait (a delay counts down).
    Wait {
        ms: f32,
    },
    /// The chapter's action.
    Action(Action),
    /// After the action: the text goes this much later.
    After {
        ms: f32,
    },
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    /// The intro: ms without a key.
    Intro { idle_ms: f32 },
    /// Arrows fading in, then waiting for Return; then fading out.
    Arrows { fading_out: bool },
    /// Arrows fading in, then the move keys; then fading out.
    Move {
        /// Each key pressed and released once.
        done: [bool; 4],
        down: [bool; 4],
        fading_out: bool,
    },
    /// Frozen until Return; then until the ball gets there; then the
    /// pointer fades out.
    Reach { waiting: bool, fading_out: bool },
    /// Frozen until Return.
    Pause,
}

/// A fade between 0 and 1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Fade {
    value: f32,
    target: f32,
}

impl Fade {
    fn frame(&mut self, ms: f32, over_ms: f32) {
        let step = if over_ms > 0.0 { ms / over_ms } else { 1.0 };
        if self.value < self.target {
            self.value = (self.value + step).min(self.target);
        } else {
            self.value = (self.value - step).max(self.target);
        }
    }

    #[expect(clippy::float_cmp, reason = "set to the target exactly")]
    fn done(self) -> bool {
        self.value == self.target
    }
}

/// A scene object the tutorial shows.
struct Arrow {
    object: usize,
    materials: Vec<usize>,
    /// Its transform in the scene (bundle space).
    model: Mat4,
    fade: Fade,
    /// Its scale along its own axes (the move arrows grow), and the scale's
    /// progress (0: normal, 1: grown).
    grow: Fade,
}

pub struct Tutorial {
    data: bb::Tutorial,
    texts: Vec<String>,
    chapter: usize,
    stage: Stage,
    /// The chapter whose text is up, and its fade.
    text: Option<usize>,
    text_fade: Fade,
    back_fade: Fade,
    /// The box fades out after this delay.
    back_hide_ms: Option<f32>,
    frozen: bool,
    /// The ball's arrows follow it.
    ball_arrows_on: bool,
    arrows: HashMap<String, Arrow>,
    /// The chapter's place (the pointer's), in bundle space.
    place: Option<Vec3>,
    /// Places by name.
    places: HashMap<String, Vec3>,
    events: Vec<Event>,
    /// `None` in tests.
    gfx: Option<Gfx>,
}

struct Gfx {
    scene: SceneRenderer,
    overlay: Overlay,
    font_texture: usize,
    white: usize,
}

impl Tutorial {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        data: &bb::Tutorial,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let scene = bundle.scene(&data.scene)?;
        let (renderer, _) =
            SceneRenderer::new(gpu, &scene, format, &mut |k| bundle.texture(k).ok())?;
        let mut overlay = Overlay::new(gpu, format);
        let tex = upload_ktx2(
            gpu,
            &data.font.texture,
            &bundle.texture(&data.font.texture)?,
        )?;
        let font_texture = overlay.add_texture(gpu, &tex);
        let white = overlay.add_texture(gpu, &ballistic_render::white(gpu));
        let mut t = Self::from_scene(data, &scene);
        t.gfx = Some(Gfx {
            scene: renderer,
            overlay,
            font_texture,
            white,
        });
        Ok(t)
    }

    fn from_scene(data: &bb::Tutorial, scene: &bb::Scene) -> Self {
        let texts = data
            .texts
            .get("english")
            .or_else(|| data.texts.values().next())
            .cloned()
            .unwrap_or_default();
        let mut arrows = HashMap::new();
        let mut places = HashMap::new();
        for (i, o) in scene.objects.iter().enumerate() {
            let model = Mat4::from_cols_array_2d(&o.transform);
            places.insert(o.name.clone(), model.transform_point3(Vec3::ZERO));
            let Some(mesh) = o.mesh.and_then(|m| scene.meshes.get(m as usize)) else {
                continue;
            };
            let materials = mesh
                .submeshes
                .iter()
                .filter_map(|s| s.material.map(|m| m as usize))
                .collect();
            arrows.insert(
                o.name.clone(),
                Arrow {
                    object: i,
                    materials,
                    model,
                    fade: Fade::default(),
                    grow: Fade::default(),
                },
            );
        }
        Self {
            data: data.clone(),
            texts,
            chapter: 0,
            stage: Stage::Start { ms: data.start_ms },
            text: None,
            text_fade: Fade::default(),
            back_fade: Fade::default(),
            back_hide_ms: None,
            // *Set Physics Globals* with time factor 0 at *Init*.
            frozen: true,
            ball_arrows_on: true,
            arrows,
            place: None,
            places,
            events: Vec::new(),
            gfx: None,
        }
    }

    /// The physics stands still.
    pub fn frozen(&self) -> bool {
        self.frozen && self.stage != Stage::Off
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The chapter on (for headless runs), `None` once it's over.
    pub fn chapter(&self) -> Option<usize> {
        (self.stage != Stage::Off).then_some(self.chapter)
    }

    fn chapter_data(&self) -> Option<&bb::TutorialChapter> {
        self.data.chapters.get(self.chapter)
    }

    /// Return or Q (scan codes). Whether the tutorial took it.
    pub fn key(&mut self, scan: u32) -> bool {
        let go_on = scan == self.data.continue_key;
        let quit = scan == self.data.quit_key;
        match self.stage {
            Stage::Action(Action::Intro { .. }) if go_on => {
                self.events.push(Event::Ready);
                self.events.push(Event::Message("Menu_Click"));
                self.font_out();
                self.stage = Stage::After {
                    ms: self.data.next_ms,
                };
                true
            }
            Stage::Action(Action::Intro { .. }) if quit => {
                self.quit();
                true
            }
            Stage::Action(Action::Arrows { fading_out: false }) if go_on => {
                self.events.push(Event::Message("Menu_Click"));
                self.font_out();
                self.fade_arrows(0.0);
                self.stage = Stage::Action(Action::Arrows { fading_out: true });
                true
            }
            Stage::Action(Action::Reach { waiting: true, .. } | Action::Pause) if go_on => {
                // *Physic on/off*: on again.
                self.events.push(Event::Message("Menu_Click"));
                self.frozen = false;
                self.font_out();
                self.hide_back();
                self.stage = match self.stage {
                    Stage::Action(Action::Pause) => Stage::After {
                        ms: self.data.next_ms,
                    },
                    _ => Stage::Action(Action::Reach {
                        waiting: false,
                        fading_out: false,
                    }),
                };
                true
            }
            _ => false,
        }
    }

    /// Q, or the intro's time-out: the physics runs, the ball comes, the
    /// text and box fade, the tutorial is over.
    fn quit(&mut self) {
        self.frozen = false;
        self.events.push(Event::Ready);
        self.events.push(Event::Message("Menu_Click"));
        self.font_out();
        self.hide_back();
        self.stage = Stage::Off;
    }

    fn font_out(&mut self) {
        self.text_fade.target = 0.0;
    }

    fn hide_back(&mut self) {
        if self.back_hide_ms.is_none() && self.back_fade.target > 0.0 {
            self.back_hide_ms = Some(self.data.back_delay_ms);
        }
    }

    fn show_back(&mut self) {
        self.back_hide_ms = None;
        self.back_fade.target = 1.0;
    }

    /// The chapter's arrows.
    fn chapter_arrows(&self) -> Vec<String> {
        match self.chapter_data().map(|c| &c.action) {
            Some(bb::TutorialAction::Arrows { arrows, .. }) => arrows.clone(),
            Some(bb::TutorialAction::Move { arrows, .. }) => arrows.to_vec(),
            Some(bb::TutorialAction::Reach { .. }) => vec![self.data.pointer.clone()],
            _ => Vec::new(),
        }
    }

    fn fade_arrows(&mut self, to: f32) {
        for name in self.chapter_arrows() {
            if let Some(a) = self.arrows.get_mut(&name) {
                a.fade.target = to;
            }
        }
    }

    fn arrows_faded(&self) -> bool {
        self.chapter_arrows()
            .iter()
            .filter_map(|n| self.arrows.get(n))
            .all(|a| a.fade.done())
    }

    /// The text comes and the action starts (`Tutorial Text`, `Kapitel
    /// Aktion`).
    fn begin_chapter(&mut self) {
        let Some(c) = self.chapter_data().cloned() else {
            self.stage = Stage::Off;
            return;
        };
        self.text = Some(self.chapter);
        self.text_fade = Fade {
            value: 0.0,
            target: 1.0,
        };
        self.place = None;
        let action = match &c.action {
            bb::TutorialAction::Intro => Action::Intro { idle_ms: 0.0 },
            bb::TutorialAction::Arrows { .. } => Action::Arrows { fading_out: false },
            bb::TutorialAction::Move { .. } => {
                // *MoveKeys*: the physics runs.
                self.frozen = false;
                Action::Move {
                    done: [false; 4],
                    down: [false; 4],
                    fading_out: false,
                }
            }
            bb::TutorialAction::Reach { object, .. } => {
                self.place = self.places.get(object).copied();
                self.freeze();
                Action::Reach {
                    waiting: true,
                    fading_out: false,
                }
            }
            bb::TutorialAction::Pause => {
                self.freeze();
                Action::Pause
            }
        };
        self.fade_arrows(1.0);
        self.stage = Stage::Action(action);
    }

    /// *Physic on/off*: stopped, with a sound.
    fn freeze(&mut self) {
        self.frozen = true;
        self.events
            .push(Event::Sound(self.data.freeze_sound.clone()));
    }

    /// One frame on.
    pub fn update(&mut self, f: &Frame<'_>) {
        let ms = f.real_dt * 1000.0;
        self.text_fade.frame(ms, self.data.fade_ms);
        if let Some(left) = self.back_hide_ms {
            let left = left - ms;
            if left <= 0.0 {
                self.back_hide_ms = None;
                self.back_fade.target = 0.0;
            } else {
                self.back_hide_ms = Some(left);
            }
        }
        self.back_fade.frame(ms, self.data.fade_ms);
        for a in self.arrows.values_mut() {
            a.fade.frame(ms, self.data.arrow_fade_ms);
            a.grow.frame(ms, self.data.grow_ms);
        }
        match self.stage {
            Stage::Start { ms: left } => {
                if left - ms > 0.0 {
                    self.stage = Stage::Start { ms: left - ms };
                } else {
                    self.show_back();
                    self.begin_chapter();
                }
            }
            Stage::Wait { ms: left } => self.wait_frame(f, left - ms),
            Stage::Action(a) => self.action_frame(f, a, ms),
            Stage::After { ms: left } => {
                if left - ms > 0.0 {
                    self.stage = Stage::After { ms: left - ms };
                } else {
                    // The text goes; the next chapter's wait.
                    self.text = None;
                    self.chapter += 1;
                    self.start_wait();
                }
            }
            Stage::Off => {}
        }
    }

    fn start_wait(&mut self) {
        let Some(c) = self.chapter_data() else {
            // *Tutorial off*.
            self.stage = Stage::Off;
            return;
        };
        self.stage = Stage::Wait {
            ms: match c.wait {
                bb::TutorialWait::Delay { ms } => ms,
                _ => 0.0,
            },
        };
    }

    fn wait_frame(&mut self, f: &Frame<'_>, left: f32) {
        let Some(c) = self.chapter_data().cloned() else {
            self.stage = Stage::Off;
            return;
        };
        match c.wait {
            // Shown next frame, without the box coming again.
            bb::TutorialWait::Now => self.begin_chapter(),
            bb::TutorialWait::Delay { .. } => {
                if left > 0.0 {
                    self.stage = Stage::Wait { ms: left };
                } else {
                    self.show_back();
                    self.begin_chapter();
                }
            }
            bb::TutorialWait::Near {
                object,
                distance,
                unless_ball,
            } => {
                if !self.near(f.ball, &object, distance) {
                    return;
                }
                if unless_ball.as_deref() == Some(f.ball_type) {
                    // Not for this ball: the next chapter's wait.
                    self.chapter += 1;
                    self.start_wait();
                } else {
                    self.show_back();
                    self.begin_chapter();
                }
            }
        }
    }

    /// The ball within `distance` of `object` on X and Z.
    fn near(&self, ball: Vec3, object: &str, distance: f32) -> bool {
        self.places.get(object).is_some_and(|p| {
            let d = ball - *p;
            d.x * d.x + d.z * d.z < distance * distance
        })
    }

    #[expect(clippy::too_many_lines, reason = "each action in turn")]
    fn action_frame(&mut self, f: &Frame<'_>, a: Action, ms: f32) {
        let Some(c) = self.chapter_data().cloned() else {
            self.stage = Stage::Off;
            return;
        };
        match (a, &c.action) {
            (Action::Intro { idle_ms }, _) => {
                let idle_ms = idle_ms + ms;
                if idle_ms >= self.data.quit_after_ms {
                    self.quit();
                } else {
                    self.stage = Stage::Action(Action::Intro { idle_ms });
                }
            }
            (Action::Arrows { fading_out: true }, bb::TutorialAction::Arrows { .. }) => {
                if self.arrows_faded() {
                    self.stage = Stage::After {
                        ms: self.data.next_ms,
                    };
                }
            }
            (
                Action::Move {
                    mut done,
                    mut down,
                    fading_out,
                },
                bb::TutorialAction::Move {
                    arrows,
                    end,
                    distance,
                },
            ) => {
                if fading_out {
                    if self.arrows_faded() {
                        self.ball_arrows_on = false;
                        self.stage = Stage::After {
                            ms: self.data.next_ms,
                        };
                    }
                    return;
                }
                // Holding the rotation key switches the move keys off.
                let rotating = f.held[4];
                for i in 0..4 {
                    let held = f.held[i] && !rotating;
                    if held != down[i] {
                        down[i] = held;
                        if let Some(a) = self.arrows.get_mut(&arrows[i]) {
                            a.grow.target = if held { 1.0 } else { 0.0 };
                        }
                        if !held {
                            done[i] = true;
                        }
                    }
                }
                let all = done.iter().all(|&d| d);
                let any = done.iter().any(|&d| d);
                let there = any && self.near(f.ball, end, *distance);
                if all || there || f.ball_off {
                    self.fade_arrows(0.0);
                    self.font_out();
                    self.hide_back();
                    self.stage = Stage::Action(Action::Move {
                        done,
                        down,
                        fading_out: true,
                    });
                } else {
                    self.stage = Stage::Action(Action::Move {
                        done,
                        down,
                        fading_out: false,
                    });
                }
            }
            (
                Action::Reach {
                    waiting: false,
                    fading_out,
                },
                bb::TutorialAction::Reach {
                    object,
                    distance,
                    away,
                },
            ) => {
                if fading_out {
                    if self.arrows_faded() {
                        self.stage = Stage::After {
                            ms: self.data.next_ms,
                        };
                    }
                    return;
                }
                let gone = away.is_some_and(|d| !self.near(f.ball, object, d));
                if self.near(f.ball, object, *distance) || gone {
                    self.fade_arrows(0.0);
                    self.stage = Stage::Action(Action::Reach {
                        waiting: false,
                        fading_out: true,
                    });
                }
            }
            _ => {}
        }
    }

    /// The arrows' places: the ball's hang on a frame at the ball turned
    /// (about the vertical) with its X axis to the camera; the pointer sits
    /// on its place turned with its Z axis to the camera.
    fn arrow_models(&self, ball: Vec3, eye: Vec3) -> Vec<(usize, Mat4, f32)> {
        let towards = |from: Vec3| {
            let d = Vec3::new(eye.x - from.x, 0.0, eye.z - from.z);
            d.try_normalize().unwrap_or(Vec3::X)
        };
        let mut out = Vec::new();
        for (name, a) in &self.arrows {
            if a.fade.value <= 0.0 {
                continue;
            }
            let pointer = *name == self.data.pointer;
            if !pointer && !self.ball_arrows_on {
                continue;
            }
            let model = if pointer {
                let Some(place) = self.place else { continue };
                let z = towards(place);
                let x = Vec3::Y.cross(z);
                let frame = Mat4::from_mat3(Mat3::from_cols(x, Vec3::Y, z));
                let rot = Mat4::from_mat3(Mat3::from_mat4(a.model));
                Mat4::from_translation(place) * frame * rot
            } else {
                let x = towards(ball);
                let z = x.cross(Vec3::Y);
                let frame = Mat4::from_mat3(Mat3::from_cols(x, Vec3::Y, z));
                let g = self.data.grow;
                let grown = Vec3::ONE.lerp(Vec3::new(g[0], g[1], g[2]), a.grow.value);
                Mat4::from_translation(ball) * frame * a.model * Mat4::from_scale(grown)
            };
            out.push((a.object, model, a.fade.value));
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
        opts: &RenderOptions,
        ball: Vec3,
    ) {
        let models = self.arrow_models(ball, camera.position);
        let Some(gfx) = self.gfx.as_mut() else { return };
        for a in self.arrows.values() {
            gfx.scene.set_object_hidden(a.object, true);
        }
        for (object, model, alpha) in &models {
            gfx.scene.set_object_hidden(*object, false);
            gfx.scene.set_object_transform(gpu, *object, *model);
            if let Some(a) = self.arrows.values().find(|a| a.object == *object) {
                for &m in &a.materials {
                    gfx.scene
                        .set_material_diffuse(gpu, m, [1.0, 1.0, 1.0, *alpha]);
                }
            }
        }
        if models.is_empty() {
            return;
        }
        gfx.scene
            .render(gpu, encoder, color, depth, camera, aspect, opts);
    }

    /// The text and its box: a 4:3 frame at the bottom of the window,
    /// centred.
    pub fn quads(&self, size: (u32, u32)) -> Vec<Quad> {
        let Some(gfx) = self.gfx.as_ref() else {
            return Vec::new();
        };
        #[expect(clippy::cast_precision_loss, reason = "window sizes")]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let fw = w.min(h * 4.0 / 3.0);
        let fh = fw * 0.75;
        let (x0, y0) = ((w - fw) * 0.5, h - fh);
        let px = |r: [f32; 4]| {
            [
                x0 + r[0] * fw,
                y0 + r[1] * fh,
                x0 + r[2] * fw,
                y0 + r[3] * fh,
            ]
        };
        let mut quads = Vec::new();
        if self.back_fade.value > 0.0 {
            let b = &self.data.back;
            let mut color = b.color;
            color[3] *= self.back_fade.value;
            quads.push(Quad {
                texture: gfx.white,
                rect: px(b.rect),
                uv: b.source,
                color,
            });
        }
        if let Some(i) = self.text
            && self.text_fade.value > 0.0
            && let Some(s) = self.texts.get(i)
        {
            let f = &self.data.font;
            let t = &self.data.text;
            let mut color = f.color;
            color[3] *= self.text_fade.value;
            let style = TextStyle {
                align: t.align,
                margins: t.margins,
                scale: f.scale,
                color,
                top: t.top,
            };
            quads.extend(text_block(
                f,
                gfx.font_texture,
                s,
                px(t.rect),
                &style,
                (fw, fh),
                t.wrap,
            ));
        }
        quads
    }

    pub fn render_text(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        let quads = self.quads(size);
        if quads.is_empty() {
            return;
        }
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.overlay.render(gpu, encoder, color, size, &quads);
        }
    }
}

/// The level's keys as the tutorial reads them.
pub fn held(keys: [bool; 6], index: impl Fn(PlayKey) -> usize) -> [bool; 5] {
    [
        keys[index(PlayKey::Up)],
        keys[index(PlayKey::Down)],
        keys[index(PlayKey::Left)],
        keys[index(PlayKey::Right)],
        keys[index(PlayKey::Rotate)],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> bb::Tutorial {
        let near = |o: &str, d: f32| bb::TutorialWait::Near {
            object: o.into(),
            distance: d,
            unless_ball: None,
        };
        let reach = |o: &str| bb::TutorialAction::Reach {
            object: o.into(),
            distance: 3.0,
            away: None,
        };
        bb::Tutorial {
            level: 1,
            scene: "tutorial".into(),
            texts: [(
                "english".to_owned(),
                (0..4).map(|i| format!("text {i}")).collect(),
            )]
            .into_iter()
            .collect(),
            fade_ms: 200.0,
            back_delay_ms: 300.0,
            start_ms: 100.0,
            next_ms: 510.0,
            arrow_fade_ms: 500.0,
            quit_after_ms: 25000.0,
            pointer: "Pointer".into(),
            grow: [1.0, 1.0, 1.8],
            grow_ms: 150.0,
            freeze_sound: "kuppel".into(),
            continue_key: 28,
            quit_key: 16,
            chapters: vec![
                bb::TutorialChapter {
                    wait: bb::TutorialWait::Now,
                    action: bb::TutorialAction::Intro,
                },
                bb::TutorialChapter {
                    wait: bb::TutorialWait::Now,
                    action: bb::TutorialAction::Move {
                        arrows: ["U", "D", "L", "R"].map(String::from),
                        end: "End".into(),
                        distance: 4.0,
                    },
                },
                bb::TutorialChapter {
                    wait: bb::TutorialWait::Near {
                        object: "Ramp".into(),
                        distance: 4.0,
                        unless_ball: Some("Ball_Wood".into()),
                    },
                    action: reach("Ramp"),
                },
                bb::TutorialChapter {
                    wait: near("Life", 16.0),
                    action: reach("Life"),
                },
            ],
            ..bb::Tutorial::default()
        }
    }

    fn scene() -> bb::Scene {
        let place = |name: &str, x: f32| bb::SceneObject {
            name: name.into(),
            kind: bb::ObjectKind::Entity,
            transform: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)).to_cols_array_2d(),
            mesh: None,
            parent: None,
            hidden: false,
            sprite: None,
        };
        bb::Scene {
            objects: vec![
                place("End", 50.0),
                place("Ramp", 100.0),
                place("Life", 200.0),
            ],
            meshes: Vec::new(),
            materials: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn frame(ball: Vec3, held: [bool; 5]) -> Frame<'static> {
        Frame {
            real_dt: 1.0 / 60.0,
            ball,
            ball_type: "Ball_Wood",
            held,
            ball_off: false,
        }
    }

    fn run(t: &mut Tutorial, seconds: f32, ball: Vec3, held: [bool; 5]) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a few seconds"
        )]
        let frames = (seconds * 60.0) as u32;
        for _ in 0..frames {
            t.update(&frame(ball, held));
        }
    }

    #[test]
    fn the_intro_waits_for_return_and_frees_the_ball() {
        let mut t = Tutorial::from_scene(&data(), &scene());
        run(&mut t, 1.0, Vec3::ZERO, [false; 5]);
        assert_eq!(t.chapter(), Some(0));
        assert!(t.frozen());
        assert_eq!(t.take_events(), Vec::new());
        assert!(t.key(28));
        assert!(t.take_events().contains(&Event::Ready));
        // Still frozen until the navigation chapter.
        run(&mut t, 1.0, Vec3::ZERO, [false; 5]);
        assert_eq!(t.chapter(), Some(1));
        assert!(!t.frozen());
    }

    #[test]
    fn q_or_the_time_out_ends_it() {
        let mut t = Tutorial::from_scene(&data(), &scene());
        run(&mut t, 0.5, Vec3::ZERO, [false; 5]);
        assert!(t.key(16));
        assert_eq!(t.chapter(), None);
        assert!(!t.frozen());
        let mut t = Tutorial::from_scene(&data(), &scene());
        run(&mut t, 26.0, Vec3::ZERO, [false; 5]);
        assert_eq!(t.chapter(), None);
        assert!(t.take_events().contains(&Event::Ready));
    }

    #[test]
    fn navigation_ends_once_every_key_was_pressed() {
        let mut t = Tutorial::from_scene(&data(), &scene());
        run(&mut t, 0.5, Vec3::ZERO, [false; 5]);
        t.key(28);
        run(&mut t, 1.0, Vec3::ZERO, [false; 5]);
        for i in 0..4 {
            let mut held = [false; 5];
            held[i] = true;
            run(&mut t, 0.2, Vec3::ZERO, held);
            run(&mut t, 0.2, Vec3::ZERO, [false; 5]);
        }
        run(&mut t, 2.0, Vec3::ZERO, [false; 5]);
        // A wooden ball skips the ramp's chapter: waiting for the life.
        assert_eq!(t.chapter(), Some(2));
        run(&mut t, 0.5, Vec3::new(100.0, 0.0, 0.0), [false; 5]);
        assert_eq!(t.chapter(), Some(3));
        assert!(!t.frozen());
        // Near the life: frozen (with the sound) until Return, then done
        // when the ball gets there.
        run(&mut t, 0.5, Vec3::new(190.0, 0.0, 0.0), [false; 5]);
        assert!(t.frozen());
        assert!(t.take_events().contains(&Event::Sound("kuppel".into())));
        assert!(t.key(28));
        assert!(!t.frozen());
        run(&mut t, 2.0, Vec3::new(199.0, 0.0, 0.0), [false; 5]);
        assert_eq!(t.chapter(), None);
    }

    #[test]
    fn holding_the_rotation_key_keeps_the_arrows_still() {
        let mut t = Tutorial::from_scene(&data(), &scene());
        run(&mut t, 0.5, Vec3::ZERO, [false; 5]);
        t.key(28);
        run(&mut t, 1.0, Vec3::ZERO, [false; 5]);
        run(&mut t, 0.5, Vec3::ZERO, [true, true, true, true, true]);
        run(&mut t, 0.5, Vec3::ZERO, [false; 5]);
        assert_eq!(t.chapter(), Some(1));
    }
}
