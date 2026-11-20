//! How the extra lives look, like the original's `P_Extra_Life` scripts
//! (research findings 21):
//!
//! - While the ball is within 60 of one (on X and Y), its sphere and the
//!   silver ball sprite inside it show and its animation runs (from the
//!   start each time).
//! - The sphere squashes, its scale following a curve over 2 s, again and
//!   again; its texture coordinates wave (`TT_TextureSine`).
//! - Over a floor (a short ray down from below it hits `Phys_Floors`) it also
//!   bounces, and its shadow shows, darkening as it rises.
//! - Taking it, or its sector going, hides it all.

use anyhow::Result;
use ballistic_bundle::{self as bb, BlendFactor, ExtraLifeLook, Scene, Vertex};
use ballistic_render::{Gpu, ParticleVertex, SceneRenderer};
use glam::{Mat4, Vec3};

use crate::particles::Facing;
use crate::play::Bundle;
use crate::props::append_prefab;

const PREFAB: &str = "ph/p_extra_life";

/// One life's render changes: sphere object, its matrix, shown, the shadow
/// material's emissive, the shadow object.
type Change = (usize, Mat4, bool, Option<(usize, [f32; 4])>, Option<usize>);

/// Sprite quads to draw: texture key, blend, triangles.
pub type SpriteBatch = (
    Option<String>,
    (BlendFactor, BlendFactor),
    Vec<ParticleVertex>,
);

struct Life {
    marker: usize,
    frame: Mat4,
    sphere: usize,
    shadow: Option<usize>,
    shadow_material: Option<usize>,
    mesh: Option<usize>,
    on_floor: bool,
    shown: bool,
    /// Into the animation (ms) and the texture wave (s).
    anim_ms: f32,
    wave_s: f32,
}

/// The silver ball sprite, relative to the sphere.
struct BallSprite {
    local: Mat4,
    texture: Option<String>,
    blend: (BlendFactor, BlendFactor),
    uv: [f32; 4],
    offset: [f32; 2],
    color: [f32; 4],
}

pub struct LifeLooks {
    look: ExtraLifeLook,
    lives: Vec<Life>,
    renderer: Option<SceneRenderer>,
    /// The sphere's place in the prefab's frame (rotation and position).
    sphere_local: Mat4,
    base_vertices: Vec<Vertex>,
    ball: Option<BallSprite>,
}

