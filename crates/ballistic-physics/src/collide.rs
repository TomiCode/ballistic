//! Narrow phase: contact points between pairs of shapes.
//!
//! Every function returns contacts for a pair (first, second) with the normal
//! pointing from the second shape to the first: moving the first shape along
//! the normal by `depth` separates them.
//!
//! Spheres use closest points. Boxes and triangles are convex polyhedra and use
//! the separating-axis test, then clip the incident face against the reference
//! face (or take the closest points of two edges) to get up to four points.

use glam::{Mat3, Quat, Vec3};

use crate::hull::Hull;
use crate::mesh::closest_point;

/// One contact point (world space) and its penetration depth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub pos: Vec3,
    pub depth: f32,
}

/// Contact points of one pair along one normal.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Contact {
    pub normal: Vec3,
    pub points: Vec<Point>,
}

impl Contact {
    pub fn max_depth(&self) -> f32 {
        self.points.iter().map(|p| p.depth).fold(0.0, f32::max)
    }

    pub fn flipped(mut self) -> Self {
        self.normal = -self.normal;
        self
    }
}

pub(crate) fn sphere_sphere(pa: Vec3, ra: f32, pb: Vec3, rb: f32) -> Option<Contact> {
    let d = pa - pb;
    let dist2 = d.length_squared();
    let r = ra + rb;
    if dist2 >= r * r {
        return None;
    }
    let dist = dist2.sqrt();
    let normal = if dist > 1e-6 { d / dist } else { Vec3::Y };
    Some(Contact {
        normal,
        points: vec![Point {
            pos: pb + normal * rb,
            depth: r - dist,
        }],
    })
}

/// A sphere (first) against a box (second).
pub(crate) fn sphere_box(
    ps: Vec3,
    radius: f32,
    pb: Vec3,
    rot: Quat,
    half: Vec3,
) -> Option<Contact> {
    let m = Mat3::from_quat(rot);
    let local = m.transpose() * (ps - pb);
    let q = local.clamp(-half, half);
    let d = local - q;
    let dist2 = d.length_squared();
    if dist2 > 1e-12 {
        if dist2 >= radius * radius {
            return None;
        }
        let dist = dist2.sqrt();
        return Some(Contact {
            normal: m * (d / dist),
            points: vec![Point {
                pos: pb + m * q,
                depth: radius - dist,
            }],
        });
    }
    // Centre inside the box: out through the nearest face.
    let gap = half - local.abs();
    let axis = if gap.x <= gap.y && gap.x <= gap.z {
        0
    } else if gap.y <= gap.z {
        1
    } else {
        2
    };
    let mut n = Vec3::ZERO;
    n[axis] = if local[axis] >= 0.0 { 1.0 } else { -1.0 };
    let mut on_face = local;
    on_face[axis] = half[axis] * n[axis];
    Some(Contact {
        normal: m * n,
        points: vec![Point {
            pos: pb + m * on_face,
            depth: radius + gap[axis],
        }],
    })
}

/// A sphere (first) against a convex polyhedron (second).
pub(crate) fn sphere_poly(center: Vec3, radius: f32, poly: &Poly) -> Option<Contact> {
    let (sep, face) = poly.max_separation(center);
    if sep >= radius {
        return None;
    }
    if sep <= 0.0 {
        // Centre inside: out through the nearest face.
        let n = poly.faces[face].0;
        return Some(Contact {
            normal: n,
            points: vec![Point {
                pos: center - n * sep,
                depth: radius - sep,
            }],
        });
    }
    // Outside: the closest point on the surface (faces as triangle fans).
    let mut best: Option<(f32, Vec3)> = None;
    for (_, l) in &poly.faces {
        for k in 1..l.len().saturating_sub(1) {
            let q = closest_point(
                center,
                poly.verts[l[0]],
                poly.verts[l[k]],
                poly.verts[l[k + 1]],
            )
            .point;
            let d2 = center.distance_squared(q);
            if best.is_none_or(|(b, _)| d2 < b) {
                best = Some((d2, q));
            }
        }
    }
    let (d2, q) = best?;
    if d2 >= radius * radius || d2 < 1e-12 {
        return None;
    }
    let dist = d2.sqrt();
    Some(Contact {
        normal: (center - q) / dist,
        points: vec![Point {
            pos: q,
            depth: radius - dist,
        }],
    })
}

