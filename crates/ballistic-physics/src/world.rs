//! The simulation world: dynamic bodies (spheres, boxes, convex hulls) against
//! static triangle geometry and each other, with hinges, sliders and springs.

use std::collections::HashSet;

use glam::{Mat3, Quat, Vec3};

use crate::bvh::Aabb;
use crate::collide::{self, Contact, Point, Poly};
use crate::mesh::{SoundIds, StaticMesh, closest_point};
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
    /// Starts asleep (the original's "Start Frozen"): it doesn't move, and
    /// others collide with it as if it were fixed, until it's woken up.
    pub asleep: bool,
    /// Takes part in collisions (the original's "Enable Collision").
    pub collides: bool,
    /// What the body sounds like when hit or rolled on.
    pub sound: SoundIds,
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
            asleep: false,
            collides: true,
            sound: SoundIds::default(),
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
    /// Inverse inertia tensor (body frame, about the origin = mass centre).
    inv_inertia: Mat3,
    /// Asleep: not moved by the simulation, and immovable for others.
    pub asleep: bool,
    /// Takes part in collisions.
    pub collides: bool,
    pub sound: SoundIds,
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
        if self.mass > 0.0 && !self.asleep {
            1.0 / self.mass
        } else {
            0.0
        }
    }

    /// World-space inverse inertia tensor (zero while asleep).
    fn inv_inertia_world(&self) -> Mat3 {
        if self.asleep {
            return Mat3::ZERO;
        }
        let r = Mat3::from_quat(self.orientation);
        r * self.inv_inertia * r.transpose()
    }

    /// World-space inverse inertia tensor applied to `v`.
    fn inv_inertia_mul(&self, v: Vec3) -> Vec3 {
        if self.asleep {
            return Vec3::ZERO;
        }
        self.orientation * (self.inv_inertia * (self.orientation.inverse() * v))
    }

    /// The shape as convex polyhedra at the current pose (boxes and hulls).
    fn polys(&self) -> Vec<Poly> {
        self.polys_at(self.position, self.orientation)
    }

    fn polys_at(&self, position: Vec3, orientation: Quat) -> Vec<Poly> {
        match &self.shape {
            Shape::Sphere { .. } => Vec::new(),
            Shape::Box { half } => vec![Poly::cuboid(position, orientation, *half)],
            Shape::Hulls(h) => h
                .iter()
                .map(|h| Poly::hull(h, position, orientation))
                .collect(),
        }
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
    /// Where it's applied, in the body's frame.
    point: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JointId {
    Hinge(u32),
    Slider(u32),
    Spring(u32),
}

/// Joint position-correction sweeps per contact pass.
const JOINT_SWEEPS: u32 = 4;

/// A hinge between body `a` and body `b` (or the world): the pivot points of
/// both coincide and their axes stay aligned; rotation about the axis is free.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Hinge {
    a: usize,
    b: Option<usize>,
    /// Pivot in `a`'s frame, and in `b`'s frame (world space without `b`).
    anchor_a: Vec3,
    anchor_b: Vec3,
    /// Axis in `a`'s frame, and in `b`'s frame (world space without `b`).
    axis_a: Vec3,
    axis_b: Vec3,
    /// A ball joint: the pivot only, rotation free about every axis.
    ball: bool,
}

/// A slider (prismatic joint): `a` keeps its orientation relative to `b` (or
/// the world) and moves only along the axis; optionally within limits.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Slider {
    a: usize,
    b: Option<usize>,
    /// The axis origin in `a`'s frame, and in `b`'s frame (world without `b`).
    anchor_a: Vec3,
    anchor_b: Vec3,
    /// Axis in `b`'s frame (world without `b`).
    axis_b: Vec3,
    /// `a`'s orientation relative to `b`'s (to the world without `b`).
    relative: Quat,
    limits: Option<(f32, f32)>,
}

/// A damped spring between a point on `a` and a point on `b` (or the world).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spring {
    a: usize,
    b: Option<usize>,
    anchor_a: Vec3,
    anchor_b: Vec3,
    length: f32,
    constant: f32,
    /// Damping of the anchors' relative velocity along the spring.
    linear_damping: f32,
    /// Damping of the anchors' whole relative velocity.
    global_damping: f32,
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
    /// The static surface's sound, for contacts with static geometry.
    surface_sound: SoundIds,
}

#[derive(Debug, Clone)]
pub struct World {
    pub config: WorldConfig,
    /// Static geometry. Call [`StaticMesh::finish`] after adding to it.
    pub statics: StaticMesh,
    bodies: Vec<Option<Body>>,
    pushes: Vec<Option<Push>>,
    joints: Vec<Option<Hinge>>,
    sliders: Vec<Option<Slider>>,
    springs: Vec<Option<Spring>>,
    /// Simulated seconds not yet stepped.
    accumulator: f32,
    /// Steps taken since creation.
    pub steps: u64,
    /// Bodies that report their collisions.
    listeners: HashSet<usize>,
    /// Pairs in contact at the last step (body, other body or `None`).
    touching: HashSet<(usize, Option<usize>)>,
    collisions: Vec<Collision>,
    /// What each reporting body touched at the last step.
    contacts: std::collections::HashMap<usize, Vec<SoundIds>>,
}

