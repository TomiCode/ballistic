//! Particle systems, like the original's (research findings 20).
//!
//! - While on, an emitter emits `emission` (± its variance, truncated)
//!   particles every `delay_ms` (each frame if 0), up to `max` alive.
//! - A particle starts at the emitter (by its shape) with a speed along the
//!   emission direction, a lifespan, a size and a colour, each ± its variance
//!   (uniform). Size and colour then change linearly to their ending values
//!   over the lifespan; colour stays within 0..1.
//! - Times are milliseconds: the real frame time, or a fixed `delta_ms`.
//! - Drawn by the system's mode:
//!   - *Fast Sprite*: one camera-facing triangle per particle, its apex up,
//!     `size` wide and tall, over the texture's top centre and bottom
//!     corners;
//!   - *Sprite*: a camera-facing square, `size` wide;
//!   - *Orientable Sprite*: a quad along the particle's last step, from its
//!     previous position to its current one, stretched by its size along it
//!     and `size` to either side across it (facing the camera);
//!   - *Line*: a line from the particle's previous position to its current
//!     one, fading from its previous colour.

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
    /// The last update's step (ms): the length of the line and orientable
    /// sprite trails.
    last_step: f32,
}

/// The camera's axes, for facing particles.
#[derive(Debug, Clone, Copy)]
pub struct Facing {
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
}

impl Facing {
    /// Facing `camera`.
    pub fn of(camera: &ballistic_render::Camera) -> Self {
        let right = camera.right();
        Self {
            right,
            up: right.cross(camera.forward()).normalize_or(Vec3::Y),
            forward: camera.forward(),
        }
    }
}

/// Camera-facing quads: texture key, blend, triangles.
pub type SpriteQuads = (
    Option<String>,
    (bb::BlendFactor, bb::BlendFactor),
    Vec<ParticleVertex>,
);

/// Where particles are drawn.
pub struct Target<'a> {
    pub gpu: &'a ballistic_render::Gpu,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub camera: &'a ballistic_render::Camera,
    pub aspect: f32,
}

/// Draws `emitters` and camera-facing `sprites` (texture key, blend,
/// triangles): one batch of triangles and one of lines per texture and
/// blend. `textures`: texture key (`None`: white) → the renderer's texture.
pub fn draw(
    renderer: &mut ballistic_render::ParticleRenderer,
    textures: &std::collections::HashMap<Option<String>, usize>,
    emitters: &[&Emitter],
    sprites: Vec<SpriteQuads>,
    t: Target<'_>,
) {
    type Group = (
        usize,
        (bb::BlendFactor, bb::BlendFactor),
        Vec<ParticleVertex>,
        Vec<ParticleVertex>,
    );
    let Target {
        gpu,
        encoder,
        color,
        depth,
        camera,
        aspect,
    } = t;
    let facing = Facing::of(camera);
    let mut groups: Vec<Group> = Vec::new();
    for e in emitters {
        let Some(&texture) = textures.get(&e.params.texture) else {
            continue;
        };
        let blend = e.params.blend;
        let i = groups
            .iter()
            .position(|g| g.0 == texture && g.1 == blend)
            .unwrap_or_else(|| {
                groups.push((texture, blend, Vec::new(), Vec::new()));
                groups.len() - 1
            });
        let g = &mut groups[i];
        e.vertices(facing, &mut g.2, &mut g.3);
    }
    for (key, blend, quads) in sprites {
        if let Some(&texture) = textures.get(&key) {
            groups.push((texture, blend, quads, Vec::new()));
        }
    }
    let batches: Vec<ballistic_render::ParticleBatch<'_>> = groups
        .iter()
        .flat_map(|(texture, blend, triangles, lines)| {
            [
                ballistic_render::ParticleBatch {
                    texture: *texture,
                    blend: *blend,
                    lines: false,
                    vertices: triangles,
                },
                ballistic_render::ParticleBatch {
                    texture: *texture,
                    blend: *blend,
                    lines: true,
                    vertices: lines,
                },
            ]
        })
        .collect();
    renderer.render(gpu, encoder, color, depth, camera, aspect, &batches);
}

pub struct Emitter {
    pub params: bb::ParticleEmitter,
    /// The entity's world transform (bundle space).
    pub transform: Mat4,
    on: bool,
    particles: Vec<Particle>,
    clock_ms: f32,
    rng: u32,
    /// A time-dependent system's place at its last emission.
    last_at: Option<Vec3>,
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
            last_at: None,
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn count(&self) -> usize {
        self.particles.len()
    }

