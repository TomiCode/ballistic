//! The scene behind the main menu, like the original's `MenuLevel_Init`
//! (research findings 28): the camera goes round a closed curve once every
//! 44 s looking at the middle, under level 5's sky, with fog; the four
//! flames burn; now and then (1 to 10 s after the last) the ambient sound
//! plays.
//!
//! The curve is taken as a closed Catmull-Rom spline through its points,
//! each segment an equal share of the progression (the original's
//! `CKCurve` evaluation isn't read).

use std::collections::HashMap;

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Camera, Gpu, ParticleRenderer, RenderOptions, SkyRenderer, upload_ktx2};
use glam::{Mat4, Vec3};

use crate::particles::Emitter;
use crate::play::{Bundle, Lighting};

pub struct MenuLevelView {
    data: bb::MenuLevel,
    target: Vec3,
    ms: f32,
    sky: Option<SkyRenderer>,
    lighting: Lighting,
    emitters: Vec<Emitter>,
    particles: ParticleRenderer,
    particle_textures: HashMap<Option<String>, usize>,
    /// The ambient sound: ms until it plays, or while it plays.
    atmo_ms: f32,
    atmo_length_ms: f32,
    rng: u64,
}

impl MenuLevelView {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        scene: &bb::Scene,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let physics = bundle.physics()?;
        let data = physics.menu_level.clone();
        let (sky, mut lighting) =
            crate::play::environment(gpu, bundle, &physics, &data.sky_level, format)?;
        let [r, g, b, _] = data.light_color;
        lighting.light_color = Vec3::new(r, g, b);
        let world = |name: &str| {
            scene
                .objects
                .iter()
                .find(|o| o.name == name)
                .map(|o| Mat4::from_cols_array_2d(&o.transform))
        };
        let target = world(&data.target).map_or(Vec3::ZERO, |m| m.transform_point3(Vec3::ZERO));
        // The flames: the scene's particle systems, where their entities are.
        let mut emitters = Vec::new();
        for (seed, p) in physics
            .particles
            .iter()
            .filter(|p| p.scene == data.scene)
            .enumerate()
        {
            let Some(m) = world(&p.object) else { continue };
            #[expect(clippy::cast_possible_truncation, reason = "a seed")]
            let mut e = Emitter::new(p.clone(), m, 0x5EED_0000 ^ seed as u32);
            e.set_on(true);
            emitters.push(e);
        }
        let mut particles = ParticleRenderer::new(gpu, format);
        let mut particle_textures = HashMap::new();
        for key in emitters.iter().map(|e| e.params.texture.clone()) {
            if particle_textures.contains_key(&key) {
                continue;
            }
            let index = match &key {
                Some(k) => particles.add_texture(gpu, &upload_ktx2(gpu, k, &bundle.texture(k)?)?),
                None => particles.add_white(gpu),
            };
            particle_textures.insert(key, index);
        }
        let atmo_length_ms = bundle
            .manifest()
            .sounds
            .iter()
            .find(|s| s.key == data.atmo)
            .map_or(0.0, |s| {
                #[expect(clippy::cast_precision_loss, reason = "a sound's length")]
                let ms = s.frames as f32 * 1000.0 / s.sample_rate.max(1) as f32;
                ms
            });
        let mut v = Self {
            target,
            ms: 0.0,
            sky,
            lighting,
            emitters,
            particles,
            particle_textures,
            atmo_ms: 0.0,
            atmo_length_ms,
            rng: 0x2545_F491_4F6C_DD1D,
            data,
        };
        v.atmo_ms = v.atmo_wait();
        Ok(v)
    }

    fn atmo_wait(&mut self) -> f32 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        #[expect(clippy::cast_precision_loss, reason = "24 random bits")]
        let unit =
            (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32;
        let [lo, hi] = self.data.atmo_gap_ms;
        lo + unit * (hi - lo)
    }

    /// One frame; returns sounds to play.
    pub fn update(&mut self, real_dt: f32) -> Vec<String> {
        let ms = real_dt * 1000.0;
        self.ms += ms;
        for e in &mut self.emitters {
            e.update(ms);
        }
        self.atmo_ms -= ms;
        if self.atmo_ms > 0.0 {
            return Vec::new();
        }
        // It plays; the next wait starts when it ends.
        self.atmo_ms = self.atmo_length_ms + self.atmo_wait();
        vec![self.data.atmo.clone()]
    }

    /// The camera on its round.
    pub fn camera(&self, base: Camera) -> Camera {
        let p = &self.data.progression;
        let period = p.duration_ms.max(1.0);
        let t = p.value(self.ms % period);
        let mut cam = Camera {
            position: along(&self.data.path, self.data.closed, t),
            ..base
        };
        cam.look_at(self.target);
        cam
    }

    /// The level's light and the menu's fog, over the sky.
    pub fn scene_options(&self, opts: &RenderOptions) -> RenderOptions {
        let l = &self.lighting;
        let [r, g, b, _] = self.data.fog_color;
        RenderOptions {
            light_dir: l.towards_light,
            light_color: l.light_color,
            ambient: l.ambient,
            clear_color: l.background,
            keep_color: self.sky.is_some(),
            fog: Some((self.data.fog[0], self.data.fog[1], Vec3::new(r, g, b))),
            ..*opts
        }
    }

    pub fn render_sky(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
    ) {
        if let Some(sky) = &self.sky {
            sky.render(
                gpu,
                encoder,
                color,
                camera,
                aspect,
                self.lighting.background,
            );
        }
    }

    pub fn render_particles(&mut self, t: crate::particles::Target<'_>) {
        let emitters: Vec<&Emitter> = self.emitters.iter().collect();
        crate::particles::draw(
            &mut self.particles,
            &self.particle_textures,
            &emitters,
            Vec::new(),
            t,
        );
    }
}

