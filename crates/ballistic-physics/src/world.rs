//! The simulation world: dynamic bodies (spheres, boxes) against static
//! triangle geometry and each other.

use glam::{Quat, Vec3};

use crate::bvh::Aabb;
use crate::collide::{self, Contact, Point, Poly};
use crate::mesh::{StaticMesh, closest_point};
use crate::shape::Shape;
use crate::{STEP_DT, WorldConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PushId(u32);

/// Parameters of a dynamic body (the original's Physicalize inputs).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyDesc {
    pub position: Vec3,
    pub orientation: Quat,
    pub shape: Shape,
    pub mass: f32,
    pub friction: f32,
    pub elasticity: f32,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    pub group: String,
}

impl Default for BodyDesc {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            shape: Shape::Sphere { radius: 1.0 },
            mass: 1.0,
            friction: 0.5,
            elasticity: 0.0,
            linear_damping: 0.0,
            rotational_damping: 0.0,
            group: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub position: Vec3,
    pub orientation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub shape: Shape,
    pub mass: f32,
    pub friction: f32,
    pub elasticity: f32,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    pub group: String,
    /// Inverse principal moments of inertia (body frame).
    inv_inertia: Vec3,
    /// State before the last step (for render interpolation).
    prev_position: Vec3,
    prev_orientation: Quat,
    /// True if the body touched static geometry during the last step.
    pub grounded: bool,
}

impl Body {
    /// Bounding radius of the shape (the radius, for a sphere).
    pub fn radius(&self) -> f32 {
        self.shape.bounding_radius()
    }

    fn inv_mass(&self) -> f32 {
        if self.mass > 0.0 {
            1.0 / self.mass
        } else {
            0.0
        }
    }

    /// World-space inverse inertia tensor applied to `v`.
    fn inv_inertia_mul(&self, v: Vec3) -> Vec3 {
        self.orientation * (self.inv_inertia * (self.orientation.inverse() * v))
    }

    fn aabb(&self) -> Aabb {
        self.shape.aabb(self.position, self.orientation)
    }
}

/// A per-step impulse on a body (the original's "force").
#[derive(Debug, Clone, Copy, PartialEq)]
struct Push {
    body: BodyId,
    /// World-space impulse applied every step.
    impulse: Vec3,
}

/// Contacts of body `a` with static geometry (`b` = `None`) or with body `b`.
/// The normal pushes `a` out of `b`.
#[derive(Debug, Clone)]
struct Manifold {
    a: usize,
    b: Option<usize>,
    contact: Contact,
    friction: f32,
    restitution: f32,
}

#[derive(Debug, Clone)]
pub struct World {
    pub config: WorldConfig,
    /// Static geometry. Call [`StaticMesh::finish`] after adding to it.
    pub statics: StaticMesh,
    bodies: Vec<Option<Body>>,
    pushes: Vec<Option<Push>>,
    /// Simulated seconds not yet stepped.
    accumulator: f32,
    /// Steps taken since creation.
    pub steps: u64,
}

impl World {
    pub fn new(config: WorldConfig, statics: StaticMesh) -> Self {
        Self {
            config,
            statics,
            bodies: Vec::new(),
            pushes: Vec::new(),
            accumulator: 0.0,
            steps: 0,
        }
    }

    pub fn add_body(&mut self, d: BodyDesc) -> BodyId {
        let inertia = d.shape.inertia(d.mass);
        let inv_inertia = Vec3::select(inertia.cmpgt(Vec3::ZERO), inertia.recip(), Vec3::ZERO);
        let body = Body {
            position: d.position,
            orientation: d.orientation,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            shape: d.shape,
            mass: d.mass,
            friction: d.friction,
            elasticity: d.elasticity,
            linear_damping: d.linear_damping,
            rotational_damping: d.rotational_damping,
            group: d.group,
            inv_inertia,
            prev_position: d.position,
            prev_orientation: d.orientation,
            grounded: false,
        };
        if let Some(slot) = self.bodies.iter().position(Option::is_none) {
            self.bodies[slot] = Some(body);
            return BodyId(slot as u32);
        }
        self.bodies.push(Some(body));
        BodyId(self.bodies.len() as u32 - 1)
    }