/// A body hitting something (like the original's collision event): a contact
/// pair that wasn't touching the step before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Collision {
    pub body: BodyId,
    /// The other body, or `None` for static geometry.
    pub other: Option<BodyId>,
    /// Relative speed at the contact point (units per simulated second).
    pub speed: f32,
    /// What was hit sounds like.
    pub other_sound: SoundIds,
    pub point: Vec3,
    /// Simulated seconds since the world was created.
    pub time: f64,
}

impl World {
    pub fn new(config: WorldConfig, statics: StaticMesh) -> Self {
        Self {
            config,
            statics,
            bodies: Vec::new(),
            pushes: Vec::new(),
            joints: Vec::new(),
            sliders: Vec::new(),
            springs: Vec::new(),
            accumulator: 0.0,
            steps: 0,
            listeners: HashSet::new(),
            touching: HashSet::new(),
            collisions: Vec::new(),
            contacts: std::collections::HashMap::new(),
        }
    }

    /// What a reporting body touched at the last step (static surfaces and
    /// bodies, one entry per contact pair).
    pub fn touching_sounds(&self, body: BodyId) -> &[SoundIds] {
        self.contacts
            .get(&(body.0 as usize))
            .map_or(&[], Vec::as_slice)
    }

    /// Whether `body` reports its collisions (see [`Self::take_collisions`]).
    pub fn report_collisions(&mut self, body: BodyId, on: bool) {
        if on {
            self.listeners.insert(body.0 as usize);
        } else {
            self.listeners.remove(&(body.0 as usize));
        }
    }

    /// Collisions of reporting bodies since the last call.
    pub fn take_collisions(&mut self) -> Vec<Collision> {
        std::mem::take(&mut self.collisions)
    }

    /// Simulated seconds since the world was created.
    pub fn time(&self) -> f64 {
        self.steps as f64 * f64::from(STEP_DT)
    }

    pub fn add_body(&mut self, d: BodyDesc) -> BodyId {
        let inertia = d.shape.inertia(d.mass);
        let inv_inertia = if inertia.determinant().abs() > 1e-12 {
            inertia.inverse()
        } else {
            Mat3::ZERO
        };
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
            sound: d.sound,
            inv_inertia,
            asleep: d.asleep,
            collides: d.collides,
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
        let i = id.0 as usize;
        self.listeners.remove(&i);
        self.contacts.remove(&i);
        self.touching.retain(|&(a, b)| a != i && b != Some(i));
        for p in &mut self.pushes {
            if p.is_some_and(|p| p.body == id) {
                *p = None;
            }
        }
        for j in &mut self.joints {
            if j.is_some_and(|j| j.a == i || j.b == Some(i)) {
                *j = None;
            }
        }
        for j in &mut self.sliders {
            if j.is_some_and(|j| j.a == i || j.b == Some(i)) {
                *j = None;
            }
        }
        for j in &mut self.springs {
            if j.is_some_and(|j| j.a == i || j.b == Some(i)) {
                *j = None;
            }
        }
    }

    /// Point (world) and orientation of a body, or the world origin.
    fn frame_of(&self, i: Option<usize>) -> Option<(Vec3, Quat)> {
        match i {
            Some(i) => self
                .bodies
                .get(i)?
                .as_ref()
                .map(|b| (b.position, b.orientation)),
            None => Some((Vec3::ZERO, Quat::IDENTITY)),
        }
    }

    /// Adds a slider: `a` moves relative to `b` (or the world) only along the
    /// world axis through `origin`, keeping its current relative orientation.
    /// `limits` bound the displacement along the axis from where it is now.
    pub fn add_slider(
        &mut self,
        a: BodyId,
        b: Option<BodyId>,
        origin: Vec3,
        axis: Vec3,
        limits: Option<(f32, f32)>,
    ) -> Option<JointId> {
        let axis = axis.try_normalize()?;
        let (pa, qa) = self.frame_of(Some(a.0 as usize))?;
        let bi = b.map(|b| b.0 as usize);
        let (pb, qb) = self.frame_of(bi)?;
        let slider = Slider {
            a: a.0 as usize,
            b: bi,
            anchor_a: qa.inverse() * (origin - pa),
            anchor_b: qb.inverse() * (origin - pb),
            axis_b: qb.inverse() * axis,
            relative: qb.inverse() * qa,
            limits,
        };
        Some(JointId::Slider(insert(&mut self.sliders, slider)))
    }

