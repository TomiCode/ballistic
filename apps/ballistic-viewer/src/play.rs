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
use ballistic_render::{
    Camera, Gpu, ParticleBatch, ParticleRenderer, RenderOptions, SkyRenderer, upload_ktx2,
};
use glam::Mat4;

use crate::ballsounds::BallSoundsPlayer;
use crate::effects::Effects;
use crate::energy::{Energy, Score};
use crate::flames::Flames;
use crate::flow::{BallPhase, DepthCubes, EventSounds, LOST_CHECK_FRAMES};
use crate::follow::Follow;
use crate::hud::Hud;
use crate::life_look::LifeLooks;
use crate::modules::Modules;
use crate::pickups::{self, Pickups};
use crate::pieces::Pieces;
use crate::point_look::PointLooks;
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
    energy: Energy,
    hud: Hud,
    /// The level's sky (none if it has no entry).
    sky: Option<SkyRenderer>,
    /// The light, ambient and background in play.
    lighting: Lighting,
    clouds: Option<Clouds>,
    flames: Flames,
    effects: Effects,
    life_looks: LifeLooks,
    point_looks: PointLooks,
    particles: ParticleRenderer,
    /// Texture key (`None`: white) → the particle renderer's texture.
    particle_textures: std::collections::HashMap<Option<String>, usize>,
    pickups: Pickups,
    flow: ballistic_bundle::LevelFlow,
    phase: BallPhase,
    depth: DepthCubes,
    events: EventSounds,
    /// The ball is drawn (not while a New Ball is coming, nor once taken away).
    ball_shown: bool,
    /// The camera stops following while a lost ball falls on.
    camera_frozen: bool,
    /// The score, once the level is finished.
    score: Option<Score>,
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
    /// `key` is the level's scene key (its number gives the level bonus).
    #[allow(clippy::too_many_lines, reason = "sets up every play module in turn")]
    pub fn new(
        gpu: &Gpu,
        bundle: &mut Bundle,
        key: &str,
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
        let energy = Energy::new(
            &physics.energy,
            crate::energy::level_index(key).unwrap_or(0),
        );
        let point_prefab = bundle.scene("ph/p_extra_point").ok();
        let pickups = Pickups::new(&physics.energy, level, point_prefab.as_ref());
        let hud = Hud::new(
            gpu,
            bundle,
            &physics.hud,
            format,
            u32::try_from(energy.lives).unwrap_or(0),
        )?;
        let (sky, lighting) = environment(gpu, bundle, &physics, key, format)?;
        let clouds = Clouds::new(&physics.environment, key, level);
        let flames = Flames::new(level, &physics.particles, |k| bundle.scene(k).ok());
        let effects = Effects::new(level, &physics.particles, |k| bundle.scene(k).ok());
        let life_looks = LifeLooks::new(gpu, bundle, level, &physics.energy.extra_life, format)?;
        let point_looks = PointLooks::new(
            gpu,
            bundle,
            level,
            &physics.energy.extra_point,
            &physics.particles,
            format,
        )?;
        let mut particles = ParticleRenderer::new(gpu, format);
        let mut particle_textures = std::collections::HashMap::new();
        let keys: Vec<Option<String>> = flames
            .emitters()
            .chain(effects.emitters())
            .chain(point_looks.emitters())
            .map(|e| e.params.texture.clone())
            .chain(life_looks.sprite_textures())
            .chain(point_looks.sprite_textures())
            .collect();
        for key in keys {
            if particle_textures.contains_key(&key) {
                continue;
            }
            let index = match &key {
                Some(k) => {
                    let bytes = bundle.texture(k)?;
                    particles.add_texture(gpu, &upload_ktx2(gpu, k, &bytes)?)
                }
                None => particles.add_white(gpu),
            };
            particle_textures.insert(key, index);
        }
        let depth = DepthCubes::new(level, &physics.flow.depth_cubes);
        let events = EventSounds::new(&physics.flow);
        let flow = physics.flow.clone();

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
            energy,
            pickups,
            hud,
            sky,
            lighting,
            clouds,
            flames,
            effects,
            life_looks,
            point_looks,
            particles,
            particle_textures,
            score: None,
            flow,
            phase: BallPhase::Alive,
            depth,
            events,
            ball_shown: true,
            camera_frozen: false,
            looping: Vec::new(),
            sounds: Vec::new(),
            sound_counts: std::collections::BTreeMap::new(),
            held: None,
            progress,
            elapsed: 0.0,
            finished_at: None,
            respawns: 0,
        };
        // Start Level, then the BallManager's New Ball.
        play.events.emit("Start Level");
        play.begin_new_ball();
        play.show_active_ball();
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
        if self.finished_at.is_some() || self.transformers.busy() || self.phase != BallPhase::Alive
        {
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

    /// Development aid: puts the ball, at rest, 2 above the nearest fan in
    /// an active sector (where its wind starts).
    pub fn warp_to_fan(&mut self) {
        let Some(id) = self.ball_id() else { return };
        let Some(ball) = self.world.body(id).map(|b| b.position) else {
            return;
        };
        let nearest = self
            .modules
            .fans()
            .into_iter()
            .filter(|f| f.1)
            .map(|f| f.0)
            .min_by(|a, b| a.distance(ball).total_cmp(&b.distance(ball)));
        if let Some(target) = nearest {
            let at = target + Vec3::Y * 2.0;
            self.world.reset_body(id, at);
            self.follow.reset(at, self.heading_view);
        }
    }

    /// Development aid: puts the ball, at rest, 8 from the nearest pickup
    /// (towards where it was), to look at it without taking it.
    pub fn warp_near_pickup(&mut self) {
        let Some(id) = self.ball_id() else { return };
        let Some(ball) = self.world.body(id).map(|b| b.position) else {
            return;
        };
        if let Some(target) = self.pickups.nearest(ball) {
            let mut away = ball - target;
            away.y = 0.0;
            let at = target + away.normalize_or(Vec3::X) * 8.0;
            self.world.reset_body(id, at);
            self.follow.reset(at, self.heading_view);
        }
    }

    /// Development aid: puts the ball, at rest, on the nearest pickup.
    pub fn warp_to_pickup(&mut self) {
        let Some(id) = self.ball_id() else { return };
        let Some(ball) = self.world.body(id).map(|b| b.position) else {
            return;
        };
        if let Some(target) = self.pickups.nearest(ball) {
            self.world.reset_body(id, target);
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
                    .set_object_hidden(*o, i != self.active_ball || !self.ball_shown);
            }
        }
    }

    /// New Ball: ball and camera rig to the reset point, the lightning, and
    /// the ball after `new_ball_ms`.
    fn begin_new_ball(&mut self) {
        if self.held.is_none() {
            self.world.remove_body(self.ball);
            self.pushes = [None; 4];
        }
        self.held = Some((self.start, Quat::IDENTITY));
        self.ball_shown = false;
        self.show_active_ball();
        self.camera_frozen = false;
        self.follow.reset(self.start, self.start_heading);
        self.heading_ref = self.start_heading;
        self.heading_view = self.start_heading;
        self.turn = None;
        self.snap_next_frame = None;
        self.events.emit("Ball_Lightning");
        self.phase = BallPhase::Coming {
            ms: self.flow.new_ball_ms,
        };
    }

    /// The ball's phase, one frame on.
    fn advance_phase(&mut self, real_dt: f32) {
        match self.phase {
            BallPhase::Coming { ms } => {
                let ms = ms - real_dt * 1000.0;
                if ms > 0.0 {
                    self.phase = BallPhase::Coming { ms };
                    return;
                }
                // physicalize new Ball, Show; Physics WakeUp a frame later.
                let desc = body_desc(&self.physics.balls[self.active_ball], self.start);
                self.ball = self.world.add_body(desc);
                self.world.report_collisions(self.ball, true);
                self.held = None;
                self.ball_shown = true;
                self.show_active_ball();
                self.phase = BallPhase::Waking;
            }
            BallPhase::Waking => {
                // Counter active, BallNav activate.
                self.phase = BallPhase::Alive;
                self.energy.set_counting(true);
                self.nav_on();
                self.ball_sounds.set_active(self.finished_at.is_none());
            }
            BallPhase::Lost { frames, ms } => match ms {
                None if frames > 1 => {
                    self.phase = BallPhase::Lost {
                        frames: frames - 1,
                        ms: None,
                    };
                }
                None => {
                    // The lives check.
                    if self.energy.has_lives() {
                        self.pieces.reset_all();
                        self.phase = BallPhase::Lost {
                            frames: 0,
                            ms: Some(self.flow.lost_ms),
                        };
                    } else {
                        self.energy.set_game_over();
                        self.phase = BallPhase::GameOver;
                        eprintln!("game over");
                    }
                }
                Some(ms) => {
                    let ms = ms - real_dt * 1000.0;
                    if ms > 0.0 {
                        self.phase = BallPhase::Lost {
                            frames: 0,
                            ms: Some(ms),
                        };
                        return;
                    }
                    // Unphysicalize, Hide; a life; the sector reset.
                    let pose = self
                        .world
                        .body(self.ball)
                        .map(|b| (b.position, b.orientation));
                    self.world.remove_body(self.ball);
                    self.pushes = [None; 4];
                    self.held = Some(pose.unwrap_or((self.start, Quat::IDENTITY)));
                    self.ball_shown = false;
                    self.show_active_ball();
                    self.energy.lose_life();
                    if let Some(m) = self.progress.reset_point() {
                        (self.start, self.start_heading) = reset_pose(m);
                    }
                    self.progress.respawn();
                    self.apply_sector_changes();
                    self.respawns += 1;
                    self.phase = BallPhase::Gone;
                }
            },
            BallPhase::Gone => self.begin_new_ball(),
            BallPhase::Alive | BallPhase::GameOver => {}
        }
    }

    /// Deactivate Ball: control, camera and counter off; the lives are checked
    /// two frames later.
    fn lose_ball(&mut self) {
        if self.phase != BallPhase::Alive || self.held.is_some() || self.finished_at.is_some() {
            return;
        }
        self.nav_off();
        self.ball_sounds.set_active(false);
        self.energy.set_counting(false);
        self.camera_frozen = true;
        self.events.emit("Ball Off");
        self.hud.ball_off();
        self.phase = BallPhase::Lost {
            frames: LOST_CHECK_FRAMES,
            ms: None,
        };
    }

    fn queue_sounds(&mut self, sounds: impl IntoIterator<Item = (String, f32)>) {
        for (key, volume) in sounds {
            *self.sound_counts.entry(key.clone()).or_default() += 1;
            self.sounds.push((key, volume));
        }
    }

    /// Level objects to show (`true`) or hide since the last call (pickups'
    /// markers).
    pub fn take_level_visibility(&mut self) -> Vec<(usize, bool)> {
        self.pickups.take_visibility()
    }

    /// Development aid: active moving elements with a part that exploded or
    /// fell below the lowest depth cube.
    pub fn fallen_modules(&self) -> Vec<String> {
        self.modules
            .fallen(&self.world, self.depth.bottom.unwrap_or(self.fall_y))
    }

    /// Points, lives, pickups and score, for headless runs.
    pub fn energy_summary(&self) -> String {
        let (lives, points) = self.pickups.count();
        format!(
            "energy: {} points, {} lives{}{}; {lives} extra lives, {points} extra points in the level",
            self.energy.points,
            self.energy.lives,
            if self.energy.game_over {
                ", game over"
            } else {
                ""
            },
            self.score
                .map(|s| format!(", score {}", s.total))
                .unwrap_or_default(),
        )
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

    /// The `BallManager`'s depth test (or, without cubes, a fall height).
    fn depth_test(&mut self) {
        let ball_box = self
            .ball_id()
            .and_then(|id| self.world.body(id))
            .map(|b| (b.position, b.orientation, Vec3::splat(b.radius())));
        if self.phase == BallPhase::Alive
            && let Some(bb) = ball_box
        {
            let lost = if self.depth.is_empty() {
                bb.0.y < self.fall_y
            } else {
                self.depth.check(bb)
            };
            if lost {
                self.lose_ball();
            }
        }
    }

    /// The pickups this frame, and their messages to the sound scripts.
    fn pickup_frame(&mut self, ball: Vec3, real_dt: f32) {
        for e in self.pickups.frame(ball, real_dt) {
            match e {
                pickups::Event::LifeUp(marker) => {
                    self.effects.life_up(marker);
                    self.energy.add_life();
                    self.events.emit("Life_Up");
                    self.hud.life_up();
                }
                pickups::Event::Activated(n) => {
                    self.energy.add_points(n);
                    self.events.emit("Extrapoint_Hit");
                    self.hud.glow();
                }
                pickups::Event::Hits(n) => {
                    self.energy.add_points(n);
                    self.events.emit("Extrapoint_Pieces");
                }
            }
        }
    }

    /// Development aid: loses the ball now, as if it fell out (it comes back
    /// at the current reset point, a life less).
    pub fn respawn(&mut self) {
        self.lose_ball();
    }

    fn apply_sector_changes(&mut self) {
        for (sector, on) in self.progress.take_changes() {
            self.props.set_sector_active(&mut self.world, sector, on);
            self.transformers.set_sector_active(sector, on);
            self.pickups.set_sector_active(sector, on);
            self.modules.set_sector_active(&mut self.world, sector, on);
        }
    }

    /// One frame: camera script, then physics.
    pub fn update(&mut self, real_dt: f32) {
        self.hud.frame(real_dt);
        if let Some(c) = self.clouds.as_mut() {
            c.seconds += real_dt;
        }
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

        self.advance_phase(real_dt);
        self.world.advance(real_dt);
        // DepthTest: props of its groups this far below the lowest cube go.
        let prop_fall_y = self
            .depth
            .bottom
            .map_or(self.fall_y, |b| b - self.flow.prop_depth_margin);
        self.props.update(&mut self.world, prop_fall_y);
        self.energy.frame(real_dt);
        let s = self.events.frame(real_dt);
        self.queue_sounds(s);
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
                Some(progress::Event::Checkpoint(i)) => {
                    self.events.emit("Checkpoint reached");
                    eprintln!("checkpoint {} reached", i + 1);
                }
                Some(progress::Event::Finished) => {
                    // Level_Finish: ball control off; the balloon lets go.
                    self.finished_at = Some((self.elapsed, self.world.steps));
                    self.modules.level_finished(&mut self.world);
                    self.nav_off();
                    self.ball_sounds.set_active(false);
                    // Counter inactive; the score screen's sum.
                    self.energy.set_counting(false);
                    let s = self.energy.score();
                    self.score = Some(s);
                    eprintln!(
                        "score {}: level bonus {} + points {} + lives {}",
                        s.total, s.level_bonus, s.points, s.life_bonus
                    );
                    eprintln!(
                        "level finished after {:.2} s ({} physics steps)",
                        self.elapsed, self.world.steps
                    );
                }
                None => {}
            }
            let events = self.transformers.frame(ball, self.active_ball, real_dt);
            self.transformer_events(events);
            self.pickup_frame(ball, real_dt);
        }
        self.apply_sector_changes();

        self.depth_test();
        let ball = self.ball_pose().map_or(self.start, |(p, _)| p);
        let reached = self.progress.reached();
        self.flames.update(ball, reached, real_dt * 1000.0);
        let lives = self.pickups.lives();
        self.life_looks.update(ball, &lives, real_dt);
        let points = self.pickups.points();
        self.point_looks.update(&points, ball, real_dt);
        let fans = self.modules.fans();
        self.effects.update(ball, &fans, real_dt * 1000.0);
        if !self.camera_frozen {
            let ball = self.ball_pose().map_or(self.start, |(p, _)| p);
            let lifted = self.keys_held[key_index(PlayKey::Lift)];
            self.follow.update(real_dt, ball, self.heading_view, lifted);
        }
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
            .chain(self.pickups.lives().into_iter().map(|(marker, _)| marker))
            .chain(self.pickups.points().into_iter().map(|p| p.marker))
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
        self.life_looks
            .render(gpu, encoder, color, depth, camera, aspect, &opts);
        self.point_looks
            .render(gpu, encoder, color, depth, camera, aspect, &opts);
        self.render_particles(gpu, encoder, color, depth, camera, aspect);
    }

    /// Flame emitters burning now (for headless runs).
    pub fn flames_burning(&self) -> usize {
        self.flames.burning()
    }

    /// Fans whose particles run now (for headless runs).
    pub fn fans_running(&self) -> usize {
        self.effects.fans_running()
    }

    /// Extra lives shown now (for headless runs).
    pub fn lives_shown(&self) -> usize {
        self.life_looks.shown()
    }

    /// Extra points shown now (for headless runs).
    pub fn points_shown(&self) -> usize {
        self.point_looks.shown()
    }

    /// The particle systems, last (blended, no depth writes).
    fn render_particles(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
    ) {
        type Group = (
            usize,
            (ballistic_bundle::BlendFactor, ballistic_bundle::BlendFactor),
            Vec<ballistic_render::ParticleVertex>,
            Vec<ballistic_render::ParticleVertex>,
        );
        let right = camera.right();
        let facing = crate::particles::Facing {
            right,
            up: right.cross(camera.forward()).normalize_or(Vec3::Y),
            forward: camera.forward(),
        };
        // One batch of triangles and one of lines per texture and blend.
        let mut groups: Vec<Group> = Vec::new();
        for e in self
            .flames
            .emitters()
            .chain(self.effects.emitters())
            .chain(self.point_looks.emitters())
        {
            let Some(&texture) = self.particle_textures.get(&e.params.texture) else {
                continue;
            };
            let blend = e.params.blend;
            let i = groups
                .iter()
                .position(|g| g.0 == texture && g.1 == blend)
                .unwrap_or_else(|| {
                    groups.push((texture, blend, Vec::new(), Vec::new()));
                    groups.len() - 1
                });
            let g = &mut groups[i];
            e.vertices(facing, &mut g.2, &mut g.3);
        }
        // The extra lives' and extra points' silver balls (sprites).
        let sprites = self
            .life_looks
            .sprites(facing)
            .into_iter()
            .chain(self.point_looks.sprites(facing));
        for (key, blend, quads) in sprites {
            if let Some(&texture) = self.particle_textures.get(&key) {
                groups.push((texture, blend, quads, Vec::new()));
            }
        }
        let batches: Vec<ParticleBatch<'_>> = groups
            .iter()
            .flat_map(|(texture, blend, triangles, lines)| {
                [
                    ParticleBatch {
                        texture: *texture,
                        blend: *blend,
                        lines: false,
                        vertices: triangles,
                    },
                    ParticleBatch {
                        texture: *texture,
                        blend: *blend,
                        lines: true,
                        vertices: lines,
                    },
                ]
            })
            .collect();
        self.particles
            .render(gpu, encoder, color, depth, camera, aspect, &batches);
    }

    /// The options to draw the level and play objects with: the level's
    /// light and the scene's ambient, over the sky.
    pub fn scene_options(&self, opts: &RenderOptions) -> RenderOptions {
        let l = &self.lighting;
        RenderOptions {
            light_dir: l.towards_light,
            light_color: l.light_color,
            ambient: l.ambient,
            clear_color: l.background,
            keep_color: self.sky.is_some(),
            fog: None,
            ..*opts
        }
    }

    /// Changes to the level's own rendering: the cloud layer (shown, blended,
    /// scrolled). Call each frame before drawing the level.
    pub fn apply_level(&mut self, gpu: &Gpu, level: &mut ballistic_render::SceneRenderer) {
        if let Some(c) = self.clouds.as_mut() {
            c.apply(gpu, level);
        }
    }

    /// Clears `color` and draws the sky (if the level has one).
    pub fn render_sky(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        camera: &Camera,
        aspect: f32,
    ) {
        if let Some(sky) = &self.sky {
            sky.render(
                gpu,
                encoder,
                color,
                camera,
                aspect,
                self.lighting.background,
            );
        }
    }

    /// The HUD over a `size` (pixels) frame.
    pub fn render_hud(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        self.hud
            .render(gpu, encoder, color, size, self.energy.points);
    }

    pub fn status(&self) -> String {
        let b = self.ball_id().and_then(|id| self.world.body(id));
        format!(
            "{} speed {:.1} points {} lives {} checkpoint {}/{} {}{}{}{}{}{}{}",
            self.ball_name(),
            b.map_or(0.0, |b| b.velocity.length()),
            self.energy.points,
            self.energy.lives,
            self.progress.reached(),
            self.progress.checkpoint_count(),
            self.finished_at
                .map(|(t, _)| format!("[finished {t:.2} s] "))
                .unwrap_or_default(),
            match self.phase {
                BallPhase::Coming { .. } | BallPhase::Waking => "[new ball] ",
                BallPhase::Lost { .. } | BallPhase::Gone => "[ball lost] ",
                _ => "",
            },
            match (self.score, self.energy.game_over) {
                (Some(s), _) => format!("[score {}] ", s.total),
                (None, true) => "[game over] ".to_owned(),
                _ => String::new(),
            },
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

/// Light, ambient and background for the scene renderer.
struct Lighting {
    towards_light: glam::Vec3,
    light_color: glam::Vec3,
    ambient: glam::Vec3,
    background: [f64; 4],
}

/// The level's sky and lighting (research findings 19). Without an entry
/// for the level, no sky and the renderer's default light.
fn environment(
    gpu: &Gpu,
    bundle: &mut Bundle,
    physics: &PhysicsData,
    key: &str,
    format: wgpu::TextureFormat,
) -> Result<(Option<SkyRenderer>, Lighting)> {
    let env = &physics.environment;
    let Some(level) = env.levels.iter().find(|l| l.level == key) else {
        let d = RenderOptions::default();
        return Ok((
            None,
            Lighting {
                towards_light: d.light_dir,
                light_color: d.light_color,
                ambient: d.ambient,
                background: d.clear_color,
            },
        ));
    };
    let faces = env
        .sky
        .sides
        .iter()
        .chain(&env.sky.top)
        .chain(&env.sky.bottom)
        .map(|suffix| {
            let key = format!("{}{suffix}", level.sky);
            let bytes = bundle.texture(&key)?;
            Ok(upload_ktx2(gpu, &key, &bytes)?)
        })
        .collect::<Result<Vec<_>>>()?;
    let sky = SkyRenderer::new(gpu, format, &env.sky, &faces);
    let [r, g, b, _] = level.light;
    Ok((
        Some(sky),
        Lighting {
            towards_light: -glam::Vec3::from(env.light_direction),
            light_color: glam::Vec3::new(r, g, b),
            ambient: glam::Vec3::from(env.ambient),
            background: env.background.map(f64::from),
        },
    ))
}

/// The level's cloud layer, if it has one and it's on (`init SkyLayer`,
/// `animate SkyLayer`): shown, prelit, blended, its texture scrolling.
struct Clouds {
    object: usize,
    materials: Vec<usize>,
    params: ballistic_bundle::CloudLayer,
    scroll: [f32; 2],
    set_up: bool,
    seconds: f32,
}

impl Clouds {
    fn new(
        env: &ballistic_bundle::Environment,
        key: &str,
        level: &ballistic_bundle::Scene,
    ) -> Option<Self> {
        let params = &env.cloud_layer;
        if !params.enabled {
            return None;
        }
        let scroll = env.levels.iter().find(|l| l.level == key)?.cloud_scroll;
        let object = level.objects.iter().position(|o| o.name == params.object)?;
        let mesh = level.meshes.get(level.objects[object].mesh? as usize)?;
        let mut materials: Vec<usize> = mesh
            .submeshes
            .iter()
            .filter_map(|s| s.material.map(|m| m as usize))
            .collect();
        materials.dedup();
        Some(Self {
            object,
            materials,
            params: params.clone(),
            scroll,
            set_up: false,
            seconds: 0.0,
        })
    }

    fn apply(&mut self, gpu: &Gpu, level: &mut ballistic_render::SceneRenderer) {
        use ballistic_bundle::BlendFactor as B;
        if !self.set_up {
            self.set_up = true;
            level.set_object_hidden(self.object, false);
            for &m in &self.materials {
                level.set_material_blend(
                    m,
                    self.params.additive.then_some((B::One, B::One)),
                    self.params.z_write,
                );
                level.set_material_prelit(gpu, m, self.params.color);
            }
        }
        let offset = self.scroll.map(|v| (v * self.seconds).rem_euclid(1.0));
        for &m in &self.materials {
            level.set_material_uv_offset(gpu, m, offset);
        }
    }
}
