//! The ball's arrival and the screen's fade (research findings 30):
//! - **Lightning** (`Ball_LightningSphere`, on each `Ball_Lightning`): at
//!   the ball's place a sphere grows from nothing over 1.5 s, turning once a
//!   second about its Y axis and changing its texture every frame, for 3 s.
//! - **Smoke puff** (`BallParticle_Frame script`): 2.5 s into the lightning
//!   (just before the ball comes), and when a transformer bursts the ball,
//!   the balls' particle system runs for 6 frames at the ball.
//! - **Fade** (*Überblendung*): when a lost ball is taken back (lives
//!   left), the screen goes white over 0.8 s and clears again by 2 s.

use anyhow::Result;
use ballistic_bundle as bb;
use ballistic_render::{Gpu, Overlay, Quad, SceneRenderer, upload_ktx2};
use glam::{Mat4, Quat, Vec3};

use crate::particles::Emitter;
use crate::play::Bundle;

pub struct Arrival {
    data: bb::BallArrival,
    /// The sphere: its object in the balls scene, its transform there, its
    /// materials, and each material's variants (one per texture).
    sphere: Option<usize>,
    sphere_local: Mat4,
    materials: Vec<(usize, Vec<usize>)>,
    /// Since the lightning started.
    ms: Option<f32>,
    frame: usize,
    angle: f32,
    puff: Option<Emitter>,
    puff_local: Mat4,
    /// Frames until the puff switches off.
    puff_frames: Option<u32>,
    puffed: bool,
}

impl Arrival {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        renderer: &mut SceneRenderer,
        scene: &bb::Scene,
        physics: &bb::PhysicsData,
    ) -> Result<Self> {
        let data = physics.flow.arrival.clone();
        let object = scene
            .objects
            .iter()
            .position(|o| o.name == data.sphere && o.mesh.is_some());
        let mut materials = Vec::new();
        let mut sphere_local = Mat4::IDENTITY;
        if let Some(i) = object {
            let o = &scene.objects[i];
            sphere_local = Mat4::from_cols_array_2d(&o.transform);
            let mesh = o.mesh.and_then(|m| scene.meshes.get(m as usize));
            let textures = data
                .textures
                .iter()
                .map(|k| -> Result<_> { Ok(upload_ktx2(gpu, k, &bundle.texture(k)?)?) })
                .collect::<Result<Vec<_>>>()?;
            for m in mesh
                .iter()
                .flat_map(|m| &m.submeshes)
                .filter_map(|s| s.material.map(|m| m as usize))
            {
                let variants = textures
                    .iter()
                    .filter_map(|t| renderer.add_material_texture(gpu, m, t))
                    .collect();
                materials.push((m, variants));
            }
            renderer.set_object_hidden(i, true);
        }
        let puff_params = physics
            .particles
            .iter()
            .find(|p| p.scene == "balls" && p.object == data.puff_object)
            .cloned();
        let puff_local = scene
            .objects
            .iter()
            .find(|o| o.name == data.puff_object)
            .map_or(Mat4::IDENTITY, |o| Mat4::from_cols_array_2d(&o.transform));
        Ok(Self {
            puff: puff_params.map(|p| Emitter::new(p, puff_local, 0xBA11)),
            data,
            sphere: object,
            sphere_local,
            materials,
            ms: None,
            frame: 0,
            angle: 0.0,
            puff_local,
            puff_frames: None,
            puffed: false,
        })
    }

    /// The puff's particle system (for the particle renderer's textures).
    pub fn puff_params(&self) -> Option<&bb::ParticleEmitter> {
        self.puff.as_ref().map(|e| &e.params)
    }

    /// `Ball_Lightning`.
    pub fn start(&mut self) {
        self.ms = Some(0.0);
        self.frame = 0;
        self.angle = 0.0;
        self.puffed = false;
    }

    /// The smoke puff, now, at `at`.
    pub fn puff(&mut self, at: Vec3) {
        if let Some(e) = self.puff.as_mut() {
            e.transform = Mat4::from_translation(at) * self.puff_local;
            e.set_on(true);
            self.puff_frames = Some(self.data.puff_frames);
        }
    }

    pub fn update(&mut self, real_dt: f32, ball: Vec3) {
        let ms = real_dt * 1000.0;
        if let Some(t) = self.ms {
            let t = t + ms;
            if !self.puffed && t >= self.data.puff_ms {
                self.puffed = true;
                self.puff(ball);
            }
            self.ms = (t < self.data.shown_ms.max(self.data.puff_ms)).then_some(t);
            self.frame += 1;
            self.angle += self.data.spin * real_dt;
        }
        if let Some(n) = self.puff_frames {
            if n == 0 {
                self.puff_frames = None;
                // *Off*: the emission stops; the particles live on.
                if let Some(e) = self.puff.as_mut() {
                    e.set_emitting(false);
                }
            } else {
                self.puff_frames = Some(n - 1);
            }
        }
        if let Some(e) = self.puff.as_mut() {
            e.update(ms);
        }
    }

    /// The sphere's place, size and texture in the balls' renderer.
    pub fn sync(&self, gpu: &Gpu, renderer: &mut SceneRenderer, ball: Vec3) {
        let Some(object) = self.sphere else { return };
        let shown = self.ms.is_some_and(|t| t < self.data.shown_ms);
        renderer.set_object_hidden(object, !shown);
        let Some(t) = self.ms.filter(|_| shown) else {
            return;
        };
        let scale = self.data.grow.value(t).clamp(0.0, 1.0).max(1e-4);
        let model = Mat4::from_translation(ball)
            * Mat4::from_quat(Quat::from_rotation_y(self.angle))
            * Mat4::from_scale(Vec3::splat(scale))
            * self.sphere_local;
        renderer.set_object_transform(gpu, object, model);
        for (m, variants) in &self.materials {
            if !variants.is_empty() {
                renderer.set_material_texture(*m, Some(variants[self.frame % variants.len()]));
            }
        }
    }

    pub fn emitters(&self) -> impl Iterator<Item = &Emitter> {
        self.puff.iter()
    }

    /// The smoke puff's particles (for headless runs; `None`: no system).
    pub fn puff_count(&self) -> Option<usize> {
        self.puff.as_ref().map(Emitter::count)
    }

    /// The lightning is on (for headless runs).
    pub fn shown(&self) -> bool {
        self.ms.is_some_and(|t| t < self.data.shown_ms)
    }
}

