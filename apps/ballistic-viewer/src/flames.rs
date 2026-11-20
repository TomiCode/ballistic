//! The level start's and the checkpoints' flames (research findings 20):
//! particle systems on the `PS_FourFlames` and `PC_TwoFlames` prefabs,
//! switched as their scripts do.
//!
//! - The start's four flames burn while the ball is within 70 of it (X and
//!   Z).
//! - The next checkpoint's big flame burns while the ball is within 70.
//!   Reaching it puts that out and lights the two small flames, which then
//!   also burn only within 70. The game has two copies of the checkpoint
//!   prefab, moved on in turn, so only the next checkpoint and the last
//!   one reached have flames.

use ballistic_bundle::{ParticleEmitter, Scene};
use glam::{Mat4, Vec3};

use crate::particles::Emitter;
use crate::progress::numbered;

/// The scripts' "near" distance (X and Z).
const NEAR: f32 = 70.0;

struct Site {
    centre: Vec3,
    /// The big flame (checkpoints) or all of them (the start).
    main: Vec<Emitter>,
    /// The checkpoints' small flames.
    small: Vec<Emitter>,
}

pub struct Flames {
    start: Option<Site>,
    checkpoints: Vec<Site>,
}

/// The emitters of placeholder `name` (prefab `ph/<name without _NN>`),
/// placed at `world`.
fn place(
    particles: &[ParticleEmitter],
    prefab: &mut impl FnMut(&str) -> Option<Scene>,
    name: &str,
    world: Mat4,
    seed: &mut u32,
) -> Vec<Emitter> {
    let Some((base, _)) = name.rsplit_once('_') else {
        return Vec::new();
    };
    let key = format!("ph/{}", base.to_lowercase());
    let Some(scene) = prefab(&key) else {
        return Vec::new();
    };
    let find = |n: &str| scene.objects.iter().find(|o| o.name == n);
    let root = find(&format!("{base}_MF"))
        .map_or(Mat4::IDENTITY, |o| Mat4::from_cols_array_2d(&o.transform));
    particles
        .iter()
        .filter(|p| p.scene == key)
        .filter_map(|p| {
            let entity = Mat4::from_cols_array_2d(&find(&p.object)?.transform);
            *seed = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
            Some(Emitter::new(
                p.clone(),
                world * root.inverse() * entity,
                *seed,
            ))
        })
        .collect()
}

impl Flames {
    pub fn new(
        level: &Scene,
        particles: &[ParticleEmitter],
        mut prefab: impl FnMut(&str) -> Option<Scene>,
    ) -> Self {
        let mut seed = 0x9E37_79B9;
        let start = level
            .groups
            .iter()
            .find(|g| g.name == "PS_Levelstart")
            .and_then(|g| g.members.first())
            .and_then(|&i| level.objects.get(i as usize))
            .map(|o| {
                let world = Mat4::from_cols_array_2d(&o.transform);
                Site {
                    centre: world.transform_point3(Vec3::ZERO),
                    main: place(particles, &mut prefab, &o.name, world, &mut seed),
                    small: Vec::new(),
                }
            });
        let checkpoints = numbered(level, "PC_Checkpoints")
            .into_iter()
            .map(|(_, world, name)| {
                let (main, small) = place(particles, &mut prefab, &name, world, &mut seed)
                    .into_iter()
                    .partition(|e| e.params.object.ends_with("_Flame_Big"));
                Site {
                    centre: world.transform_point3(Vec3::ZERO),
                    main,
                    small,
                }
            })
            .collect();
        Self { start, checkpoints }
    }

    /// One frame: `reached` checkpoints passed so far.
    pub fn update(&mut self, ball: Vec3, reached: usize, real_ms: f32) {
        let near = |c: Vec3| {
            let d = ball - c;
            d.x * d.x + d.z * d.z <= NEAR * NEAR
        };
        if let Some(s) = self.start.as_mut() {
            let on = near(s.centre);
            for e in &mut s.main {
                e.set_on(on);
                e.update(real_ms);
            }
        }
        for (i, s) in self.checkpoints.iter_mut().enumerate() {
            let close = near(s.centre);
            for e in &mut s.main {
                e.set_on(close && i == reached);
                e.update(real_ms);
            }
            for e in &mut s.small {
                e.set_on(close && i + 1 == reached);
                e.update(real_ms);
            }
        }
    }

    pub fn emitters(&self) -> impl Iterator<Item = &Emitter> {
        self.start
            .iter()
            .chain(&self.checkpoints)
            .flat_map(|s| s.main.iter().chain(&s.small))
    }

    /// Flame emitters burning now.
    pub fn burning(&self) -> usize {
        self.emitters().filter(|e| e.is_on()).count()
    }
}
