//! The old ball's pieces when a transformer bursts it (research findings 14).
//!
//! `Ball_Explosion_<Type>`: the type's pieces frame goes to the ball's
//! position and is shown. Each piece becomes a convex body from its own mesh,
//! with the mass and friction drawn from the script's ranges, and gets one
//! *Physics Impulse* of a random size along its own +Y (outwards: each piece's
//! Y axis points away from the ball's centre), a little off its centre so it
//! spins.
//!
//! The pieces are in collision group "Ball", like the ball: they don't touch
//! the ball or each other, only the level.
//!
//! The *Fadeout Manager* starts a 20 s timer for the old type at each
//! transformation. When it runs out, or when the ball transforms from that type
//! again first, `Ball_ResetPieces_<Type>` fades the pieces' materials out
//! (2 s) and removes them. A respawn fades all types' pieces out.
//!
//! The original draws the random values from its own generator; here a fixed
//! seed keeps runs repeatable. Pieces can't affect the ball either way.
//!
//! Sounds (`Ball_CollSound_Pieces`): the paper pieces play `Pieces_Paper` once
//! at the burst. For 3 s after a wood or stone burst, six of the pieces play
//! `Pieces_Wood` / `Pieces_Stone` when they hit something faster than 2, at
//! `min(speed / 25, 1)` volume (the original's `PhysicsCollDetection`: relative
//! speed over its maximum), then stay quiet for 0.5 s of simulated time.

use ballistic_bundle::{BallPieces, PhysicsData, Scene};
use ballistic_physics::modules::convex_shape;
use ballistic_physics::{BodyDesc, BodyId, Mat4, Vec3, World};
use ballistic_render::{Gpu, SceneRenderer};
use std::collections::HashMap;

struct Piece {
    /// Object index in the balls scene.
    object: usize,
    /// Its matrix relative to the pieces frame.
    local: Mat4,
    /// Its mesh's vertices in its own frame.
    points: Vec<Vec3>,
}

struct Set {
    params: BallPieces,
    /// Ball type (index into the physics data's balls).
    ball: usize,
    /// The pieces frame's matrix in the balls scene (its orientation is kept
    /// when it's moved to the ball).
    frame_rest: Mat4,
    pieces: Vec<Piece>,
    /// Faded materials (balls scene indices) and their colours at rest.
    materials: Vec<(usize, [f32; 4])>,
    /// Pieces in the world: body, piece, the object's matrix relative to the
    /// body.
    out: Vec<(BodyId, usize, Mat4)>,
    /// The Fadeout Manager's timer for this type.
    timer_ms: Option<f32>,
    /// The fade, while it runs.
    fade_ms: Option<f32>,
    /// Pieces (indices) whose collisions play the sound.
    colliders: Vec<usize>,
    /// Time left to listen for collisions, and each listening body's last
    /// sound (simulated seconds).
    listen_ms: Option<f32>,
    last_hit: HashMap<BodyId, f64>,
    /// Draw state changed (shown/hidden, materials).
    dirty: bool,
}

pub struct Pieces {
    sets: Vec<Set>,
    fade_after_ms: f32,
    rng: u64,
}

