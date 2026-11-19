//! Play mode: a physics ball on the level, controlled like the original.
//!
//! The control logic mirrors the decoded scripts (research findings 11):
//! - direction keys create a per-step push on press and destroy it on release,
//!   along the *reference* heading at that moment;
//! - holding the rotate modifier (Shift) switches ball control off;
//! - Shift + Left/Right turns the camera 90° over 250 ms; one frame after the
//!   turn the reference heading snaps to it, and if Shift is up, control is
//!   switched off and back on one frame later (held keys re-fire, re-aiming).
//!
//! Camera rig from `Camera.nmo`: 22 units behind, 35 up, looking at the ball.
//! On spawn the scripts set both the ball and the rig root (`Cam_MF`) to the
//! current reset point's world matrix, so the rig looks along the reset
//! point's −X axis.

use std::io::BufReader;

use anyhow::{Context, Result};
use ballistic_bundle::{BallParams, BundleReader, PhysicsData};
use ballistic_physics::{BodyDesc, BodyId, PushId, Quat, Vec3, World, WorldConfig, build_level};
use ballistic_render::{Camera, Gpu, RenderOptions};
use glam::Mat4;

use crate::{LoadedScene, load_scene};

type Bundle = BundleReader<BufReader<std::fs::File>>;

const CAM_BACK: f32 = 22.0;
const CAM_UP: f32 = 35.0;
const TURN_SECONDS: f32 = 0.25;
/// Camera follow smoothing rate (1/s). Not measured from the original yet.
const FOLLOW_RATE: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayKey {
    Up,
    Down,
    Left,
    Right,
    Rotate,
}

/// Direction keys in the push order used by the scripts.
const DIRS: [PlayKey; 4] = [PlayKey::Up, PlayKey::Down, PlayKey::Left, PlayKey::Right];

struct Turn {
    from: f32,
    to: f32,
    elapsed: f32,
}

pub struct Play {
    pub world: World,
    ball: BodyId,
    physics: PhysicsData,
    active_ball: usize,
    balls: LoadedScene,
    /// Scene object index of each ball type in the balls scene.
    ball_objects: Vec<Option<usize>>,
    start: Vec3,
    start_heading: f32,
    fall_y: f32,
    held: [bool; 5],
    pushes: [Option<PushId>; 4],
    nav_on: bool,
    /// Frames until control switches back on after an *Update*.
    nav_on_in: Option<u32>,
    /// Reference heading (`Cam_OrientRef`), radians about +Y.
    heading_ref: f32,
    /// Visual heading (`Cam_Orient`).
    heading_view: f32,
    turn: Option<Turn>,
    /// Snap of the reference heading pending for the next frame.
    snap_next_frame: Option<f32>,
    cam_target: Vec3,
    pub respawns: u32,
}

fn key_index(k: PlayKey) -> usize {
    match k {
        PlayKey::Up => 0,
        PlayKey::Down => 1,
        PlayKey::Left => 2,
        PlayKey::Right => 3,
        PlayKey::Rotate => 4,
    }
}

impl Play {
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        level: &ballistic_bundle::Scene,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let physics = bundle
            .physics()
            .context("bundle has no physics data; re-run ballistic-pack")?;
        let (statics, _) = build_level(level, &physics);
        let fall_y = statics.min_y() - 50.0;
        let config = WorldConfig {
            gravity: Vec3::from(physics.gravity),
            time_factor: physics.time_factor,
            ..WorldConfig::default()
        };
        let mut world = World::new(config, statics);

        let (start, heading) = spawn_point(level)?;

        let active_ball = physics
            .balls
            .iter()
            .position(|b| b.name == "Ball_Wood")
            .unwrap_or(0);
        let ball = world.add_body(body_desc(&physics.balls[active_ball], start));

        let balls = load_scene(gpu, bundle, "balls", format)?;
        let ball_scene = bundle.scene("balls")?;
        let ball_objects = physics
            .balls
            .iter()
            .map(|b| {
                ball_scene
                    .objects
                    .iter()
                    .position(|o| o.name == b.name && o.mesh.is_some())
            })
            .collect();