/// *Überblendung*: the screen covered by a colour along a progression.
pub struct ScreenFade {
    data: bb::ScreenFade,
    ms: Option<f32>,
    /// `None` in tests.
    gfx: Option<(Overlay, usize)>,
}

impl ScreenFade {
    pub fn new(gpu: &Gpu, data: &bb::ScreenFade, format: wgpu::TextureFormat) -> Self {
        let mut overlay = Overlay::new(gpu, format);
        let white = overlay.add_texture(gpu, &ballistic_render::white(gpu));
        Self {
            data: data.clone(),
            ms: None,
            gfx: Some((overlay, white)),
        }
    }

    pub fn start(&mut self) {
        self.ms = Some(0.0);
    }

    pub fn update(&mut self, real_dt: f32) {
        if let Some(t) = self.ms {
            let t = t + real_dt * 1000.0;
            self.ms = (t < self.data.progression.duration_ms).then_some(t);
        }
    }

    /// The colour over the screen now, if any.
    pub fn color(&self) -> Option<[f32; 4]> {
        let t = self.ms?;
        let v = self.data.progression.value(t).clamp(0.0, 1.0);
        let (a, b) = (self.data.from, self.data.to);
        Some(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * v))
    }

    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        let Some(c) = self.color() else { return };
        let Some((overlay, white)) = self.gfx.as_mut() else {
            return;
        };
        #[expect(clippy::cast_precision_loss, reason = "window sizes")]
        let rect = [0.0, 0.0, size.0 as f32, size.1 as f32];
        let quad = Quad {
            texture: *white,
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: c,
        };
        overlay.render(gpu, encoder, color, size, &[quad]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_goes_white_and_clears() {
        let p = |x: f32, y: f32| bb::CurvePoint {
            position: [x, y],
            in_tangent: [1.0, 0.0],
            out_tangent: [1.0, 0.0],
        };
        let mut f = ScreenFade {
            data: bb::ScreenFade {
                from: [0.0; 4],
                to: [1.0; 4],
                progression: bb::Progression {
                    duration_ms: 2000.0,
                    curve: vec![p(0.0, 0.0), p(0.4, 1.0), p(1.0, 0.0)],
                },
            },
            ms: None,
            gfx: None,
        };
        assert_eq!(f.color(), None);
        f.start();
        for _ in 0..48 {
            f.update(1.0 / 60.0);
        }
        assert!(f.color().unwrap()[3] > 0.95);
        for _ in 0..80 {
            f.update(1.0 / 60.0);
        }
        assert_eq!(f.color(), None);
    }
}