/// Möller-Trumbore: whether the segment from `o` along `d` (unit) up to
/// `len` hits the triangle.
pub(crate) fn hits(o: Vec3, d: Vec3, len: f32, [a, b, c]: [Vec3; 3]) -> bool {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-8 {
        return false;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = e2.dot(q) * inv;
    (0.0..=len).contains(&t)
}

/// The world triangles of the level group `group`.
pub(crate) fn group_triangles(level: &Scene, group: &str) -> Vec<[Vec3; 3]> {
    let Some(g) = level.groups.iter().find(|g| g.name == group) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for &i in &g.members {
        let Some(o) = level.objects.get(i as usize) else {
            continue;
        };
        let Some(mesh) = o.mesh.and_then(|m| level.meshes.get(m as usize)) else {
            continue;
        };
        let m = Mat4::from_cols_array_2d(&o.transform);
        let v: Vec<Vec3> = mesh
            .vertices()
            .iter()
            .map(|v| m.transform_point3(Vec3::from(v.position)))
            .collect();
        let idx: Vec<u16> = mesh
            .index_data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        for t in idx.as_chunks::<3>().0 {
            let get = |k: u16| v.get(k as usize).copied();
            if let (Some(a), Some(b), Some(c)) = (get(t[0]), get(t[1]), get(t[2])) {
                out.push([a, b, c]);
            }
        }
    }
    out
}

impl LifeLooks {
    #[allow(
        clippy::too_many_lines,
        reason = "one pass: prefab parts, copies, floor tests"
    )]
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        level: &Scene,
        params: &bb::ExtraLife,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let look = params.look.clone();
        let placed = ballistic_physics::props::placeholders(level, &params.group);
        let Ok(prefab) = bundle.scene(PREFAB) else {
            return Ok(Self {
                look,
                lives: Vec::new(),
                renderer: None,
                sphere_local: Mat4::IDENTITY,
                base_vertices: Vec::new(),
                ball: None,
            });
        };
        let find = |n: &str| prefab.objects.iter().position(|o| o.name == n);
        let world = |i: usize| Mat4::from_cols_array_2d(&prefab.objects[i].transform);
        let root = find(&format!("{}_MF", params.group)).map_or(Mat4::IDENTITY, world);
        let sphere_i = find(&look.sphere);
        let shadow_i = find(&look.shadow);
        let sphere_world = sphere_i.map_or(Mat4::IDENTITY, world);
        // Without its scale: the animation sets the scale absolutely.
        let (_, turn, offset) = (root.inverse() * sphere_world).to_scale_rotation_translation();
        let sphere_local = Mat4::from_rotation_translation(turn, offset);
        let sphere_mesh = sphere_i.and_then(|i| prefab.objects[i].mesh);
        let base_vertices = sphere_mesh
            .and_then(|m| prefab.meshes.get(m as usize))
            .map(bb::Mesh::vertices)
            .unwrap_or_default();
        let shadow_mat = prefab
            .materials
            .iter()
            .position(|m| m.name == look.shadow_material);
        let ball = find(&look.ball).and_then(|i| {
            let o = &prefab.objects[i];
            let s = o.sprite.as_ref()?;
            let mat = s.material.and_then(|m| prefab.materials.get(m as usize));
            Some(BallSprite {
                local: sphere_world.inverse() * world(i),
                texture: mat.and_then(|m| m.texture.clone()),
                blend: mat.map_or(
                    (BlendFactor::SrcAlpha, BlendFactor::InvSrcAlpha),
                    sprite_blend,
                ),
                uv: s.uv,
                offset: s.offset,
                color: mat.map_or([1.0; 4], |m| m.diffuse),
            })
        });

        let floors = group_triangles(level, &look.floor_group);
        let mut merged = Scene {
            meshes: Vec::new(),
            materials: Vec::new(),
            objects: Vec::new(),
            groups: Vec::new(),
        };
        let mut lives = Vec::new();
        for p in placed {
            let material_offset = merged.materials.len();
            let mesh_offset = append_prefab(&mut merged, &prefab) as usize;
            let mut copy = |i: usize| {
                let mut o = prefab.objects[i].clone();
                o.mesh = o.mesh.map(|m| m + mesh_offset as u32);
                o.parent = None;
                o.hidden = true;
                merged.objects.push(o);
                merged.objects.len() - 1
            };
            let Some(sphere) = sphere_i.map(&mut copy) else {
                continue;
            };
            let shadow = shadow_i.map(&mut copy);
            if let Some(i) = shadow_i {
                let w = p.transform * root.inverse() * world(i);
                merged.objects[shadow.unwrap_or(0)].transform = w.to_cols_array_2d();
            }
            let origin = p
                .transform
                .transform_point3(Vec3::from(look.floor_ray_origin));
            let on_floor = floors
                .iter()
                .any(|&t| hits(origin, Vec3::NEG_Y, look.floor_ray_depth, t));
            lives.push(Life {
                marker: p.placeholder as usize,
                frame: p.transform,
                sphere,
                shadow,
                shadow_material: shadow_mat.map(|m| m + material_offset),
                mesh: sphere_mesh.map(|m| m as usize + mesh_offset),
                on_floor,
                shown: false,
                anim_ms: 0.0,
                wave_s: 0.0,
            });
        }
        let renderer = if lives.is_empty() {
            None
        } else {
            Some(SceneRenderer::new(gpu, &merged, format, &mut |k| bundle.texture(k).ok())?.0)
        };
        Ok(Self {
            look,
            lives,
            renderer,
            sphere_local,
            base_vertices,
            ball,
        })
    }

    /// The ball sprite's texture (`None`: untextured), to load into the
    /// particle renderer.
    pub fn sprite_textures(&self) -> Vec<Option<String>> {
        self.ball.iter().map(|b| b.texture.clone()).collect()
    }

    fn sphere_world(&self, l: &Life) -> Mat4 {
        let lk = &self.look;
        let t = l.anim_ms;
        let s = lk.squash.value(t % lk.squash.duration_ms.max(1.0));
        let lerp3 = |a: [f32; 3], b: [f32; 3], k: f32| Vec3::from(a).lerp(Vec3::from(b), k);
        let scale = lerp3(lk.squash_from, lk.squash_to, s);
        let mut local = self.sphere_local;
        if l.on_floor {
            let b = lk.bounce.value(t % lk.bounce.duration_ms.max(1.0));
            local.w_axis = lerp3(lk.bounce_from, lk.bounce_to, b).extend(1.0);
        }
        l.frame * local * Mat4::from_scale(scale)
    }

    /// One frame. `lives`: each extra life's marker and whether it can show
    /// (its sector active, not taken).
    pub fn update(&mut self, ball: Vec3, lives: &[(usize, bool)], real_dt: f32) {
        let lk = &self.look;
        for l in &mut self.lives {
            let can = lives
                .iter()
                .find(|(m, _)| *m == l.marker)
                .is_some_and(|&(_, c)| c);
            let at = l.frame.transform_point3(Vec3::ZERO);
            let (dx, dy) = (ball.x - at.x, ball.y - at.y);
            let near = dx * dx + dy * dy <= lk.show_distance * lk.show_distance;
            let shown = can && near;
            if shown && !l.shown {
                // *Activate Script* with reset.
                l.anim_ms = 0.0;
                l.wave_s = 0.0;
            }
            l.shown = shown;
            if shown {
                l.anim_ms += real_dt * 1000.0;
                l.wave_s += real_dt;
            }
        }
    }

    /// Puts the current state into the renderer.
    fn sync(&mut self, gpu: &Gpu) {
        let lk = self.look.clone();
        let mut changes: Vec<Change> = Vec::new();
        for l in &self.lives {
            let b = lk.bounce.value(l.anim_ms % lk.bounce.duration_ms.max(1.0));
            let emissive = Vec3::from_slice(&lk.shadow_from[..3])
                .lerp(Vec3::from_slice(&lk.shadow_to[..3]), b)
                .extend(1.0)
                .to_array();
            changes.push((
                l.sphere,
                self.sphere_world(l),
                l.shown,
                l.shadow_material.map(|m| (m, emissive)),
                l.shadow,
            ));
        }
        let Some(r) = self.renderer.as_mut() else {
            return;
        };
        for (sphere, world, shown, shadow_mat, shadow) in changes {
            r.set_object_hidden(sphere, !shown);
            r.set_object_transform(gpu, sphere, world);
            if let Some(s) = shadow {
                let on_floor = self
                    .lives
                    .iter()
                    .find(|l| l.sphere == sphere)
                    .is_some_and(|l| l.on_floor);
                r.set_object_hidden(s, !(shown && on_floor));
            }
            if let Some((m, e)) = shadow_mat {
                r.set_material_emissive(gpu, m, e);
            }
        }
        // The texture wave on each shown sphere.
        let w = lk.uv_wave;
        #[expect(clippy::cast_precision_loss, reason = "a few hundred vertices")]
        let n = self.base_vertices.len().max(1) as f32;
        for l in self.lives.iter().filter(|l| l.shown) {
            let Some(mesh) = l.mesh else { continue };
            let verts: Vec<Vertex> = self
                .base_vertices
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    #[expect(clippy::cast_precision_loss, reason = "a few hundred vertices")]
                    let a = l.wave_s * w.velocity + (i as f32 / n - 0.5) * w.spread;
                    let [u0, v0] = v.uv;
                    Vertex {
                        uv: [
                            u0 + (0.5 - u0) * a.cos() * w.amplitude[0],
                            v0 + (0.5 - v0) * a.sin() * w.amplitude[1],
                        ],
                        ..*v
                    }
                })
                .collect();
            r.set_mesh_vertices(gpu, mesh, &verts);
        }
    }

    /// The shown silver balls, as camera-facing quads (texture, blend,
    /// triangles).
    pub fn sprites(&self, f: Facing) -> Option<SpriteBatch> {
        let b = self.ball.as_ref()?;
        let mut out = Vec::new();
        for l in self.lives.iter().filter(|l| l.shown) {
            let m = self.sphere_world(l) * b.local;
            let centre = m.transform_point3(Vec3::ZERO);
            let (w, h) = (m.x_axis.truncate().length(), m.y_axis.truncate().length());
            let r = f.right * w;
            let u = f.up * h;
            let corner = |x: f32, y: f32| centre + r * (x - b.offset[0]) + u * (b.offset[1] - y);
            let [u0, v0, u1, v1] = b.uv;
            let v = |x: f32, y: f32, uv: [f32; 2]| ParticleVertex {
                position: corner(x, y).to_array(),
                uv,
                color: b.color,
            };
            let (tl, tr) = (v(0.0, 0.0, [u0, v0]), v(1.0, 0.0, [u1, v0]));
            let (br, bl) = (v(1.0, 1.0, [u1, v1]), v(0.0, 1.0, [u0, v1]));
            out.extend_from_slice(&[tl, bl, br, tl, br, tr]);
        }
        Some((b.texture.clone(), b.blend, out))
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
        self.sync(gpu);
        if let Some(r) = self.renderer.as_mut() {
            r.render(gpu, encoder, color, depth, camera, aspect, opts);
        }
    }

    /// Extra lives shown now (for headless runs).
    pub fn shown(&self) -> usize {
        self.lives.iter().filter(|l| l.shown).count()
    }
}

/// How a sprite's material blends. An unblended one (*One* / *Zero*) still
/// cuts out its texture's colour key, as the original's alpha test does
/// (the extra point's silver balls): drawn by the texture's alpha.
pub fn sprite_blend(m: &bb::Material) -> (BlendFactor, BlendFactor) {
    match (m.blend, m.src_blend, m.dest_blend) {
        (false, ..) | (true, BlendFactor::One, BlendFactor::Zero) => {
            (BlendFactor::SrcAlpha, BlendFactor::InvSrcAlpha)
        }
        (true, s, d) => (s, d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_down_hits_a_floor_below() {
        let tri = [
            Vec3::new(-1.0, -3.0, -1.0),
            Vec3::new(1.0, -3.0, -1.0),
            Vec3::new(0.0, -3.0, 1.0),
        ];
        let o = Vec3::new(0.0, -2.0, 0.0);
        assert!(hits(o, Vec3::NEG_Y, 1.5, tri));
        assert!(!hits(o, Vec3::NEG_Y, 0.5, tri));
        assert!(!hits(Vec3::new(5.0, -2.0, 0.0), Vec3::NEG_Y, 1.5, tri));
    }
}