    /// Adds a spring between world points on `a` and on `b` (or the world),
    /// attached where they are now.
    #[allow(clippy::too_many_arguments)]
    pub fn add_spring(
        &mut self,
        a: BodyId,
        b: Option<BodyId>,
        point_a: Vec3,
        point_b: Vec3,
        length: f32,
        constant: f32,
        damping: (f32, f32),
    ) -> Option<JointId> {
        let (pa, qa) = self.frame_of(Some(a.0 as usize))?;
        let bi = b.map(|b| b.0 as usize);
        let (pb, qb) = self.frame_of(bi)?;
        let spring = Spring {
            a: a.0 as usize,
            b: bi,
            anchor_a: qa.inverse() * (point_a - pa),
            anchor_b: qb.inverse() * (point_b - pb),
            length,
            constant,
            linear_damping: damping.0,
            global_damping: damping.1,
        };
        Some(JointId::Spring(insert(&mut self.springs, spring)))
    }

    /// Wakes a sleeping body (the original's `Physics WakeUp`).
    pub fn wake(&mut self, id: BodyId) {
        if let Some(b) = self.body_mut(id) {
            b.asleep = false;
        }
    }

    /// Adds a hinge at a world pivot about a world axis, between `a` and `b`
    /// (or the world), in their current poses.
    pub fn add_hinge(
        &mut self,
        a: BodyId,
        b: Option<BodyId>,
        pivot: Vec3,
        axis: Vec3,
    ) -> Option<JointId> {
        let axis = axis.try_normalize()?;
        let local = |i: usize| {
            let body = self.bodies.get(i)?.as_ref()?;
            let inv = body.orientation.inverse();
            Some((inv * (pivot - body.position), inv * axis))
        };
        let (anchor_a, axis_a) = local(a.0 as usize)?;
        let (anchor_b, axis_b) = match b {
            Some(b) => local(b.0 as usize)?,
            None => (pivot, axis),
        };
        let hinge = Hinge {
            a: a.0 as usize,
            b: b.map(|b| b.0 as usize),
            anchor_a,
            anchor_b,
            axis_a,
            axis_b,
            ball: false,
        };
        Some(JointId::Hinge(insert(&mut self.joints, hinge)))
    }

    /// Adds a ball joint at a world pivot between `a` and `b` (or the world).
    pub fn add_ball_joint(&mut self, a: BodyId, b: Option<BodyId>, pivot: Vec3) -> Option<JointId> {
        let id = self.add_hinge(a, b, pivot, Vec3::Y)?;
        if let JointId::Hinge(i) = id
            && let Some(Some(j)) = self.joints.get_mut(i as usize)
        {
            j.ball = true;
        }
        Some(id)
    }

