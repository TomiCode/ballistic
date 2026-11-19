//! Moving elements in play mode (`P_Modul_*`), placed like the original's
//! level init and run like their prefab scripts (research findings 13):
//!
//! - Modules are always drawn (level init shows every prefab), but only
//!   physical while their sector is active. Activation physicalizes the bodies
//!   (frozen ones asleep), creates the hinges and arms the wake-up proximity
//!   test; deactivation removes them and restores the initial pose.
//! - A module whose script also uses ball joints or forces is a fixed
//!   stand-in for now: its bodies are static geometry where they're placed.

use anyhow::{Context, Result};
use ballistic_bundle::{ModuleSetup, PhysicsData, Scene};
use ballistic_physics::modules::{self, ModulePlan};
use ballistic_physics::{BodyId, Mat4, SurfaceMaterial, Vec3, World, props};
use ballistic_render::{Camera, Gpu, RenderOptions, SceneRenderer};

use crate::play::Bundle;
use crate::progress::{Edge, Proximity};
use crate::props::append_prefab;

/// A drawn object: its index in the merged scene, the plan body it moves
/// with (if any), and its matrix relative to that body's initial pose (or
/// its world matrix, if static).
struct Part {
    object: usize,
    body: Option<usize>,
    local: Mat4,
}

struct Module {
    group: String,
    sector: u32,
    plan: ModulePlan,
    /// Bodies while active (same order as `plan.bodies`).
    bodies: Vec<BodyId>,
    /// One static material per fixed part (switched with the sector).
    fixed_materials: Vec<u32>,
    wake: Option<Proximity>,
    wake_template: Option<Proximity>,
    parts: Vec<Part>,
}

pub struct Modules {
    items: Vec<Module>,
    renderer: Option<SceneRenderer>,
    /// Level objects the modules replace.
    pub placeholders: Vec<usize>,
}

impl Modules {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        level: &Scene,
        physics: &PhysicsData,
        world: &mut World,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let mut merged = Scene {
            meshes: Vec::new(),
            materials: Vec::new(),
            objects: Vec::new(),
            groups: Vec::new(),
        };
        let mut items = Vec::new();
        let mut placeholders = Vec::new();
        for setup in &physics.modules {
            let spawns = props::placeholders(level, &setup.group);
            if spawns.is_empty() {
                continue;
            }
            let key = format!("ph/{}", setup.group.to_lowercase());
            let prefab = bundle
                .scene(&key)
                .with_context(|| format!("loading {key}"))?;
            let mesh_offset = append_prefab(&mut merged, &prefab);
            for s in &spawns {
                placeholders.push(s.placeholder as usize);
                let plan = match modules::plan(setup, &prefab, s.transform) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("warning: {}: {e:#}; drawn only", setup.group);
                        ModulePlan::default()
                    }
                };
                let module = place(
                    setup,
                    &prefab,
                    s.transform,
                    s.sector,
                    plan,
                    mesh_offset,
                    &mut merged,
                    world,
                );
                items.push(module);
            }
        }
        world.statics.finish();
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
            placeholders,
        })
    }

    pub fn count(&self) -> (usize, usize) {
        let simulated = self
            .items
            .iter()
            .filter(|m| !m.plan.bodies.is_empty())
            .count();
        (self.items.len(), simulated)
    }

    /// Script start with *Activation Phase?* true (physicalize, hinges, arm
    /// the wake-up test) or false (unphysicalize, restore the initial pose).
    pub fn set_sector_active(&mut self, world: &mut World, sector: u32, on: bool) {
        for m in self.items.iter_mut().filter(|m| m.sector == sector) {
            for &mat in &m.fixed_materials {
                world.statics.set_material_enabled(mat, on);
            }
            for id in m.bodies.drain(..) {
                world.remove_body(id);
            }
            m.wake = None;
            if !on {
                continue;
            }
            m.bodies = m
                .plan
                .bodies
                .iter()
                .map(|b| world.add_body(b.desc.clone()))
                .collect();
            for h in &m.plan.hinges {
                let other = h.other.map(|o| m.bodies[o]);
                if world
                    .add_hinge(m.bodies[h.body], other, h.pivot, h.axis)
                    .is_none()
                {
                    eprintln!("warning: {}: hinge not created", m.group);
                }
            }
            for s in &m.plan.sliders {
                let other = s.other.map(|o| m.bodies[o]);
                if world
                    .add_slider(m.bodies[s.body], other, s.origin, s.axis, s.limits)
                    .is_none()
                {
                    eprintln!("warning: {}: slider not created", m.group);
                }
            }
            for s in &m.plan.springs {
                let other = s.other.map(|o| m.bodies[o]);
                world.add_spring(
                    m.bodies[s.body],
                    other,
                    s.point,
                    s.other_point,
                    s.length,
                    s.constant,
                    s.damping,
                );
            }
            m.wake.clone_from(&m.wake_template);
            if let Some(w) = m.wake.as_mut() {
                w.switch_on();
            }
        }
    }

    /// One frame: the wake-up tests (`TT Scaleable Proximity` → `Physics WakeUp`).
    pub fn frame(&mut self, ball: Vec3, world: &mut World) {
        for m in &mut self.items {
            let (Some(w), Some(target)) = (m.wake.as_mut(), m.plan.wake_target) else {
                continue;
            };
            if w.frame(ball, target) == Some(Edge::Enter) {
                w.switch_off();
                for &i in &m.plan.wake_bodies {
                    if let Some(&id) = m.bodies.get(i) {
                        world.wake(id);
                    }
                }
            }
        }
    }

    /// Development aid: wakes every active module as its proximity test
    /// would (its wake-up targets; modules without a test are awake anyway).
    pub fn wake_all(&mut self, world: &mut World) {
        for m in &mut self.items {
            if m.wake.take().is_none() {
                continue;
            }
            for &i in &m.plan.wake_bodies {
                if let Some(&id) = m.bodies.get(i) {
                    world.wake(id);
                }
            }
        }
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
        world: &World,
    ) {
        let Some(r) = self.renderer.as_mut() else {
            return;
        };
        for m in &self.items {
            for part in &m.parts {
                let Some(bi) = part.body else { continue };
                let pose = m
                    .bodies
                    .get(bi)
                    .and_then(|&id| world.interpolated(id))
                    .map_or_else(
                        || {
                            let d = &m.plan.bodies[bi].desc;
                            Mat4::from_rotation_translation(d.orientation, d.position)
                        },
                        |(p, q)| Mat4::from_rotation_translation(q, p),
                    );
                r.set_object_transform(gpu, part.object, pose * part.local);
            }
        }
        r.render(gpu, encoder, color, depth, camera, aspect, opts);
    }
}

