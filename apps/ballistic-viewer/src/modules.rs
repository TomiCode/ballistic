//! Moving elements in play mode (`P_Modul_*`), placed like the original's
//! level init and run like their prefab scripts (research findings 13):
//!
//! - Modules are always drawn (level init shows every prefab), but only
//!   physical while their sector is active. Activation physicalizes the bodies
//!   (frozen ones asleep), creates the hinges and arms the wake-up proximity
//!   test; deactivation removes them and restores the initial pose.
//! - Scripted forces, as their scripts switch them (research findings 13):
//!   the swing (08) pushes one way for 500 ms, rests 500 ms, pushes the other
//!   way, rests, and so on; the punching bag (26) alternates every 1500 ms.
//!   Creating the first force wakes the module each time. The fan (18):
//!   within 80 of it (on X/Z) an inner test runs; while the ball is within 7,
//!   each check tests the ball's box against the fan's box, and a hit switches
//!   an upward push on the ball on, a miss switches it off.
//! - The level end's balloon (`PE_Balloon`, research findings 12, 13) is a
//!   module of the last sector: when the ball comes within 70 its platform
//!   wakes (and with it everything joined to it), and the balloons pull up.
//!   At the finish the slide box is pushed away, and the hinge between the
//!   platform and the bridge breaks.

use anyhow::{Context, Result};
use ballistic_bundle::{ModuleSetup, PhysicsData, Scene};
use ballistic_physics::modules::PlannedForce;
use ballistic_physics::modules::{self, ModulePlan};
use ballistic_physics::{
    BodyId, JointId, Mat4, PushId, Quat, SurfaceMaterial, Vec3, World, boxes_overlap, props,
};
use ballistic_render::{Camera, Gpu, RenderOptions, SceneRenderer};

use crate::play::Bundle;
use crate::progress::{Edge, Proximity};
use crate::props::append_prefab;

/// A drawn object: its index in the merged scene, the plan body it moves
/// with (if any), and its matrix relative to that body's initial pose (or
/// its world matrix, if static).
struct Part {
    object: usize,
    body: Option<usize>,
    local: Mat4,
}

/// Two forces switched on in turn (`Delayer` chains in the swing's and the
/// bag's scripts): `first` for `on`, nothing for `gap`, `second` for `on`,
/// nothing for `gap`, again.
#[derive(Debug, Clone)]
struct Alternate {
    first: usize,
    second: usize,
    on_ms: f32,
    gap_ms: f32,
    phase: u8,
    left_ms: f32,
    push: Option<PushId>,
}

/// The fan's script (`P_Modul_18_MF Script` and its *Physics Force* graph).
#[derive(Debug, Clone)]
struct Fan {
    outer_template: Proximity,
    inner_template: Proximity,
    outer: Proximity,
    inner: Proximity,
    /// What both tests measure to (`P_Modul_18_Particle`).
    target: Vec3,
    /// The fan's box (`P_Modul_18_Kollisionsquader`): centre, orientation,
    /// half extents.
    area: (Vec3, Quat, Vec3),
    force: PlannedForce,
    push: Option<(BodyId, PushId)>,
    /// The script's `TurnON` / `TurnOFF` permissions.
    turn_on: bool,
    turn_off: bool,
    active: bool,
}

/// The level end (`PE_Balloon` at the `PE_Levelende` placeholder).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelEnd {
    /// `PE_Balloon_Platform`'s origin: the ball must get within 1 of it.
    pub platform: Vec3,
    /// The last sector, whose activation starts the balloon's script.
    pub sector: u32,
}

/// Level-end prefab objects not drawn: the UFO (last level only) and the
/// hidden slide box.
const LEVEL_END_HIDDEN: [&str; 2] = ["PE_UFO", "PE_Box_slide"];