/// The point at `t` (0–1) along a Catmull-Rom spline through `points`.
fn along(points: &[[f32; 3]], closed: bool, t: f32) -> Vec3 {
    let n = points.len();
    if n == 0 {
        return Vec3::ZERO;
    }
    let segments = if closed { n } else { n - 1 }.max(1);
    #[expect(clippy::cast_precision_loss, reason = "a few points")]
    let s = t.clamp(0.0, 1.0) * segments as f32;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a segment"
    )]
    let i = (s.floor() as usize).min(segments - 1);
    #[expect(clippy::cast_precision_loss, reason = "a few points")]
    let u = s - i as f32;
    let at = |k: isize| {
        #[expect(
            clippy::cast_possible_wrap,
            clippy::cast_sign_loss,
            reason = "a few points"
        )]
        let k = if closed {
            k.rem_euclid(n as isize) as usize
        } else {
            k.clamp(0, n as isize - 1) as usize
        };
        Vec3::from(points[k])
    };
    #[expect(clippy::cast_possible_wrap, reason = "a few points")]
    let i = i as isize;
    let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let (u2, u3) = (u * u, u * u * u);
    0.5 * (2.0 * p1
        + (p2 - p0) * u
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u2
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * u3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_curve_goes_round_through_its_points() {
        let square = [
            [95.0, 40.0, 0.0],
            [0.0, 40.0, 95.0],
            [-95.0, 40.0, 0.0],
            [0.0, 40.0, -95.0],
        ];
        assert!(along(&square, true, 0.0).distance(Vec3::from(square[0])) < 1e-3);
        assert!(along(&square, true, 0.25).distance(Vec3::from(square[1])) < 1e-3);
        assert!(along(&square, true, 1.0).distance(Vec3::from(square[0])) < 1e-3);
        // Between points it bulges outwards, like a circle.
        let mid = along(&square, true, 0.125);
        assert!(Vec3::new(mid.x, 0.0, mid.z).length() > 80.0);
        assert!((mid.y - 40.0).abs() < 1e-3);
    }
}
