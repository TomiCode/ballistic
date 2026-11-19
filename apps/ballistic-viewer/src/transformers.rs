//! Ball transformers, like the original's `Trafo Manager` (research findings
//! 14).
//!
//! The transformers (`P_Trafo_Paper`, `_Stone`, `_Wood`) are placed at their
//! placeholders and shown while their sector is active. Every frame the script
//! takes the transformer nearest to the ball, shown or not. If the ball's centre
//! is nearer than the reach (4.3) and the ball is of another type:
//! 1. ball control goes off;
//! 2. two frames later the ball is taken out of the simulation and pulled into
//!    the transformer for 1.35 s, by *TT Set Dynamic Position* in the
//!    transformer's frame (towards 3 above its origin);
//! 3. it hangs there for 1 s, bursts, and 0.15 s later the new ball appears in
//!    its place;
//! 4. two frames later the new ball is physicalized, at rest, and control
//!    comes back on;
//! 5. three frames later the script looks for transformers again.
//!
//! The delays run on real time. The pull is per frame in the original; here it
//! runs at the camera's fixed rate ([`crate::follow::TICK_HZ`]).
//!
//! With the pull, `AnimTrafo_MF Script` plays at the transformer: the
//! transformer is replaced by the animation's copy, coloured like it. One frame
//! later its four ring parts open (0.35 s). Then the bars rise and come down
//! again (2 s), the flash field between them shown and flickering (its texture
//! scrolled every 50 ms), and the type's shadow fading. Then the ring closes
//! (0.2 s) and the transformer is back. At the burst the old ball's pieces fly
//! off ([`crate::pieces`]).
//!
//! The trigger's message to the sound scripts plays `Misc_Trafo` one frame
//! later.

use anyhow::{Context, Result};
use ballistic_bundle::{PhysicsData, Progression, Scene, TransformerParams};
use ballistic_physics::{Mat4, Vec3, modules, props};
use ballistic_render::{Camera, Gpu, RenderOptions, SceneRenderer};

use crate::follow::{Chase, TICK_HZ};
use crate::play::Bundle;
use crate::props::append_prefab;

/// Frames from the trigger to the pull (the script's two-frame link delay).
const START_FRAMES: u32 = 2;
/// Frames from the new ball appearing to its physicalization.
const SWAP_FRAMES: u32 = 2;
/// Frames from control coming back until the search runs again (it runs in
/// the frame after: 2 + 1 link delays).
const SETTLE_FRAMES: u32 = 2;

struct Item {
    /// Index into the physics data's balls.
    ball: usize,
    sector: u32,
    /// The transformer's frame (`P_Trafo_*_MF`'s world matrix).
    frame: Mat4,
    /// Its drawn objects in the merged scene.
    objects: Vec<usize>,
    /// The transformer object itself (`_MF`), hidden while the animation plays.
    mf_object: Option<usize>,
    /// Its mesh's second material (the ring takes its colours), and its type's
    /// shadow material, in the merged scene.
    color_material: Option<usize>,
    shadow_material: Option<usize>,
    /// Shown (its sector is active).
    shown: bool,
}

/// The animation's objects in the merged scene.
struct AnimObjects {
    /// Every drawn object, and its matrix relative to the animation's root.
    all: Vec<(usize, Mat4)>,
    flash: Option<usize>,
    /// Ring parts: the part, and its frame's matrix relative to the root.
    ring: Vec<(usize, Mat4)>,
    ring_material: Option<usize>,
    flash_material: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// Placed and shown; the ring opens next frame.
    Intro,
    Open,
    /// *Up 'n Down* with `FlashAnim`.
    Lift,
    Close,
    /// Over: the transformer is shown again (drawn once more, then idle).
    Exit,
}

#[derive(Debug, Clone)]
struct Anim {
    item: usize,
    stage: Stage,
    elapsed_ms: f32,
    /// The flash field's texture scrolls (Texture Scroller steps taken), the
    /// time since the last one, and whether the next one is due this frame.
    flash_steps: u32,
    flash_wait_ms: f32,
    flash_due: bool,
}