    pub fn remove_body(&mut self, id: BodyId) {
        if let Some(b) = self.bodies.get_mut(id.0 as usize) {
            *b = None;
        }
        for p in &mut self.pushes {
            if p.is_some_and(|p| p.body == id) {
                *p = None;
            }
        }
    }

    pub fn body(&self, id: BodyId) -> Option<&Body> {
        self.bodies.get(id.0 as usize).and_then(Option::as_ref)
    }

    pub fn body_mut(&mut self, id: BodyId) -> Option<&mut Body> {
        self.bodies.get_mut(id.0 as usize).and_then(Option::as_mut)
    }

    /// Teleports a body upright and clears its motion (respawn).
    pub fn reset_body(&mut self, id: BodyId, position: Vec3) {
        self.set_pose(id, position, Quat::IDENTITY);
    }

    /// Teleports a body to a pose and clears its motion.
    pub fn set_pose(&mut self, id: BodyId, position: Vec3, orientation: Quat) {
        if let Some(b) = self.body_mut(id) {
            b.position = position;
            b.prev_position = position;
            b.orientation = orientation;
            b.prev_orientation = orientation;
            b.velocity = Vec3::ZERO;
            b.angular_velocity = Vec3::ZERO;
        }
    }

    /// Starts a per-step push of `magnitude` along `direction` (world space,
    /// normalised here, captured now: later camera changes don't affect it).
    pub fn add_push(&mut self, body: BodyId, direction: Vec3, magnitude: f32) -> PushId {
        let dir = direction.try_normalize().unwrap_or(Vec3::X);
        let push = Push {
            body,
            impulse: dir * magnitude,
        };
        if let Some(slot) = self.pushes.iter().position(Option::is_none) {
            self.pushes[slot] = Some(push);
            return PushId(slot as u32);
        }
        self.pushes.push(Some(push));
        PushId(self.pushes.len() as u32 - 1)
    }

    pub fn remove_push(&mut self, id: PushId) {
        if let Some(p) = self.pushes.get_mut(id.0 as usize) {
            *p = None;
        }
    }

    /// Advances by `real_dt` seconds of real time. Returns the number of steps taken.
    pub fn advance(&mut self, real_dt: f32) -> u32 {
        self.accumulator += real_dt.max(0.0) * self.config.time_factor;
        // Avoid a spiral of death after long pauses (at most 1 s of simulation).
        self.accumulator = self.accumulator.min(1.0);
        let mut n = 0;
        while self.accumulator >= STEP_DT {
            self.step();
            self.accumulator -= STEP_DT;
            n += 1;
        }
        n
    }

    /// Fraction of a step accumulated but not yet simulated (for interpolation).
    pub fn alpha(&self) -> f32 {
        (self.accumulator / STEP_DT).clamp(0.0, 1.0)
    }

    /// Render pose between the last two steps.
    pub fn interpolated(&self, id: BodyId) -> Option<(Vec3, Quat)> {
        let b = self.body(id)?;
        let t = self.alpha();
        Some((
            b.prev_position.lerp(b.position, t),
            b.prev_orientation.slerp(b.orientation, t),
        ))
    }

    /// One fixed step of 1/66 s simulated time.
    pub fn step(&mut self) {
        let dt = STEP_DT;
        let cfg = self.config;

        // 1. Pushes (per-step impulses, no Δt factor).
        for p in self.pushes.iter().flatten() {
            if let Some(Some(b)) = self.bodies.get_mut(p.body.0 as usize) {
                b.velocity += p.impulse * b.inv_mass();
            }
        }

        for b in self.bodies.iter_mut().flatten() {
            b.prev_position = b.position;
            b.prev_orientation = b.orientation;

            // 2. Gravity, 3. damping.
            b.velocity += cfg.gravity * dt;
            b.velocity *= cfg.contact.damping.factor(b.linear_damping, dt);
            b.angular_velocity *= cfg.contact.damping.factor(b.rotational_damping, dt);

            // 4. Integrate.
            b.position += b.velocity * dt;
            let w = b.angular_velocity;
            if w.length_squared() > 0.0 {
                let dq = Quat::from_xyzw(w.x, w.y, w.z, 0.0) * b.orientation;
                b.orientation = (b.orientation + dq * (0.5 * dt)).normalize();
            }
            b.grounded = false;
        }

        // 5. Contacts: detect, push apart, solve velocities; repeat.
        for _ in 0..cfg.contact.iterations {
            let manifolds = self.detect();
            if manifolds.is_empty() {
                break;
            }
            for m in &manifolds {
                if m.b.is_none()
                    && let Some(b) = self.bodies[m.a].as_mut()
                {
                    b.grounded = true;
                }
            }
            self.project(&manifolds);
            self.solve_velocities(&manifolds);
        }
        self.steps += 1;
    }

