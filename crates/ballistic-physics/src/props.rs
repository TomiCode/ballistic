//! Placed props: the level's placeholder objects for the prefab groups in
//! `PhysicsData::props` (`P_Box`, `P_Ball_*`, `P_Dome`).
//!
//! As in the original's level init (research findings 09): every member of a
//! prop group is a placeholder whose world matrix (and sector) is recorded.
//! The prefab `PH/<group>` replaces it, and its main frame `<group>_MF` is
//! physicalized at that matrix, as a ball primitive (`Physicalize_Balls`) or as
//! the convex hull of its mesh (`Physicalize_Convex`). Fixed props don't move.

use ballistic_bundle as bb;
use glam::{Mat4, Vec3};

use crate::mesh::{StaticMesh, SurfaceMaterial};
use crate::shape::Shape;
use crate::world::BodyDesc;

/// One placeholder in a level.
#[derive(Debug, Clone, PartialEq)]
pub struct PropSpawn {
    /// Prefab group, e.g. `P_Box` (a key of `PhysicsData::props`).
    pub group: String,
    /// The placeholder's index in the level's objects.
    pub placeholder: u32,
    /// World matrix of the placeholder.
    pub transform: Mat4,
    /// Number of the `Sector_NN` group the placeholder is in.
    pub sector: u32,
}

/// Every placeholder of every prop group, in group then member order. Like
/// level init, it skips placeholders that are in no `Sector_NN` group.
pub fn find_props(level: &bb::Scene, physics: &bb::PhysicsData) -> Vec<PropSpawn> {
    physics
        .props
        .iter()
        .flat_map(|p| placeholders(level, &p.group))
        .collect()
}

/// The placeholders of one prefab group that are in a `Sector_NN` group.
pub fn placeholders(level: &bb::Scene, group: &str) -> Vec<PropSpawn> {
    let sector_of = |object: u32| {
        level.groups.iter().find_map(|g| {
            let n = g.name.strip_prefix("Sector_")?.parse().ok()?;
            g.members.contains(&object).then_some(n)
        })
    };
    let Some(g) = level.groups.iter().find(|g| g.name == group) else {
        return Vec::new();
    };
    g.members
        .iter()
        .filter_map(|&m| {
            let obj = level.objects.get(m as usize)?;
            Some(PropSpawn {
                group: group.to_owned(),
                placeholder: m,
                transform: Mat4::from_cols_array_2d(&obj.transform),
                sector: sector_of(m)?,
            })
        })
        .collect()
}

/// The prefab's main frame (`<group>_MF`), whose mesh is physicalized.
pub fn prefab_root(prefab: &bb::Scene, group: &str) -> Option<usize> {
    let name = format!("{group}_MF");
    prefab.objects.iter().position(|o| o.name == name)
}

/// A prop's collision shape in its main frame's local space.
#[derive(Debug, Clone, PartialEq)]
pub enum Collider {
    Sphere {
        radius: f32,
    },
    /// The mesh's convex hull is this box.
    Box {
        center: Vec3,
        half: Vec3,
    },
    /// Not a box: approximated by the bounding sphere around the frame origin
    /// (the paper ball, like the player's paper ball for now).
    HullSphere {
        radius: f32,
    },
    /// Fixed props: the mesh's triangles, as static geometry.
    Mesh {
        triangles: Vec<[Vec3; 3]>,
    },
}

/// The collider of a prop group's prefab, or `None` if the prefab has no
/// physicalized mesh or the group has collision off.
pub fn collider(params: &bb::PropParams, prefab: &bb::Scene) -> Option<Collider> {
    if !params.collision {
        return None;
    }
    if let Some(radius) = params.sphere_radius {
        return Some(Collider::Sphere { radius });
    }
    let root = &prefab.objects[prefab_root(prefab, &params.group)?];
    let mesh = &prefab.meshes[root.mesh? as usize];
    let verts: Vec<Vec3> = mesh
        .vertices()
        .iter()
        .map(|v| Vec3::from(v.position))
        .collect();
    if verts.is_empty() {
        return None;
    }
    if params.fixed {
        let triangles = mesh
            .indices()
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|i| verts[usize::from(i)]))
            .collect();
        return Some(Collider::Mesh { triangles });
    }
    if let Some((center, half)) = as_box(&verts) {
        return Some(Collider::Box { center, half });
    }
    let radius = verts.iter().map(|v| v.length()).fold(0.0, f32::max);
    Some(Collider::HullSphere { radius })
}