/// Adds a placed module's drawn objects to the merged scene and its fixed
/// parts to static geometry (switched off until its sector activates).
#[allow(clippy::too_many_arguments)]
fn place(
    setup: &ModuleSetup,
    prefab: &Scene,
    transform: Mat4,
    sector: u32,
    plan: ModulePlan,
    mesh_offset: u32,
    merged: &mut Scene,
    world: &mut World,
) -> Module {
    let placed = modules::placed_objects(prefab, &setup.group, transform).unwrap_or_default();
    // The body an object moves with: its own, or an ancestor's.
    let body_of = |mut i: usize| loop {
        if let Some(b) = plan.bodies.iter().position(|b| b.object == i) {
            return Some(b);
        }
        i = prefab.objects[i].parent? as usize;
    };
    let mut parts = Vec::new();
    for (i, o) in prefab.objects.iter().enumerate() {
        let (Some(mesh), Some(&world_m)) = (o.mesh, placed.get(i)) else {
            continue;
        };
        let body = body_of(i);
        let local = match body {
            Some(b) => {
                let d = &plan.bodies[b].desc;
                Mat4::from_rotation_translation(d.orientation, d.position).inverse() * world_m
            }
            None => world_m,
        };
        let mut obj = o.clone();
        obj.mesh = Some(mesh + mesh_offset);
        obj.parent = None;
        obj.hidden = false;
        obj.transform = world_m.to_cols_array_2d();
        parts.push(Part {
            object: merged.objects.len(),
            body,
            local,
        });
        merged.objects.push(obj);
    }
    let mut fixed_materials = Vec::new();
    for f in &plan.fixed {
        let mat = world.statics.add_material(SurfaceMaterial {
            friction: f.friction,
            elasticity: f.elasticity,
            group: f.group.clone(),
        });
        world.statics.set_material_enabled(mat, false);
        for t in &f.triangles {
            world.statics.add_triangle(t[0], t[1], t[2], mat);
        }
        fixed_materials.push(mat);
    }
    let wake_template = setup.wake.as_ref().map(|w| {
        Proximity::new(
            w.distance,
            w.exactness[0],
            w.exactness[1],
            (w.frame_delay[0], w.frame_delay[1]),
            w.xz,
        )
    });
    Module {
        group: setup.group.clone(),
        sector,
        plan,
        bodies: Vec::new(),
        fixed_materials,
        wake: None,
        wake_template,
        parts,
    }
}