impl Pieces {
    /// `scene` is the `balls` scene (the pieces are its objects).
    pub fn new(physics: &PhysicsData, scene: &Scene) -> Self {
        let world = placed(scene);
        let index = |name: &str| scene.objects.iter().position(|o| o.name == name);
        let mut sets = Vec::new();
        for p in &physics.transformers.pieces {
            let (Some(ball), Some(frame)) = (
                physics.balls.iter().position(|b| b.name == p.ball),
                index(&p.frame),
            ) else {
                eprintln!("warning: pieces of {}: ball or frame missing", p.ball);
                continue;
            };
            let frame_rest = world[frame];
            let members = scene
                .groups
                .iter()
                .find(|g| g.name == p.group)
                .map(|g| g.members.clone())
                .unwrap_or_default();
            let pieces: Vec<Piece> = members
                .iter()
                .filter_map(|&m| {
                    let m = m as usize;
                    let mesh = &scene.meshes[scene.objects.get(m)?.mesh? as usize];
                    Some(Piece {
                        object: m,
                        local: frame_rest.inverse() * world[m],
                        points: mesh
                            .vertices()
                            .iter()
                            .map(|v| Vec3::from(v.position))
                            .collect(),
                    })
                })
                .collect();
            let colliders = p
                .sounds
                .colliders
                .iter()
                .filter_map(|n| {
                    let o = index(n)?;
                    pieces.iter().position(|p: &Piece| p.object == o)
                })
                .collect();
            let materials = p
                .materials
                .iter()
                .filter_map(|n| {
                    let i = scene.materials.iter().position(|m| &m.name == n)?;
                    Some((i, scene.materials[i].diffuse))
                })
                .collect();
            sets.push(Set {
                params: p.clone(),
                ball,
                frame_rest,
                pieces,
                materials,
                out: Vec::new(),
                timer_ms: None,
                fade_ms: None,
                colliders,
                listen_ms: None,
                last_hit: HashMap::new(),
                dirty: false,
            });
        }
        Self {
            sets,
            fade_after_ms: physics.transformers.pieces_fade_after_ms,
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// A transformation from `ball` starts (*set Piecesflag*).
    pub fn triggered(&mut self, ball: usize) {
        for s in self.sets.iter_mut().filter(|s| s.ball == ball) {
            if s.timer_ms.is_some() {
                s.fade_ms.get_or_insert(0.0);
            }
            s.timer_ms = Some(0.0);
        }
    }

    /// `Ball_Explosion_<Type>` at `at` (the old ball's position), then
    /// `Ball_CollSound_Pieces`. Returns sounds to play now (key, volume).
    pub fn burst(&mut self, world: &mut World, ball: usize, at: Vec3) -> Vec<(String, f32)> {
        let Some(si) = self.sets.iter().position(|s| s.ball == ball) else {
            return Vec::new();
        };
        let mut rng = self.rng;
        let mut rand = |r: [f32; 2]| {
            // xorshift64*
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            let u = (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32;
            r[0] + (r[1] - r[0]) * u
        };
        let s = &mut self.sets[si];
        for (id, _, _) in s.out.drain(..) {
            world.remove_body(id);
        }
        s.fade_ms = None;
        let (_, rot, _) = s.frame_rest.to_scale_rotation_translation();
        let frame = Mat4::from_translation(at) * Mat4::from_quat(rot);
        let p = &s.params;
        for (pi, piece) in s.pieces.iter().enumerate() {
            let m = frame * piece.local;
            let points: Vec<Vec3> = piece
                .points
                .iter()
                .map(|&v| m.transform_point3(v))
                .collect();
            // Automatic mass centre off, no shift: the object's origin.
            let center = m.transform_point3(Vec3::ZERO);
            let (shape, object_from_body) = convex_shape(&[points], m, center);
            let (_, orientation, _) = m.to_scale_rotation_translation();
            let id = world.add_body(BodyDesc {
                position: center,
                orientation,
                shape,
                mass: rand(p.mass),
                friction: rand(p.friction),
                elasticity: p.elasticity,
                linear_damping: p.linear_damping,
                rotational_damping: p.rotational_damping,
                group: p.collision_group.clone(),
                asleep: false,
                collides: true,
                sound: ballistic_physics::SoundIds::default(),
            });
            // Physics Impulse: direction and position in the piece's frame.
            world.apply_impulse_at(
                id,
                m.transform_vector3(Vec3::from(p.impulse_direction)),
                rand(p.impulse),
                m.transform_point3(Vec3::from(p.impulse_position)),
            );
            if s.colliders.contains(&pi) {
                world.report_collisions(id, true);
            }
            s.out.push((id, pi, object_from_body));
        }
        s.dirty = true;
        self.rng = rng;
        s.last_hit.clear();
        if s.colliders.is_empty() {
            // Paper: one Wave Player play.
            s.listen_ms = None;
            vec![(s.params.sounds.sound.clone(), 1.0)]
        } else {
            s.listen_ms = Some(s.params.sounds.listen_ms);
            Vec::new()
        }
    }

    /// A respawn: every type's pieces fade out (*reset Ballpieces*).
    pub fn reset_all(&mut self) {
        for s in &mut self.sets {
            if !s.out.is_empty() {
                s.fade_ms.get_or_insert(0.0);
            }
        }
    }

    /// One frame: the collision sounds, the timers and fades. Returns sounds
    /// to play (key, volume).
    /// `hits`: the world's collisions this frame.
    pub fn frame(
        &mut self,
        world: &mut World,
        hits: &[ballistic_physics::Collision],
        real_dt: f32,
    ) -> Vec<(String, f32)> {
        let ms = real_dt * 1000.0;
        let mut sounds = Vec::new();
        for s in &mut self.sets {
            let snd = &s.params.sounds;
            for h in hits {
                let piece = s
                    .out
                    .iter()
                    .find(|(id, _, _)| *id == h.body)
                    .map(|(_, pi, _)| *pi);
                let Some(pi) = piece else { continue };
                if s.listen_ms.is_none() || !s.colliders.contains(&pi) || h.speed <= snd.min_speed {
                    continue;
                }
                if s.last_hit
                    .get(&h.body)
                    .is_some_and(|&t| h.time - t < f64::from(snd.sleep_s))
                {
                    continue;
                }
                s.last_hit.insert(h.body, h.time);
                let volume = if snd.max_speed > 0.0 {
                    (h.speed / snd.max_speed).min(1.0)
                } else {
                    1.0
                };
                sounds.push((snd.sound.clone(), volume));
            }
            if let Some(l) = s.listen_ms.as_mut() {
                *l -= ms;
                if *l <= 0.0 {
                    // The Delayer stops the collision tests.
                    s.listen_ms = None;
                    for &(id, _, _) in &s.out {
                        world.report_collisions(id, false);
                    }
                }
            }
            if let Some(t) = s.timer_ms.as_mut() {
                *t += ms;
                if *t >= self.fade_after_ms {
                    s.timer_ms = None;
                    s.fade_ms.get_or_insert(0.0);
                }
            }
            if let Some(f) = s.fade_ms.as_mut() {
                *f += ms;
                s.dirty = true;
                if s.params.fade.done(*f) {
                    // De Physicalize, Restore IC (materials and frame).
                    s.fade_ms = None;
                    for (id, _, _) in s.out.drain(..) {
                        world.remove_body(id);
                    }
                }
            }
        }
        sounds
    }

    /// Pieces in the world (all types).
    pub fn count(&self) -> usize {
        self.sets.iter().map(|s| s.out.len()).sum()
    }

    /// Draws the pieces into the balls scene's renderer.
    pub fn sync(&mut self, gpu: &Gpu, r: &mut SceneRenderer, world: &World) {
        for s in &mut self.sets {
            for &(id, pi, object_from_body) in &s.out {
                if let Some((p, q)) = world.interpolated(id) {
                    let pose = Mat4::from_rotation_translation(q, p);
                    r.set_object_transform(gpu, s.pieces[pi].object, pose * object_from_body);
                }
            }
            if !s.dirty {
                continue;
            }
            s.dirty = false;
            for (pi, piece) in s.pieces.iter().enumerate() {
                let out = s.out.iter().any(|&(_, i, _)| i == pi);
                r.set_object_hidden(piece.object, !out);
            }
            let [c0, c1] = s.params.fade_colors;
            for &(m, rest) in &s.materials {
                let c = match s.fade_ms {
                    Some(f) => {
                        let v = s.params.fade.value(f);
                        std::array::from_fn(|i| c0[i] + (c1[i] - c0[i]) * v)
                    }
                    None => rest,
                };
                r.set_material_diffuse(gpu, m, c);
            }
        }
    }
}

/// World matrices of a scene's objects (its hierarchy applied).
fn placed(scene: &Scene) -> Vec<Mat4> {
    fn world_of(scene: &Scene, i: usize, out: &mut [Option<Mat4>]) -> Mat4 {
        if let Some(m) = out[i] {
            return m;
        }
        let o = &scene.objects[i];
        let local = Mat4::from_cols_array_2d(&o.transform);
        let m = match o.parent {
            Some(p) => world_of(scene, p as usize, out) * local,
            None => local,
        };
        out[i] = Some(m);
        m
    }
    let mut out: Vec<Option<Mat4>> = vec![None; scene.objects.len()];
    (0..scene.objects.len())
        .map(|i| world_of(scene, i, &mut out))
        .collect()
}