    /// Activating or deactivating its script: off clears the particles.
    pub fn set_on(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        self.set_emitting(on);
        if !on {
            self.particles.clear();
        }
    }

    /// The *On* and *Off* inputs: off stops emitting, the particles live
    /// on. On emits at once (the emission clock starts full).
    pub fn set_emitting(&mut self, on: bool) {
        if on && !self.on {
            self.clock_ms = self.params.delay_ms;
        }
        self.on = on;
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
            p.last_step = step;
            true
        });
        if !self.on {
            self.last_at = None;
            return;
        }
        if self.params.time_dependent {
            self.emit_along(dt);
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

    /// `TT_TimedependentPointParticlesystem` (`TT_ParticleSystems_RT.dll`
    /// `0x25083540`): the frame times add up; once they × the rate (per
    /// second) reach 0.9, that many particles come at once, spaced from
    /// where the entity is back to where it was at the last emission.
    fn emit_along(&mut self, dt: f32) {
        let at = self.transform.transform_point3(Vec3::ZERO);
        let Some(last) = self.last_at else {
            // The first frame only notes the place.
            self.last_at = Some(at);
            return;
        };
        let count = (self.clock_ms + dt) * 0.001 * self.params.delay_ms;
        if count <= 0.9 {
            self.clock_ms += dt;
            return;
        }
        self.clock_ms = 0.0;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a few particles"
        )]
        let n = (count as usize).max(1);
        #[expect(clippy::cast_precision_loss, reason = "a few particles")]
        let step = (last - at) / n as f32;
        for k in 0..n {
            if self.particles.len() >= self.params.max as usize {
                break;
            }
            self.spawn();
            if let Some(p) = self.particles.last_mut() {
                #[expect(clippy::cast_precision_loss, reason = "a few particles")]
                let offset = step * k as f32;
                p.position += offset;
            }
        }
        self.last_at = Some(at);
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
            last_step: 0.01,
        });
    }

    /// Appends the system's triangles (or, for *Line*, line vertices).
    pub fn vertices(
        &self,
        f: Facing,
        triangles: &mut Vec<ParticleVertex>,
        lines: &mut Vec<ParticleVertex>,
    ) {
        let v = |pos: Vec3, uv: [f32; 2], color: [f32; 4]| ParticleVertex {
            position: pos.to_array(),
            uv,
            color,
        };
        for p in &self.particles {
            let size = p.size.max(0.0);
            let mut tail_color = p.color;
            for (c, d) in tail_color.iter_mut().zip(p.color_step) {
                *c = (*c - d * p.last_step).clamp(0.0, 1.0);
            }
            match self.params.render {
                bb::ParticleRender::FastSprite => {
                    let (r, u) = (f.right * size * 0.5, f.up * size * 0.5);
                    triangles.push(v(p.position + u, [0.5, 0.0], p.color));
                    triangles.push(v(p.position - r - u, [0.0, 1.0], p.color));
                    triangles.push(v(p.position + r - u, [1.0, 1.0], p.color));
                }
                bb::ParticleRender::Sprite => {
                    let (r, u) = (f.right * size * 0.5, f.up * size * 0.5);
                    let tl = v(p.position - r + u, [0.0, 0.0], p.color);
                    let tr = v(p.position + r + u, [1.0, 0.0], p.color);
                    let br = v(p.position + r - u, [1.0, 1.0], p.color);
                    let bl = v(p.position - r - u, [0.0, 1.0], p.color);
                    triangles.extend_from_slice(&[tl, bl, br, tl, br, tr]);
                }
                bb::ParticleRender::OrientableSprite => {
                    let tail = p.position - p.velocity * p.last_step;
                    let d = p.position - tail;
                    let across = d.normalize_or_zero().cross(f.forward).normalize_or(f.right);
                    let prev = (p.size - p.size_step * p.last_step).max(0.0);
                    let h0 = v(p.position + d * size + across * size, [0.0, 0.0], p.color);
                    let h1 = v(p.position + d * size - across * size, [1.0, 0.0], p.color);
                    let t1 = v(tail - d * prev - across * prev, [1.0, 1.0], tail_color);
                    let t0 = v(tail - d * prev + across * prev, [0.0, 1.0], tail_color);
                    triangles.extend_from_slice(&[h0, t0, t1, h0, t1, h1]);
                }
                bb::ParticleRender::Line => {
                    let tail = p.position - p.velocity * p.last_step;
                    lines.push(v(tail, [0.0, 0.0], tail_color));
                    lines.push(v(p.position, [1.0, 1.0], p.color));
                }
                bb::ParticleRender::Other(_) => {}
            }
        }
    }
}

