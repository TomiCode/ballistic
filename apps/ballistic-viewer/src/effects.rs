//! Other prefabs' particle effects (research findings 20):
//! - **Fans** (`P_Modul_18`): wind lines and smoke from `P_Modul_18_Particle`
//!   while the fan's sector is active and the ball is within 80 (X and Z),
//!   when its particle script runs (the rotor turns then too, in
//!   [`crate::modules`]).
//! - **Extra lives** (`P_Extra_Life`): taking one bursts its blob and fizz
//!   systems: switched on, and off again 5 frames later (their particles
//!   live on).

use ballistic_bundle::{ParticleEmitter, Scene};
use glam::{Mat4, Vec3};

use crate::modules::FAN_NEAR;
use crate::particles::{Emitter, place};

/// The extra life's systems: *On*, then *Off* this many frames later.
const BURST_FRAMES: u32 = 5;

struct Fan {
    target: Vec3,
    emitters: Vec<Emitter>,
}

struct Burst {
    /// The extra life's level object.
    marker: usize,
    emitters: Vec<Emitter>,
    frames_left: u32,
}

pub struct Effects {
    fans: Vec<Fan>,
    lives: Vec<Burst>,
}

/// The members of level group `group`: (object index, name, world matrix).
fn members<'a>(level: &'a Scene, group: &str) -> Vec<(usize, &'a str, Mat4)> {
    level
        .groups
        .iter()
        .filter(|g| g.name == group)
        .flat_map(|g| g.members.iter())
        .filter_map(|&i| {
            let o = level.objects.get(i as usize)?;
            Some((
                i as usize,
                o.name.as_str(),
                Mat4::from_cols_array_2d(&o.transform),
            ))
        })
        .collect()
}

impl Effects {
    pub fn new(
        level: &Scene,
        particles: &[ParticleEmitter],
        mut prefab: impl FnMut(&str) -> Option<Scene>,
    ) -> Self {
        let mut seed = 0x51ED_270B;
        let fans = members(level, "P_Modul_18")
            .into_iter()
            .filter_map(|(_, name, world)| {
                let emitters = place(particles, &mut prefab, name, world, &mut seed);
                let target = emitters.first()?.transform.transform_point3(Vec3::ZERO);
                Some(Fan { target, emitters })
            })
            .collect();
        let lives = members(level, "P_Extra_Life")
            .into_iter()
            .map(|(marker, name, world)| Burst {
                marker,
                emitters: place(particles, &mut prefab, name, world, &mut seed),
                frames_left: 0,
            })
            .collect();
        Self { fans, lives }
    }

    /// The extra life `marker` was taken.
    pub fn life_up(&mut self, marker: usize) {
        if let Some(b) = self.lives.iter_mut().find(|b| b.marker == marker) {
            for e in &mut b.emitters {
                e.set_emitting(true);
            }
            b.frames_left = BURST_FRAMES;
        }
    }

    /// One frame. `fans`: each fan's target and whether its sector is
    /// active ([`crate::modules::Modules::fans`]).
    pub fn update(&mut self, ball: Vec3, fans: &[(Vec3, bool)], real_ms: f32) {
        for f in &mut self.fans {
            let active = fans
                .iter()
                .find(|(t, _)| t.distance(f.target) < 0.5)
                .is_some_and(|&(_, a)| a);
            let d = ball - f.target;
            let on = active && d.x * d.x + d.z * d.z <= FAN_NEAR * FAN_NEAR;
            for e in &mut f.emitters {
                e.set_on(on);
                e.update(real_ms);
            }
        }
        for b in &mut self.lives {
            for e in &mut b.emitters {
                e.update(real_ms);
            }
            if b.frames_left > 0 {
                b.frames_left -= 1;
                if b.frames_left == 0 {
                    for e in &mut b.emitters {
                        e.set_emitting(false);
                    }
                }
            }
        }
    }

    pub fn emitters(&self) -> impl Iterator<Item = &Emitter> {
        self.fans
            .iter()
            .flat_map(|f| f.emitters.iter())
            .chain(self.lives.iter().flat_map(|b| b.emitters.iter()))
    }

    /// Fans whose particles run now (for headless runs).
    pub fn fans_running(&self) -> usize {
        self.fans
            .iter()
            .filter(|f| f.emitters.iter().any(Emitter::is_on))
            .count()
    }
}