        let mut play = Self {
            world,
            ball,
            physics,
            active_ball,
            balls,
            ball_objects,
            start,
            start_heading: heading,
            fall_y,
            held: [false; 5],
            pushes: [None; 4],
            nav_on: true,
            nav_on_in: None,
            heading_ref: heading,
            heading_view: heading,
            turn: None,
            snap_next_frame: None,
            cam_target: start,
            respawns: 0,
        };
        // Show only the active ball.
        for (i, o) in play.ball_objects.clone().iter().enumerate() {
            if let Some(o) = o {
                play.balls
                    .renderer
                    .set_object_hidden(*o, i != play.active_ball);
            }
        }
        Ok(play)
    }

    pub fn ball_name(&self) -> &str {
        &self.physics.balls[self.active_ball].name
    }

    fn params(&self) -> &BallParams {
        &self.physics.balls[self.active_ball]
    }

    /// Push direction for a key in the reference frame (bundle space).
    fn direction(&self, k: PlayKey) -> Vec3 {
        let r = Quat::from_rotation_y(self.heading_ref);
        // Original local axes (left-handed): Up +Z, Down −Z, Left −X, Right +X;
        // mirrored into bundle space Z flips.
        let local = match k {
            PlayKey::Up => Vec3::NEG_Z,
            PlayKey::Down => Vec3::Z,
            PlayKey::Left => Vec3::NEG_X,
            PlayKey::Right => Vec3::X,
            PlayKey::Rotate => Vec3::ZERO,
        };
        r * local
    }

    fn create_push(&mut self, k: PlayKey) {
        let i = key_index(k);
        if self.pushes[i].is_none() {
            let dir = self.direction(k);
            let force = self.params().force;
            self.pushes[i] = Some(self.world.add_push(self.ball, dir, force));
        }
    }

    fn destroy_push(&mut self, k: PlayKey) {
        if let Some(p) = self.pushes[key_index(k)].take() {
            self.world.remove_push(p);
        }
    }

    fn nav_off(&mut self) {
        self.nav_on = false;
        for k in DIRS {
            self.destroy_push(k);
        }
    }

    /// Key Event "On": state resets, so held keys fire Pressed immediately.
    fn nav_on(&mut self) {
        self.nav_on = true;
        for k in DIRS {
            if self.held[key_index(k)] {
                self.create_push(k);
            }
        }
    }

    pub fn input(&mut self, k: PlayKey, pressed: bool) {
        let i = key_index(k);
        if self.held[i] == pressed {
            return;
        }
        self.held[i] = pressed;
        let rotating = self.held[key_index(PlayKey::Rotate)];
        match k {
            PlayKey::Rotate if pressed => self.nav_off(),
            PlayKey::Rotate => {
                self.nav_on_in = None;
                self.nav_on();
            }
            PlayKey::Left | PlayKey::Right if rotating => {
                if pressed && self.turn.is_none() && self.snap_next_frame.is_none() {
                    // Cam Left = +90° in the original's left-handed space = −90° here.
                    let delta = if k == PlayKey::Left {
                        -std::f32::consts::FRAC_PI_2
                    } else {
                        std::f32::consts::FRAC_PI_2
                    };
                    self.turn = Some(Turn {
                        from: self.heading_ref,
                        to: self.heading_ref + delta,
                        elapsed: 0.0,
                    });
                }
            }
            _ if self.nav_on => {
                if pressed {
                    self.create_push(k);
                } else {
                    self.destroy_push(k);
                }
            }
            _ => {}
        }
    }

    pub fn cycle_ball(&mut self) {
        self.active_ball = (self.active_ball + 1) % self.physics.balls.len();
        let pos = self
            .world
            .body(self.ball)
            .map_or(self.start, |b| b.position);
        self.world.remove_body(self.ball);
        self.pushes = [None; 4];
        let desc = body_desc(&self.physics.balls[self.active_ball], pos);
        self.ball = self.world.add_body(desc);
        if self.nav_on {
            self.nav_on();
        }
        for (i, o) in self.ball_objects.clone().iter().enumerate() {
            if let Some(o) = o {
                self.balls
                    .renderer
                    .set_object_hidden(*o, i != self.active_ball);
            }
        }
    }

    /// Back to the spawn point; the camera rig is reset with the ball.
    pub fn respawn(&mut self) {
        self.world.reset_body(self.ball, self.start);
        self.cam_target = self.start;
        self.heading_ref = self.start_heading;
        self.heading_view = self.start_heading;
        self.turn = None;
        self.snap_next_frame = None;
        self.respawns += 1;
    }

    /// One frame: camera script, then physics.
    pub fn update(&mut self, real_dt: f32) {
        // Deferred steps from earlier frames (the scripts' one-frame link delays).
        if let Some(h) = self.snap_next_frame.take() {
            self.heading_ref = h;
            self.heading_view = h;
            if !self.held[key_index(PlayKey::Rotate)] {
                // Cam Navigation "Update": control off now, on one frame later.
                self.nav_off();
                self.nav_on_in = Some(1);
            }
        } else if let Some(n) = self.nav_on_in {
            if n == 0 {
                self.nav_on_in = None;
                self.nav_on();
            } else {
                self.nav_on_in = Some(n - 1);
            }
        }

        if let Some(t) = &mut self.turn {
            t.elapsed += real_dt;
            let u = (t.elapsed / TURN_SECONDS).min(1.0);
            let eased = u * u * (3.0 - 2.0 * u);
            self.heading_view = t.from + (t.to - t.from) * eased;
            if u >= 1.0 {
                self.snap_next_frame = Some(t.to);
                self.turn = None;
            }
        }

        self.world.advance(real_dt);

        if self
            .world
            .body(self.ball)
            .is_some_and(|b| b.position.y < self.fall_y)
        {
            self.respawn();
        }
    }

    pub fn ball_pose(&self) -> Option<(Vec3, Quat)> {
        self.world.interpolated(self.ball)
    }

    pub fn camera(&mut self, real_dt: f32, base: Camera) -> Camera {
        let target = self.ball_pose().map_or(self.start, |(p, _)| p);
        let k = 1.0 - (-FOLLOW_RATE * real_dt).exp();
        self.cam_target += (target - self.cam_target) * k;
        let r = Quat::from_rotation_y(self.heading_view);
        let forward = r * Vec3::NEG_Z;
        let mut cam = Camera {
            position: self.cam_target - forward * CAM_BACK + Vec3::Y * CAM_UP,
            fov_y: 45f32.to_radians(),
            ..base
        };
        cam.look_at(self.cam_target);
        cam
    }

    pub fn sync_render(&mut self, gpu: &Gpu) {
        let Some((p, q)) = self.ball_pose() else {
            return;
        };
        if let Some(Some(o)) = self.ball_objects.get(self.active_ball) {
            let m = Mat4::from_rotation_translation(q, p);
            self.balls.renderer.set_object_transform(gpu, *o, m);
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
    ) {
        self.sync_render(gpu);
        let opts = RenderOptions {
            clear: false,
            show_hidden: false,
            ..*opts
        };
        self.balls
            .renderer
            .render(gpu, encoder, color, depth, camera, aspect, &opts);
    }

    pub fn status(&self) -> String {
        let b = self.world.body(self.ball);
        format!(
            "{} speed {:.1} {}{}",
            self.ball_name(),
            b.map_or(0.0, |b| b.velocity.length()),
            if self.nav_on { "" } else { "[no control] " },
            if self.turn.is_some() { "[turning]" } else { "" }
        )
    }
}