/// A convex polyhedron in world space.
#[derive(Debug, Clone)]
pub(crate) struct Poly {
    verts: Vec<Vec3>,
    /// Outward normal and vertex loop, counter-clockwise around the normal.
    faces: Vec<(Vec3, Vec<usize>)>,
    edges: Vec<(usize, usize)>,
    /// One-sided (a level triangle): only this side may push the other shape out.
    one_sided: Option<Vec3>,
}

impl Poly {
    pub fn cuboid(pos: Vec3, rot: Quat, half: Vec3) -> Self {
        let m = Mat3::from_quat(rot);
        let corner = |i: usize| -> Vec3 {
            let s = Vec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            s * half
        };
        let verts: Vec<Vec3> = (0..8).map(|i| pos + m * corner(i)).collect();
        let mut faces = Vec::with_capacity(6);
        for axis in 0..3 {
            for side in [0usize, 1] {
                let bit = 1 << axis;
                let mut loop_: Vec<usize> =
                    (0..8).filter(|i| (i & bit != 0) == (side == 1)).collect();
                let mut ln = Vec3::ZERO;
                ln[axis] = if side == 1 { 1.0 } else { -1.0 };
                // Order the four corners counter-clockwise around the normal.
                let u = Vec3::new(ln.y.abs(), ln.z.abs(), ln.x.abs());
                let v = ln.cross(u);
                loop_.sort_by(|&a, &b| {
                    let (ca, cb) = (corner(a), corner(b));
                    ca.dot(v)
                        .atan2(ca.dot(u))
                        .total_cmp(&cb.dot(v).atan2(cb.dot(u)))
                });
                faces.push((m * ln, loop_));
            }
        }
        let edges = (0..8usize)
            .flat_map(|i| [1usize, 2, 4].map(move |bit| (i, i ^ bit)))
            .filter(|(i, j)| i < j)
            .collect();
        Self {
            verts,
            faces,
            edges,
            one_sided: None,
        }
    }

    /// A convex hull (body frame) placed at a pose.
    pub fn hull(h: &Hull, pos: Vec3, rot: Quat) -> Self {
        let m = Mat3::from_quat(rot);
        Self {
            verts: h.verts.iter().map(|v| pos + m * *v).collect(),
            faces: h.faces.iter().map(|(n, l)| (m * *n, l.clone())).collect(),
            edges: h.edges.clone(),
            one_sided: None,
        }
    }

    /// Signed distance of `p` to each face plane; the largest, and its face.
    fn max_separation(&self, p: Vec3) -> (f32, usize) {
        self.faces
            .iter()
            .enumerate()
            .map(|(i, (n, l))| (n.dot(p - self.verts[l[0]]), i))
            .fold((f32::MIN, 0), |a, b| if b.0 > a.0 { b } else { a })
    }

    /// A level triangle, counter-clockwise around `normal`.
    pub fn triangle(a: Vec3, b: Vec3, c: Vec3, normal: Vec3) -> Self {
        Self {
            verts: vec![a, b, c],
            faces: vec![(normal, vec![0, 1, 2])],
            edges: vec![(0, 1), (1, 2), (2, 0)],
            one_sided: Some(normal),
        }
    }

    fn project(&self, axis: Vec3) -> (f32, f32) {
        self.verts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            let d = v.dot(axis);
            (lo.min(d), hi.max(d))
        })
    }

    fn center(&self) -> Vec3 {
        self.verts.iter().copied().sum::<Vec3>() / self.verts.len() as f32
    }

    fn face_points(&self, f: usize) -> Vec<Vec3> {
        self.faces[f].1.iter().map(|&i| self.verts[i]).collect()
    }

    fn edge(&self, e: usize) -> (Vec3, Vec3) {
        let (i, j) = self.edges[e];
        (self.verts[i], self.verts[j])
    }
}

#[derive(Debug, Clone, Copy)]
enum Feature {
    FaceA(usize),
    FaceB(usize),
    Edges,
}