    pub fn remove_joint(&mut self, id: JointId) {
        match id {
            JointId::Hinge(i) => remove(&mut self.joints, i),
            JointId::Slider(i) => remove(&mut self.sliders, i),
            JointId::Spring(i) => remove(&mut self.springs, i),
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
        let point = self.body(body).map_or(Vec3::ZERO, |b| b.position);
        self.add_push_at(body, direction, magnitude, point)
    }

    /// Like [`Self::add_push`], applied at a world point (fixed on the body
    /// from now on): off the mass centre, it also turns the body.
    pub fn add_push_at(
        &mut self,
        body: BodyId,
        direction: Vec3,
        magnitude: f32,
        point: Vec3,
    ) -> PushId {
        let dir = direction.try_normalize().unwrap_or(Vec3::X);
        let point = self.body(body).map_or(Vec3::ZERO, |b| {
            b.orientation.inverse() * (point - b.position)
        });
        let push = Push {
            body,
            impulse: dir * magnitude,
            point,
        };
        if let Some(slot) = self.pushes.iter().position(Option::is_none) {
            self.pushes[slot] = Some(push);
            return PushId(slot as u32);
        }
        self.pushes.push(Some(push));
        PushId(self.pushes.len() as u32 - 1)
    }

    /// A one-off impulse (the original's *Physics Impulse*): the velocity
    /// change of one step of a push of `magnitude` along `direction`
    /// (normalised), at a world point.
    pub fn apply_impulse_at(&mut self, body: BodyId, direction: Vec3, magnitude: f32, point: Vec3) {
        let impulse = direction.try_normalize().unwrap_or(Vec3::X) * magnitude;
        if let Some(Some(b)) = self.bodies.get_mut(body.0 as usize) {
            b.velocity += impulse * b.inv_mass();
            let r = point - b.position;
            b.angular_velocity += b.inv_inertia_mul(r.cross(impulse));
        }
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

        // 1. Pushes (per-step impulses, no Δt factor), and springs.
        for p in self.pushes.iter().flatten() {
            if let Some(Some(b)) = self.bodies.get_mut(p.body.0 as usize) {
                b.velocity += p.impulse * b.inv_mass();
                if p.point != Vec3::ZERO {
                    let r = b.orientation * p.point;
                    b.angular_velocity += b.inv_inertia_mul(r.cross(p.impulse));
                }
            }
        }
        self.apply_springs(dt);

        for b in self.bodies.iter_mut().flatten() {
            b.prev_position = b.position;
            b.prev_orientation = b.orientation;
            b.grounded = false;
            if b.asleep {
                continue;
            }

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
        }

        // 5. Contacts and joints: detect, push apart, solve velocities; repeat.
        let has_joints =
            self.joints.iter().any(Option::is_some) || self.sliders.iter().any(Option::is_some);
        for pass in 0..cfg.contact.iterations {
            let manifolds = self.detect();
            if pass == 0 {
                self.record_collisions(&manifolds);
            }
            if manifolds.is_empty() && !has_joints {
                break;
            }
            self.wake_touched(&manifolds);
            for m in &manifolds {
                if m.b.is_none()
                    && let Some(b) = self.bodies[m.a].as_mut()
                {
                    b.grounded = true;
                }
            }
            // Chains (plank bridges) need several sweeps to close up.
            for _ in 0..JOINT_SWEEPS {
                self.project_joints();
                self.project_sliders();
            }
            self.project(&manifolds);
            self.solve_velocities(&manifolds);
        }
        self.steps += 1;
    }

    /// New contact pairs of reporting bodies become collisions.
    fn record_collisions(&mut self, manifolds: &[Manifold]) {
        let mut now = HashSet::new();
        let time = self.time();
        let sound_of = |bodies: &[Option<Body>], i: Option<usize>, surface: SoundIds| {
            i.map_or(surface, |i| {
                bodies[i].as_ref().map_or(SoundIds::default(), |b| b.sound)
            })
        };
        self.contacts.clear();
        for m in manifolds {
            let key = match m.b {
                Some(b) => (m.a.min(b), Some(m.a.max(b))),
                None => (m.a, None),
            };
            let ends = [Some(m.a), m.b];
            if !ends.iter().flatten().any(|i| self.listeners.contains(i)) {
                now.insert(key);
                continue;
            }
            let first_of_pair = now.insert(key);
            // What each reporting end touches (once per pair).
            if first_of_pair {
                for (me, other) in [(Some(m.a), m.b), (m.b, Some(m.a))] {
                    if let Some(me) = me
                        && self.listeners.contains(&me)
                    {
                        let s = sound_of(&self.bodies, other, m.surface_sound);
                        self.contacts.entry(me).or_default().push(s);
                    }
                }
            }
            if !first_of_pair || self.touching.contains(&key) {
                continue;
            }
            let Some(p) = m.contact.points.first().map(|p| p.pos) else {
                continue;
            };
            let point_velocity = |i: Option<usize>| {
                i.and_then(|i| self.bodies[i].as_ref())
                    .map_or(Vec3::ZERO, |b| {
                        b.velocity + b.angular_velocity.cross(p - b.position)
                    })
            };
            let speed = (point_velocity(Some(m.a)) - point_velocity(m.b)).length();
            for (me, other) in [(Some(m.a), m.b), (m.b, Some(m.a))] {
                if let Some(me) = me
                    && self.listeners.contains(&me)
                {
                    self.collisions.push(Collision {
                        body: BodyId(me as u32),
                        other: other.map(|o| BodyId(o as u32)),
                        speed,
                        other_sound: sound_of(&self.bodies, other, m.surface_sound),
                        point: p,
                        time,
                    });
                }
            }
        }
        self.touching = now;
    }

    /// A sleeping body touched by an awake one, or hinged to one, wakes up.
    fn wake_touched(&mut self, manifolds: &[Manifold]) {
        let awake = |bodies: &[Option<Body>], i: usize| {
            bodies[i]
                .as_ref()
                .is_some_and(|b| !b.asleep && b.mass > 0.0)
        };
        let mut wake = Vec::new();
        let pairs = manifolds
            .iter()
            .filter_map(|m| Some((m.a, m.b?)))
            .chain(
                self.joints
                    .iter()
                    .flatten()
                    .filter_map(|j| Some((j.a, j.b?))),
            )
            .chain(
                self.sliders
                    .iter()
                    .flatten()
                    .filter_map(|j| Some((j.a, j.b?))),
            )
            .chain(
                self.springs
                    .iter()
                    .flatten()
                    .filter_map(|j| Some((j.a, j.b?))),
            );
        for (a, b) in pairs {
            match (awake(&self.bodies, a), awake(&self.bodies, b)) {
                (true, false) => wake.push(b),
                (false, true) => wake.push(a),
                _ => {}
            }
        }
        for i in wake {
            if let Some(b) = self.bodies[i].as_mut() {
                b.asleep = false;
            }
        }
    }

    /// Spring forces, as impulses over `dt`, at the anchors.
    fn apply_springs(&mut self, dt: f32) {
        let mut deltas = Vec::new();
        for s in self.springs.iter().flatten() {
            let Some(a) = self.bodies[s.a].as_ref() else {
                continue;
            };
            let b = s.b.and_then(|b| self.bodies[b].as_ref());
            let pa = a.position + a.orientation * s.anchor_a;
            let pb = b.map_or(s.anchor_b, |b| b.position + b.orientation * s.anchor_b);
            let (ra, rb) = (pa - a.position, b.map_or(Vec3::ZERO, |b| pb - b.position));
            let va = a.velocity + a.angular_velocity.cross(ra);
            let vb = b.map_or(Vec3::ZERO, |b| b.velocity + b.angular_velocity.cross(rb));
            let rel = vb - va;
            let d = pb - pa;
            let dist = d.length();
            let n = if dist > 1e-6 { d / dist } else { Vec3::ZERO };
            // Pulls a towards b when stretched, and damps their relative motion.
            let force = n * (s.constant * (dist - s.length))
                + n * (s.linear_damping * rel.dot(n))
                + rel * s.global_damping;
            let j = force * dt;
            deltas.push((s.a, j * a.inv_mass(), a.inv_inertia_mul(ra.cross(j))));
            if let (Some(bi), Some(b)) = (s.b, b) {
                deltas.push((bi, -j * b.inv_mass(), -b.inv_inertia_mul(rb.cross(j))));
            }
        }
        for (i, dv, dw) in deltas {
            if let Some(b) = self.bodies[i].as_mut() {
                b.velocity += dv;
                b.angular_velocity += dw;
            }
        }
    }

    /// World axis origin on `a` and on `b`, the world axis, and the target
    /// orientation of `a`.
    fn slider_frames(&self, s: &Slider) -> Option<(Vec3, Vec3, Vec3, Quat)> {
        let a = self.bodies[s.a].as_ref()?;
        let (pb0, qb) = self.frame_of(s.b)?;
        Some((
            a.position + a.orientation * s.anchor_a,
            pb0 + qb * s.anchor_b,
            qb * s.axis_b,
            qb * s.relative,
        ))
    }

    /// Position projection of sliders: restore the relative orientation, then
    /// move back onto the axis (and inside the limits), by inverse mass.
    fn project_sliders(&mut self) {
        let k = self.config.contact.penetration_correction;
        for si in 0..self.sliders.len() {
            let Some(s) = self.sliders[si] else { continue };
            let ima = self.bodies[s.a].as_ref().map_or(0.0, Body::inv_mass);
            let imb =
                s.b.and_then(|b| self.bodies[b].as_ref())
                    .map_or(0.0, Body::inv_mass);
            let total = ima + imb;
            if total <= 0.0 {
                continue;
            }
            let (wa, wb) = (ima / total, imb / total);
            let Some((_, _, _, target)) = self.slider_frames(&s) else {
                continue;
            };
            let Some(qa) = self.bodies[s.a].as_ref().map(|a| a.orientation) else {
                continue;
            };
            let q = (target * qa.inverse()).normalize();
            if let Some(a) = self.bodies[s.a].as_mut() {
                a.orientation = (Quat::IDENTITY.slerp(q, wa * k) * a.orientation).normalize();
            }
            if let Some(b) = s.b.and_then(|b| self.bodies[b].as_mut()) {
                b.orientation =
                    (Quat::IDENTITY.slerp(q.inverse(), wb * k) * b.orientation).normalize();
            }
            let Some((pa, pb, x, _)) = self.slider_frames(&s) else {
                continue;
            };
            let d = pa - pb;
            let along = d.dot(x);
            let mut err = d - x * along;
            if let Some((lo, hi)) = s.limits {
                if along < lo {
                    err += x * (along - lo);
                } else if along > hi {
                    err += x * (along - hi);
                }
            }
            if let Some(a) = self.bodies[s.a].as_mut() {
                a.position -= err * (k * wa);
            }
            if let Some(b) = s.b.and_then(|b| self.bodies[b].as_mut()) {
                b.position += err * (k * wb);
            }
        }
    }

    /// World pivots of `a` and `b`, and the world axes.
    fn hinge_frames(&self, j: &Hinge) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
        let a = self.bodies[j.a].as_ref()?;
        let pa = a.position + a.orientation * j.anchor_a;
        let xa = a.orientation * j.axis_a;
        let (pb, xb) = match j.b {
            Some(bi) => {
                let b = self.bodies[bi].as_ref()?;
                (
                    b.position + b.orientation * j.anchor_b,
                    b.orientation * j.axis_b,
                )
            }
            None => (j.anchor_b, j.axis_b),
        };
        Some((pa, pb, xa, xb))
    }

