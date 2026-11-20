//! Particle systems, like the original's (research findings 20).
//!
//! - While on, an emitter emits `emission` (± its variance, truncated)
//!   particles every `delay_ms` (each frame if 0), up to `max` alive.
//! - A particle starts at the emitter (by its shape) with a speed along the
//!   emission direction, a lifespan, a size and a colour, each ± its variance
//!   (uniform). Size and colour then change linearly to their ending values
//!   over the lifespan; colour stays within 0..1.
//! - Times are milliseconds: the real frame time, or a fixed `delta_ms`.
//! - Drawn as *Fast Sprite*: one camera-facing triangle per particle, its
//!   apex up, `size` wide and tall, over the texture's top centre and bottom
//!   corners.

use ballistic_bundle as bb;
use ballistic_render::ParticleVertex;
use glam::{Mat4, Vec3};

struct Particle {
    position: Vec3,
    /// Units per millisecond.
    velocity: Vec3,
    life_ms: f32,
    color: [f32; 4],
    color_step: [f32; 4],
    size: f32,
    size_step: f32,
}

pub struct Emitter {
    pub params: bb::ParticleEmitter,
    /// The entity's world transform (bundle space).
    pub transform: Mat4,
    on: bool,
    particles: Vec<Particle>,
    clock_ms: f32,
    rng: u32,
}

impl Emitter {
    pub fn new(params: bb::ParticleEmitter, transform: Mat4, seed: u32) -> Self {
        Self {
            params,
            transform,
            on: false,
            particles: Vec::new(),
            clock_ms: 0.0,
            rng: seed.max(1),
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.particles.len()
    }

    /// Activating or deactivating its script: off clears the particles.
    pub fn set_on(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        self.on = on;
        self.clock_ms = 0.0;
        if !on {
            self.particles.clear();
        }
    }

    /// Uniform in −1..1.
    fn unit(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        #[expect(clippy::cast_precision_loss, reason = "a random fraction")]
        let f = (x >> 8) as f32 / (1u32 << 24) as f32;
        f * 2.0 - 1.0
    }

    fn varied(&mut self, v: bb::Varied) -> f32 {
        if v.variance == 0.0 {
            v.value
        } else {
            v.value + self.unit() * v.variance
        }
    }

    /// One frame of `real_ms` milliseconds.
    pub fn update(&mut self, real_ms: f32) {
        let dt = if self.params.real_time {
            real_ms
        } else {
            self.params.delta_ms
        };
        self.particles.retain_mut(|p| {
            if p.life_ms <= 0.0 {
                return false;
            }
            let step = dt.min(p.life_ms);
            p.position += p.velocity * step;
            for (c, d) in p.color.iter_mut().zip(p.color_step) {
                *c = (*c + d * step).clamp(0.0, 1.0);
            }
            p.size += p.size_step * step;
            p.life_ms -= dt;
            true
        });
        if !self.on {
            return;
        }
        if self.params.delay_ms <= 0.0 {
            self.emit();
            return;
        }
        self.clock_ms += dt;
        let mut n = 0;
        while self.clock_ms >= self.params.delay_ms && n < 64 {
            self.clock_ms -= self.params.delay_ms;
            self.emit();
            n += 1;
        }
    }

    fn emit(&mut self) {
        let e = self.params.emission;
        let count = if e.variance == 0.0 {
            e.value
        } else {
            e.value + (self.unit() * e.variance).trunc()
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a small count"
        )]
        let count = count.max(0.0) as usize;
        for _ in 0..count {
            if self.particles.len() >= self.params.max as usize {
                break;
            }
            self.spawn();
        }
    }

    /// The emission direction in the entity's frame (Virtools: along +Z,
    /// spread by yaw and pitch), and the start point there.
    fn local_start(&mut self) -> (Vec3, Vec3) {
        let p = &self.params;
        let (yaw_var, pitch_var, shape) = (p.yaw_variance, p.pitch_variance, p.shape);
        let yaw = self.unit() * yaw_var;
        let pitch = self.unit() * pitch_var;
        let dir = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        match shape {
            bb::EmitterShape::Point => (Vec3::ZERO, dir),
            bb::EmitterShape::Planar => (Vec3::new(self.unit(), self.unit(), 0.0), dir),
            bb::EmitterShape::Spherical => {
                let v = Vec3::new(self.unit(), self.unit(), self.unit()).normalize_or(Vec3::Y);
                (v, v)
            }
        }
    }

