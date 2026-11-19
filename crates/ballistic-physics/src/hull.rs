//! Convex hulls of collision meshes, and their mass properties.

use glam::{Mat3, Vec3};

/// A convex polyhedron in its body's frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Hull {
    pub verts: Vec<Vec3>,
    /// Outward normal and vertex loop (indices), counter-clockwise around it.
    pub faces: Vec<(Vec3, Vec<usize>)>,
    pub edges: Vec<(usize, usize)>,
}

impl Hull {
    /// The convex hull of a point set (collision meshes are small, so every
    /// candidate plane is tested). `None` if the points are flat or too few.
    pub fn new(points: &[Vec3]) -> Option<Self> {
        let mut verts: Vec<Vec3> = Vec::new();
        let size = points.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| {
            (lo.min(*p), hi.max(*p))
        });
        let eps = ((size.1 - size.0).max_element() * 1e-4).max(1e-6);
        for p in points {
            if verts.iter().all(|v| v.distance(*p) > eps) {
                verts.push(*p);
            }
        }
        let n = verts.len();
        if n < 4 {
            return None;
        }
        // Planes with every point on one side.
        let mut planes: Vec<(Vec3, f32)> = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                for k in j + 1..n {
                    let Some(normal) = (verts[j] - verts[i])
                        .cross(verts[k] - verts[i])
                        .try_normalize()
                    else {
                        continue;
                    };
                    let d = normal.dot(verts[i]);
                    let (mut above, mut below) = (false, false);
                    for v in &verts {
                        let s = normal.dot(*v) - d;
                        above |= s > eps;
                        below |= s < -eps;
                        if above && below {
                            break;
                        }
                    }
                    let plane = match (above, below) {
                        (false, true) => (normal, d),
                        (true, false) => (-normal, -d),
                        _ => continue,
                    };
                    if planes
                        .iter()
                        .all(|(pn, pd)| pn.dot(plane.0) < 1.0 - 1e-5 || (pd - plane.1).abs() > eps)
                    {
                        planes.push(plane);
                    }
                }
            }
        }
        if planes.len() < 4 {
            return None;
        }
        // Corners lie on three or more hull planes; points inside a face or an
        // edge are left out of the loops.
        let on = |i: usize, (pn, pd): &(Vec3, f32)| (pn.dot(verts[i]) - pd).abs() <= eps;
        let corner: Vec<bool> = (0..n)
            .map(|i| planes.iter().filter(|p| on(i, p)).count() >= 3)
            .collect();
        let mut faces = Vec::with_capacity(planes.len());
        for plane in &planes {
            let normal = plane.0;
            let mut loop_: Vec<usize> = (0..n).filter(|&i| corner[i] && on(i, plane)).collect();
            if loop_.len() < 3 {
                continue;
            }
            let c = loop_.iter().map(|&i| verts[i]).sum::<Vec3>() / loop_.len() as f32;
            let u = (verts[loop_[0]] - c).normalize_or_zero();
            let v = normal.cross(u);
            loop_.sort_by(|&a, &b| {
                let (pa, pb) = (verts[a] - c, verts[b] - c);
                pa.dot(v)
                    .atan2(pa.dot(u))
                    .total_cmp(&pb.dot(v).atan2(pb.dot(u)))
            });
            faces.push((normal, loop_));
        }
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (_, l) in &faces {
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                let e = (a.min(b), a.max(b));
                if !edges.contains(&e) {
                    edges.push(e);
                }
            }
        }
        Some(Self {
            verts,
            faces,
            edges,
        })
    }

    #[must_use]
    pub fn translated(&self, by: Vec3) -> Self {
        Self {
            verts: self.verts.iter().map(|v| *v + by).collect(),
            faces: self.faces.clone(),
            edges: self.edges.clone(),
        }
    }

    /// Volume, first moment (volume × centroid) and second moment
    /// `∫ x xᵀ dV` about the frame origin, for unit density.
    fn moments(&self) -> (f32, Vec3, Mat3) {
        // Tetrahedra from the origin to each face triangle (signed volumes).
        let canonical =
            Mat3::from_cols_array(&[2.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 2.0]) / 120.0;
        let (mut vol, mut first, mut second) = (0.0, Vec3::ZERO, Mat3::ZERO);
        for (_, l) in &self.faces {
            for k in 1..l.len().saturating_sub(1) {
                let (a, b, c) = (self.verts[l[0]], self.verts[l[k]], self.verts[l[k + 1]]);
                let m = Mat3::from_cols(a, b, c);
                let det = m.determinant();
                vol += det / 6.0;
                first += (a + b + c) * (det / 24.0);
                second += m * canonical * m.transpose() * det;
            }
        }
        (vol, first, second)
    }

    /// Radius of the smallest origin-centred sphere containing the hull.
    pub fn radius(&self) -> f32 {
        self.verts.iter().map(|v| v.length()).fold(0.0, f32::max)
    }
}

/// Volume, centroid and inertia tensor (about the frame origin, for `mass`
/// spread uniformly) of a set of hulls.
pub fn mass_properties(hulls: &[Hull], mass: f32) -> (f32, Vec3, Mat3) {
    let (mut vol, mut first, mut second) = (0.0, Vec3::ZERO, Mat3::ZERO);
    for h in hulls {
        let (v, f, s) = h.moments();
        vol += v;
        first += f;
        second += s;
    }
    if vol <= 0.0 {
        return (0.0, Vec3::ZERO, Mat3::ZERO);
    }
    let density = mass / vol;
    let c = second * density;
    let trace = c.x_axis.x + c.y_axis.y + c.z_axis.z;
    let inertia = Mat3::from_diagonal(Vec3::splat(trace)) - c;
    (vol, first / vol, inertia)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(h: f32) -> Vec<Vec3> {
        (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { -h } else { h },
                    if i & 2 == 0 { -h } else { h },
                    if i & 4 == 0 { -h } else { h },
                )
            })
            .collect()
    }

    #[test]
    fn cube_hull_has_six_square_faces() {
        let mut pts = cube(1.0);
        pts.push(Vec3::ZERO); // interior point, dropped
        pts.push(Vec3::new(1.0, 0.0, 0.0)); // on a face, not a vertex of the hull
        let h = Hull::new(&pts).unwrap();
        assert_eq!(h.faces.len(), 6);
        assert_eq!(h.edges.len(), 12);
        for (n, l) in &h.faces {
            assert!(n.abs().max_element() > 0.999, "{n}");
            assert!(l.len() >= 4);
        }
    }

    #[test]
    fn cube_mass_properties() {
        let h = Hull::new(&cube(1.0)).unwrap();
        let (vol, centroid, inertia) = mass_properties(std::slice::from_ref(&h), 3.0);
        assert!((vol - 8.0).abs() < 1e-4);
        assert!(centroid.length() < 1e-5);
        // Solid cube of side 2, mass 3: I = m·(2² + 2²)/12 = 2.
        assert!(
            inertia.abs_diff_eq(Mat3::from_diagonal(Vec3::splat(2.0)), 1e-4),
            "{inertia}"
        );
        // Moved off the origin: parallel axes.
        let (_, c, i) = mass_properties(&[h.translated(Vec3::X * 2.0)], 3.0);
        assert!(c.abs_diff_eq(Vec3::X * 2.0, 1e-5));
        assert!((i.y_axis.y - (2.0 + 3.0 * 4.0)).abs() < 1e-3, "{i}");
    }
}
