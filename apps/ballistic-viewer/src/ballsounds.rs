//! The ball's hit and roll sounds, like `Sound.nmo`'s `Sound_Manager`
//! (research findings 15).
//!
//! Each ball type has its own sounds (`BallSound`). Surfaces and props carry a
//! hit ID (1 stone, 2 wood, 3 metal, 4 dome) and a roll ID (1 stone, 2 wood,
//! 3 metal): level objects from their sound groups, prefab objects from their
//! attributes.
//!
//! While ball control is on (between the `BallNav activate` and `deactivate`
//! messages: not during a transformation or after the finish):
//! - **Hits**: one test per hit ID. A hit on something with that ID, faster
//!   than the test's minimum, plays the ball's hit sound for that ID at
//!   `min(speed / max, 1)`; then that test waits its sleep time (simulated).
//! - **Rolls**: a roll ID's sound starts looping after 0.3 s of continuous
//!   contact with something of that ID, and stops after 0.3 s without
//!   (simulated time). The paper ball has one roll sound for all IDs: any ID
//!   going off stops it.
//!
//! Always: the roll sounds' volume is the ball's speed (units per real second,
//! measured from frame to frame) × 0.05, their pitch 0.5 + speed × 0.01.

use std::collections::BTreeMap;

use ballistic_bundle::{BallSounds, PhysicsData};
use ballistic_physics::{BodyId, Collision, Vec3, World};

#[derive(Debug, Clone, Copy, Default)]
struct RollGroup {
    on: bool,
    /// When continuous contact started (while off).
    touching_since: Option<f64>,
    /// When contact ended (while on).
    apart_since: Option<f64>,
}

pub struct BallSoundsPlayer {
    params: BallSounds,
    /// Sound set per ball type (index into the physics data's balls).
    set_of: Vec<Option<usize>>,
    active: bool,
    /// Last report of each hit test (simulated seconds).
    last_hit: Vec<Option<f64>>,
    roll: Vec<RollGroup>,
    /// Roll sounds playing (key → playing).
    playing: BTreeMap<String, bool>,
    last_pos: Option<Vec3>,
    speed: f32,
    /// The wooden flaps' last hit (simulated seconds).
    last_flap: Option<f64>,
}

impl BallSoundsPlayer {
    pub fn new(physics: &PhysicsData) -> Self {
        let params = physics.sounds.clone();
        let set_of = physics
            .balls
            .iter()
            .map(|b| params.sets.iter().position(|s| s.ball == b.name))
            .collect();
        let groups = params.sets.first().map_or(0, |s| s.roll.len());
        Self {
            last_hit: vec![None; params.hits.len()],
            roll: vec![RollGroup::default(); groups],
            params,
            set_of,
            active: false,
            playing: BTreeMap::new(),
            last_pos: None,
            last_flap: None,
            speed: 0.0,
        }
    }

    /// `BallNav activate` (`true`) creates the hit and roll tests afresh;
    /// `deactivate` stops them, and the roll sounds with them.
    pub fn set_active(&mut self, on: bool) {
        if on == self.active {
            return;
        }
        self.active = on;
        self.last_hit.fill(None);
        self.roll.fill(RollGroup::default());
        for p in self.playing.values_mut() {
            *p = false;
        }
    }