#[derive(Debug, Clone)]
enum Phase {
    Idle,
    Starting {
        item: usize,
        frames: u32,
    },
    /// The pull, in the transformer's frame.
    Pulling {
        item: usize,
        chase: Chase,
        elapsed_ms: f32,
        tick_acc: f32,
    },
    Holding {
        item: usize,
        elapsed_ms: f32,
    },
    Bursting {
        item: usize,
        elapsed_ms: f32,
    },
    Swapped {
        frames: u32,
    },
    Settling {
        frames: u32,
    },
}

/// What the play mode does for the transformer script this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A sound to play (bundle key).
    Sound(String),
    /// Ball control off for the whole sequence.
    Begin,
    /// The ball leaves the simulation; the play mode keeps its pose.
    Release,
    /// Where the pull has moved the ball.
    Move(Vec3),
    /// The old ball (of this type) bursts into its pieces.
    Burst(usize),
    /// The new ball type (index into the physics data's balls) is shown in
    /// place of the old one.
    Swap(usize),
    /// The new ball joins the simulation at rest; control comes back on.
    Physicalize,
}

pub struct Transformers {
    items: Vec<Item>,
    renderer: Option<SceneRenderer>,
    params: TransformerParams,
    phase: Phase,
    anim_objects: Option<AnimObjects>,
    anim: Option<Anim>,
    /// Frames until the trigger's sound.
    sound_in: Option<u32>,
    /// Level objects the transformers replace.
    pub placeholders: Vec<usize>,
}

impl Transformers {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        level: &Scene,
        physics: &PhysicsData,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let mut merged = Scene {
            meshes: Vec::new(),
            materials: Vec::new(),
            objects: Vec::new(),
            groups: Vec::new(),
        };
        let params = physics.transformers.clone();
        let mut items = Vec::new();
        let mut placeholders = Vec::new();
        for t in &params.types {
            let Some(ball) = physics.balls.iter().position(|b| b.name == t.ball) else {
                eprintln!("warning: {}: unknown ball {}", t.group, t.ball);
                continue;
            };
            let spawns = props::placeholders(level, &t.group);
            if spawns.is_empty() {
                continue;
            }
            let key = format!("ph/{}", t.group.to_lowercase());
            let prefab = bundle
                .scene(&key)
                .with_context(|| format!("loading {key}"))?;
            let material_offset = merged.materials.len();
            let mesh_offset = append_prefab(&mut merged, &prefab);
            let mf = prefab
                .objects
                .iter()
                .position(|o| o.name == format!("{}_MF", t.group))
                .with_context(|| format!("{key}: no {}_MF", t.group))?;
            // *Get Colors*: element 1 of the transformer mesh's material list.
            let color_material = prefab.objects[mf]
                .mesh
                .and_then(|m| prefab.meshes[m as usize].submeshes.get(1))
                .and_then(|sm| sm.material)
                .map(|i| i as usize + material_offset);
            // *Up 'n Down*: the shadow material named after the type's prefab.
            let shadow_material = prefab
                .materials
                .iter()
                .position(|m| m.name == format!("{}_Shadow", t.group))
                .map(|i| i + material_offset);
            for s in &spawns {
                placeholders.push(s.placeholder as usize);
                let placed = modules::placed_objects(&prefab, &t.group, s.transform)
                    .with_context(|| format!("placing {}", t.group))?;
                let mut objects = Vec::new();
                let mut mf_object = None;
                for (i, o) in prefab.objects.iter().enumerate() {
                    let Some(mesh) = o.mesh else { continue };
                    if i == mf {
                        mf_object = Some(merged.objects.len());
                    }
                    let mut obj = o.clone();
                    obj.mesh = Some(mesh + mesh_offset);
                    obj.parent = None;
                    // Shown when the sector activates (activation type 1).
                    obj.hidden = true;
                    obj.transform = placed[i].to_cols_array_2d();
                    objects.push(merged.objects.len());
                    merged.objects.push(obj);
                }
                items.push(Item {
                    ball,
                    sector: s.sector,
                    frame: placed[mf],
                    objects,
                    mf_object,
                    color_material,
                    shadow_material,
                    shown: false,
                });
            }
        }
        let anim_objects = if items.is_empty() {
            None
        } else {
            Some(anim_objects(bundle, &params, &mut merged)?)
        };
        let renderer = if merged.objects.is_empty() {
            None
        } else {
            let (r, warnings) =
                SceneRenderer::new(gpu, &merged, format, &mut |k| bundle.texture(k).ok())?;
            for w in warnings {
                eprintln!("warning: {w}");
            }
            Some(r)
        };
        Ok(Self {
            items,
            renderer,
            params,
            phase: Phase::Idle,
            anim_objects,
            anim: None,
            sound_in: None,
            placeholders,
        })
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// Shown while active, hidden otherwise (reset type 0).
    pub fn set_sector_active(&mut self, sector: u32, on: bool) {
        for t in self.items.iter_mut().filter(|t| t.sector == sector) {
            t.shown = on;
            if let Some(r) = self.renderer.as_mut() {
                for &o in &t.objects {
                    r.set_object_hidden(o, !on);
                }
            }
        }
    }