/// Spawn position and camera heading: the first reset point (sector 1), or the
/// level start marker if a level has none.
fn spawn_point(level: &ballistic_bundle::Scene) -> Result<(Vec3, f32)> {
    let object = |group: &str| {
        let g = level.groups.iter().find(|g| g.name == group)?;
        g.members
            .iter()
            .map(|&i| &level.objects[i as usize])
            .min_by(|a, b| a.name.cmp(&b.name))
    };
    let (obj, lift) = match object("PR_Resetpoints") {
        Some(o) => (o, 0.0),
        None => (
            object("PS_Levelstart").context("level has no reset point or start marker")?,
            3.0,
        ),
    };
    let m = Mat4::from_cols_array_2d(&obj.transform);
    let position = m.transform_point3(Vec3::ZERO) + Vec3::Y * lift;
    // The rig looks along its root's −X (`Camera.nmo`); snap to the 90° grid.
    let heading = snap90(heading_of(m.transform_vector3(Vec3::NEG_X)));
    Ok((position, heading))
}

fn body_desc(b: &BallParams, position: Vec3) -> BodyDesc {
    BodyDesc {
        position,
        radius: b.shape.radius(),
        mass: b.mass,
        friction: b.friction,
        elasticity: b.elasticity,
        linear_damping: b.linear_damping,
        rotational_damping: b.rotational_damping,
        group: b.collision_group.clone(),
    }
}

/// Heading angle `h` such that `rot_y(h) · (0, 0, −1)` points along `f`.
fn heading_of(f: Vec3) -> f32 {
    (-f.x).atan2(-f.z)
}

fn snap90(h: f32) -> f32 {
    let q = std::f32::consts::FRAC_PI_2;
    (h / q).round() * q
}