    /// One frame. Returns one-shot sounds (key, volume).
    pub fn frame(
        &mut self,
        world: &World,
        ball: Option<(BodyId, Vec3)>,
        ball_type: usize,
        hits: &[Collision],
        real_dt: f32,
    ) -> Vec<(String, f32)> {
        let mut out = Vec::new();
        // TT SpeedOMeter: distance since the last frame per real second.
        if let Some((_, pos)) = ball {
            if let Some(last) = self.last_pos
                && real_dt > 0.0
            {
                self.speed = pos.distance(last) / real_dt;
            }
            self.last_pos = Some(pos);
        }
        // HitSound Woodenflaps: from Start Music on, whatever the control.
        let f = &self.params.flaps;
        if let Some((id, _)) = ball
            && !f.sound.is_empty()
        {
            for h in hits.iter().filter(|h| h.body == id && h.other_sound.flap) {
                if h.speed < f.min_speed
                    || self
                        .last_flap
                        .is_some_and(|l| h.time - l < f64::from(f.sleep_s))
                {
                    continue;
                }
                self.last_flap = Some(h.time);
                let range = (f.max_speed - f.min_speed).max(f32::EPSILON);
                let volume = ((h.speed - f.min_speed) / range).clamp(0.0, 1.0);
                out.push((f.sound.clone(), volume));
            }
        }
        let Some(set) = self.set_of.get(ball_type).copied().flatten() else {
            return out;
        };
        let set = &self.params.sets[set];
        if !self.active {
            return out;
        }
        let Some((id, _)) = ball else { return out };
        // Hit Sounds.
        for h in hits.iter().filter(|h| h.body == id) {
            for (i, t) in self.params.hits.iter().enumerate() {
                if h.other_sound.hit != t.id || h.speed <= t.min_speed {
                    continue;
                }
                if self.last_hit[i].is_some_and(|l| h.time - l < f64::from(t.sleep_s)) {
                    continue;
                }
                self.last_hit[i] = Some(h.time);
                if let Some(key) = set.hit.get(usize::from(t.id) - 1) {
                    let volume = if t.max_speed > 0.0 {
                        (h.speed / t.max_speed).min(1.0)
                    } else {
                        1.0
                    };
                    out.push((key.clone(), volume));
                }
            }
        }
        // PhysicsContinuousContact, then the roll graph's Wave Players.
        let now = world.time();
        let touching = world.touching_sounds(id);
        let (start, end) = (
            f64::from(self.params.roll_delay_start_s),
            f64::from(self.params.roll_delay_end_s),
        );
        for (g, state) in self.roll.iter_mut().enumerate() {
            let roll_id = u8::try_from(g + 1).unwrap_or(u8::MAX);
            let key = &set.roll[g];
            if touching.iter().any(|s| s.roll == roll_id) {
                state.apart_since = None;
                if !state.on {
                    let since = *state.touching_since.get_or_insert(now);
                    if now - since >= start {
                        state.on = true;
                        state.touching_since = None;
                        self.playing.insert(key.clone(), true);
                    }
                }
            } else {
                state.touching_since = None;
                if state.on {
                    let since = *state.apart_since.get_or_insert(now);
                    if now - since >= end {
                        state.on = false;
                        state.apart_since = None;
                        self.playing.insert(key.clone(), false);
                    }
                }
            }
        }
        out
    }