    fn detect(&self) -> Vec<Manifold> {
        let cfg = &self.config;
        let mut out = Vec::new();
        for (i, slot) in self.bodies.iter().enumerate() {
            let Some(b) = slot else { continue };
            let contacts = match b.shape {
                Shape::Sphere { radius } => sphere_statics(&self.statics, b, radius, cfg),
                Shape::Box { half } => box_statics(&self.statics, b, half, cfg),
            };
            for (contact, material) in contacts {
                let m = &self.statics.materials[material as usize];
                out.push(Manifold {
                    a: i,
                    b: None,
                    contact,
                    friction: cfg.contact.friction.apply(b.friction, m.friction),
                    restitution: cfg.contact.restitution.apply(b.elasticity, m.elasticity),
                });
            }
        }
        for i in 0..self.bodies.len() {
            let Some(a) = &self.bodies[i] else { continue };
            let box_a = a.aabb();
            for (j, slot) in self.bodies.iter().enumerate().skip(i + 1) {
                let Some(b) = slot else { continue };
                if cfg.contact.same_group_ignores && !a.group.is_empty() && a.group == b.group {
                    continue;
                }
                if !box_a.overlaps(&b.aabb()) {
                    continue;
                }
                if let Some(contact) = body_body(a, b) {
                    out.push(Manifold {
                        a: i,
                        b: Some(j),
                        contact,
                        friction: cfg.contact.friction.apply(a.friction, b.friction),
                        restitution: cfg.contact.restitution.apply(a.elasticity, b.elasticity),
                    });
                }
            }
        }
        out
    }

    /// Position projection: a fraction of each manifold's deepest penetration,
    /// split between the bodies by inverse mass.
    fn project(&mut self, manifolds: &[Manifold]) {
        let k = self.config.contact.penetration_correction;
        for m in manifolds {
            let ima = self.bodies[m.a].as_ref().map_or(0.0, Body::inv_mass);
            let imb =
                m.b.and_then(|b| self.bodies[b].as_ref())
                    .map_or(0.0, Body::inv_mass);
            let total = ima + imb;
            if total <= 0.0 {
                continue;
            }
            let shift = m.contact.normal * (m.contact.max_depth() * k / total);
            if let Some(a) = self.bodies[m.a].as_mut() {
                a.position += shift * ima;
            }
            if let Some(b) = m.b.and_then(|b| self.bodies[b].as_mut()) {
                b.position -= shift * imb;
            }
        }
    }

