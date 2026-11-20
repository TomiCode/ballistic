//! How the extra points look, like the original's *TT Extra* and
//! `P_Extra_Point` scripts (research findings 22):
//!
//! - While the ball is within 80 (and its sector is active), *TT Extra* is on:
//!   the centre ball, the six small balls circling it with their trails, and,
//!   over a floor, the floor glow.
//! - Taking it hides the centre ball and the glow and bursts sparks from the
//!   centre; the small balls fly off and then home in on the ball, trailing.
//! - Each small ball that reaches the ball disappears with a burst of sparks
//!   there (the next of the prefab's hit frames).

use anyhow::Result;
use ballistic_bundle::{self as bb, BlendFactor, ExtraPointLook, ParticleEmitter, Scene};
use ballistic_render::{Gpu, ParticleVertex, SceneRenderer};
use glam::{Mat4, Vec3};

use crate::life_look::{SpriteBatch, group_triangles, hits};
use crate::particles::{Emitter, Facing, place};
use crate::pickups::PointState;
use crate::play::Bundle;
use crate::props::append_prefab;

const PREFAB: &str = "ph/p_extra_point";

/// The centre's burst: *Off* this many frames after *On*.
const CENTRE_BURST_FRAMES: u32 = 2;
/// A hit frame's burst: *Off* this many frames after *On*.
const HIT_BURST_FRAMES: u32 = 1;

/// A sprite of the prefab: its scale (quad size), and how it's drawn.
#[derive(Clone)]
struct SpriteLook {
    size: [f32; 2],
    offset: [f32; 2],
    uv: [f32; 4],
    texture: Option<String>,
    blend: (BlendFactor, BlendFactor),
    color: [f32; 4],
}

struct Point {
    marker: usize,
    floor: Option<usize>,
    on_floor: bool,
    on: bool,
    activated: bool,
    hits: usize,
    balls: Vec<(Vec3, bool)>,
    /// Each small ball's trail, the centre's burst, the hit bursts.
    trails: Vec<Emitter>,
    centre_burst: Vec<Emitter>,
    centre_frames: u32,
    hit_bursts: Vec<Emitter>,
    hit_frames: Vec<u32>,
    at: Vec3,
}

pub struct PointLooks {
    look: ExtraPointLook,
    points: Vec<Point>,
    renderer: Option<SceneRenderer>,
    centre: Option<SpriteLook>,
    ball: Option<SpriteLook>,
}

/// The sprite `name` of the prefab.
fn sprite_look(prefab: &Scene, name: &str) -> Option<SpriteLook> {
    let o = prefab.objects.iter().find(|o| o.name == name)?;
    let s = o.sprite.as_ref()?;
    let m = Mat4::from_cols_array_2d(&o.transform);
    let mat = s.material.and_then(|i| prefab.materials.get(i as usize));
    Some(SpriteLook {
        size: [m.x_axis.truncate().length(), m.y_axis.truncate().length()],
        offset: s.offset,
        uv: s.uv,
        texture: mat.and_then(|m| m.texture.clone()),
        blend: mat.map_or(
            (BlendFactor::SrcAlpha, BlendFactor::InvSrcAlpha),
            crate::life_look::sprite_blend,
        ),
        color: mat.map_or([1.0; 4], |m| m.diffuse),
    })
}

/// A camera-facing quad of `s` at `centre`.
fn quad(s: &SpriteLook, centre: Vec3, f: Facing, out: &mut Vec<ParticleVertex>) {
    let (r, u) = (f.right * s.size[0], f.up * s.size[1]);
    let corner = |x: f32, y: f32| centre + r * (x - s.offset[0]) + u * (s.offset[1] - y);
    let [u0, v0, u1, v1] = s.uv;
    let v = |x: f32, y: f32, uv: [f32; 2]| ParticleVertex {
        position: corner(x, y).to_array(),
        uv,
        color: s.color,
    };
    let (tl, tr) = (v(0.0, 0.0, [u0, v0]), v(1.0, 0.0, [u1, v0]));
    let (br, bl) = (v(1.0, 1.0, [u1, v1]), v(0.0, 1.0, [u0, v1]));
    out.extend_from_slice(&[tl, bl, br, tl, br, tr]);
}