    /// Position projection of joints: align the axes, then join the pivots,
    /// split between the bodies by inverse mass.
    fn project_joints(&mut self) {
        let k = self.config.contact.penetration_correction;
        for ji in 0..self.joints.len() {
            let Some(j) = self.joints[ji] else { continue };
            let ima = self.bodies[j.a].as_ref().map_or(0.0, Body::inv_mass);
            let imb =
                j.b.and_then(|b| self.bodies[b].as_ref())
                    .map_or(0.0, Body::inv_mass);
            let total = ima + imb;
            if total <= 0.0 {
                continue;
            }
            let (wa, wb) = (ima / total, imb / total);
            let Some((_, _, xa, xb)) = self.hinge_frames(&j) else {
                continue;
            };
            let q = if j.ball {
                Quat::IDENTITY
            } else {
                Quat::from_rotation_arc(xa, xb)
            };
            if let Some(a) = self.bodies[j.a].as_mut() {
                a.orientation = (Quat::IDENTITY.slerp(q, wa * k) * a.orientation).normalize();
            }
            if let Some(b) = j.b.and_then(|b| self.bodies[b].as_mut()) {
                b.orientation =
                    (Quat::IDENTITY.slerp(q.inverse(), wb * k) * b.orientation).normalize();
            }
            // Pivots: a position impulse through the full effective mass, so
            // a hinged plank turns about its other pivot instead of sliding.
            let Some((pa, pb, _, _)) = self.hinge_frames(&j) else {
                continue;
            };
            let err = (pa - pb) * k;
            let a = self.bodies[j.a].as_ref();
            let b = j.b.and_then(|b| self.bodies[b].as_ref());
            let Some(a) = a else { continue };
            let ra = pa - a.position;
            let rb = b.map_or(Vec3::ZERO, |b| pb - b.position);
            let (ia, ib) = (
                a.inv_inertia_world(),
                b.map_or(Mat3::ZERO, Body::inv_inertia_world),
            );
            let kmat = Mat3::from_diagonal(Vec3::splat(ima + imb))
                - skew(ra) * ia * skew(ra)
                - skew(rb) * ib * skew(rb);
            if kmat.determinant().abs() < 1e-12 {
                continue;
            }
            let p = kmat.inverse() * -err;
            if let Some(a) = self.bodies[j.a].as_mut() {
                a.position += p * ima;
                let turn = ia * ra.cross(p);
                a.orientation = (Quat::from_scaled_axis(turn) * a.orientation).normalize();
            }
            if let Some(b) = j.b.and_then(|b| self.bodies[b].as_mut()) {
                b.position -= p * imb;
                let turn = ib * rb.cross(p);
                b.orientation = (Quat::from_scaled_axis(-turn) * b.orientation).normalize();
            }
        }
    }

