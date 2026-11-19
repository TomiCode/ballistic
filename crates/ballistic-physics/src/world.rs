//! The simulation world: sphere bodies against static triangle geometry.

use glam::{Quat, Vec3};

use crate::bvh::Aabb;
use crate::mesh::{StaticMesh, closest_point};
use crate::{STEP_DT, WorldConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PushId(u32);

/// Parameters of a sphere body (the original's Physicalize inputs).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyDesc {
    pub position: Vec3,
    pub radius: f32,
    pub mass: f32,
    pub friction: f32,
    pub elasticity: f32,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    pub group: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub position: Vec3,
    pub orientation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub radius: f32,
    pub mass: f32,
    pub friction: f32,
    pub elasticity: f32,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    pub group: String,
    /// State before the last step (for render interpolation).
    prev_position: Vec3,
    prev_orientation: Quat,
    /// True if the body touched static geometry during the last step.
    pub grounded: bool,
}

impl Body {
    fn inv_mass(&self) -> f32 {
        if self.mass > 0.0 {
            1.0 / self.mass
        } else {
            0.0
        }
    }

    /// Inverse moment of inertia of a solid sphere (2/5·m·r²).
    fn inv_inertia(&self) -> f32 {
        let i = 0.4 * self.mass * self.radius * self.radius;
        if i > 0.0 { 1.0 / i } else { 0.0 }
    }
}

/// A per-step impulse on a body (the original's "force").
#[derive(Debug, Clone, Copy, PartialEq)]
struct Push {
    body: BodyId,
    /// World-space impulse applied every step.
    impulse: Vec3,
}

#[derive(Debug, Clone, Copy)]
struct Contact {
    normal: Vec3,
    depth: f32,
    friction: f32,
    restitution: f32,
}

#[derive(Debug, Clone)]
pub struct World {
    pub config: WorldConfig,
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
        self.bodies.push(Some(Body {
            position: d.position,
            orientation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            radius: d.radius,
            mass: d.mass,
            friction: d.friction,
            elasticity: d.elasticity,
            linear_damping: d.linear_damping,
            rotational_damping: d.rotational_damping,
            group: d.group,
            prev_position: d.position,
            prev_orientation: Quat::IDENTITY,
            grounded: false,
        }));
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

    /// Teleports a body and clears its motion (respawn).
    pub fn reset_body(&mut self, id: BodyId, position: Vec3) {
        if let Some(b) = self.body_mut(id) {
            b.position = position;
            b.prev_position = position;
            b.velocity = Vec3::ZERO;
            b.angular_velocity = Vec3::ZERO;
            b.orientation = Quat::IDENTITY;
            b.prev_orientation = Quat::IDENTITY;
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

        let statics = &self.statics;
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

            // 5. Contacts.
            b.grounded = false;
            for _ in 0..cfg.contact.iterations {
                let contacts = find_contacts(statics, b, &cfg);
                if contacts.is_empty() {
                    break;
                }
                b.grounded = true;
                for c in &contacts {
                    resolve(b, c, &cfg);
                }
            }
        }
        self.steps += 1;
    }
}

/// Contacts of a sphere with static geometry, deduplicated so that a ball on a
/// tessellated surface gets one contact per distinct normal (prevents bumps at
/// internal edges).
fn find_contacts(statics: &StaticMesh, b: &Body, cfg: &WorldConfig) -> Vec<Contact> {
    let r = b.radius;
    let query = Aabb {
        min: b.position - Vec3::splat(r),
        max: b.position + Vec3::splat(r),
    };
    let mut raw: Vec<(Contact, bool)> = Vec::new();
    statics.query(&query, |ti| {
        let t = &statics.triangles[ti as usize];
        let m = &statics.materials[t.material as usize];
        if cfg.contact.same_group_ignores && !m.group.is_empty() && m.group == b.group {
            return;
        }
        let c = closest_point(b.position, t.a, t.b, t.c);
        let d = b.position - c.point;
        let dist2 = d.length_squared();
        if dist2 >= r * r || dist2 < 1e-12 {
            return;
        }
        let dist = dist2.sqrt();
        let normal = d / dist;
        // One-sided: ignore triangles we're behind (their back faces).
        if t.normal.dot(b.position - t.a) < 0.0 && c.on_face {
            return;
        }
        raw.push((
            Contact {
                normal,
                depth: r - dist,
                friction: cfg.contact.friction.apply(b.friction, m.friction),
                restitution: cfg.contact.restitution.apply(b.elasticity, m.elasticity),
            },
            c.on_face,
        ));
    });
    // Faces first, deepest first; then drop contacts with near-duplicate normals.
    raw.sort_by(|(a, fa), (b2, fb)| fb.cmp(fa).then(b2.depth.total_cmp(&a.depth)));
    let mut out: Vec<Contact> = Vec::new();
    for (c, _) in raw {
        if out.iter().all(|o| o.normal.dot(c.normal) < 0.995) {
            out.push(c);
        }
    }
    out
}

fn resolve(b: &mut Body, c: &Contact, cfg: &WorldConfig) {
    let n = c.normal;
    // Position projection.
    b.position += n * (c.depth * cfg.contact.penetration_correction);

    let r = -n * b.radius; // contact point relative to centre
    let v_contact = b.velocity + b.angular_velocity.cross(r);
    let vn = v_contact.dot(n);
    if vn >= 0.0 {
        return;
    }
    let inv_m = b.inv_mass();
    if inv_m == 0.0 {
        return;
    }
    let e = if -vn < cfg.contact.bounce_threshold {
        0.0
    } else {
        c.restitution
    };
    // Normal impulse (r ∥ n, so no angular term).
    let jn = -(1.0 + e) * vn / inv_m;
    b.velocity += n * (jn * inv_m);

    // Coulomb friction at the contact point; couples sliding and spin (rolling).
    let v_contact = b.velocity + b.angular_velocity.cross(r);
    let vt = v_contact - n * v_contact.dot(n);
    let speed = vt.length();
    if speed < 1e-6 {
        return;
    }
    let t = vt / speed;
    let inv_i = b.inv_inertia();
    let k = inv_m + inv_i * r.cross(t).length_squared();
    let jt = (speed / k).min(c.friction * jn);
    let impulse = -t * jt;
    b.velocity += impulse * inv_m;
    b.angular_velocity += r.cross(impulse) * inv_i;
}