impl PointLooks {
    #[allow(
        clippy::too_many_lines,
        reason = "one pass: prefab parts, copies, emitters"
    )]
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        level: &Scene,
        params: &bb::ExtraPoint,
        particles: &[ParticleEmitter],
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let look = params.look.clone();
        let placed = ballistic_physics::props::placeholders(level, &params.group);
        let Ok(prefab) = bundle.scene(PREFAB) else {
            return Ok(Self {
                look,
                points: Vec::new(),
                renderer: None,
                centre: None,
                ball: None,
            });
        };
        let find = |n: &str| prefab.objects.iter().position(|o| o.name == n);
        let world = |i: usize| Mat4::from_cols_array_2d(&prefab.objects[i].transform);
        let root = find(&format!("{}_MF", params.group)).map_or(Mat4::IDENTITY, world);
        let floor_i = find(&look.floor);
        let centre = sprite_look(&prefab, &look.centre);
        let ball = look.balls.first().and_then(|n| sprite_look(&prefab, n));
        // The emitters by the entity they're on: the balls' frames (their
        // children), the centre's frame, the hit frames.
        let child_of = |parent: &str| {
            let p = find(parent)? as u32;
            prefab
                .objects
                .iter()
                .find(|o| o.parent == Some(p))
                .map(|o| o.name.clone())
        };
        let trail_objects: Vec<Option<String>> = look.balls.iter().map(|b| child_of(b)).collect();
        let centre_object = child_of(&look.centre);
        let hit_objects: Vec<String> = level_group_names(&prefab, &params.hit_frames);
        let floors = group_triangles(level, &look.floor_group);

        let mut merged = Scene {
            meshes: Vec::new(),
            materials: Vec::new(),
            objects: Vec::new(),
            groups: Vec::new(),
        };
        let mut seed = 0x2545_F491;
        let mut points = Vec::new();
        for p in placed {
            let mesh_offset = append_prefab(&mut merged, &prefab);
            let floor = floor_i.map(|i| {
                let mut o = prefab.objects[i].clone();
                o.mesh = o.mesh.map(|m| m + mesh_offset);
                o.parent = None;
                o.hidden = true;
                o.transform = (p.transform * root.inverse() * world(i)).to_cols_array_2d();
                merged.objects.push(o);
                merged.objects.len() - 1
            });
            let origin = p
                .transform
                .transform_point3(Vec3::from(look.floor_ray_origin));
            let on_floor = floors
                .iter()
                .any(|&t| hits(origin, Vec3::NEG_Y, look.floor_ray_depth, t));
            // `place` finds the prefab from a placeholder-like name.
            let name = format!("{}_00", params.group);
            let all = place(
                particles,
                &mut |k| (k == PREFAB).then(|| prefab.clone()),
                &name,
                p.transform,
                &mut seed,
            );
            let take = |all: &[Emitter], object: &str| {
                all.iter()
                    .position(|e| e.params.object == object)
                    .map(|i| Emitter::new(all[i].params.clone(), all[i].transform, 1 + i as u32))
            };
            let trails = trail_objects
                .iter()
                .filter_map(|o| take(&all, o.as_deref()?))
                .collect();
            let centre_burst = centre_object.iter().filter_map(|o| take(&all, o)).collect();
            let hit_bursts: Vec<Emitter> =
                hit_objects.iter().filter_map(|o| take(&all, o)).collect();
            points.push(Point {
                marker: p.placeholder as usize,
                floor,
                on_floor,
                on: false,
                activated: false,
                hits: 0,
                balls: Vec::new(),
                trails,
                centre_burst,
                centre_frames: 0,
                hit_frames: vec![0; hit_bursts.len()],
                hit_bursts,
                at: p.transform.transform_point3(Vec3::ZERO),
            });
        }
        let renderer = if merged.objects.is_empty() {
            None
        } else {
            Some(SceneRenderer::new(gpu, &merged, format, &mut |k| bundle.texture(k).ok())?.0)
        };
        Ok(Self {
            look,
            points,
            renderer,
            centre,
            ball,
        })
    }

    /// Every emitter (to load their textures and draw them).
    pub fn emitters(&self) -> impl Iterator<Item = &Emitter> {
        self.points
            .iter()
            .flat_map(|p| p.trails.iter().chain(&p.centre_burst).chain(&p.hit_bursts))
    }

    /// The sprites' textures (`None`: untextured).
    pub fn sprite_textures(&self) -> Vec<Option<String>> {
        self.centre
            .iter()
            .chain(&self.ball)
            .map(|s| s.texture.clone())
            .collect()
    }

    /// One frame: the pickups' state, the ball, the frame time.
    pub fn update(&mut self, states: &[PointState], ball: Vec3, real_dt: f32) {
        let real_ms = real_dt * 1000.0;
        let show = self.look.show_distance;
        for p in &mut self.points {
            let Some(s) = states.iter().find(|s| s.marker == p.marker) else {
                continue;
            };
            let near = ball.distance_squared(p.at) <= show * show;
            // After the activation TT Extra stays on until it's done.
            let on = s.active && (s.activated || near);
            if s.activated && !p.activated && on {
                for e in &mut p.centre_burst {
                    e.transform = Mat4::from_translation(p.at);
                    e.set_emitting(true);
                }
                p.centre_frames = CENTRE_BURST_FRAMES;
            }
            p.activated = s.activated;
            p.on = on;
            // New hits: the next hit frames burst there.
            let hit_now = s.balls.iter().filter(|b| b.1).count();
            for (k, b) in s
                .balls
                .iter()
                .filter(|b| b.1)
                .enumerate()
                .skip(p.hits)
                .take(hit_now.saturating_sub(p.hits))
            {
                if let Some(e) = p.hit_bursts.get_mut(k) {
                    e.transform = Mat4::from_translation(b.0);
                    e.set_emitting(true);
                    p.hit_frames[k] = HIT_BURST_FRAMES;
                }
            }
            p.hits = hit_now;
            p.balls.clone_from(&s.balls);
            for (e, &(pos, hit)) in p.trails.iter_mut().zip(&s.balls) {
                e.transform = Mat4::from_translation(pos);
                e.set_on(on && !hit);
            }
            for e in &mut p.trails {
                e.update(real_ms);
            }
            for e in &mut p.centre_burst {
                e.update(real_ms);
            }
            if p.centre_frames > 0 {
                p.centre_frames -= 1;
                if p.centre_frames == 0 {
                    for e in &mut p.centre_burst {
                        e.set_emitting(false);
                    }
                }
            }
            for (e, frames) in p.hit_bursts.iter_mut().zip(&mut p.hit_frames) {
                e.update(real_ms);
                if *frames > 0 {
                    *frames -= 1;
                    if *frames == 0 {
                        e.set_emitting(false);
                    }
                }
            }
        }
    }

    /// The sprites (centre ball, small balls) as camera-facing quads, by
    /// look: (texture, blend, triangles).
    pub fn sprites(&self, f: Facing) -> Vec<SpriteBatch> {
        let mut out = Vec::new();
        if let Some(c) = &self.centre {
            let mut v = Vec::new();
            for p in self.points.iter().filter(|p| p.on && !p.activated) {
                quad(c, p.at, f, &mut v);
            }
            out.push((c.texture.clone(), c.blend, v));
        }
        if let Some(b) = &self.ball {
            let mut v = Vec::new();
            for p in self.points.iter().filter(|p| p.on) {
                for &(pos, hit) in &p.balls {
                    if !hit {
                        quad(b, pos, f, &mut v);
                    }
                }
            }
            out.push((b.texture.clone(), b.blend, v));
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
        camera: &ballistic_render::Camera,
        aspect: f32,
        opts: &ballistic_render::RenderOptions,
    ) {
        let Some(r) = self.renderer.as_mut() else {
            return;
        };
        for p in &self.points {
            if let Some(floor) = p.floor {
                r.set_object_hidden(floor, !(p.on && !p.activated && p.on_floor));
            }
        }
        r.render(gpu, encoder, color, depth, camera, aspect, opts);
    }

    /// Extra points shown now (for headless runs).
    pub fn shown(&self) -> usize {
        self.points.iter().filter(|p| p.on).count()
    }
}

/// The names of a prefab group's members.
fn level_group_names(prefab: &Scene, group: &str) -> Vec<String> {
    let mut names: Vec<String> = prefab
        .groups
        .iter()
        .filter(|g| g.name == group)
        .flat_map(|g| g.members.iter())
        .filter_map(|&i| prefab.objects.get(i as usize).map(|o| o.name.clone()))
        .collect();
    names.sort();
    names
}