    fn detect(&self) -> Vec<Manifold> {
        let cfg = &self.config;
        let mut out = Vec::new();
        for (i, slot) in self.bodies.iter().enumerate() {
            let Some(b) = slot else { continue };
            if b.asleep || !b.collides {
                continue;
            }
            let contacts = match &b.shape {
                Shape::Sphere { radius } => sphere_statics(&self.statics, b, *radius, cfg),
                Shape::Box { .. } | Shape::Hulls(_) => poly_statics(&self.statics, b, cfg),
            };
            for (contact, material) in contacts {
                let m = &self.statics.materials[material as usize];
                out.push(Manifold {
                    a: i,
                    b: None,
                    contact,
                    friction: cfg.contact.friction.apply(b.friction, m.friction),
                    restitution: cfg.contact.restitution.apply(b.elasticity, m.elasticity),
                    surface_sound: m.sound,
                });
            }
        }
        for i in 0..self.bodies.len() {
            let Some(a) = &self.bodies[i] else { continue };
            let box_a = a.aabb();
            for (j, slot) in self.bodies.iter().enumerate().skip(i + 1) {
                let Some(b) = slot else { continue };
                if !a.collides
                    || !b.collides
                    || (a.asleep && b.asleep)
                    || (cfg.contact.same_group_ignores && !a.group.is_empty() && a.group == b.group)
                {
                    continue;
                }
                if !box_a.overlaps(&b.aabb()) {
                    continue;
                }
                for contact in body_body(a, b) {
                    out.push(Manifold {
                        a: i,
                        b: Some(j),
                        contact,
                        friction: cfg.contact.friction.apply(a.friction, b.friction),
                        restitution: cfg.contact.restitution.apply(a.elasticity, b.elasticity),
                        surface_sound: SoundIds::default(),
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
        /// A hinge: world pivots and axes, effective masses.
        struct JointRow {
            a: usize,
            b: Option<usize>,
            ra: Vec3,
            rb: Vec3,
            /// Inverse of the 3×3 effective mass of the pivot constraint.
            k_inv: Mat3,
            /// Two directions perpendicular to the axis, and their effective masses.
            t: [Vec3; 2],
            kt: [f32; 2],
        }
        /// A slider: the axis origin on `a` as the constraint point.
        struct SliderRow {
            a: usize,
            b: Option<usize>,
            ra: Vec3,
            rb: Vec3,
            /// Inverse of the 3×3 effective mass of the relative rotation.
            k_rot_inv: Mat3,
            /// Directions across the axis and their effective masses.
            t: [Vec3; 2],
            kt: [f32; 2],
            axis: Vec3,
            kx: f32,
            /// -1 below the lower limit, +1 above the upper, else 0.
            limit: f32,
        }
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

        let mut jrows = Vec::new();
        for j in self.joints.iter().flatten() {
            let Some((pa, _, xa, _)) = self.hinge_frames(j) else {
                continue;
            };
            let Some(a) = get(j.a) else { continue };
            let b = j.b.and_then(get);
            let ra = pa - a.position;
            let rb = b.map_or(Vec3::ZERO, |b| pa - b.position);
            let (ia, ib) = (
                a.inv_inertia_world(),
                b.map_or(Mat3::ZERO, Body::inv_inertia_world),
            );
            let (ma, mb) = (a.inv_mass(), b.map_or(0.0, Body::inv_mass));
            let k = Mat3::from_diagonal(Vec3::splat(ma + mb))
                - skew(ra) * ia * skew(ra)
                - skew(rb) * ib * skew(rb);
            if k.determinant().abs() < 1e-12 {
                continue;
            }
            let (t1, t2) = xa.any_orthonormal_pair();
            // A ball joint has no axis rows (zero effective mass skips them).
            let kt = if j.ball {
                [0.0; 2]
            } else {
                [t1, t2].map(|t| t.dot(ia * t) + t.dot(ib * t))
            };
            jrows.push(JointRow {
                a: j.a,
                b: j.b,
                ra,
                rb,
                k_inv: k.inverse(),
                t: [t1, t2],
                kt,
            });
        }
        let mut srows = Vec::new();
        for s in self.sliders.iter().flatten() {
            let Some((pa, pb, x, _)) = self.slider_frames(s) else {
                continue;
            };
            let Some(a) = get(s.a) else { continue };
            let b = s.b.and_then(get);
            let ra = pa - a.position;
            let rb = b.map_or(Vec3::ZERO, |b| pa - b.position);
            let k_rot = a.inv_inertia_world() + b.map_or(Mat3::ZERO, Body::inv_inertia_world);
            let k_rot_inv = if k_rot.determinant().abs() > 1e-12 {
                k_rot.inverse()
            } else {
                Mat3::ZERO
            };
            let (t1, t2) = x.any_orthonormal_pair();
            let kt = [t1, t2].map(|t| eff_mass(Some(s.a), ra, t) + eff_mass(s.b, rb, t));
            let along = (pa - pb).dot(x);
            let limit = match s.limits {
                Some((lo, _)) if along < lo => -1.0,
                Some((_, hi)) if along > hi => 1.0,
                _ => 0.0,
            };
            srows.push(SliderRow {
                a: s.a,
                b: s.b,
                ra,
                rb,
                k_rot_inv,
                t: [t1, t2],
                kt,
                axis: x,
                kx: eff_mass(Some(s.a), ra, x) + eff_mass(s.b, rb, x),
                limit,
            });
        }
        let apply_link = |vel: &mut [(Vec3, Vec3)],
                          (ai, bi, ra, rb): (usize, Option<usize>, Vec3, Vec3),
                          linear: Vec3,
                          angular: Vec3| {
            if let Some(a) = get(ai) {
                vel[ai].0 += linear * a.inv_mass();
                vel[ai].1 += a.inv_inertia_mul(ra.cross(linear) + angular);
            }
            if let Some(bi) = bi
                && let Some(b) = get(bi)
            {
                vel[bi].0 -= linear * b.inv_mass();
                vel[bi].1 -= b.inv_inertia_mul(rb.cross(linear) + angular);
            }
        };
        let apply_joint = |vel: &mut [(Vec3, Vec3)], r: &JointRow, linear: Vec3, angular: Vec3| {
            apply_link(vel, (r.a, r.b, r.ra, r.rb), linear, angular);
        };

        for _ in 0..cfg.velocity_iterations.max(1) {
            for r in &srows {
                let link = (r.a, r.b, r.ra, r.rb);
                // No relative rotation.
                let dw = vel[r.a].1 - r.b.map_or(Vec3::ZERO, |b| vel[b].1);
                apply_link(&mut vel, link, Vec3::ZERO, r.k_rot_inv * -dw);
                // No motion across the axis.
                for (t, kt) in r.t.iter().zip(r.kt) {
                    if kt <= 0.0 {
                        continue;
                    }
                    let dv =
                        point_velocity(&vel, Some(r.a), r.ra) - point_velocity(&vel, r.b, r.rb);
                    apply_link(&mut vel, link, *t * (-dv.dot(*t) / kt), Vec3::ZERO);
                }
                // Past a limit: no further motion out of range.
                if r.limit != 0.0 && r.kx > 0.0 {
                    let dv =
                        point_velocity(&vel, Some(r.a), r.ra) - point_velocity(&vel, r.b, r.rb);
                    let vx = dv.dot(r.axis);
                    if vx * r.limit > 0.0 {
                        apply_link(&mut vel, link, r.axis * (-vx / r.kx), Vec3::ZERO);
                    }
                }
            }
            for r in &jrows {
                // Pivot: equal point velocities.
                let dv = point_velocity(&vel, Some(r.a), r.ra) - point_velocity(&vel, r.b, r.rb);
                apply_joint(&mut vel, r, r.k_inv * -dv, Vec3::ZERO);
                // Axis: no relative rotation across it.
                for (t, kt) in r.t.iter().zip(r.kt) {
                    if kt <= 0.0 {
                        continue;
                    }
                    let dw = vel[r.a].1 - r.b.map_or(Vec3::ZERO, |b| vel[b].1);
                    apply_joint(&mut vel, r, Vec3::ZERO, *t * (-dw.dot(*t) / kt));
                }
            }
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
            if let Some(b) = slot
                && !b.asleep
            {
                b.velocity = v;
                b.angular_velocity = w;
            }
        }
    }
}

/// The cross-product matrix: `skew(r) * v == r.cross(v)`.
fn skew(r: Vec3) -> Mat3 {
    Mat3::from_cols(
        Vec3::new(0.0, r.z, -r.y),
        Vec3::new(-r.z, 0.0, r.x),
        Vec3::new(r.y, -r.x, 0.0),
    )
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
        if !statics.material_enabled(t.material)
            || ignores(cfg, &b.group, &statics.materials[t.material as usize].group)
        {
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

/// How far a box or hull may already be behind a one-sided level triangle's
/// plane at the start of a step and still collide with it. A real hit from
/// the front gets that deep only by not being pushed out in one step.
const BEHIND: f32 = 0.5;

/// Contacts of a box or hulls with static geometry: one manifold per distinct
/// normal, with the points of every triangle sharing it (a box on a floor made
/// of two triangles gets its four corners).
fn poly_statics(statics: &StaticMesh, b: &Body, cfg: &WorldConfig) -> Vec<(Contact, u32)> {
    let polys = b.polys();
    // Where the shape was at the start of the step.
    let before = b.polys_at(b.prev_position, b.prev_orientation);
    let mut merged: Vec<(Contact, u32)> = Vec::new();
    statics.query(&b.aabb(), |ti| {
        let t = &statics.triangles[ti as usize];
        if !statics.material_enabled(t.material)
            || ignores(cfg, &b.group, &statics.materials[t.material as usize].group)
        {
            return;
        }
        let tri = Poly::triangle(t.a, t.b, t.c, t.normal);
        for (poly, before) in polys.iter().zip(&before) {
            // Already well behind the triangle's plane before this step:
            // it's on the other side, not hitting the face (a plank landing
            // on a shelf, behind the bevel under the shelf's lip, was pushed
            // down through the shelf by it).
            if !poly.in_front_of(&tri) || before.depth_behind(&tri) > BEHIND {
                continue;
            }
            let Some(c) = collide::poly_poly(poly, &tri) else {
                continue;
            };
            merge(&mut merged, c, t.material);
        }
    });
    for (c, _) in &mut merged {
        c.points = collide::reduce(std::mem::take(&mut c.points));
    }
    merged.sort_by(|(a, _), (b, _)| b.max_depth().total_cmp(&a.max_depth()));
    merged
}

/// Adds a contact to the manifold with the same normal, or as a new one.
fn merge<T: Copy>(merged: &mut Vec<(Contact, T)>, c: Contact, tag: T) {
    if let Some((m, _)) = merged
        .iter_mut()
        .find(|(m, _)| m.normal.dot(c.normal) >= 0.995)
    {
        m.points.extend(c.points);
    } else {
        merged.push((c, tag));
    }
}

/// Contacts between two bodies; the normals push `a` out of `b`.
fn body_body(a: &Body, b: &Body) -> Vec<Contact> {
    let sphere_vs = |center: Vec3, radius: f32, other: &Body| -> Option<Contact> {
        match &other.shape {
            Shape::Sphere { radius: r } => {
                collide::sphere_sphere(center, radius, other.position, *r)
            }
            Shape::Box { half } => {
                collide::sphere_box(center, radius, other.position, other.orientation, *half)
            }
            Shape::Hulls(_) => other
                .polys()
                .iter()
                .filter_map(|p| collide::sphere_poly(center, radius, p))
                .max_by(|x, y| x.max_depth().total_cmp(&y.max_depth())),
        }
    };
    match (&a.shape, &b.shape) {
        (Shape::Sphere { radius }, _) => sphere_vs(a.position, *radius, b).into_iter().collect(),
        (_, Shape::Sphere { radius }) => sphere_vs(b.position, *radius, a)
            .map(Contact::flipped)
            .into_iter()
            .collect(),
        _ => {
            let mut merged: Vec<(Contact, ())> = Vec::new();
            let pb = b.polys();
            for pa in a.polys() {
                for q in &pb {
                    if let Some(c) = collide::poly_poly(&pa, q) {
                        merge(&mut merged, c, ());
                    }
                }
            }
            merged
                .into_iter()
                .map(|(mut c, ())| {
                    c.points = collide::reduce(c.points);
                    c
                })
                .collect()
        }
    }
}

/// Stores a joint in the first free slot; returns its index.
fn insert<T>(slots: &mut Vec<Option<T>>, value: T) -> u32 {
    if let Some(i) = slots.iter().position(Option::is_none) {
        slots[i] = Some(value);
        return i as u32;
    }
    slots.push(Some(value));
    slots.len() as u32 - 1
}

fn remove<T>(slots: &mut [Option<T>], i: u32) {
    if let Some(s) = slots.get_mut(i as usize) {
        *s = None;
    }
}