    /// Roll sounds that should be looping now: key, volume, pitch.
    pub fn loops(&self) -> Vec<(String, f32, f32)> {
        let volume = (self.speed * self.params.roll_volume).clamp(0.0, 1.0);
        let [a, b] = self.params.roll_pitch;
        let pitch = a + self.speed * b;
        self.playing
            .iter()
            .filter(|(_, on)| **on)
            .map(|(k, _)| (k.clone(), volume, pitch))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballistic_bundle::{BallSoundSet, HitTest};
    use ballistic_physics::{SoundIds, StaticMesh, SurfaceMaterial, WorldConfig};

    fn params() -> BallSounds {
        BallSounds {
            sets: vec![BallSoundSet {
                ball: "Ball_Wood".into(),
                roll: vec!["roll_s".into(), "roll_w".into(), "roll_m".into()],
                hit: vec![
                    "hit_s".into(),
                    "hit_w".into(),
                    "hit_m".into(),
                    "hit_d".into(),
                ],
            }],
            hits: (1..=4)
                .map(|id| HitTest {
                    id,
                    min_speed: 2.0,
                    max_speed: 20.0,
                    sleep_s: 1.0,
                })
                .collect(),
            roll_delay_start_s: 0.3,
            roll_delay_end_s: 0.3,
            roll_volume: 0.05,
            roll_pitch: [0.5, 0.01],
            hit_groups: Vec::new(),
            roll_groups: Vec::new(),
            objects: Vec::new(),
            flaps: ballistic_bundle::GroupHitSound {
                group: "Phys_FloorStopper".into(),
                sound: "hit_woodenflap".into(),
                min_speed: 0.3,
                max_speed: 10.0,
                sleep_s: 0.5,
            },
        }
    }

    fn player() -> BallSoundsPlayer {
        let p = params();
        BallSoundsPlayer {
            last_hit: vec![None; 4],
            roll: vec![RollGroup::default(); 3],
            set_of: vec![Some(0)],
            params: p,
            active: true,
            playing: BTreeMap::new(),
            last_pos: None,
            last_flap: None,
            speed: 0.0,
        }
    }

    fn hit(body: BodyId, id: u8, speed: f32, time: f64) -> Collision {
        Collision {
            body,
            other: None,
            speed,
            other_sound: SoundIds {
                hit: id,
                ..SoundIds::default()
            },
            point: Vec3::ZERO,
            time,
        }
    }

    #[test]
    fn hits_play_by_id_speed_and_sleep() {
        let mut p = player();
        let mut world = World::new(WorldConfig::default(), StaticMesh::default());
        let b = world.add_body(ballistic_physics::BodyDesc::default());
        let ball = Some((b, Vec3::ZERO));
        let f = |p: &mut BallSoundsPlayer, hits: &[Collision]| {
            p.frame(&world, ball, 0, hits, 1.0 / 60.0)
        };
        let hit = |id, speed, time| hit(b, id, speed, time);
        // Wood at 10 of max 20: half volume.
        assert_eq!(
            f(&mut p, &[hit(2, 10.0, 1.0)]),
            vec![("hit_w".to_owned(), 0.5)]
        );
        // Too slow; then within the wood test's sleep; another ID is separate.
        assert_eq!(f(&mut p, &[hit(1, 1.5, 1.1)]), Vec::new());
        assert_eq!(f(&mut p, &[hit(2, 30.0, 1.5)]), Vec::new());
        assert_eq!(
            f(&mut p, &[hit(4, 30.0, 1.5)]),
            vec![("hit_d".to_owned(), 1.0)]
        );
        assert_eq!(
            f(&mut p, &[hit(2, 4.0, 2.1)]),
            vec![("hit_w".to_owned(), 0.2)]
        );
        // No ID: nothing. Control off: nothing.
        assert_eq!(f(&mut p, &[hit(0, 30.0, 9.0)]), Vec::new());
        p.set_active(false);
        assert_eq!(f(&mut p, &[hit(1, 30.0, 20.0)]), Vec::new());
    }

    #[test]
    fn the_flaps_sound_in_their_speed_range_then_sleep() {
        let mut p = player();
        let mut world = World::new(WorldConfig::default(), StaticMesh::default());
        let b = world.add_body(ballistic_physics::BodyDesc::default());
        let flap = |speed, time| Collision {
            other_sound: SoundIds {
                flap: true,
                ..SoundIds::default()
            },
            ..hit(b, 0, speed, time)
        };
        p.set_active(false);
        let mut f =
            |hits: &[Collision]| p.frame(&world, Some((b, Vec3::ZERO)), 0, hits, 1.0 / 60.0);
        // (5.15 − 0.3) / (10 − 0.3) = 0.5; even with control off.
        let out = f(&[flap(5.15, 1.0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "hit_woodenflap");
        assert!((out[0].1 - 0.5).abs() < 1e-4);
        assert_eq!(f(&[flap(5.0, 1.4)]), Vec::new(), "asleep");
        assert_eq!(f(&[flap(0.2, 2.0)]), Vec::new(), "too slow");
        assert_eq!(f(&[flap(50.0, 2.0)]).first().map(|o| o.1), Some(1.0));
    }

    #[test]
    fn rolls_start_and_stop_after_their_delays() {
        // A sphere resting on a wood-roll floor.
        let mut m = StaticMesh::default();
        let mat = m.add_material(SurfaceMaterial {
            friction: 0.7,
            sound: SoundIds {
                hit: 2,
                roll: 2,
                flap: false,
            },
            ..SurfaceMaterial::default()
        });
        let s = 50.0;
        m.add_triangle(
            Vec3::new(-s, 0.0, -s),
            Vec3::new(-s, 0.0, s),
            Vec3::new(s, 0.0, s),
            mat,
        );
        m.add_triangle(
            Vec3::new(-s, 0.0, -s),
            Vec3::new(s, 0.0, s),
            Vec3::new(s, 0.0, -s),
            mat,
        );
        m.finish();
        let mut world = World::new(WorldConfig::default(), m);
        // Resting on it (radius 1).
        let id = world.add_body(ballistic_physics::BodyDesc {
            position: Vec3::new(0.0, 1.0, 0.0),
            ..ballistic_physics::BodyDesc::default()
        });
        world.report_collisions(id, true);
        let mut p = player();
        let mut started = None;
        for frame in 0..120 {
            world.step();
            let hits = world.take_collisions();
            // Collisions carry what was hit.
            assert!(hits.iter().all(|h| h.other_sound
                == SoundIds {
                    hit: 2,
                    roll: 2,
                    flap: false,
                }));
            let pos = world.body(id).unwrap().position;
            p.frame(&world, Some((id, pos)), 0, &hits, 1.0 / 66.0);
            if started.is_none() && !p.loops().is_empty() {
                started = Some(frame);
            }
        }
        // 0.3 s of simulated time is 20 steps.
        let started = started.expect("roll never started");
        assert!((18..=22).contains(&started), "started at step {started}");
        assert_eq!(
            p.loops().iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
            ["roll_w"]
        );
        // Lifted off: it stops 0.3 s later.
        world.set_pose(
            id,
            Vec3::new(0.0, 30.0, 0.0),
            ballistic_physics::Quat::IDENTITY,
        );
        let mut stopped = None;
        for frame in 0..60 {
            world.step();
            let hits = world.take_collisions();
            let pos = world.body(id).unwrap().position;
            p.frame(&world, Some((id, pos)), 0, &hits, 1.0 / 66.0);
            if stopped.is_none() && p.loops().is_empty() {
                stopped = Some(frame);
            }
        }
        let stopped = stopped.expect("roll never stopped");
        assert!((18..=22).contains(&stopped), "stopped at step {stopped}");
    }
}