    /// Sequential impulses with accumulated, clamped normal and friction
    /// impulses per contact point. With one contact point this is a single
    /// normal impulse (with restitution above the bounce threshold) followed by
    /// Coulomb friction at the contact point, which spins a ball up (rolling).
    #[allow(
        clippy::too_many_lines,
        reason = "one solver: rows, iterations, write-back"
    )]
    fn solve_velocities(&mut self, manifolds: &[Manifold]) {
        struct Row {
            m: usize,
            ra: Vec3,
            rb: Vec3,
            kn: f32,
            target: f32,
            acc_n: f32,
            acc_t: Vec3,
        }
        let cfg = self.config.contact;
        let bodies = &self.bodies;
        let get = |i: usize| bodies[i].as_ref();
        // Velocities are solved on a copy, then written back.
        let mut vel: Vec<(Vec3, Vec3)> = bodies
            .iter()
            .map(|b| {
                b.as_ref().map_or((Vec3::ZERO, Vec3::ZERO), |b| {
                    (b.velocity, b.angular_velocity)
                })
            })
            .collect();
        let point_velocity = |vel: &[(Vec3, Vec3)], i: Option<usize>, r: Vec3| {
            i.map_or(Vec3::ZERO, |i| vel[i].0 + vel[i].1.cross(r))
        };
        let eff_mass = |i: Option<usize>, r: Vec3, d: Vec3| {
            i.and_then(get).map_or(0.0, |b| {
                b.inv_mass() + d.dot(b.inv_inertia_mul(r.cross(d)).cross(r))
            })
        };

        let mut rows = Vec::new();
        for (mi, m) in manifolds.iter().enumerate() {
            let n = m.contact.normal;
            let pa = get(m.a).map_or(Vec3::ZERO, |b| b.position);
            let pb = m.b.and_then(get).map_or(Vec3::ZERO, |b| b.position);
            for p in &m.contact.points {
                let ra = p.pos - pa;
                let rb = p.pos - pb;
                let kn = eff_mass(Some(m.a), ra, n) + eff_mass(m.b, rb, n);
                if kn <= 0.0 {
                    continue;
                }
                let vn =
                    (point_velocity(&vel, Some(m.a), ra) - point_velocity(&vel, m.b, rb)).dot(n);
                let target = if -vn < cfg.bounce_threshold {
                    0.0
                } else {
                    -m.restitution * vn
                };
                rows.push(Row {
                    m: mi,
                    ra,
                    rb,
                    kn,
                    target,
                    acc_n: 0.0,
                    acc_t: Vec3::ZERO,
                });
            }
        }

        let apply = |vel: &mut [(Vec3, Vec3)], m: &Manifold, row: &Row, j: Vec3| {
            if let Some(a) = get(m.a) {
                vel[m.a].0 += j * a.inv_mass();
                vel[m.a].1 += a.inv_inertia_mul(row.ra.cross(j));
            }
            if let Some(bi) = m.b
                && let Some(b) = get(bi)
            {
                vel[bi].0 -= j * b.inv_mass();
                vel[bi].1 -= b.inv_inertia_mul(row.rb.cross(j));
            }
        };

        for _ in 0..cfg.velocity_iterations.max(1) {
            for row in &mut rows {
                let m = &manifolds[row.m];
                let n = m.contact.normal;
                let rel = |vel: &[(Vec3, Vec3)]| {
                    point_velocity(vel, Some(m.a), row.ra) - point_velocity(vel, m.b, row.rb)
                };
                // Normal.
                let vn = rel(&vel).dot(n);
                let acc = (row.acc_n + (row.target - vn) / row.kn).max(0.0);
                let dj = acc - row.acc_n;
                row.acc_n = acc;
                apply(&mut vel, m, row, n * dj);

                // Friction (Coulomb cone, accumulated).
                let v = rel(&vel);
                let vt = v - n * v.dot(n);
                let speed = vt.length();
                if speed < 1e-6 {
                    continue;
                }
                let t = vt / speed;
                let kt = eff_mass(Some(m.a), row.ra, t) + eff_mass(m.b, row.rb, t);
                if kt <= 0.0 {
                    continue;
                }
                let mut acc_t = row.acc_t - t * (speed / kt);
                let max = m.friction * row.acc_n;
                if acc_t.length() > max {
                    acc_t = acc_t.normalize_or_zero() * max;
                }
                let dj = acc_t - row.acc_t;
                row.acc_t = acc_t;
                apply(&mut vel, m, row, dj);
            }
        }

        for (slot, (v, w)) in self.bodies.iter_mut().zip(vel) {
            if let Some(b) = slot {
                b.velocity = v;
                b.angular_velocity = w;
            }
        }
    }
}

fn ignores(cfg: &WorldConfig, body_group: &str, surface_group: &str) -> bool {
    cfg.contact.same_group_ignores && !surface_group.is_empty() && surface_group == body_group
}