    /// Ball control stays off (from the trigger until the new ball is
    /// physicalized).
    pub fn busy(&self) -> bool {
        !matches!(self.phase, Phase::Idle | Phase::Settling { .. })
    }

    /// Development aid: where to put the ball for the nearest transformer of
    /// another type (resting on its origin), shown ones first, if any.
    pub fn nearest_other(&self, ball: Vec3, ball_type: usize) -> Option<Vec3> {
        let nearest = |shown: bool| {
            self.items
                .iter()
                .filter(|t| t.ball != ball_type && t.shown == shown)
                .map(|t| t.frame.transform_point3(Vec3::ZERO))
                .min_by(|a, b| a.distance(ball).total_cmp(&b.distance(ball)))
        };
        nearest(true)
            .or_else(|| nearest(false))
            .map(|p| p + Vec3::Y * 2.0)
    }

    /// One frame of the script. `ball` is the ball's position, `ball_type` its
    /// index into the physics data's balls.
    #[allow(clippy::too_many_lines)]
    pub fn frame(&mut self, ball: Vec3, ball_type: usize, real_dt: f32) -> Vec<Event> {
        let ms = real_dt * 1000.0;
        let mut out = Vec::new();
        self.advance_anim(ms);
        match self.sound_in {
            Some(0) => {
                self.sound_in = None;
                if let Some(c) = &self.params.sound {
                    out.push(Event::Sound(c.sound.clone()));
                }
            }
            Some(n) => self.sound_in = Some(n - 1),
            None => {}
        }
        self.phase = match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => {
                // Get Nearest In Group, then the distance test and the type test.
                let nearest = self.items.iter().enumerate().min_by(|(_, a), (_, b)| {
                    let d = |t: &Item| t.frame.transform_point3(Vec3::ZERO).distance(ball);
                    d(a).total_cmp(&d(b))
                });
                match nearest {
                    Some((i, t))
                        if t.frame.transform_point3(Vec3::ZERO).distance(ball)
                            < self.params.reach
                            && t.ball != ball_type =>
                    {
                        out.push(Event::Begin);
                        if let Some(c) = &self.params.sound {
                            if c.delay_frames == 0 {
                                out.push(Event::Sound(c.sound.clone()));
                            } else {
                                // Counted down from the next frame.
                                self.sound_in = Some(c.delay_frames - 1);
                            }
                        }
                        Phase::Starting {
                            item: i,
                            frames: START_FRAMES,
                        }
                    }
                    _ => Phase::Idle,
                }
            }
            Phase::Starting { item, frames } if frames > 1 => Phase::Starting {
                item,
                frames: frames - 1,
            },
            Phase::Starting { item, .. } => {
                // starte Trafo, dephysic Ball: unphysicalize, pull on (it
                // moves from the next frame), its Delayer starts.
                out.push(Event::Release);
                // starte Trafo: AnimTrafo_MF Script at this transformer.
                self.anim = Some(Anim {
                    item,
                    stage: Stage::Intro,
                    elapsed_ms: 0.0,
                    flash_steps: 0,
                    flash_wait_ms: 0.0,
                    flash_due: false,
                });
                let local = self.items[item].frame.inverse().transform_point3(ball);
                Phase::Pulling {
                    item,
                    chase: Chase::new(
                        local,
                        Vec3::from(self.params.pull_force),
                        Vec3::from(self.params.pull_damping),
                    ),
                    elapsed_ms: 0.0,
                    tick_acc: 0.0,
                }
            }
            Phase::Pulling {
                item,
                mut chase,
                elapsed_ms,
                mut tick_acc,
            } => {
                let elapsed_ms = elapsed_ms + ms;
                if elapsed_ms >= self.params.pull_ms {
                    Phase::Holding {
                        item,
                        elapsed_ms: 0.0,
                    }
                } else {
                    let tick = 1.0 / TICK_HZ;
                    tick_acc += real_dt;
                    while tick_acc >= tick {
                        tick_acc -= tick;
                        chase.tick(Vec3::ZERO, Vec3::from(self.params.pull_offset), tick);
                    }
                    out.push(Event::Move(
                        self.items[item].frame.transform_point3(chase.pos),
                    ));
                    Phase::Pulling {
                        item,
                        chase,
                        elapsed_ms,
                        tick_acc,
                    }
                }
            }
            Phase::Holding { item, elapsed_ms } => {
                let elapsed_ms = elapsed_ms + ms;
                if elapsed_ms >= self.params.hold_ms {
                    // start Explosion, then its Delayer.
                    out.push(Event::Burst(ball_type));
                    Phase::Bursting {
                        item,
                        elapsed_ms: 0.0,
                    }
                } else {
                    Phase::Holding { item, elapsed_ms }
                }
            }
            Phase::Bursting { item, elapsed_ms } => {
                let elapsed_ms = elapsed_ms + ms;
                if elapsed_ms >= self.params.swap_ms {
                    // set new Ball: the transformer's type, at the old ball's matrix.
                    out.push(Event::Swap(self.items[item].ball));
                    Phase::Swapped {
                        frames: SWAP_FRAMES,
                    }
                } else {
                    Phase::Bursting { item, elapsed_ms }
                }
            }
            Phase::Swapped { frames } if frames > 1 => Phase::Swapped { frames: frames - 1 },
            Phase::Swapped { .. } => {
                out.push(Event::Physicalize);
                Phase::Settling {
                    frames: SETTLE_FRAMES,
                }
            }
            Phase::Settling { frames } if frames > 1 => Phase::Settling { frames: frames - 1 },
            Phase::Settling { .. } => Phase::Idle,
        };
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
    ) {
        self.sync_anim(gpu);
        if let Some(r) = self.renderer.as_mut() {
            r.render(gpu, encoder, color, depth, camera, aspect, opts);
        }
    }

    /// The animation's script, one frame on.
    fn advance_anim(&mut self, ms: f32) {
        let a = &self.params.animation;
        let Some(anim) = self.anim.as_mut() else {
            return;
        };
        match anim.stage {
            // lntro → Ring_Open, one frame later.
            Stage::Intro => {
                anim.stage = Stage::Open;
                anim.elapsed_ms = 0.0;
            }
            Stage::Open => {
                anim.elapsed_ms += ms;
                if a.ring_open.done(anim.elapsed_ms) {
                    // Up 'n Down and FlashAnim, whose first scroll is at once.
                    anim.stage = Stage::Lift;
                    anim.elapsed_ms = 0.0;
                    anim.flash_steps = 1;
                    anim.flash_wait_ms = 0.0;
                }
            }
            Stage::Lift => {
                anim.elapsed_ms += ms;
                // Delayer, then one frame to the Sequencer's next scroll.
                if anim.flash_due {
                    anim.flash_due = false;
                    anim.flash_steps += 1;
                    anim.flash_wait_ms = 0.0;
                } else {
                    anim.flash_wait_ms += ms;
                    anim.flash_due = anim.flash_wait_ms >= a.flash_ms;
                }
                if a.lift.done(anim.elapsed_ms) {
                    anim.stage = Stage::Close;
                    anim.elapsed_ms = 0.0;
                }
            }
            Stage::Close => {
                anim.elapsed_ms += ms;
                if a.ring_close.done(anim.elapsed_ms) {
                    anim.stage = Stage::Exit;
                }
            }
            Stage::Exit => self.anim = None,
        }
    }

    /// Draws the animation's state: transforms, visibility, colours.
    fn sync_anim(&mut self, gpu: &Gpu) {
        let (Some(anim), Some(objs), Some(r)) = (
            self.anim.as_ref(),
            self.anim_objects.as_ref(),
            self.renderer.as_mut(),
        ) else {
            return;
        };
        let a = &self.params.animation;
        let item = &self.items[anim.item];
        let playing = anim.stage != Stage::Exit;
        if let Some(o) = item.mf_object {
            r.set_object_hidden(o, playing || !item.shown);
        }
        for &(o, _) in &objs.all {
            r.set_object_hidden(o, !playing);
        }
        if let Some(f) = objs.flash {
            r.set_object_hidden(f, anim.stage != Stage::Lift);
        }
        if !playing {
            return;
        }
        let end = |p: &Progression| p.value(p.duration_ms);
        let ring = match anim.stage {
            Stage::Intro | Stage::Exit => 0.0,
            Stage::Open => a.ring_open.value(anim.elapsed_ms),
            Stage::Lift => end(&a.ring_open),
            Stage::Close => a.ring_close.value(anim.elapsed_ms),
        };
        let lift = match anim.stage {
            Stage::Lift => a.lift.value(anim.elapsed_ms),
            Stage::Close => end(&a.lift),
            _ => 0.0,
        };
        // Set World Matrix (hierarchy) on the root; Set Position on the bars
        // in the root's frame (with their children); on each ring part in its
        // frame.
        let root = item.frame;
        let bars_move =
            Mat4::from_translation(root.transform_vector3(Vec3::from(a.bars_offset) * lift));
        for &(o, rest) in &objs.all {
            r.set_object_transform(gpu, o, bars_move * root * rest);
        }
        for &(o, frame) in &objs.ring {
            let part_move = Mat4::from_translation(
                (root * frame).transform_vector3(Vec3::from(a.ring_offset) * ring),
            );
            let rest = objs
                .all
                .iter()
                .find(|(i, _)| *i == o)
                .map_or(Mat4::IDENTITY, |(_, m)| *m);
            r.set_object_transform(gpu, o, part_move * bars_move * root * rest);
        }
        // Get Colors: the ring takes the transformer's colour (alpha kept).
        if let (Some(c), Some(m)) = (item.color_material, objs.ring_material)
            && let (Some((diffuse, emissive)), Some((ring_diffuse, _))) =
                (r.material_colors(c), r.material_colors(m))
        {
            r.set_material_diffuse(
                gpu,
                m,
                [diffuse[0], diffuse[1], diffuse[2], ring_diffuse[3]],
            );
            r.set_material_emissive(gpu, m, emissive);
        }
        // The shadow fades with the bars (all of this type's shadows: they
        // share the material).
        if matches!(anim.stage, Stage::Lift | Stage::Close)
            && let Some(m) = item.shadow_material
        {
            let [c0, c1] = a.shadow_colors;
            let c = std::array::from_fn(|i| c0[i] + (c1[i] - c0[i]) * lift);
            r.set_material_diffuse(gpu, m, c);
        }
        if let Some(m) = objs.flash_material {
            let [s0, s1] = a.flash_scroll;
            // Each step adds the next offset: they alternate.
            let n = anim.flash_steps;
            let k0 = n.div_ceil(2) as f32;
            let k1 = (n / 2) as f32;
            r.set_material_uv_offset(gpu, m, [s0[0] * k0 + s1[0] * k1, s0[1] * k0 + s1[1] * k1]);
        }
    }
}

