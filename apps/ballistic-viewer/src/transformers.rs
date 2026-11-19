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
//! Not yet: the ring animation (`AnimTrafo`), the burst's particles and the
//! pieces of the old ball, and the sounds.

use anyhow::{Context, Result};
use ballistic_bundle::{PhysicsData, Scene, TransformerParams};
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
    /// Shown (its sector is active).
    shown: bool,
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Ball control off for the whole sequence.
    Begin,
    /// The ball leaves the simulation; the play mode keeps its pose.
    Release,
    /// Where the pull has moved the ball.
    Move(Vec3),
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
            let mesh_offset = append_prefab(&mut merged, &prefab);
            let mf = prefab
                .objects
                .iter()
                .position(|o| o.name == format!("{}_MF", t.group))
                .with_context(|| format!("{key}: no {}_MF", t.group))?;
            for s in &spawns {
                placeholders.push(s.placeholder as usize);
                let placed = modules::placed_objects(&prefab, &t.group, s.transform)
                    .with_context(|| format!("placing {}", t.group))?;
                let mut objects = Vec::new();
                for (i, o) in prefab.objects.iter().enumerate() {
                    let Some(mesh) = o.mesh else { continue };
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
                    shown: false,
                });
            }
        }
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
        if let Some(r) = self.renderer.as_mut() {
            r.render(gpu, encoder, color, depth, camera, aspect, opts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballistic_bundle::TransformerType;

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
        }
    }

    fn one(at: Vec3) -> Transformers {
        Transformers {
            items: vec![Item {
                ball: 2,
                sector: 1,
                frame: Mat4::from_translation(at),
                objects: Vec::new(),
                shown: true,
            }],
            renderer: None,
            params: params(),
            phase: Phase::Idle,
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
        assert_eq!(at_frame(Event::Begin), Some(0));
        assert_eq!(at_frame(Event::Release), Some(2));
        // 1350 + 1000 + 150 ms of 60 Hz frames after the release, give or take
        // a frame per Delayer.
        let swap = at_frame(Event::Swap(2)).unwrap();
        assert!((150..=155).contains(&(swap - 2)), "swap at frame {swap}");
        assert_eq!(at_frame(Event::Physicalize), Some(swap + 2));
        assert_eq!(log.len(), 4, "{log:?}");
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
}
