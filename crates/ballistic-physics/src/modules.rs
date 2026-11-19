//! Moving elements (`P_Modul_*`): bodies and hinges from a module's setup
//! (`PhysicsData::modules`, extracted from its prefab script) at a placement.
//!
//! As the original's Physicalize: each body is the union of its "convex n"
//! meshes' hulls (in the object's frame). With automatic mass centre off, the
//! mass centre is the object's origin plus *Shift Mass Center*, in the object's
//! frame (Physicalize puts that translation in the object template; research
//! findings 13). Hinges pivot at their joint frame, about its Z axis;
//! `FixCube` is the world. Sliders run along the line from their first axis
//! frame to their second. Springs join a point on the body (position in the
//! first referential) and one on the other body (in the second). Fixed bodies
//! are static geometry.

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
    /// The script breaks it (see `HingeParams::breakable`).
    pub breakable: bool,
    /// The other body, or `None` for the world.
    pub other: Option<usize>,
    pub pivot: Vec3,
    pub axis: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlannedSlider {
    pub body: usize,
    pub other: Option<usize>,
    pub origin: Vec3,
    pub axis: Vec3,
    pub limits: Option<(f32, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlannedBallJoint {
    pub body: usize,
    pub other: Option<usize>,
    pub pivot: Vec3,
}

/// A force the module's script can switch on: on `body` (`None`: the player's
/// ball), at a world point, along a world direction.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedForce {
    pub body: Option<usize>,
    pub point: Vec3,
    pub direction: Vec3,
    pub value: f32,
    /// The script graph it's in.
    pub graph: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlannedSpring {
    pub body: usize,
    pub other: Option<usize>,
    /// World attachment points on `body` and on `other`.
    pub point: Vec3,
    pub other_point: Vec3,
    pub length: f32,
    pub constant: f32,
    /// (linear, global) damping.
    pub damping: (f32, f32),
}

#[derive(Debug, Clone, Default)]
pub struct ModulePlan {
    pub bodies: Vec<PlannedBody>,
    pub hinges: Vec<PlannedHinge>,
    pub sliders: Vec<PlannedSlider>,
    pub springs: Vec<PlannedSpring>,
    pub ball_joints: Vec<PlannedBallJoint>,
    pub forces: Vec<PlannedForce>,
    /// Static geometry: fixed bodies, or every body of a module whose other
    /// physics (ball joints, forces) isn't simulated yet.
    pub fixed: Vec<FixedPart>,
    /// Where the wake-up proximity test measures to.
    pub wake_target: Option<Vec3>,
    /// The bodies it wakes (indices into `bodies`); the rest wake by contact
    /// or through their joints.
    pub wake_bodies: Vec<usize>,
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
        // Fixed bodies without collision do nothing; dynamic ones still move
        // (the punching bag's rope).
        if !b.collision && b.fixed {
            continue;
        }
        let oi = object(&b.object)?;
        let m = world[oi];
        // No convex list: the object's own mesh ("Get Current").
        let own: Vec<String> = prefab.objects[oi]
            .mesh
            .and_then(|mi| prefab.meshes.get(mi as usize))
            .map(|mesh| vec![mesh.name.clone()])
            .unwrap_or_default();
        let convex = if b.convex.is_empty() { &own } else { &b.convex };
        let hull_points: Vec<Vec<Vec3>> = convex
            .iter()
            .map(|n| mesh_points(n, m))
            .collect::<Result<_>>()?;
        if b.fixed || (!simulate && b.collision) {
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
                collides: b.collision,
            },
            object_from_body: pose.inverse() * m,
        });
    }
    let index = |name: &str| body_of.iter().find(|(n, _)| n == name).map(|(_, i)| *i);
    let other_of = |o: &Option<String>| -> Result<Option<usize>> {
        o.as_ref()
            .map(|o| index(o).with_context(|| format!("joint to unknown body {o}")))
            .transpose()
    };
    if simulate {
        for s in &setup.sliders {
            let Some(body) = index(&s.body) else { continue };
            let from = world[object(&s.axis_from)?].transform_point3(Vec3::ZERO);
            let to = world[object(&s.axis_to)?].transform_point3(Vec3::ZERO);
            plan.sliders.push(PlannedSlider {
                body,
                other: other_of(&s.other)?,
                origin: from,
                axis: (to - from).normalize_or_zero(),
                // The original's limits bound Object2's travel relative to
                // the body, the opposite of the body's own travel (research
                // findings 13).
                limits: s.limits.map(|[lo, hi]| (-hi, -lo)),
            });
        }
        let point = |a: &bb::SpringAnchor| -> Result<Vec3> {
            let p = Vec3::from(a.position);
            Ok(match &a.frame {
                Some(f) => world[object(f)?].transform_point3(p),
                // No referential: the position is in world space.
                None => p,
            })
        };
        for s in &setup.springs {
            let Some(body) = index(&s.body) else { continue };
            plan.springs.push(PlannedSpring {
                body,
                other: other_of(&s.other)?,
                point: point(&s.anchor)?,
                other_point: point(&s.other_anchor)?,
                length: s.length,
                constant: s.constant,
                damping: (s.linear_damping, s.global_damping),
            });
        }
        for j in &setup.ball_joints {
            let Some(body) = index(&j.body) else { continue };
            plan.ball_joints.push(PlannedBallJoint {
                body,
                other: other_of(&j.other)?,
                pivot: point(&j.pivot)?,
            });
        }
        for f in &setup.forces {
            let body = match &f.body {
                Some(b) => match index(b) {
                    Some(i) => Some(i),
                    None => continue,
                },
                None => None,
            };
            let direction = match &f.direction_frame {
                Some(fr) => world[object(fr)?].transform_vector3(Vec3::from(f.direction)),
                None => Vec3::from(f.direction),
            };
            plan.forces.push(PlannedForce {
                body,
                point: point(&f.position)?,
                direction: direction.normalize_or_zero(),
                value: f.value,
                graph: f.graph.clone(),
            });
        }
        for h in &setup.hinges {
            let Some(body) = index(&h.body) else { continue };
            let other = other_of(&h.other)?;
            let f = world[object(&h.frame)?];
            plan.hinges.push(PlannedHinge {
                body,
                breakable: h.breakable,
                other,
                pivot: f.transform_point3(Vec3::ZERO),
                axis: f.transform_vector3(Vec3::Z).normalize_or_zero(),
            });
        }
    }
    if let Some(w) = &setup.wake {
        plan.wake_target = Some(world[object(&w.object)?].transform_point3(Vec3::ZERO));
        plan.wake_bodies = w.bodies.iter().filter_map(|b| index(b)).collect();
    }
    Ok(plan)
}

/// Rotation of a matrix without scale (for objects drawn with a body).
pub fn rotation_of(m: Mat4) -> Quat {
    m.to_scale_rotation_translation().1
}