/// The balloon's script: forces of its `Balloon Forces` graph switch on when
/// it wakes, the others (the slide box's) at the finish, which also breaks the
/// breakable hinges.
#[derive(Debug, Clone, Default)]
struct Balloon {
    lift: Vec<usize>,
    finish: Vec<usize>,
    pushes: Vec<PushId>,
    lifting: bool,
    finished: bool,
    /// The platform: index into the plan's bodies, and its object's matrix
    /// relative to the body (its origin is the finish target).
    platform: Option<(usize, Mat4)>,
}

struct Module {
    group: String,
    sector: u32,
    plan: ModulePlan,
    program: Option<Alternate>,
    fan: Option<Fan>,
    balloon: Option<Balloon>,
    /// Hinges while active (same order as `plan.hinges`).
    hinge_ids: Vec<Option<JointId>>,
    /// Bodies while active (same order as `plan.bodies`).
    bodies: Vec<BodyId>,
    /// One static material per fixed part (switched with the sector).
    fixed_materials: Vec<u32>,
    wake: Option<Proximity>,
    wake_template: Option<Proximity>,
    parts: Vec<Part>,
}

pub struct Modules {
    items: Vec<Module>,
    renderer: Option<SceneRenderer>,
    /// Level objects the modules replace.
    pub placeholders: Vec<usize>,
    pub level_end: Option<LevelEnd>,
}