#[derive(Debug, Clone, Copy)]
struct Axis {
    depth: f32,
    normal: Vec3,
    feature: Feature,
}

/// Penetration of `a` along `normal` (moving `a` by `normal · depth` separates
/// the projections), or `None` if the shapes are separated on this axis.
fn overlap(a: &Poly, b: &Poly, axis: Vec3) -> Option<(f32, f32)> {
    let (amin, amax) = a.project(axis);
    let (bmin, bmax) = b.project(axis);
    let up = bmax - amin;
    let down = amax - bmin;
    (up > 0.0 && down > 0.0).then_some((up, down))
}

/// Tolerances that prefer face contacts over edge contacts, and the second
/// shape's faces (level triangles) over the first's, when depths are close.
/// This keeps boxes from catching on the internal edges of a tessellated floor.
const FACE_REL: f32 = 0.98;
const EDGE_REL: f32 = 0.95;
const ABS_TOL: f32 = 0.005;

/// Contact of convex polyhedron `a` against convex polyhedron `b`.
pub(crate) fn poly_poly(a: &Poly, b: &Poly) -> Option<Contact> {
    let allowed = |n: Vec3| b.one_sided.is_none_or(|s| n.dot(s) >= -1e-4);
    let mut best: Option<Axis> = None;

    // Faces of `b`: `a` leaves along the outward normal.
    for (i, &(n, _)) in b.faces.iter().enumerate() {
        let (up, _) = overlap(a, b, n)?;
        if best.is_none_or(|bst| up < bst.depth) {
            best = Some(Axis {
                depth: up,
                normal: n,
                feature: Feature::FaceB(i),
            });
        }
    }
    // Faces of `a`: `a` leaves against its own outward normal.
    for (i, &(n, _)) in a.faces.iter().enumerate() {
        let (_, down) = overlap(a, b, n)?;
        if allowed(-n) && best.is_none_or(|bst| down < bst.depth * FACE_REL - ABS_TOL) {
            best = Some(Axis {
                depth: down,
                normal: -n,
                feature: Feature::FaceA(i),
            });
        }
    }
    let face_depth = best.map_or(f32::MAX, |b| b.depth);
    let mut edge_best: Option<Axis> = None;
    for &(a0, a1) in &a.edges {
        let da = a.verts[a1] - a.verts[a0];
        for &(b0, b1) in &b.edges {
            let db = b.verts[b1] - b.verts[b0];
            let Some(l) = da.cross(db).try_normalize() else {
                continue;
            };
            let (up, down) = overlap(a, b, l)?;
            for (depth, n) in [(up, l), (down, -l)] {
                if allowed(n) && edge_best.is_none_or(|e| depth < e.depth) {
                    edge_best = Some(Axis {
                        depth,
                        normal: n,
                        feature: Feature::Edges,
                    });
                }
            }
        }
    }
    if let Some(e) = edge_best
        && e.depth < face_depth * EDGE_REL - ABS_TOL
    {
        best = Some(e);
    }
    let best = best?;

    let points = match best.feature {
        Feature::FaceB(f) => clip_contacts(b, f, a),
        Feature::FaceA(f) => clip_contacts(a, f, b),
        Feature::Edges => edge_contact(a, b, best.normal, best.depth),
    };
    (!points.is_empty()).then(|| Contact {
        normal: best.normal,
        points: reduce(points),
    })
}

/// Points of `inc`'s face most opposed to `reference`'s face `f`, clipped to
/// that face's sides and kept where they are behind it.
fn clip_contacts(reference: &Poly, f: usize, inc: &Poly) -> Vec<Point> {
    let (nr, ref_loop) = &reference.faces[f];
    let inc_face = (0..inc.faces.len())
        .min_by(|&x, &y| inc.faces[x].0.dot(*nr).total_cmp(&inc.faces[y].0.dot(*nr)))
        .unwrap_or(0);
    let mut poly = inc.face_points(inc_face);
    let n = ref_loop.len();
    for k in 0..n {
        let p0 = reference.verts[ref_loop[k]];
        let p1 = reference.verts[ref_loop[(k + 1) % n]];
        let Some(side) = (p1 - p0).cross(*nr).try_normalize() else {
            continue;
        };
        poly = clip(&poly, side, p0);
        if poly.is_empty() {
            return Vec::new();
        }
    }
    let origin = reference.verts[ref_loop[0]];
    poly.into_iter()
        .filter_map(|p| {
            let s = (p - origin).dot(*nr);
            (s < 0.0).then_some(Point { pos: p, depth: -s })
        })
        .collect()
}