/// The emitters of prefab `ph/<base>` placed at `world`, where `base` is
/// `name` without its `_NN` suffix (a level placeholder's name): placeholder
/// x the prefab root's inverse x the entity.
pub fn place(
    particles: &[bb::ParticleEmitter],
    prefab: &mut impl FnMut(&str) -> Option<bb::Scene>,
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
            time_dependent: false,
        }
    }

    /// The flames' entities point their Z axis up: in the bundle that's a
    /// rotation taking −Z (Virtools +Z) to +Y.
    fn upright() -> Mat4 {
        Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
    }

    #[test]
    fn a_time_dependent_system_leaves_a_trail_along_its_path() {
        // The extra point's trail: a rate of 90 a second, 1 s each, spread
        // between frames. The count is truncated and the rest dropped, so
        // at 60 frames a second that's one a frame.
        let mut params = flame();
        params.time_dependent = true;
        params.delay_ms = 90.0;
        params.max = 100;
        params.lifespan_ms = bb::Varied {
            value: 1000.0,
            variance: 0.0,
        };
        params.speed = bb::Varied {
            value: 0.0,
            variance: 0.0,
        };
        let mut e = Emitter::new(params, Mat4::IDENTITY, 1);
        e.set_on(true);
        // Moving 12 a second along +X for 1 s.
        for frame in 0..=60 {
            #[expect(clippy::cast_precision_loss, reason = "a few frames")]
            let x = frame as f32 * 0.2;
            e.transform = Mat4::from_translation(Vec3::new(x, 0.0, 0.0));
            e.update(1000.0 / 60.0);
        }
        let n = e.count();
        assert!((55..=70).contains(&n), "{n} particles");
        // Spread along the path, not bunched at the frames' places.
        let mut xs: Vec<f32> = e.particles.iter().map(|p| p.position.x).collect();
        xs.sort_by(f32::total_cmp);
        let widest = xs.windows(2).map(|w| w[1] - w[0]).fold(0.0, f32::max);
        assert!(widest < 0.25, "a gap of {widest}");
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

    fn facing() -> Facing {
        Facing {
            right: Vec3::X,
            up: Vec3::Y,
            forward: Vec3::NEG_Z,
        }
    }

    #[test]
    fn fast_sprites_are_upright_triangles() {
        let mut e = Emitter::new(flame(), Mat4::IDENTITY, 1);
        e.set_on(true);
        e.update(20.0);
        let (mut v, mut lines) = (Vec::new(), Vec::new());
        e.vertices(facing(), &mut v, &mut lines);
        assert_eq!(lines.len(), 0);
        assert_eq!(v.len() % 3, 0);
        let apex = Vec3::from(v[0].position);
        let left = Vec3::from(v[1].position);
        let right = Vec3::from(v[2].position);
        assert!(apex.y > left.y && (left.y - right.y).abs() < 1e-5);
        assert!((right.x - left.x - e.particles[0].size).abs() < 1e-4);
    }

    #[test]
    fn lines_trail_the_last_step() {
        let mut params = flame();
        params.render = bb::ParticleRender::Line;
        params.yaw_variance = 0.0;
        params.pitch_variance = 0.0;
        let mut e = Emitter::new(params, upright(), 3);
        e.set_on(true);
        e.update(20.0);
        e.update(20.0);
        let (mut tris, mut lines) = (Vec::new(), Vec::new());
        e.vertices(facing(), &mut tris, &mut lines);
        assert_eq!(tris.len(), 0);
        assert_ne!(lines.len(), 0);
        // The first particle moved 20 ms x 0.01 upwards in its last step.
        let (a, b) = (Vec3::from(lines[0].position), Vec3::from(lines[1].position));
        assert!((b.y - a.y - 0.2).abs() < 0.02, "{a} {b}");
    }

    #[test]
    fn orientable_sprites_follow_the_motion() {
        let mut params = flame();
        params.render = bb::ParticleRender::OrientableSprite;
        let mut e = Emitter::new(params, upright(), 5);
        e.set_on(true);
        e.update(20.0);
        e.update(20.0);
        let (mut tris, mut lines) = (Vec::new(), Vec::new());
        e.vertices(facing(), &mut tris, &mut lines);
        assert_eq!(tris.len() % 6, 0);
        // Head above tail: along the (upward) motion.
        let head = Vec3::from(tris[0].position);
        let tail = Vec3::from(tris[1].position);
        assert!(head.y > tail.y);
    }
}
