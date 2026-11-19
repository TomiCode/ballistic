//! Moving elements (`P_Modul_*`): bodies and hinges from a module's setup
//! (`PhysicsData::modules`, extracted from its prefab script) at a placement.
//!
//! As the original's Physicalize: each body is the union of its "convex n"
//! meshes' hulls (in the object's frame). With automatic mass centre off, the
//! mass centre is the object's origin plus *Shift Mass Center*, in the object's
//! frame (Physicalize puts that translation in the object template; research
//! findings 13). Hinges pivot at their joint frame, about its Z axis;
//! `FixCube` is the world. Fixed bodies are static geometry.

use std::sync::Arc;

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use glam::{Mat4, Quat, Vec3};

use crate::hull::{Hull, mass_properties};
use crate::shape::Shape;
use crate::world::BodyDesc;

/// A dynamic body of a placed module.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// The physicalized object's index in the prefab scene.
    pub object: usize,
    pub desc: BodyDesc,
    /// The object's world matrix relative to the body pose (for drawing).
    pub object_from_body: Mat4,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlannedHinge {
    /// Index into [`ModulePlan::bodies`].
    pub body: usize,
    /// The other body, or `None` for the world.
    pub other: Option<usize>,
    pub pivot: Vec3,
    pub axis: Vec3,
}

#[derive(Debug, Clone, Default)]
pub struct ModulePlan {
    pub bodies: Vec<PlannedBody>,
    pub hinges: Vec<PlannedHinge>,
    /// Static geometry: fixed bodies, or every body of a module whose other
    /// physics (sliders, springs, forces) isn't simulated yet.
    pub fixed: Vec<FixedPart>,
    /// Where the wake-up proximity test measures to.
    pub wake_target: Option<Vec3>,
}

#[derive(Debug, Clone)]
pub struct FixedPart {
    pub object: usize,
    pub friction: f32,
    pub elasticity: f32,
    pub group: String,
    pub triangles: Vec<[Vec3; 3]>,
}

/// Whether a module is simulated (all its physics is supported), or a fixed
/// stand-in.
pub fn simulated(setup: &bb::ModuleSetup) -> bool {
    setup.unsupported.is_empty()
}

/// World matrices of the prefab's objects when the module is placed at
/// `transform` (the prefab's main frame `<group>_MF` goes there).
pub fn placed_objects(prefab: &bb::Scene, group: &str, transform: Mat4) -> Option<Vec<Mat4>> {
    let root = prefab
        .objects
        .iter()
        .find(|o| o.name == format!("{group}_MF"))?;
    let root_inv = Mat4::from_cols_array_2d(&root.transform).inverse();
    Some(
        prefab
            .objects
            .iter()
            .map(|o| transform * root_inv * Mat4::from_cols_array_2d(&o.transform))
            .collect(),
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "one pass over the setup: bodies, hinges, wake-up"
)]
pub fn plan(setup: &bb::ModuleSetup, prefab: &bb::Scene, transform: Mat4) -> Result<ModulePlan> {
    let world = placed_objects(prefab, &setup.group, transform)
        .with_context(|| format!("{}: no main frame", setup.group))?;
    let object = |name: &str| {
        prefab
            .objects
            .iter()
            .position(|o| o.name == name)
            .with_context(|| format!("{}: object {name} missing", setup.group))
    };
    let mesh_points = |name: &str, m: Mat4| -> Result<Vec<Vec3>> {
        let mesh = prefab
            .meshes
            .iter()
            .find(|x| x.name == name)
            .with_context(|| {
                format!(
                    "{}: mesh {name} missing (re-run ballistic-pack)",
                    setup.group
                )
            })?;
        Ok(mesh
            .vertices()
            .iter()
            .map(|v| m.transform_point3(Vec3::from(v.position)))
            .collect())
    };
    let simulate = simulated(setup);
    let mut plan = ModulePlan::default();
    let mut body_of = Vec::new(); // (object name, index in plan.bodies)
    for b in &setup.bodies {
        if !b.collision {
            continue;
        }
        let oi = object(&b.object)?;
        let m = world[oi];
        let hull_points: Vec<Vec<Vec3>> = b
            .convex
            .iter()
            .map(|n| mesh_points(n, m))
            .collect::<Result<_>>()?;
        if b.fixed || !simulate {
            let mut triangles = Vec::new();
            for pts in &hull_points {
                let Some(h) = Hull::new(pts) else { continue };
                for (_, l) in &h.faces {
                    for k in 1..l.len() - 1 {
                        triangles.push([h.verts[l[0]], h.verts[l[k]], h.verts[l[k + 1]]]);
                    }
                }
            }
            plan.fixed.push(FixedPart {
                object: oi,
                friction: b.friction,
                elasticity: b.elasticity,
                group: b.collision_group.clone(),
                triangles,
            });
            continue;
        }
        let (_, rotation, _) = m.to_scale_rotation_translation();
        let hulls_world: Vec<Hull> = hull_points.iter().filter_map(|p| Hull::new(p)).collect();
        let center = if b.auto_mass_center {
            mass_properties(&hulls_world, b.mass).1
        } else {
            m.transform_point3(Vec3::from(b.shift_mass_center))
        };
        let inv = rotation.inverse();
        let hulls: Vec<Hull> = hull_points
            .iter()
            .filter_map(|pts| {
                let local: Vec<Vec3> = pts.iter().map(|p| inv * (*p - center)).collect();
                Hull::new(&local)
            })
            .collect();
        let pose = Mat4::from_rotation_translation(rotation, center);
        body_of.push((b.object.clone(), plan.bodies.len()));
        plan.bodies.push(PlannedBody {
            object: oi,
            desc: BodyDesc {
                position: center,
                orientation: rotation,
                shape: Shape::Hulls(Arc::from(hulls)),
                mass: b.mass,
                friction: b.friction,
                elasticity: b.elasticity,
                linear_damping: b.linear_damping,
                rotational_damping: b.rotational_damping,
                group: b.collision_group.clone(),
                asleep: b.start_frozen,
            },
            object_from_body: pose.inverse() * m,
        });
    }
    if simulate {
        let index = |name: &str| body_of.iter().find(|(n, _)| n == name).map(|(_, i)| *i);
        for h in &setup.hinges {
            let Some(body) = index(&h.body) else { continue };
            let other = match &h.other {
                Some(o) => Some(index(o).with_context(|| format!("hinge to unknown body {o}"))?),
                None => None,
            };
            let f = world[object(&h.frame)?];
            plan.hinges.push(PlannedHinge {
                body,
                other,
                pivot: f.transform_point3(Vec3::ZERO),
                axis: f.transform_vector3(Vec3::Z).normalize_or_zero(),
            });
        }
    }
    if let Some(w) = &setup.wake {
        plan.wake_target = Some(world[object(&w.object)?].transform_point3(Vec3::ZERO));
    }
    Ok(plan)
}

/// Rotation of a matrix without scale (for objects drawn with a body).
pub fn rotation_of(m: Mat4) -> Quat {
    m.to_scale_rotation_translation().1
}