/// Contacts of a sphere with static geometry, deduplicated so that a ball on a
/// tessellated surface gets one contact per distinct normal (prevents bumps at
/// internal edges).
fn sphere_statics(
    statics: &StaticMesh,
    b: &Body,
    r: f32,
    cfg: &WorldConfig,
) -> Vec<(Contact, u32)> {
    let mut raw: Vec<(Contact, u32, bool)> = Vec::new();
    statics.query(&b.aabb(), |ti| {
        let t = &statics.triangles[ti as usize];
        if ignores(cfg, &b.group, &statics.materials[t.material as usize].group) {
            return;
        }
        let c = closest_point(b.position, t.a, t.b, t.c);
        let d = b.position - c.point;
        let dist2 = d.length_squared();
        if dist2 >= r * r || dist2 < 1e-12 {
            return;
        }
        // One-sided: ignore triangles we're behind (their back faces).
        if t.normal.dot(b.position - t.a) < 0.0 && c.on_face {
            return;
        }
        let dist = dist2.sqrt();
        let normal = d / dist;
        raw.push((
            Contact {
                normal,
                points: vec![Point {
                    pos: b.position - normal * r,
                    depth: r - dist,
                }],
            },
            t.material,
            c.on_face,
        ));
    });
    // Faces first, deepest first; then drop contacts with near-duplicate normals.
    raw.sort_by(|(a, _, fa), (b2, _, fb)| {
        fb.cmp(fa).then(b2.max_depth().total_cmp(&a.max_depth()))
    });
    let mut out: Vec<(Contact, u32)> = Vec::new();
    for (c, m, _) in raw {
        if out.iter().all(|(o, _)| o.normal.dot(c.normal) < 0.995) {
            out.push((c, m));
        }
    }
    out
}

/// Contacts of a box with static geometry: one manifold per distinct normal,
/// with the points of every triangle sharing it (a box on a floor made of two
/// triangles gets its four corners).
fn box_statics(
    statics: &StaticMesh,
    b: &Body,
    half: Vec3,
    cfg: &WorldConfig,
) -> Vec<(Contact, u32)> {
    let poly = Poly::cuboid(b.position, b.orientation, half);
    let mut merged: Vec<(Contact, u32)> = Vec::new();
    statics.query(&b.aabb(), |ti| {
        let t = &statics.triangles[ti as usize];
        if ignores(cfg, &b.group, &statics.materials[t.material as usize].group) {
            return;
        }
        let tri = Poly::triangle(t.a, t.b, t.c, t.normal);
        if !poly.in_front_of(&tri) {
            return;
        }
        let Some(c) = collide::poly_poly(&poly, &tri) else {
            return;
        };
        if let Some((m, _)) = merged
            .iter_mut()
            .find(|(m, _)| m.normal.dot(c.normal) >= 0.995)
        {
            m.points.extend(c.points);
        } else {
            merged.push((c, t.material));
        }
    });
    for (c, _) in &mut merged {
        c.points = collide::reduce(std::mem::take(&mut c.points));
    }
    merged.sort_by(|(a, _), (b, _)| b.max_depth().total_cmp(&a.max_depth()));
    merged
}

/// Contact between two bodies; the normal pushes `a` out of `b`.
fn body_body(a: &Body, b: &Body) -> Option<Contact> {
    match (a.shape, b.shape) {
        (Shape::Sphere { radius: ra }, Shape::Sphere { radius: rb }) => {
            collide::sphere_sphere(a.position, ra, b.position, rb)
        }
        (Shape::Sphere { radius }, Shape::Box { half }) => {
            collide::sphere_box(a.position, radius, b.position, b.orientation, half)
        }
        (Shape::Box { half }, Shape::Sphere { radius }) => {
            collide::sphere_box(b.position, radius, a.position, a.orientation, half)
                .map(Contact::flipped)
        }
        (Shape::Box { half: ha }, Shape::Box { half: hb }) => collide::poly_poly(
            &Poly::cuboid(a.position, a.orientation, ha),
            &Poly::cuboid(b.position, b.orientation, hb),
        ),
    }
}