    fn spawn(&mut self) {
        let (local_pos, local_dir) = self.local_start();
        // The bundle mirrors Virtools' Z: a local vector (x, y, z) is
        // (x, y, −z) in the bundle object's frame.
        let flip = |v: Vec3| Vec3::new(v.x, v.y, -v.z);
        let position = self.transform.transform_point3(flip(local_pos));
        let dir = self
            .transform
            .transform_vector3(flip(local_dir))
            .normalize_or(Vec3::Y);
        let speed = self.varied(self.params.speed);
        let life_ms = self.varied(self.params.lifespan_ms).max(1.0);
        let mut color = self.params.color;
        for i in 0..4 {
            let v = self.params.color_variance[i];
            if v != 0.0 {
                color[i] += self.unit() * v;
            }
        }
        let color_step = match self.params.end_color {
            Some(end) => {
                let mut s = [0.0; 4];
                for i in 0..4 {
                    let v = self.params.end_color_variance[i];
                    let target = end[i] + if v == 0.0 { 0.0 } else { self.unit() * v };
                    s[i] = (target - color[i]) / life_ms;
                }
                s
            }
            None => [0.0; 4],
        };
        let size = self.varied(self.params.size);
        let size_step = match self.params.end_size {
            Some(end) => (self.varied(end) - size) / life_ms,
            None => 0.0,
        };
        self.particles.push(Particle {
            position,
            velocity: dir * speed,
            life_ms,
            color: color.map(|c| c.clamp(0.0, 1.0)),
            color_step,
            size,
            size_step,
        });
    }

    /// Appends the triangles, facing the camera (`right`, `up`: its axes).
    pub fn vertices(&self, right: Vec3, up: Vec3, out: &mut Vec<ParticleVertex>) {
        if !matches!(self.params.render, bb::ParticleRender::FastSprite) {
            return;
        }
        for p in &self.particles {
            let half = p.size.max(0.0) * 0.5;
            let (r, u) = (right * half, up * half);
            let v = |pos: Vec3, uv: [f32; 2]| ParticleVertex {
                position: pos.to_array(),
                uv,
                color: p.color,
            };
            out.push(v(p.position + u, [0.5, 0.0]));
            out.push(v(p.position - r - u, [0.0, 1.0]));
            out.push(v(p.position + r - u, [1.0, 1.0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flame() -> bb::ParticleEmitter {
        bb::ParticleEmitter {
            scene: "ph/pc_twoflames".into(),
            object: "Flame".into(),
            shape: bb::EmitterShape::Point,
            render: bb::ParticleRender::FastSprite,
            texture: None,
            blend: (bb::BlendFactor::SrcAlpha, bb::BlendFactor::One),
            delay_ms: 20.0,
            yaw_variance: 0.05,
            pitch_variance: 0.05,
            speed: bb::Varied {
                value: 0.01,
                variance: 0.0,
            },
            lifespan_ms: bb::Varied {
                value: 1000.0,
                variance: 0.0,
            },
            max: 50,
            emission: bb::Varied {
                value: 1.0,
                variance: 1.0,
            },
            size: bb::Varied {
                value: 3.0,
                variance: 0.0,
            },
            end_size: Some(bb::Varied {
                value: 0.0,
                variance: 0.0,
            }),
            color: [1.0, 0.5, 0.0, 1.0],
            color_variance: [0.0; 4],
            end_color: Some([0.0; 4]),
            end_color_variance: [0.0; 4],
            frames: 1,
            real_time: true,
            delta_ms: 20.0,
        }
    }

    /// The flames' entities point their Z axis up: in the bundle that's a
    /// rotation taking −Z (Virtools +Z) to +Y.
    fn upright() -> Mat4 {
        Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
    }

    #[test]
    fn a_flame_rises_and_fades() {
        let mut e = Emitter::new(flame(), upright(), 7);
        e.set_on(true);
        for _ in 0..30 {
            e.update(1000.0 / 60.0);
        }
        // One per 20 ms over 0.5 s.
        assert!((20..=30).contains(&e.count()), "{}", e.count());
        let oldest = &e.particles[0];
        assert!(oldest.position.y > 4.0, "{:?}", oldest.position);
        assert!(oldest.position.x.abs() < 0.5 && oldest.position.z.abs() < 0.5);
        assert!(oldest.size < 2.0 && oldest.color[3] < 0.6);
        // A second later the first ones are gone and the count holds at max.
        for _ in 0..120 {
            e.update(1000.0 / 60.0);
        }
        assert!(e.count() <= 50);
        assert!(e.particles.iter().all(|p| p.life_ms > 0.0));
    }

    #[test]
    fn off_clears_and_stops() {
        let mut e = Emitter::new(flame(), upright(), 7);
        e.set_on(true);
        e.update(100.0);
        assert!(e.count() > 0);
        e.set_on(false);
        e.update(100.0);
        assert_eq!(e.count(), 0);
    }

    #[test]
    fn fast_sprites_are_upright_triangles() {
        let mut e = Emitter::new(flame(), Mat4::IDENTITY, 1);
        e.set_on(true);
        e.update(20.0);
        let mut v = Vec::new();
        e.vertices(Vec3::X, Vec3::Y, &mut v);
        assert_eq!(v.len() % 3, 0);
        let apex = Vec3::from(v[0].position);
        let left = Vec3::from(v[1].position);
        let right = Vec3::from(v[2].position);
        assert!(apex.y > left.y && (left.y - right.y).abs() < 1e-5);
        assert!((right.x - left.x - e.particles[0].size).abs() < 1e-4);
    }
}
