//! Collision shapes of dynamic bodies.

use std::sync::Arc;

use glam::{Mat3, Quat, Vec3};

use crate::bvh::Aabb;
use crate::hull::{Hull, mass_properties};

/// A body's collision shape, in the body's frame (origin = mass centre).
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// The original's ball primitive.
    Sphere { radius: f32 },
    /// A cuboid with these half extents (a convex hull that is a box, e.g. `P_Box`).
    Box { half: Vec3 },
    /// Convex hulls (the original's "convex 1…n" meshes of one body).
    Hulls(Arc<[Hull]>),
}

impl Shape {
    /// Radius of the smallest sphere around the origin containing the shape.
    pub fn bounding_radius(&self) -> f32 {
        match self {
            Self::Sphere { radius } => *radius,
            Self::Box { half } => half.length(),
            Self::Hulls(h) => h.iter().map(Hull::radius).fold(0.0, f32::max),
        }
    }

    /// Inertia tensor about the origin of a solid body of uniform density.
    pub fn inertia(&self, mass: f32) -> Mat3 {
        match self {
            Self::Sphere { radius } => {
                Mat3::from_diagonal(Vec3::splat(0.4 * mass * radius * radius))
            }
            Self::Box { half } => {
                let s = *half * *half;
                Mat3::from_diagonal(Vec3::new(s.y + s.z, s.x + s.z, s.x + s.y) * (mass / 3.0))
            }
            Self::Hulls(h) => mass_properties(h, mass).2,
        }
    }

    pub(crate) fn aabb(&self, position: Vec3, orientation: Quat) -> Aabb {
        let m = Mat3::from_quat(orientation);
        match self {
            Self::Sphere { radius } => Aabb {
                min: position - Vec3::splat(*radius),
                max: position + Vec3::splat(*radius),
            },
            Self::Box { half } => {
                let e = m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
                Aabb {
                    min: position - e,
                    max: position + e,
                }
            }
            Self::Hulls(h) => {
                let mut b = Aabb::EMPTY;
                for v in h.iter().flat_map(|h| h.verts.iter()) {
                    b.grow(position + m * *v);
                }
                b
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_inertia_matches_textbook() {
        // Cube of side 2 (half 1), mass 3: I = m·(2² + 2²)/12 = 2.
        let i = Shape::Box { half: Vec3::ONE }.inertia(3.0);
        assert!(
            i.abs_diff_eq(Mat3::from_diagonal(Vec3::splat(2.0)), 1e-6),
            "{i}"
        );
    }

    #[test]
    fn rotated_box_bounds() {
        let b = Shape::Box {
            half: Vec3::new(1.0, 1.0, 1.0),
        }
        .aabb(
            Vec3::ZERO,
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
        );
        let s2 = std::f32::consts::SQRT_2;
        assert!(b.max.abs_diff_eq(Vec3::new(s2, 1.0, s2), 1e-5), "{b:?}");
    }
}
