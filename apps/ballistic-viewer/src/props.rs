//! Placed props in play mode (`P_Box`, `P_Ball_*`, `P_Dome`): physics bodies at
//! the level's placeholders, drawn with their prefabs' meshes.
//!
//! Like the original (research findings 12), props are active per sector: an
//! inactive prop is out of the simulation and hidden, and activating a sector
//! puts its props back at their placeholders. [`crate::progress`] decides when.

use std::collections::HashMap;

use anyhow::{Context, Result};
use ballistic_bundle::{PhysicsData, PropParams, Scene};
use ballistic_physics::props::{self, Collider};
use ballistic_physics::{BodyDesc, BodyId, Mat4, Vec3, World};
use ballistic_render::{Camera, Gpu, RenderOptions, SceneRenderer};

use crate::play::Bundle;

/// One drawn object of a prop: its index in the merged scene, and its
/// transform relative to the body pose (or to the placement, if fixed).
struct Part {
    object: usize,
    local: Mat4,
}

struct Prop {
    sector: u32,
    active: bool,
    /// Spawn state of a dynamic prop (`None`: fixed).
    desc: Option<BodyDesc>,
    /// `None` when inactive, fixed, or fallen out of the level.
    body: Option<BodyId>,
    /// A fixed prop's own static material (switched with its sector).
    fixed_material: Option<u32>,
    parts: Vec<Part>,
}

pub struct Props {
    items: Vec<Prop>,
    renderer: Option<SceneRenderer>,
    /// Level objects the props replace (the game deletes the placeholders).
    pub placeholders: Vec<usize>,
}

struct Prefab {
    params: PropParams,
    collider: Collider,
    scene: Scene,
    root: usize,
    /// Index offsets of its meshes and materials in the merged scene.
    mesh_offset: u32,
}