/// Adds the animation's objects (hidden) to the merged scene.
fn anim_objects(
    bundle: &mut Bundle,
    params: &TransformerParams,
    merged: &mut Scene,
) -> Result<AnimObjects> {
    let a = &params.animation;
    let scene = bundle.scene("animtrafo").context("loading animtrafo")?;
    let material_offset = merged.materials.len();
    let mesh_offset = append_prefab(merged, &scene);
    let group = a.root.strip_suffix("_MF").unwrap_or(&a.root);
    let rest = modules::placed_objects(&scene, group, Mat4::IDENTITY)
        .with_context(|| format!("animtrafo: placing {}", a.root))?;
    let index = |name: &str| scene.objects.iter().position(|o| o.name == name);
    let mut all = Vec::new();
    let mut merged_of = vec![None; scene.objects.len()];
    for (i, o) in scene.objects.iter().enumerate() {
        let Some(mesh) = o.mesh else { continue };
        let mut obj = o.clone();
        obj.mesh = Some(mesh + mesh_offset);
        obj.parent = None;
        obj.hidden = true;
        obj.transform = rest[i].to_cols_array_2d();
        merged_of[i] = Some(merged.objects.len());
        all.push((merged.objects.len(), rest[i]));
        merged.objects.push(obj);
    }
    let merged_index = |name: &str| index(name).and_then(|i| merged_of[i]);
    let ring = a
        .ring_parts
        .iter()
        .filter_map(|p| Some((merged_index(&p.object)?, rest[index(&p.frame)?])))
        .collect();
    let material = |name: &str| {
        scene
            .materials
            .iter()
            .position(|m| m.name == name)
            .map(|i| i + material_offset)
    };
    let flash_material = index(&a.flash)
        .and_then(|i| scene.objects[i].mesh)
        .and_then(|m| scene.meshes[m as usize].submeshes.first())
        .and_then(|sm| sm.material)
        .map(|i| i as usize + material_offset);
    Ok(AnimObjects {
        all,
        flash: merged_index(&a.flash),
        ring,
        ring_material: material(&a.ring_material),
        flash_material,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballistic_bundle::{CurvePoint, TransformerAnimation, TransformerType};

    /// A straight 0 → 1 progression.
    fn linear(duration_ms: f32) -> Progression {
        let p = |x: f32| CurvePoint {
            position: [x, x],
            in_tangent: [1.0, 1.0],
            out_tangent: [1.0, 1.0],
        };
        Progression {
            duration_ms,
            curve: vec![p(0.0), p(1.0)],
        }
    }

    fn params() -> TransformerParams {
        TransformerParams {
            types: vec![TransformerType {
                group: "P_Trafo_Stone".into(),
                ball: "Ball_Stone".into(),
            }],
            reach: 4.3,
            pull_force: [2.0; 3],
            pull_damping: [0.7; 3],
            pull_offset: [0.0, -3.0, 0.0],
            pull_ms: 1350.0,
            hold_ms: 1000.0,
            swap_ms: 150.0,
            animation: TransformerAnimation {
                root: "AnimTrafo_MF".into(),
                bars: "AnimTrafo_Bars".into(),
                ring_parts: Vec::new(),
                ring_offset: [-0.5, 0.0, 0.5],
                ring_open: linear(350.0),
                ring_close: linear(200.0),
                bars_offset: [0.0, 5.2, 0.0],
                lift: linear(2000.0),
                shadow_colors: [[1.0; 4], [1.0, 1.0, 1.0, 0.2]],
                flash: "AnimTrafo_Flashfield".into(),
                flash_scroll: [[0.5, 0.0], [-0.5, 0.0]],
                flash_ms: 50.0,
                ring_material: "AnimTrafo_RingParts_Color".into(),
            },
            pieces: Vec::new(),
            pieces_fade_after_ms: 20000.0,
            sound: Some(ballistic_bundle::SoundCue {
                sound: "misc_trafo".into(),
                delay_frames: 1,
            }),
        }
    }

    fn one(at: Vec3) -> Transformers {
        Transformers {
            items: vec![Item {
                ball: 2,
                sector: 1,
                frame: Mat4::from_translation(at),
                objects: Vec::new(),
                mf_object: None,
                color_material: None,
                shadow_material: None,
                shown: true,
            }],
            renderer: None,
            params: params(),
            phase: Phase::Idle,
            anim_objects: None,
            anim: None,
            sound_in: None,
            placeholders: Vec::new(),
        }
    }

    #[test]
    fn swaps_the_ball_like_the_script() {
        let at = Vec3::new(10.0, 0.0, 5.0);
        let mut t = one(at);
        let dt = 1.0 / 60.0;
        // Same type: nothing happens.
        assert_eq!(t.frame(at + Vec3::Y * 2.0, 2, dt), Vec::new());
        // Out of reach: nothing happens.
        assert_eq!(t.frame(at + Vec3::X * 4.4, 0, dt), Vec::new());

        let mut ball = at + Vec3::Y * 2.0;
        let mut kind = 0;
        let mut log = Vec::new();
        for frame in 0..400 {
            for e in t.frame(ball, kind, dt) {
                match e {
                    Event::Move(p) => ball = p,
                    Event::Swap(k) => {
                        kind = k;
                        log.push((frame, e));
                    }
                    _ => log.push((frame, e)),
                }
            }
        }
        let at_frame = |e: Event| log.iter().find(|(_, x)| *x == e).map(|(f, _)| *f);
        assert_eq!(at_frame(Event::Sound("misc_trafo".into())), Some(1));
        assert_eq!(at_frame(Event::Begin), Some(0));
        assert_eq!(at_frame(Event::Release), Some(2));
        // 1350 + 1000 + 150 ms of 60 Hz frames after the release, give or take
        // a frame per Delayer.
        let swap = at_frame(Event::Swap(2)).unwrap();
        assert!((150..=155).contains(&(swap - 2)), "swap at frame {swap}");
        assert_eq!(at_frame(Event::Physicalize), Some(swap + 2));
        // The burst 150 ms before the swap, the old type's.
        let burst = at_frame(Event::Burst(0)).unwrap();
        assert!(
            (8..=10).contains(&(swap - burst)),
            "burst at {burst}, swap at {swap}"
        );
        assert_eq!(log.len(), 6, "{log:?}");
        // Pulled to 3 above the transformer's origin.
        assert!(ball.distance(at + Vec3::Y * 3.0) < 0.05, "ended at {ball}");
        // The script searches again three frames after control is back: with
        // the wrong type the ball would transform again then.
        assert!(!t.busy());
        let physicalized = at_frame(Event::Physicalize).unwrap();
        let mut again = one(at);
        let mut first = None;
        for frame in 0..400 {
            if again.frame(ball, 0, dt).contains(&Event::Begin) && frame > physicalized {
                first.get_or_insert(frame);
            }
        }
        assert_eq!(first, Some(physicalized + 3));
    }

    #[test]
    fn plays_the_animation_with_the_pull() {
        let at = Vec3::ZERO;
        let mut t = one(at);
        let dt = 1.0 / 60.0;
        let mut stages = Vec::new();
        for frame in 0..300 {
            t.frame(at + Vec3::Y * 2.0, 0, dt);
            let stage = t.anim.as_ref().map(|a| a.stage);
            if stages.last().map(|(_, s)| *s) != Some(stage) {
                stages.push((frame, stage));
            }
        }
        let start = |s: Stage| stages.iter().find(|(_, x)| *x == Some(s)).map(|(f, _)| *f);
        // starte Trafo with the release (frame 2), the ring opens a frame later.
        assert_eq!(start(Stage::Intro), Some(2));
        assert_eq!(start(Stage::Open), Some(3));
        let lift = start(Stage::Lift).unwrap();
        let close = start(Stage::Close).unwrap();
        let exit = start(Stage::Exit).unwrap();
        // 350, 2000 and 200 ms of 60 Hz frames.
        assert!((21..=22).contains(&(lift - 3)), "{stages:?}");
        assert!((120..=121).contains(&(close - lift)), "{stages:?}");
        assert!((12..=13).contains(&(exit - close)), "{stages:?}");
        // Then it's over (until the same old ball type transforms again here).
        let after = stages.iter().position(|(f, _)| *f == exit).unwrap() + 1;
        assert_eq!(stages.get(after), Some(&(exit + 1, None)), "{stages:?}");
    }
}
