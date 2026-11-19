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
//! Camera rig from `Camera.nmo` (follow behaviour in [`crate::follow`]).
//! On spawn the scripts set both the ball and the rig root (`Cam_MF`) to the
//! current reset point's world matrix, so the rig looks along the reset
//! point's −X axis.

use std::io::BufReader;

use anyhow::{Context, Result};
use ballistic_bundle::{BallParams, BundleReader, PhysicsData};
use ballistic_physics::{
    BodyDesc, BodyId, PushId, Quat, Shape, Vec3, World, WorldConfig, build_level,
};
use ballistic_render::{Camera, Gpu, RenderOptions};
use glam::Mat4;

use crate::ballsounds::BallSoundsPlayer;
use crate::follow::Follow;
use crate::modules::Modules;
use crate::pieces::Pieces;
use crate::progress::{self, Progress};
use crate::props::Props;
use crate::transformers::{self, Transformers};
use crate::{LoadedScene, load_scene};

pub type Bundle = BundleReader<BufReader<std::fs::File>>;

const TURN_SECONDS: f32 = 0.25;
/// `InGameCam`: 1.012291 rad horizontal at 4:3 (`Camera.nmo`), kept as the
/// equivalent vertical angle at every aspect ratio.
fn fov_y() -> f32 {
    2.0 * ((1.012_291_f32 / 2.0).tan() * 0.75).atan()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayKey {
    Up,
    Down,
    Left,
    Right,
    Rotate,
    /// `CamUp`: the lift view.
    Lift,
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
    keys_held: [bool; 6],
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
    follow: Follow,
    props: Props,
    modules: Modules,
    transformers: Transformers,
    pieces: Pieces,
    ball_sounds: BallSoundsPlayer,
    /// Roll sounds looping at the last frame (for counting starts).
    looping: Vec<String>,
    /// Sounds to play (bundle key, volume), taken by the window.
    sounds: Vec<(String, f32)>,
    /// How many times each sound was played (for headless runs).
    pub sound_counts: std::collections::BTreeMap<String, u32>,
    /// The ball's pose while a transformer holds it out of the simulation.
    held: Option<(Vec3, Quat)>,
    progress: Progress,
    /// Real seconds of play since the level started.
    elapsed: f32,
    /// When the level end was reached: (play time, physics steps).
    finished_at: Option<(f32, u64)>,
    pub respawns: u32,
}

fn key_index(k: PlayKey) -> usize {
    match k {
        PlayKey::Up => 0,
        PlayKey::Down => 1,
        PlayKey::Left => 2,
        PlayKey::Right => 3,
        PlayKey::Rotate => 4,
        PlayKey::Lift => 5,
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
        let props = Props::new(gpu, bundle, level, &physics, &mut world, format)?;
        let modules = Modules::new(gpu, bundle, level, &physics, &mut world, format)?;
        let transformers = Transformers::new(gpu, bundle, level, &physics, format)?;
        let layout = progress::layout(level, |key| bundle.scene(key).ok());
        let spawn_matrix = layout.reset_points.first().copied();
        let progress = Progress::start(layout, modules.level_end);

        let (start, heading) = match spawn_matrix {
            Some(m) => reset_pose(m),
            None => level_start(level)?,
        };

        let active_ball = physics
            .balls
            .iter()
            .position(|b| b.name == "Ball_Wood")
            .unwrap_or(0);
        let ball = world.add_body(body_desc(&physics.balls[active_ball], start));
        world.report_collisions(ball, true);
        // Start Level, then the new ball's BallNav activate.
        let mut ball_sounds = BallSoundsPlayer::new(&physics);
        ball_sounds.set_active(true);

        let balls = load_scene(gpu, bundle, "balls", format)?;
        let ball_scene = bundle.scene("balls")?;
        let pieces = Pieces::new(&physics, &ball_scene);
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
            keys_held: [false; 6],
            pushes: [None; 4],
            nav_on: true,
            nav_on_in: None,
            heading_ref: heading,
            heading_view: heading,
            turn: None,
            snap_next_frame: None,
            follow: Follow::new(start, heading),
            props,
            modules,
            transformers,
            pieces,
            ball_sounds,
            looping: Vec::new(),
            sounds: Vec::new(),
            sound_counts: std::collections::BTreeMap::new(),
            held: None,
            progress,
            elapsed: 0.0,
            finished_at: None,
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
            PlayKey::Rotate | PlayKey::Lift => Vec3::ZERO,
        };
        r * local
    }

    fn create_push(&mut self, k: PlayKey) {
        let i = key_index(k);
        if self.pushes[i].is_none() {
            let dir = self.direction(k);
            let force = self.params().force;
            if let Some(ball) = self.ball_id() {
                self.pushes[i] = Some(self.world.add_push(ball, dir, force));
            }
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
        // After Level_Finish the original switches ball and camera navigation
        // off for good.
        if self.finished_at.is_some() || self.transformers.busy() {
            return;
        }
        self.nav_on = true;
        for k in DIRS {
            if self.keys_held[key_index(k)] {
                self.create_push(k);
            }
        }
    }

    pub fn input(&mut self, k: PlayKey, pressed: bool) {
        let i = key_index(k);
        if self.keys_held[i] == pressed {
            return;
        }
        self.keys_held[i] = pressed;
        if self.finished_at.is_some() {
            return;
        }
        let rotating = self.keys_held[key_index(PlayKey::Rotate)];
        match k {
            // Only the camera reads it (in `update`).
            PlayKey::Lift => {}
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

    /// Development aid: puts the ball, at rest, on the next checkpoint's
    /// flame, or after the last one on the level end's platform.
    pub fn warp_to_next_checkpoint(&mut self) {
        let Some(target) = self.progress.next_target() else {
            return;
        };
        if self.held.is_some() {
            return;
        }
        self.world.reset_body(self.ball, target);
        self.follow.reset(target, self.heading_view);
    }

    /// Development aid: puts the ball, at rest, on the nearest transformer of
    /// another type.
    pub fn warp_to_transformer(&mut self) {
        let Some(ball) = self
            .ball_id()
            .and_then(|id| self.world.body(id))
            .map(|b| b.position)
        else {
            return;
        };
        if let Some(target) = self.transformers.nearest_other(ball, self.active_ball) {
            self.world.reset_body(self.ball, target);
            self.follow.reset(target, self.heading_view);
        }
    }

    /// Development aid: wakes every active moving element.
    pub fn wake_modules(&mut self) {
        self.modules.wake_all(&mut self.world);
    }

    pub fn cycle_ball(&mut self) {
        if self.held.is_some() {
            return;
        }
        self.active_ball = (self.active_ball + 1) % self.physics.balls.len();
        let pos = self
            .world
            .body(self.ball)
            .map_or(self.start, |b| b.position);
        self.world.remove_body(self.ball);
        self.pushes = [None; 4];
        let desc = body_desc(&self.physics.balls[self.active_ball], pos);
        self.ball = self.world.add_body(desc);
        self.world.report_collisions(self.ball, true);
        if self.nav_on {
            self.nav_on();
        }
        self.show_active_ball();
    }

    fn show_active_ball(&mut self) {
        for (i, o) in self.ball_objects.clone().iter().enumerate() {
            if let Some(o) = o {
                self.balls
                    .renderer
                    .set_object_hidden(*o, i != self.active_ball);
            }
        }
    }

    fn queue_sounds(&mut self, sounds: impl IntoIterator<Item = (String, f32)>) {
        for (key, volume) in sounds {
            *self.sound_counts.entry(key.clone()).or_default() += 1;
            self.sounds.push((key, volume));
        }
    }

    /// Roll sounds that should be looping now: key, volume, pitch.
    pub fn loops(&mut self) -> Vec<(String, f32, f32)> {
        let loops = self.ball_sounds.loops();
        for (k, _, _) in &loops {
            if !self.looping.contains(k) {
                *self.sound_counts.entry(format!("{k} (loop)")).or_default() += 1;
            }
        }
        self.looping = loops.iter().map(|l| l.0.clone()).collect();
        loops
    }

    /// Sounds to play since the last call (bundle key, volume 0–1).
    pub fn take_sounds(&mut self) -> Vec<(String, f32)> {
        std::mem::take(&mut self.sounds)
    }

    /// The transformer script's requests for this frame.
    fn transformer_events(&mut self, events: Vec<transformers::Event>) {
        use transformers::Event as E;
        for e in events {
            match e {
                E::Begin => {
                    // BallNav deactivate: the hit and roll tests stop too.
                    self.nav_off();
                    self.ball_sounds.set_active(false);
                    self.pieces.triggered(self.active_ball);
                }
                E::Burst(ball) => {
                    if let Some((p, _)) = self.held {
                        let s = self.pieces.burst(&mut self.world, ball, p);
                        self.queue_sounds(s);
                    }
                }
                E::Sound(key) => self.queue_sounds([(key, 1.0)]),
                E::Release => {
                    self.held = self
                        .world
                        .body(self.ball)
                        .map(|b| (b.position, b.orientation));
                    self.world.remove_body(self.ball);
                    self.pushes = [None; 4];
                }
                E::Move(p) => {
                    if let Some(h) = self.held.as_mut() {
                        h.0 = p;
                    }
                }
                E::Swap(ball) => {
                    self.active_ball = ball;
                    self.show_active_ball();
                }
                E::Physicalize => {
                    if let Some((p, q)) = self.held.take() {
                        let mut desc = body_desc(&self.physics.balls[self.active_ball], p);
                        desc.orientation = q;
                        self.ball = self.world.add_body(desc);
                        self.world.report_collisions(self.ball, true);
                    }
                    self.nav_on();
                    self.ball_sounds.set_active(true);
                }
            }
        }
    }

    /// Back to the spawn point; the camera rig is reset with the ball.
    pub fn respawn(&mut self) {
        if self.held.is_some() {
            return;
        }
        // The ball comes back at the current reset point (the start, until a
        // checkpoint is reached); the current sector is reset.
        if let Some(m) = self.progress.reset_point() {
            (self.start, self.start_heading) = reset_pose(m);
        }
        self.progress.respawn();
        self.pieces.reset_all();
        // Deactivate Ball (BallNav deactivate), New Ball (BallNav activate).
        self.ball_sounds.set_active(false);
        if self.finished_at.is_none() {
            self.ball_sounds.set_active(true);
        }
        self.apply_sector_changes();
        self.world.reset_body(self.ball, self.start);
        self.follow.reset(self.start, self.start_heading);
        self.heading_ref = self.start_heading;
        self.heading_view = self.start_heading;
        self.turn = None;
        self.snap_next_frame = None;
        self.respawns += 1;
    }

    fn apply_sector_changes(&mut self) {
        for (sector, on) in self.progress.take_changes() {
            self.props.set_sector_active(&mut self.world, sector, on);
            self.transformers.set_sector_active(sector, on);
            self.modules.set_sector_active(&mut self.world, sector, on);
        }
    }

    /// One frame: camera script, then physics.
    pub fn update(&mut self, real_dt: f32) {
        // Deferred steps from earlier frames (the scripts' one-frame link delays).
        if let Some(h) = self.snap_next_frame.take() {
            self.heading_ref = h;
            self.heading_view = h;
            if !self.keys_held[key_index(PlayKey::Rotate)] {
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
        self.props.update(&mut self.world, self.fall_y);
        let hits = self.world.take_collisions();
        let s = self.pieces.frame(&mut self.world, &hits, real_dt);
        self.queue_sounds(s);
        let ball_now = self.ball_id().zip(self.ball_pose().map(|(p, _)| p));
        let s = self
            .ball_sounds
            .frame(&self.world, ball_now, self.active_ball, &hits, real_dt);
        self.queue_sounds(s);
        self.elapsed += real_dt;
        if let Some(ball) = self.ball_pose().map(|(p, _)| p) {
            if let Some(id) = self.ball_id() {
                self.modules.frame(id, real_dt, &mut self.world);
            }
            if let Some(p) = self.modules.level_end_platform(&self.world) {
                self.progress.set_level_end_target(p);
            }
            match self.progress.frame(ball) {
                Some(progress::Event::Checkpoint(i)) => eprintln!("checkpoint {} reached", i + 1),
                Some(progress::Event::Finished) => {
                    // Level_Finish: ball control off; the balloon lets go.
                    self.finished_at = Some((self.elapsed, self.world.steps));
                    self.modules.level_finished(&mut self.world);
                    self.nav_off();
                    self.ball_sounds.set_active(false);
                    eprintln!(
                        "level finished after {:.2} s ({} physics steps)",
                        self.elapsed, self.world.steps
                    );
                }
                None => {}
            }
            let events = self.transformers.frame(ball, self.active_ball, real_dt);
            self.transformer_events(events);
        }
        self.apply_sector_changes();

        if self
            .ball_id()
            .and_then(|id| self.world.body(id))
            .is_some_and(|b| b.position.y < self.fall_y)
        {
            self.respawn();
        }
        let ball = self.ball_pose().map_or(self.start, |(p, _)| p);
        let lifted = self.keys_held[key_index(PlayKey::Lift)];
        self.follow.update(real_dt, ball, self.heading_view, lifted);
    }

    /// Level objects that props and modules replace; hide them in the
    /// level's renderer.
    pub fn placeholders(&self) -> impl Iterator<Item = usize> + '_ {
        self.props
            .placeholders
            .iter()
            .chain(&self.modules.placeholders)
            .chain(&self.transformers.placeholders)
            .copied()
    }

    /// (modules placed, modules simulated).
    pub fn module_count(&self) -> (usize, usize) {
        self.modules.count()
    }

    pub fn prop_count(&self) -> usize {
        self.props.count()
    }

    /// (active props, checkpoints reached, checkpoints).
    pub fn progress(&self) -> (usize, usize, usize) {
        (
            self.props.active_count(),
            self.progress.reached(),
            self.progress.checkpoint_count(),
        )
    }

    /// The ball's body, unless a transformer holds the ball out of the world
    /// (its old id may then belong to another body).
    fn ball_id(&self) -> Option<BodyId> {
        self.held.is_none().then_some(self.ball)
    }

    pub fn ball_pose(&self) -> Option<(Vec3, Quat)> {
        self.held.or_else(|| self.world.interpolated(self.ball))
    }

    pub fn transformer_count(&self) -> usize {
        self.transformers.count()
    }

    pub fn camera(&self, base: Camera) -> Camera {
        let (eye, target) = self.follow.pose();
        let mut cam = Camera {
            position: eye,
            fov_y: fov_y(),
            ..base
        };
        cam.look_at(target);
        cam
    }

    pub fn sync_render(&mut self, gpu: &Gpu) {
        self.pieces.sync(gpu, &mut self.balls.renderer, &self.world);
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
        self.props.render(
            gpu,
            encoder,
            color,
            depth,
            camera,
            aspect,
            &opts,
            &self.world,
        );
        self.modules.render(
            gpu,
            encoder,
            color,
            depth,
            camera,
            aspect,
            &opts,
            &self.world,
        );
        self.transformers
            .render(gpu, encoder, color, depth, camera, aspect, &opts);
    }

    pub fn status(&self) -> String {
        let b = self.ball_id().and_then(|id| self.world.body(id));
        format!(
            "{} speed {:.1} checkpoint {}/{} {}{}{}{}{}",
            self.ball_name(),
            b.map_or(0.0, |b| b.velocity.length()),
            self.progress.reached(),
            self.progress.checkpoint_count(),
            self.finished_at
                .map(|(t, _)| format!("[finished {t:.2} s] "))
                .unwrap_or_default(),
            if self.nav_on { "" } else { "[no control] " },
            if self.turn.is_some() {
                "[turning] "
            } else {
                ""
            },
            if self.keys_held[key_index(PlayKey::Lift)] {
                "[lift] "
            } else {
                ""
            },
            match self.pieces.count() {
                0 => String::new(),
                n => format!("[{n} pieces]"),
            }
        )
    }
}

/// Spawn position and camera heading: the first reset point (sector 1), or the
/// level start marker if a level has none.
/// Ball position and camera heading for a reset point's world matrix: the
/// scripts put both the ball and the rig root `Cam_MF` there, and the rig
/// looks along its root's −X (`Camera.nmo`), snapped to the 90° grid.
fn reset_pose(m: Mat4) -> (Vec3, f32) {
    let heading = snap90(heading_of(m.transform_vector3(Vec3::NEG_X)));
    (m.transform_point3(Vec3::ZERO), heading)
}

/// Fallback for a level without reset points: above the start marker.
fn level_start(level: &ballistic_bundle::Scene) -> Result<(Vec3, f32)> {
    let g = level
        .groups
        .iter()
        .find(|g| g.name == "PS_Levelstart")
        .context("level has no reset point or start marker")?;
    let obj = g
        .members
        .iter()
        .map(|&i| &level.objects[i as usize])
        .min_by(|a, b| a.name.cmp(&b.name))
        .context("empty PS_Levelstart group")?;
    let (pos, heading) = reset_pose(Mat4::from_cols_array_2d(&obj.transform));
    Ok((pos + Vec3::Y * 3.0, heading))
}

fn body_desc(b: &BallParams, position: Vec3) -> BodyDesc {
    BodyDesc {
        position,
        shape: Shape::Sphere {
            radius: b.shape.radius(),
        },
        mass: b.mass,
        friction: b.friction,
        elasticity: b.elasticity,
        linear_damping: b.linear_damping,
        rotational_damping: b.rotational_damping,
        group: b.collision_group.clone(),
        ..BodyDesc::default()
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
