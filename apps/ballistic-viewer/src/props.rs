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
use ballistic_physics::{BodyDesc, BodyId, Mat4, SurfaceMaterial, Vec3, World};
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
    pub level_end: Option<LevelEnd>,
}

/// The level end (`PE_Balloon` at the `PE_Levelende` placeholder).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelEnd {
    /// `PE_Balloon_Platform`'s origin: the ball must get within 1 of it.
    pub platform: Vec3,
    /// The last sector, whose activation starts the balloon's script.
    pub sector: u32,
}

/// Level-end prefab objects that are physicalized (all as static geometry
/// for now: in the original the platform hangs on a slider with a spring and
/// the planks on hinges), with their Physicalize values (friction 0.7,
/// elasticity 0.4, collision group "Floor").
const LEVEL_END_SOLID: [&str; 2] = ["PE_Balloon_Platform", "PE_Balloon_Platte"];
/// Level-end prefab objects not drawn: the UFO (last level only) and the
/// hidden slide box.
const LEVEL_END_HIDDEN: [&str; 2] = ["PE_UFO", "PE_Box_slide"];

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
                let mesh_offset = merged.meshes.len() as u32;
                let material_offset = merged.materials.len() as u32;
                merged.materials.extend(scene.materials.iter().cloned());
                merged
                    .meshes
                    .extend(scene.meshes.iter().cloned().map(|mut m| {
                        for sm in &mut m.submeshes {
                            sm.material = sm.material.map(|i| i + material_offset);
                        }
                        m
                    }));
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
            let fixed_material =
                props::add_fixed(&mut world.statics, &pf.params, &pf.collider, s.transform);
            if let Some(m) = fixed_material {
                world.statics.set_material_enabled(m, false);
            }
            let desc = props::body_desc(&pf.params, &pf.collider, s.transform);
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
        let mut placeholders: Vec<usize> = spawns.iter().map(|s| s.placeholder as usize).collect();
        let level_end = match add_level_end(bundle, level, &mut merged, world) {
            Some((prop, end, placeholder)) => {
                list.push(prop);
                placeholders.push(placeholder);
                Some(end)
            }
            None => None,
        };
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
            level_end,
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

/// Places the level end like level init: the `PE_Levelende` member's matrix,
/// the prefab named like it without `_NN`, in the last sector. Returns the
/// prop (fixed, switched off), the level end, and the placeholder's index.
fn add_level_end(
    bundle: &mut Bundle,
    level: &Scene,
    merged: &mut Scene,
    world: &mut World,
) -> Option<(Prop, LevelEnd, usize)> {
    let group = level.groups.iter().find(|g| g.name == "PE_Levelende")?;
    let &index = group.members.first()?;
    let placeholder = level.objects.get(index as usize)?;
    let base = placeholder.name.rsplit_once('_')?.0;
    let Ok(prefab) = bundle.scene(&format!("ph/{}", base.to_lowercase())) else {
        eprintln!("warning: level end prefab {base} not in the bundle");
        return None;
    };
    let transform = Mat4::from_cols_array_2d(&placeholder.transform);
    let world_of = |name: &str| {
        prefab
            .objects
            .iter()
            .find(|o| o.name == name)
            .map(|o| Mat4::from_cols_array_2d(&o.transform))
    };
    let root_inv = world_of(&format!("{base}_MF"))?.inverse();
    let platform = world_of(&format!("{base}_Platform"))
        .map(|m| (transform * root_inv * m).transform_point3(Vec3::ZERO))?;

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
    let material = world.statics.add_material(SurfaceMaterial {
        friction: 0.7,
        elasticity: 0.4,
        group: "Floor".into(),
    });
    world.statics.set_material_enabled(material, false);
    let mut parts = Vec::new();
    for o in &prefab.objects {
        let Some(mesh) = o.mesh else { continue };
        if LEVEL_END_HIDDEN.iter().any(|p| o.name.starts_with(p)) {
            continue;
        }
        let model = transform * root_inv * Mat4::from_cols_array_2d(&o.transform);
        if LEVEL_END_SOLID.iter().any(|p| o.name.starts_with(p)) {
            let m = &prefab.meshes[mesh as usize];
            let verts: Vec<Vec3> = m
                .vertices()
                .iter()
                .map(|v| model.transform_point3(Vec3::from(v.position)))
                .collect();
            for t in m.indices().as_chunks::<3>().0 {
                let [a, b, c] = t.map(|i| verts[usize::from(i)]);
                world.statics.add_triangle(a, b, c, material);
            }
        }
        let mut obj = o.clone();
        obj.mesh = Some(mesh + mesh_offset);
        obj.parent = None;
        obj.hidden = false;
        obj.transform = model.to_cols_array_2d();
        parts.push(Part {
            object: merged.objects.len(),
            local: model,
        });
        merged.objects.push(obj);
    }
    let sector = crate::progress::last_sector(level);
    let prop = Prop {
        sector,
        active: false,
        desc: None,
        body: None,
        fixed_material: Some(material),
        parts,
    };
    Some((prop, LevelEnd { platform, sector }, index as usize))
}