impl Modules {
    #[allow(clippy::too_many_lines)]
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        level: &Scene,
        physics: &PhysicsData,
        world: &mut World,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let mut merged = Scene {
            meshes: Vec::new(),
            materials: Vec::new(),
            objects: Vec::new(),
            groups: Vec::new(),
        };
        let mut items = Vec::new();
        let mut placeholders = Vec::new();
        for setup in &physics.modules {
            let spawns = props::placeholders(level, &setup.group);
            if spawns.is_empty() {
                continue;
            }
            let key = format!("ph/{}", setup.group.to_lowercase());
            let prefab = bundle
                .scene(&key)
                .with_context(|| format!("loading {key}"))?;
            let mesh_offset = append_prefab(&mut merged, &prefab);
            for s in &spawns {
                placeholders.push(s.placeholder as usize);
                let mut plan = match modules::plan(setup, &prefab, s.transform) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("warning: {}: {e:#}; drawn only", setup.group);
                        ModulePlan::default()
                    }
                };
                modules::apply_sounds(&mut plan, &prefab, &key, &physics.sounds);
                let module = place(
                    setup,
                    &prefab,
                    s.transform,
                    s.sector,
                    plan,
                    mesh_offset,
                    &mut merged,
                    world,
                );
                items.push(module);
            }
        }
        // The level end: the PE_Levelende member's matrix, the prefab named
        // like it without "_NN", in the last sector (level init).
        let mut level_end = None;
        if let Some(member) = level
            .groups
            .iter()
            .find(|g| g.name == "PE_Levelende")
            .and_then(|g| g.members.first())
            && let Some(ph) = level.objects.get(*member as usize)
            && let Some(base) = ph.name.rsplit_once('_').map(|(b, _)| b.to_owned())
            && let Some(setup) = physics.modules.iter().find(|m| m.group == base)
        {
            let key = format!("ph/{}", base.to_lowercase());
            let prefab = bundle
                .scene(&key)
                .with_context(|| format!("loading {key}"))?;
            let mesh_offset = append_prefab(&mut merged, &prefab);
            let transform = Mat4::from_cols_array_2d(&ph.transform);
            let sector = crate::progress::last_sector(level);
            let mut plan = modules::plan(setup, &prefab, transform).unwrap_or_else(|e| {
                eprintln!("warning: {base}: {e:#}; drawn only");
                ModulePlan::default()
            });
            modules::apply_sounds(&mut plan, &prefab, &key, &physics.sounds);
            let module = place(
                setup,
                &prefab,
                transform,
                sector,
                plan,
                mesh_offset,
                &mut merged,
                world,
            );
            let platform = modules::placed_objects(&prefab, &base, transform).and_then(|w| {
                let i = prefab
                    .objects
                    .iter()
                    .position(|o| o.name == format!("{base}_Platform"))?;
                Some(w[i].transform_point3(Vec3::ZERO))
            });
            if let Some(platform) = platform {
                level_end = Some(LevelEnd { platform, sector });
            }
            placeholders.push(*member as usize);
            items.push(module);
        }
        world.statics.finish();
        let renderer = if merged.objects.is_empty() {
            None
        } else {
            let (r, warnings) =
                SceneRenderer::new(gpu, &merged, format, &mut |k| bundle.texture(k).ok())?;
            for w in warnings {
                eprintln!("warning: {w}");
            }
            Some(r)
        };
        Ok(Self {
            items,
            renderer,
            placeholders,
            level_end,
        })
    }

    pub fn count(&self) -> (usize, usize) {
        let simulated = self
            .items
            .iter()
            .filter(|m| !m.plan.bodies.is_empty())
            .count();
        (self.items.len(), simulated)
    }

    /// Script start with *Activation Phase?* true (physicalize, hinges, arm
    /// the wake-up test) or false (unphysicalize, restore the initial pose).
    pub fn set_sector_active(&mut self, world: &mut World, sector: u32, on: bool) {
        for m in self.items.iter_mut().filter(|m| m.sector == sector) {
            for &mat in &m.fixed_materials {
                world.statics.set_material_enabled(mat, on);
            }
            if let Some(f) = m.fan.as_mut() {
                if let Some((_, id)) = f.push.take() {
                    world.remove_push(id);
                }
                f.active = on;
                f.outer.clone_from(&f.outer_template);
                f.inner.clone_from(&f.inner_template);
                if on {
                    f.outer.switch_on();
                }
            }
            for id in m.bodies.drain(..) {
                world.remove_body(id);
            }
            m.hinge_ids.clear();
            if let Some(b) = m.balloon.as_mut() {
                b.pushes.clear();
                b.lifting = false;
                b.finished = false;
            }
            m.wake = None;
            if !on {
                continue;
            }
            m.bodies = m
                .plan
                .bodies
                .iter()
                .map(|b| world.add_body(b.desc.clone()))
                .collect();
            for h in &m.plan.hinges {
                let other = h.other.map(|o| m.bodies[o]);
                let id = world.add_hinge(m.bodies[h.body], other, h.pivot, h.axis);
                if id.is_none() {
                    eprintln!("warning: {}: hinge not created", m.group);
                }
                m.hinge_ids.push(id);
            }
            for s in &m.plan.sliders {
                let other = s.other.map(|o| m.bodies[o]);
                if world
                    .add_slider(m.bodies[s.body], other, s.origin, s.axis, s.limits)
                    .is_none()
                {
                    eprintln!("warning: {}: slider not created", m.group);
                }
            }
            for s in &m.plan.springs {
                let other = s.other.map(|o| m.bodies[o]);
                world.add_spring(
                    m.bodies[s.body],
                    other,
                    s.point,
                    s.other_point,
                    s.length,
                    s.constant,
                    s.damping,
                );
            }
            for j in &m.plan.ball_joints {
                let other = j.other.map(|o| m.bodies[o]);
                if world
                    .add_ball_joint(m.bodies[j.body], other, j.pivot)
                    .is_none()
                {
                    eprintln!("warning: {}: ball joint not created", m.group);
                }
            }
            if let Some(p) = m.program.as_mut() {
                p.phase = 0;
                p.left_ms = p.on_ms;
                p.push = push(world, &m.plan, &m.bodies, p.first);
                wake_module(world, &m.plan, &m.bodies);
            }
            m.wake.clone_from(&m.wake_template);
            if let Some(w) = m.wake.as_mut() {
                w.switch_on();
            }
        }
    }

    /// One frame: the scripted forces and the wake-up tests (`TT Scaleable
    /// Proximity` → `Physics WakeUp`).
    pub fn frame(&mut self, ball_id: BodyId, real_dt: f32, world: &mut World) {
        let Some((ball, ball_rot, radius)) = world
            .body(ball_id)
            .map(|b| (b.position, b.orientation, b.radius()))
        else {
            return;
        };
        for m in &mut self.items {
            if let Some(f) = m.fan.as_mut()
                && f.active
            {
                fan_frame(f, world, ball_id, (ball, ball_rot, Vec3::splat(radius)));
            }
            if !m.bodies.is_empty()
                && let Some(p) = m.program.as_mut()
            {
                p.left_ms -= real_dt * 1000.0;
                if p.left_ms <= 0.0 {
                    if let Some(id) = p.push.take() {
                        world.remove_push(id);
                    }
                    // first on → gap → second on → gap → first on …
                    p.phase = (p.phase + if p.gap_ms > 0.0 { 1 } else { 2 }) % 4;
                    p.left_ms += if p.phase % 2 == 0 { p.on_ms } else { p.gap_ms };
                    match p.phase {
                        0 => {
                            p.push = push(world, &m.plan, &m.bodies, p.first);
                            wake_module(world, &m.plan, &m.bodies);
                        }
                        2 => p.push = push(world, &m.plan, &m.bodies, p.second),
                        _ => {}
                    }
                }
            }
            let (Some(w), Some(target)) = (m.wake.as_mut(), m.plan.wake_target) else {
                continue;
            };
            if w.frame(ball, target) == Some(Edge::Enter) {
                w.switch_off();
                for &i in &m.plan.wake_bodies {
                    if let Some(&id) = m.bodies.get(i) {
                        world.wake(id);
                    }
                }
                // The balloon: Physics WakeUp, then Balloon Forces.
                if let Some(b) = m.balloon.as_mut()
                    && !b.lifting
                {
                    b.lifting = true;
                    for &f in &b.lift.clone() {
                        b.pushes.extend(push(world, &m.plan, &m.bodies, f));
                    }
                }
            }
        }
    }

    /// Where the level end's platform is now (its object's origin).
    pub fn level_end_platform(&self, world: &World) -> Option<Vec3> {
        let m = self.items.iter().find(|m| m.balloon.is_some())?;
        let (i, object_from_body) = m.balloon.as_ref()?.platform?;
        let b = world.body(*m.bodies.get(i)?)?;
        let pose = Mat4::from_rotation_translation(b.orientation, b.position);
        Some((pose * object_from_body).transform_point3(Vec3::ZERO))
    }

    /// `Level_Finish`: the balloon's slide box is pushed, and its bridge lets go.
    pub fn level_finished(&mut self, world: &mut World) {
        for m in &mut self.items {
            let Some(b) = m.balloon.as_mut() else {
                continue;
            };
            if b.finished || m.bodies.is_empty() {
                continue;
            }
            b.finished = true;
            for &f in &b.finish.clone() {
                b.pushes.extend(push(world, &m.plan, &m.bodies, f));
            }
            for (h, id) in m.plan.hinges.iter().zip(&mut m.hinge_ids) {
                if h.breakable
                    && let Some(id) = id.take()
                {
                    world.remove_joint(id);
                }
            }
        }
    }

    /// Development aid: active modules with a body that exploded or fell
    /// below `y`.
    pub fn fallen(&self, world: &World, y: f32) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|m| {
                let parts: Vec<String> = m
                    .bodies
                    .iter()
                    .enumerate()
                    .filter(|&(_, &id)| {
                        world
                            .body(id)
                            .is_some_and(|b| !b.position.is_finite() || b.position.y < y)
                    })
                    .map(|(i, _)| i.to_string())
                    .collect();
                (!parts.is_empty()).then(|| format!("{} (bodies {})", m.group, parts.join(" ")))
            })
            .collect()
    }

    /// Development aid: wakes every active module as its proximity test
    /// would (its wake-up targets; modules without a test are awake anyway).
    pub fn wake_all(&mut self, world: &mut World) {
        for m in &mut self.items {
            if m.wake.take().is_none() {
                continue;
            }
            for &i in &m.plan.wake_bodies {
                if let Some(&id) = m.bodies.get(i) {
                    world.wake(id);
                }
            }
            if let Some(b) = m.balloon.as_mut()
                && !b.lifting
            {
                b.lifting = true;
                for &f in &b.lift.clone() {
                    b.pushes.extend(push(world, &m.plan, &m.bodies, f));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
        opts: &RenderOptions,
        world: &World,
    ) {
        let Some(r) = self.renderer.as_mut() else {
            return;
        };
        for m in &self.items {
            for part in &m.parts {
                let Some(bi) = part.body else { continue };
                let pose = m
                    .bodies
                    .get(bi)
                    .and_then(|&id| world.interpolated(id))
                    .map_or_else(
                        || {
                            let d = &m.plan.bodies[bi].desc;
                            Mat4::from_rotation_translation(d.orientation, d.position)
                        },
                        |(p, q)| Mat4::from_rotation_translation(q, p),
                    );
                r.set_object_transform(gpu, part.object, pose * part.local);
            }
        }
        r.render(gpu, encoder, color, depth, camera, aspect, opts);
    }
}

/// Adds a placed module's drawn objects to the merged scene and its fixed
/// parts to static geometry (switched off until its sector activates).
#[allow(clippy::too_many_arguments)]
fn place(
    setup: &ModuleSetup,
    prefab: &Scene,
    transform: Mat4,
    sector: u32,
    plan: ModulePlan,
    mesh_offset: u32,
    merged: &mut Scene,
    world: &mut World,
) -> Module {
    let placed = modules::placed_objects(prefab, &setup.group, transform).unwrap_or_default();
    // The body an object moves with: its own, or an ancestor's.
    let body_of = |mut i: usize| loop {
        if let Some(b) = plan.bodies.iter().position(|b| b.object == i) {
            return Some(b);
        }
        i = prefab.objects[i].parent? as usize;
    };
    let mut parts = Vec::new();
    for (i, o) in prefab.objects.iter().enumerate() {
        let (Some(mesh), Some(&world_m)) = (o.mesh, placed.get(i)) else {
            continue;
        };
        if LEVEL_END_HIDDEN.iter().any(|p| o.name.starts_with(p)) {
            continue;
        }
        let body = body_of(i);
        let local = match body {
            Some(b) => {
                let d = &plan.bodies[b].desc;
                Mat4::from_rotation_translation(d.orientation, d.position).inverse() * world_m
            }
            None => world_m,
        };
        let mut obj = o.clone();
        obj.mesh = Some(mesh + mesh_offset);
        obj.parent = None;
        obj.hidden = false;
        obj.transform = world_m.to_cols_array_2d();
        parts.push(Part {
            object: merged.objects.len(),
            body,
            local,
        });
        merged.objects.push(obj);
    }
    let mut fixed_materials = Vec::new();
    for f in &plan.fixed {
        let mat = world.statics.add_material(SurfaceMaterial {
            friction: f.friction,
            elasticity: f.elasticity,
            group: f.group.clone(),
            sound: f.sound,
        });
        world.statics.set_material_enabled(mat, false);
        for t in &f.triangles {
            world.statics.add_triangle(t[0], t[1], t[2], mat);
        }
        fixed_materials.push(mat);
    }
    let wake_template = setup.wake.as_ref().map(|w| {
        Proximity::new(
            w.distance,
            w.exactness[0],
            w.exactness[1],
            (w.frame_delay[0], w.frame_delay[1]),
            w.xz,
        )
    });
    let program = alternate(setup, &plan);
    let fan = fan(setup, prefab, &placed, &plan);
    let balloon = balloon(setup, prefab, &plan);
    Module {
        group: setup.group.clone(),
        sector,
        program,
        fan,
        balloon,
        hinge_ids: Vec::new(),
        plan,
        bodies: Vec::new(),
        fixed_materials,
        wake: None,
        wake_template,
        parts,
    }
}

/// The two-force program of the swing and the punching bag: two forces on the
/// same body and Delayer times. The script creates the force whose (original)
/// direction is +Z first, which is −Z in bundle space. With four delays (the
/// swing) the second is the rest between pushes; with one (the bag), none.
fn alternate(setup: &ModuleSetup, plan: &ModulePlan) -> Option<Alternate> {
    if plan.bodies.is_empty() || plan.forces.len() != 2 || setup.forces.len() != 2 {
        return None;
    }
    if plan.forces[0].body.is_none() || plan.forces[0].body != plan.forces[1].body {
        return None;
    }
    let on_ms = *setup.delays_ms.first()?;
    let gap_ms = if setup.delays_ms.len() >= 4 {
        setup.delays_ms[1]
    } else {
        0.0
    };
    let first = usize::from(setup.forces[0].direction[2] >= 0.0);
    Some(Alternate {
        first,
        second: 1 - first,
        on_ms,
        gap_ms,
        phase: 0,
        left_ms: on_ms,
        push: None,
    })
}

/// Switches a planned force on (fixed in world space from now, like the
/// original's `Physics Force`).
fn push(world: &mut World, plan: &ModulePlan, bodies: &[BodyId], force: usize) -> Option<PushId> {
    let f = plan.forces.get(force)?;
    let body = *bodies.get(f.body?)?;
    Some(world.add_push_at(body, f.direction, f.value, f.point))
}

/// `Physics WakeUp` on the module's wake-up targets (or all its bodies if the
/// script names none).
fn wake_module(world: &mut World, plan: &ModulePlan, bodies: &[BodyId]) {
    if plan.wake_bodies.is_empty() {
        for &id in bodies {
            world.wake(id);
        }
    }
    for &i in &plan.wake_bodies {
        if let Some(&id) = bodies.get(i) {
            world.wake(id);
        }
    }
}

/// The fan program: a force on the ball, the script's proximity tests to one
/// object (outer: the largest, inner: the smallest), and the box it tests the
/// ball's box against.
fn fan(setup: &ModuleSetup, prefab: &Scene, placed: &[Mat4], plan: &ModulePlan) -> Option<Fan> {
    let force = plan.forces.iter().find(|f| f.body.is_none())?.clone();
    let inner_p = setup
        .proximities
        .iter()
        .min_by(|a, b| a.distance.total_cmp(&b.distance))?;
    let outer_p = setup
        .proximities
        .iter()
        .filter(|p| p.object == inner_p.object)
        .max_by(|a, b| a.distance.total_cmp(&b.distance))?;
    let index = |name: &str| prefab.objects.iter().position(|o| o.name == name);
    let target = placed
        .get(index(&inner_p.object)?)?
        .transform_point3(Vec3::ZERO);
    let bi = index(setup.boxes.first()?)?;
    let m = *placed.get(bi)?;
    let mesh = &prefab.meshes[prefab.objects[bi].mesh? as usize];
    let (lo, hi) = (Vec3::from(mesh.bounds_min), Vec3::from(mesh.bounds_max));
    let (scale, rot, _) = m.to_scale_rotation_translation();
    let area = (
        m.transform_point3((lo + hi) * 0.5),
        rot,
        (hi - lo) * 0.5 * scale.abs(),
    );
    let proximity = |p: &ballistic_bundle::WakeParams| {
        Proximity::new(
            p.distance,
            p.exactness[0],
            p.exactness[1],
            (p.frame_delay[0], p.frame_delay[1]),
            p.xz,
        )
    };
    let (outer, inner) = (proximity(outer_p), proximity(inner_p));
    Some(Fan {
        outer_template: outer.clone(),
        inner_template: inner.clone(),
        outer,
        inner,
        target,
        area,
        force,
        push: None,
        turn_on: true,
        turn_off: false,
        active: false,
    })
}

fn fan_frame(f: &mut Fan, world: &mut World, ball_id: BodyId, ball_box: (Vec3, Quat, Vec3)) {
    let ball = ball_box.0;
    match f.outer.frame(ball, f.target) {
        Some(Edge::Enter) => {
            // Physics Force.input0: arm the inner test, reset, no force.
            f.inner.clone_from(&f.inner_template);
            f.inner.switch_on();
            f.turn_on = true;
            f.turn_off = false;
            if let Some((_, id)) = f.push.take() {
                world.remove_push(id);
            }
        }
        // In 1: the inner test stops (a force that's on stays on).
        Some(Edge::Exit) => f.inner.switch_off(),
        None => {}
    }
    // InRange (in range, not just entered): the box test.
    if let Some(r) = f.inner.check(ball, f.target)
        && r.inside
        && r.edge.is_none()
    {
        if boxes_overlap(ball_box, f.area) {
            if f.turn_on {
                f.turn_on = false;
                f.turn_off = true;
                let id = world.add_push(ball_id, f.force.direction, f.force.value);
                f.push = Some((ball_id, id));
            }
        } else if f.turn_off {
            f.turn_off = false;
            f.turn_on = true;
            if let Some((_, id)) = f.push.take() {
                world.remove_push(id);
            }
        }
    }
}

/// The balloon program: the level end's module (`PE_*`) with the forces of a
/// `Balloon Forces` graph.
fn balloon(setup: &ModuleSetup, prefab: &Scene, plan: &ModulePlan) -> Option<Balloon> {
    if !setup.group.starts_with("PE_") {
        return None;
    }
    let (lift, finish): (Vec<usize>, Vec<usize>) =
        (0..plan.forces.len()).partition(|&i| plan.forces[i].graph == "Balloon Forces");
    let platform_name = format!("{}_Platform", setup.group);
    let platform = plan.bodies.iter().enumerate().find_map(|(i, b)| {
        (prefab.objects[b.object].name == platform_name).then_some((i, b.object_from_body))
    });
    Some(Balloon {
        lift,
        finish,
        platform,
        ..Balloon::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballistic_physics::{BodyDesc, Shape, StaticMesh, WorldConfig};

    fn fan_at_origin() -> Fan {
        let outer = Proximity::new(80.0, 85.0, 100.0, (10, 60), true);
        let inner = Proximity::new(7.0, 12.0, 20.0, (1, 10), true);
        Fan {
            outer_template: outer.clone(),
            inner_template: inner.clone(),
            outer,
            inner,
            target: Vec3::ZERO,
            area: (
                Vec3::new(0.0, 10.0, 0.0),
                Quat::IDENTITY,
                Vec3::new(3.0, 10.0, 3.0),
            ),
            force: PlannedForce {
                body: None,
                point: Vec3::ZERO,
                direction: Vec3::Y,
                value: 0.1,
                graph: String::new(),
            },
            push: None,
            turn_on: true,
            turn_off: false,
            active: true,
        }
    }

    /// Height after 2 s above the fan, for a ball of `mass`.
    fn rise(mass: f32) -> f32 {
        let mut w = World::new(WorldConfig::default(), StaticMesh::default());
        let ball = w.add_body(BodyDesc {
            position: Vec3::new(0.0, 2.0, 0.0),
            shape: Shape::Sphere { radius: 2.0 },
            mass,
            ..BodyDesc::default()
        });
        let mut f = fan_at_origin();
        f.outer.switch_on();
        for _ in 0..120 {
            let b = w.body(ball).unwrap();
            let bx = (b.position, b.orientation, Vec3::splat(2.0));
            fan_frame(&mut f, &mut w, ball, bx);
            w.advance(1.0 / 60.0);
        }
        w.body(ball).unwrap().position.y
    }

    #[test]
    fn fan_lifts_a_paper_ball_but_not_a_wooden_one() {
        assert!(rise(0.2) > 2.0, "paper ball didn't rise: {}", rise(0.2));
        assert!(rise(1.9) < 0.0, "wooden ball should fall: {}", rise(1.9));
    }
}