/// The box a vertex set's hull is, if every vertex is a corner of its bounds.
fn as_box(verts: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let (lo, hi) = verts.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
        (lo.min(*v), hi.max(*v))
    });
    let eps = (hi - lo).max_element() * 1e-3;
    let corner = |x: f32, a: f32, b: f32| (x - a).abs() <= eps || (x - b).abs() <= eps;
    let all_corners = verts
        .iter()
        .all(|v| corner(v.x, lo.x, hi.x) && corner(v.y, lo.y, hi.y) && corner(v.z, lo.z, hi.z));
    (all_corners && (hi - lo).min_element() > eps).then(|| ((lo + hi) * 0.5, (hi - lo) * 0.5))
}

/// Body for a dynamic prop placed at `transform`; `None` for fixed props.
/// The body's origin is the shape's centre (a box's centre, else the frame origin).
pub fn body_desc(
    params: &bb::PropParams,
    collider: &Collider,
    transform: Mat4,
) -> Option<BodyDesc> {
    let (scale, orientation, _) = transform.to_scale_rotation_translation();
    let scale = scale.abs();
    let (shape, center) = match *collider {
        Collider::Sphere { radius } | Collider::HullSphere { radius } => (
            Shape::Sphere {
                radius: radius * scale.max_element(),
            },
            Vec3::ZERO,
        ),
        Collider::Box { center, half } => (Shape::Box { half: half * scale }, center),
        Collider::Mesh { .. } => return None,
    };
    if params.fixed {
        return None;
    }
    Some(BodyDesc {
        position: transform.transform_point3(center),
        orientation,
        shape,
        mass: params.mass,
        friction: params.friction,
        elasticity: params.elasticity,
        linear_damping: params.linear_damping,
        rotational_damping: params.rotational_damping,
        group: params.collision_group.clone(),
        asleep: params.start_frozen,
    })
}

/// Adds a fixed prop's triangles at `transform` to static geometry, with a
/// material of its own (to switch it off with [`StaticMesh::set_material_enabled`]).
/// Returns that material, or `None` if the prop isn't fixed. Call
/// [`StaticMesh::finish`] afterwards.
pub fn add_fixed(
    statics: &mut StaticMesh,
    params: &bb::PropParams,
    collider: &Collider,
    transform: Mat4,
) -> Option<u32> {
    let Collider::Mesh { triangles } = collider else {
        return None;
    };
    let material = statics.add_material(SurfaceMaterial {
        friction: params.friction,
        elasticity: params.elasticity,
        group: params.collision_group.clone(),
    });
    for t in triangles {
        let [a, b, c] = t.map(|v| transform.transform_point3(v));
        statics.add_triangle(a, b, c, material);
    }
    Some(material)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_vertices_are_a_box() {
        let mut v = Vec::new();
        for i in 0..24 {
            let c = i % 8;
            v.push(Vec3::new(
                if c & 1 == 0 { -2.5 } else { 2.5 },
                if c & 2 == 0 { -2.5 } else { 2.5 },
                if c & 4 == 0 { -2.5 } else { 2.5 },
            ));
        }
        let (c, h) = as_box(&v).unwrap();
        assert_eq!(c, Vec3::ZERO);
        assert_eq!(h, Vec3::splat(2.5));
        v.push(Vec3::new(0.0, 3.0, 0.0));
        assert!(as_box(&v).is_none());
    }
}