/// Sutherland–Hodgman: keeps the part of a polygon where `(x − p)·n ≤ 0`.
fn clip(poly: &[Vec3], n: Vec3, p: Vec3) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (i, &a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        let da = (a - p).dot(n);
        let db = (b - p).dot(n);
        if da <= 0.0 {
            out.push(a);
        }
        if (da <= 0.0) != (db <= 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// Edge–edge contact: the closest points of the two supporting edges.
fn edge_contact(a: &Poly, b: &Poly, normal: Vec3, depth: f32) -> Vec<Point> {
    // The supporting edges are the ones furthest into the other shape along the
    // normal; pick among the edges perpendicular to it.
    let pick = |p: &Poly, sign: f32| {
        (0..p.edges.len())
            .filter(|&e| {
                let (e0, e1) = p.edge(e);
                (e1 - e0).normalize_or_zero().dot(normal).abs() < 0.05
            })
            .min_by(|&x, &y| {
                let mx = {
                    let (e0, e1) = p.edge(x);
                    (e0 + e1).dot(normal) * sign
                };
                let my = {
                    let (e0, e1) = p.edge(y);
                    (e0 + e1).dot(normal) * sign
                };
                mx.total_cmp(&my)
            })
    };
    let (Some(ea), Some(eb)) = (pick(a, 1.0), pick(b, -1.0)) else {
        return Vec::new();
    };
    let (a0, a1) = a.edge(ea);
    let (b0, b1) = b.edge(eb);
    let (pa, pb) = closest_segments(a0, a1, b0, b1);
    vec![Point {
        pos: (pa + pb) * 0.5,
        depth,
    }]
}

/// Closest points of segments `p0p1` and `q0q1` (Ericson §5.1.9).
fn closest_segments(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> (Vec3, Vec3) {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let r = p0 - q0;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= 1e-12 && e <= 1e-12 {
        return (p0, q0);
    }
    if a <= 1e-12 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > 1e-12 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    (p0 + d1 * s, q0 + d2 * t)
}

/// Keeps at most four points: the deepest, then each next one furthest from
/// those already kept (spreads the support).
pub(crate) fn reduce(mut points: Vec<Point>) -> Vec<Point> {
    // Drop near-duplicates (shared triangle edges give the same corner twice).
    let mut unique: Vec<Point> = Vec::with_capacity(points.len());
    points.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    for p in points {
        if unique.iter().all(|u| u.pos.distance_squared(p.pos) > 1e-4) {
            unique.push(p);
        }
    }
    if unique.len() <= 4 {
        return unique;
    }
    let mut kept = vec![unique.remove(0)];
    while kept.len() < 4 && !unique.is_empty() {
        let far = (0..unique.len())
            .max_by(|&x, &y| {
                let dx = kept
                    .iter()
                    .map(|k| k.pos.distance_squared(unique[x].pos))
                    .fold(f32::MAX, f32::min);
                let dy = kept
                    .iter()
                    .map(|k| k.pos.distance_squared(unique[y].pos))
                    .fold(f32::MAX, f32::min);
                dx.total_cmp(&dy)
            })
            .unwrap_or(0);
        kept.push(unique.remove(far));
    }
    kept
}

impl Poly {
    /// True if the polyhedron's centre is in front of a one-sided face.
    pub fn in_front_of(&self, other: &Poly) -> bool {
        other
            .one_sided
            .is_none_or(|n| (self.center() - other.verts[0]).dot(n) >= 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor_tri() -> Poly {
        // Large triangle in y = 0, counter-clockwise from above.
        Poly::triangle(
            Vec3::new(-50.0, 0.0, -50.0),
            Vec3::new(-50.0, 0.0, 50.0),
            Vec3::new(50.0, 0.0, 0.0),
            Vec3::Y,
        )
    }

    #[test]
    fn box_resting_on_triangle_has_four_points() {
        let b = Poly::cuboid(Vec3::new(0.0, 0.9, 0.0), Quat::IDENTITY, Vec3::ONE);
        let c = poly_poly(&b, &floor_tri()).expect("contact");
        assert!(c.normal.abs_diff_eq(Vec3::Y, 1e-5), "{}", c.normal);
        assert_eq!(c.points.len(), 4);
        for p in &c.points {
            assert!((p.depth - 0.1).abs() < 1e-4, "{p:?}");
        }
    }

    #[test]
    fn separated_box_has_no_contact() {
        let b = Poly::cuboid(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, Vec3::ONE);
        assert!(poly_poly(&b, &floor_tri()).is_none());
    }

    #[test]
    fn box_on_its_edge_touches_along_a_line() {
        let rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let y = std::f32::consts::SQRT_2 - 0.05;
        let b = Poly::cuboid(Vec3::new(0.0, y, 0.0), rot, Vec3::ONE);
        let c = poly_poly(&b, &floor_tri()).expect("contact");
        assert!(c.normal.abs_diff_eq(Vec3::Y, 1e-4), "{}", c.normal);
        assert_eq!(c.points.len(), 2, "{:?}", c.points);
    }

    #[test]
    fn stacked_boxes() {
        let lower = Poly::cuboid(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        let upper = Poly::cuboid(Vec3::new(0.3, 1.95, 0.0), Quat::IDENTITY, Vec3::ONE);
        let c = poly_poly(&upper, &lower).expect("contact");
        assert!(c.normal.abs_diff_eq(Vec3::Y, 1e-5), "{}", c.normal);
        assert_eq!(c.points.len(), 4);
    }

    #[test]
    fn sphere_against_box_face_and_corner() {
        let c = sphere_box(
            Vec3::new(0.0, 1.5, 0.0),
            1.0,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ONE,
        )
        .unwrap();
        assert!(c.normal.abs_diff_eq(Vec3::Y, 1e-6));
        assert!((c.points[0].depth - 0.5).abs() < 1e-6);
        let corner = Vec3::splat(1.0 + 0.5 / 3f32.sqrt());
        let c = sphere_box(corner, 1.0, Vec3::ZERO, Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!((c.points[0].depth - 0.5).abs() < 1e-5);
    }

    #[test]
    fn one_sided_triangle_never_pushes_down() {
        // A box mostly below the floor plane is pushed up, not through.
        let b = Poly::cuboid(Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY, Vec3::ONE);
        let c = poly_poly(&b, &floor_tri()).expect("contact");
        assert!(c.normal.y > 0.99, "{}", c.normal);
    }

    #[test]
    fn sphere_against_hull() {
        let cube: Vec<Vec3> = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                )
            })
            .collect();
        let h = Hull::new(&cube).unwrap();
        let p = Poly::hull(&h, Vec3::ZERO, Quat::IDENTITY);
        let c = sphere_poly(Vec3::new(0.0, 1.5, 0.0), 1.0, &p).unwrap();
        assert!(c.normal.abs_diff_eq(Vec3::Y, 1e-5));
        assert!((c.points[0].depth - 0.5).abs() < 1e-5);
        let corner = Vec3::splat(1.0 + 0.5 / 3f32.sqrt());
        let c = sphere_poly(corner, 1.0, &p).unwrap();
        assert!((c.points[0].depth - 0.5).abs() < 1e-4);
        assert!(sphere_poly(Vec3::new(0.0, 2.5, 0.0), 1.0, &p).is_none());
        // A hull on a floor triangle: four corner points.
        let floor = Poly::triangle(
            Vec3::new(-50.0, -1.1, -50.0),
            Vec3::new(-50.0, -1.1, 50.0),
            Vec3::new(50.0, -1.1, 0.0),
            Vec3::Y,
        );
        let p = Poly::hull(&h, Vec3::new(0.0, -0.0, 0.0), Quat::IDENTITY);
        assert!(poly_poly(&p, &floor).is_none());
        let p = Poly::hull(&h, Vec3::new(0.0, -0.2, 0.0), Quat::IDENTITY);
        assert_eq!(poly_poly(&p, &floor).unwrap().points.len(), 4);
    }
}
