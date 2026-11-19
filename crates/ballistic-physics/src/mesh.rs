//! Static collision geometry: world-space triangles with surface materials.

use glam::Vec3;

use crate::bvh::{Aabb, Bvh};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SurfaceMaterial {
    pub friction: f32,
    pub elasticity: f32,
    /// Collision group name ("" = none).
    pub group: String,
    /// What the surface sounds like when hit or rolled on.
    pub sound: SoundIds,
}

/// A surface's hit and roll sound IDs (the original's *Coll Detection ID* and
/// *Continuous Contact ID* attributes; 0: none).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SoundIds {
    pub hit: u8,
    pub roll: u8,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub normal: Vec3,
    pub material: u32,
}

#[derive(Debug, Clone, Default)]
pub struct StaticMesh {
    pub(crate) triangles: Vec<Triangle>,
    pub(crate) materials: Vec<SurfaceMaterial>,
    /// Per material: switched off (its triangles don't collide).
    disabled: Vec<bool>,
    bvh: Bvh,
    dirty: bool,
}

/// A point on a triangle closest to a query point, and which feature it's on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Closest {
    pub point: Vec3,
    /// True when the point is in the triangle's interior.
    pub on_face: bool,
}

impl StaticMesh {
    pub fn add_material(&mut self, m: SurfaceMaterial) -> u32 {
        self.materials.push(m);
        self.disabled.push(false);
        self.materials.len() as u32 - 1
    }

    /// Switches a material's triangles on or off (e.g. a fixed prop whose
    /// sector is inactive). No rebuild needed.
    pub fn set_material_enabled(&mut self, material: u32, enabled: bool) {
        if let Some(d) = self.disabled.get_mut(material as usize) {
            *d = !enabled;
        }
    }

    pub(crate) fn material_enabled(&self, material: u32) -> bool {
        !self
            .disabled
            .get(material as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Adds a world-space triangle. Degenerate triangles are skipped.
    pub fn add_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, material: u32) {
        let n = (b - a).cross(c - a);
        if n.length_squared() < 1e-12 {
            return;
        }
        self.triangles.push(Triangle {
            a,
            b,
            c,
            normal: n.normalize(),
            material,
        });
        self.dirty = true;
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Builds the acceleration structure. Call after adding triangles.
    pub fn finish(&mut self) {
        let boxes: Vec<Aabb> = self
            .triangles
            .iter()
            .map(|t| {
                let mut b = Aabb::EMPTY;
                b.grow(t.a);
                b.grow(t.b);
                b.grow(t.c);
                b
            })
            .collect();
        self.bvh = Bvh::build(&boxes);
        self.dirty = false;
    }

    pub(crate) fn query(&self, bounds: &Aabb, f: impl FnMut(u32)) {
        debug_assert!(!self.dirty, "StaticMesh::finish not called");
        self.bvh.query(bounds, f);
    }

    /// Lowest point of the geometry (for fall-out detection).
    pub fn min_y(&self) -> f32 {
        self.triangles
            .iter()
            .map(|t| t.a.y.min(t.b.y).min(t.c.y))
            .fold(f32::MAX, f32::min)
    }
}

/// Closest point on triangle `abc` to `p` (Ericson, *Real-Time Collision Detection* §5.1.5).
pub(crate) fn closest_point(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Closest {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return Closest {
            point: a,
            on_face: false,
        };
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return Closest {
            point: b,
            on_face: false,
        };
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return Closest {
            point: a + ab * v,
            on_face: false,
        };
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return Closest {
            point: c,
            on_face: false,
        };
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return Closest {
            point: a + ac * w,
            on_face: false,
        };
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return Closest {
            point: b + (c - b) * w,
            on_face: false,
        };
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    Closest {
        point: a + ab * v + ac * w,
        on_face: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_point_regions() {
        let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Z);
        let face = closest_point(Vec3::new(0.2, 1.0, 0.2), a, b, c);
        assert!(face.on_face);
        assert!(face.point.abs_diff_eq(Vec3::new(0.2, 0.0, 0.2), 1e-6));
        let vertex = closest_point(Vec3::new(-1.0, 0.5, -1.0), a, b, c);
        assert_eq!(vertex.point, a);
        assert!(!vertex.on_face);
        let edge = closest_point(Vec3::new(0.5, 0.0, -1.0), a, b, c);
        assert!(edge.point.abs_diff_eq(Vec3::new(0.5, 0.0, 0.0), 1e-6));
    }
}