impl Props {
    /// Adds the level's props to `world`, all inactive (fixed ones to its
    /// static geometry, switched off).
    #[allow(
        clippy::too_many_lines,
        reason = "one pass: prefabs, merged scene, bodies"
    )]
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        level: &Scene,
        physics: &PhysicsData,
        world: &mut World,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let spawns = props::find_props(level, physics);
        let mut merged = Scene {
            meshes: Vec::new(),
            materials: Vec::new(),
            objects: Vec::new(),
            groups: Vec::new(),
        };
        let mut prefabs: HashMap<String, Option<Prefab>> = HashMap::new();
        for s in &spawns {
            if prefabs.contains_key(&s.group) {
                continue;
            }
            let params = physics
                .props
                .iter()
                .find(|p| p.group == s.group)
                .context("prop group without parameters")?
                .clone();
            let key = format!("ph/{}", s.group.to_lowercase());
            let scene = bundle
                .scene(&key)
                .with_context(|| format!("loading {key}"))?;
            let prefab = props::prefab_root(&scene, &s.group).and_then(|root| {
                let collider = props::collider(&params, &scene)?;
                let mesh_offset = append_prefab(&mut merged, &scene);
                Some(Prefab {
                    params,
                    collider,
                    scene,
                    root,
                    mesh_offset,
                })
            });
            if prefab.is_none() {
                eprintln!("warning: {key} has no physicalized main frame; skipped");
            }
            prefabs.insert(s.group.clone(), prefab);
        }

        let mut list = Vec::new();
        for s in &spawns {
            let Some(Some(pf)) = prefabs.get(&s.group) else {
                continue;
            };
            // The prefab's main frame's Coll Detection / Continuous Contact IDs.
            let sound = props::object_sound(
                &physics.sounds,
                &format!("ph/{}", s.group.to_lowercase()),
                &pf.scene.objects[pf.root].name,
            );
            let fixed_material = props::add_fixed(
                &mut world.statics,
                &pf.params,
                &pf.collider,
                s.transform,
                sound,
            );
            if let Some(m) = fixed_material {
                world.statics.set_material_enabled(m, false);
            }
            let desc = props::body_desc(&pf.params, &pf.collider, s.transform).map(|mut d| {
                d.sound = sound;
                d
            });
            // Drawn relative to the body: the body's origin is the shape centre.
            let center = match pf.collider {
                Collider::Box { center, .. } => center,
                _ => Vec3::ZERO,
            };
            let (scale, _, _) = s.transform.to_scale_rotation_translation();
            let to_body = match desc {
                Some(_) => Mat4::from_scale(scale) * Mat4::from_translation(-center),
                None => s.transform,
            };
            let root_inv = Mat4::from_cols_array_2d(&pf.scene.objects[pf.root].transform).inverse();
            let mut parts = Vec::new();
            for o in &pf.scene.objects {
                let Some(mesh) = o.mesh else { continue };
                let local = to_body * root_inv * Mat4::from_cols_array_2d(&o.transform);
                let mut obj = o.clone();
                obj.mesh = Some(mesh + pf.mesh_offset);
                obj.parent = None;
                // Activation shows the whole prefab hierarchy.
                obj.hidden = false;
                obj.transform = (s.transform * root_inv * Mat4::from_cols_array_2d(&o.transform))
                    .to_cols_array_2d();
                parts.push(Part {
                    object: merged.objects.len(),
                    local,
                });
                merged.objects.push(obj);
            }
            list.push(Prop {
                sector: s.sector,
                active: false,
                desc,
                body: None,
                fixed_material,
                parts,
            });
        }
        let placeholders: Vec<usize> = spawns.iter().map(|s| s.placeholder as usize).collect();
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
            items: list,
            renderer,
            placeholders,
        })
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// Activates (back at the placeholder, at rest; fallen props return) or
    /// deactivates (out of the simulation, hidden) every prop of a sector.
    pub fn set_sector_active(&mut self, world: &mut World, sector: u32, on: bool) {
        for p in self.items.iter_mut().filter(|p| p.sector == sector) {
            p.active = on;
            if let Some(m) = p.fixed_material {
                world.statics.set_material_enabled(m, on);
            }
            match (on, p.body, &p.desc) {
                (true, Some(id), Some(d)) => world.set_pose(id, d.position, d.orientation),
                (true, None, Some(d)) => p.body = Some(world.add_body(d.clone())),
                (false, Some(id), _) => {
                    world.remove_body(id);
                    p.body = None;
                }
                _ => {}
            }
        }
    }

    /// Number of active props.
    pub fn active_count(&self) -> usize {
        self.items.iter().filter(|p| p.active).count()
    }

    /// Removes props that fell below `fall_y`.
    pub fn update(&mut self, world: &mut World, fall_y: f32) {
        for p in &mut self.items {
            if let Some(id) = p.body
                && world.body(id).is_none_or(|b| b.position.y < fall_y)
            {
                world.remove_body(id);
                p.body = None;
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
        for p in &self.items {
            if p.desc.is_none() {
                // Fixed: drawn where placed, while active.
                for part in &p.parts {
                    r.set_object_hidden(part.object, !p.active);
                }
                continue;
            }
            let pose = p
                .body
                .and_then(|id| world.interpolated(id))
                .map(|(pos, rot)| Mat4::from_rotation_translation(rot, pos));
            for part in &p.parts {
                r.set_object_hidden(part.object, pose.is_none());
                if let Some(pose) = pose {
                    r.set_object_transform(gpu, part.object, pose * part.local);
                }
            }
        }
        r.render(gpu, encoder, color, depth, camera, aspect, opts);
    }
}

/// Appends a prefab's meshes and materials to a merged render scene; returns
/// the index of its first mesh there.
pub fn append_prefab(merged: &mut Scene, prefab: &Scene) -> u32 {
    let mesh_offset = merged.meshes.len() as u32;
    let material_offset = merged.materials.len() as u32;
    merged.materials.extend(prefab.materials.iter().cloned());
    merged
        .meshes
        .extend(prefab.meshes.iter().cloned().map(|mut m| {
            for sm in &mut m.submeshes {
                sm.material = sm.material.map(|i| i + material_offset);
            }
            m
        }));
    mesh_offset
}
