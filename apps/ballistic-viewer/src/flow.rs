//! The level flow, like the original's `BallManager` (research findings 17).
//!
//! - **New Ball** (at the level start and after a loss): the ball and the
//!   camera rig go to the reset point, the lightning plays, and 3 s later the
//!   ball appears. It's woken a frame later, and then the counter and ball
//!   control come on.
//! - **Losing the ball**: when the ball's box meets one of the level's
//!   `DepthTestCubes` (tested one cube per frame, round robin), control, the
//!   camera and the counter stop. Two frames later the lives are checked: with
//!   none left the game is over. Otherwise the ball falls on for 1 s, is taken
//!   away, a life is used, the sector is reset, and a New Ball follows.
//! - Props of the depth-test groups that fall 200 below the lowest cube are
//!   taken away.
//! - The sound scripts' event sounds, with their frame and millisecond
//!   delays.

use ballistic_bundle::{EventSound, LevelFlow, Scene};
use ballistic_physics::{Mat4, Quat, Vec3, boxes_overlap};

/// Where the ball is in the flow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BallPhase {
    /// New Ball: waiting this long before the ball appears.
    Coming {
        ms: f32,
    },
    /// Shown; woken (and control on) next frame.
    Waking,
    Alive,
    /// Deactivate Ball: frames to the lives check, then the time left
    /// before the ball is taken away.
    Lost {
        frames: u32,
        ms: Option<f32>,
    },
    /// Taken away; a New Ball starts next frame.
    Gone,
    GameOver,
}

/// Frames from the loss to the lives check (*Deactivate Ball*'s link delay).
pub const LOST_CHECK_FRAMES: u32 = 2;

/// The level's depth-test cubes.
pub struct DepthCubes {
    boxes: Vec<(Vec3, Quat, Vec3)>,
    /// Round robin: one cube per frame, then a frame to restart the group.
    slot: usize,
    /// The lowest cube's bottom.
    pub bottom: Option<f32>,
}

impl DepthCubes {
    pub fn new(level: &Scene, group: &str) -> Self {
        let mut boxes = Vec::new();
        let mut bottom: Option<f32> = None;
        let members = level
            .groups
            .iter()
            .find(|g| g.name == group)
            .map(|g| g.members.clone())
            .unwrap_or_default();
        for m in members {
            let Some(o) = level.objects.get(m as usize) else {
                continue;
            };
            let Some(mesh) = o.mesh.and_then(|i| level.meshes.get(i as usize)) else {
                continue;
            };
            let t = Mat4::from_cols_array_2d(&o.transform);
            let (lo, hi) = (Vec3::from(mesh.bounds_min), Vec3::from(mesh.bounds_max));
            let (scale, rot, _) = t.to_scale_rotation_translation();
            let center = t.transform_point3((lo + hi) * 0.5);
            let half = (hi - lo) * 0.5 * scale.abs();
            // The world box's bottom.
            let corners = (0..8).map(|i| {
                let c = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                );
                t.transform_point3(c).y
            });
            let low = corners.fold(f32::MAX, f32::min);
            bottom = Some(bottom.map_or(low, |b| b.min(low)));
            boxes.push((center, rot, half));
        }
        Self {
            boxes,
            slot: 0,
            bottom,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// One frame: whether this frame's cube meets the ball's box.
    pub fn check(&mut self, ball: (Vec3, Quat, Vec3)) -> bool {
        if self.boxes.is_empty() {
            return false;
        }
        let slot = self.slot;
        self.slot = (self.slot + 1) % (self.boxes.len() + 1);
        self.boxes
            .get(slot)
            .is_some_and(|&cube| boxes_overlap(cube, ball))
    }
}

/// Event sounds waiting for their delays.
pub struct EventSounds {
    table: Vec<EventSound>,
    pending: Vec<(String, u32, f32)>,
}

impl EventSounds {
    pub fn new(flow: &LevelFlow) -> Self {
        Self {
            table: flow.event_sounds.clone(),
            pending: Vec::new(),
        }
    }

    /// A message to the sound scripts (by name).
    pub fn emit(&mut self, message: &str) {
        if let Some(e) = self.table.iter().find(|e| e.message == message) {
            for c in &e.cues {
                self.pending
                    .push((c.sound.clone(), c.delay_frames, c.delay_ms));
            }
        }
    }

    /// One frame: the sounds due now (key, volume).
    pub fn frame(&mut self, real_dt: f32) -> Vec<(String, f32)> {
        let ms = real_dt * 1000.0;
        let mut due = Vec::new();
        self.pending.retain_mut(|(key, frames, wait)| {
            if *frames > 0 {
                *frames -= 1;
                return true;
            }
            if *wait > 0.0 {
                *wait -= ms;
                return true;
            }
            due.push((key.clone(), 1.0));
            false
        });
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballistic_bundle::SoundCue;

    #[test]
    fn event_sounds_wait_their_delays() {
        let flow = LevelFlow {
            new_ball_ms: 3000.0,
            lost_ms: 1000.0,
            depth_cubes: String::new(),
            prop_depth_margin: 200.0,
            prop_depth_groups: Vec::new(),
            event_sounds: vec![EventSound {
                message: "Life_Up".into(),
                cues: vec![
                    SoundCue {
                        sound: "blob".into(),
                        delay_frames: 1,
                        delay_ms: 0.0,
                    },
                    SoundCue {
                        sound: "extraball".into(),
                        delay_frames: 0,
                        delay_ms: 317.0,
                    },
                ],
            }],
        };
        let mut e = EventSounds::new(&flow);
        e.emit("Life_Up");
        e.emit("nothing");
        let mut log = Vec::new();
        for frame in 0..40 {
            for (k, _) in e.frame(1.0 / 60.0) {
                log.push((frame, k));
            }
        }
        assert_eq!(log[0], (1, "blob".to_owned()));
        // 317 ms of 60 Hz frames.
        assert_eq!(log[1].1, "extraball");
        assert!((19..=20).contains(&log[1].0), "{log:?}");
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn cubes_are_tested_in_turn() {
        let mut d = DepthCubes {
            boxes: vec![
                (
                    Vec3::new(0.0, -100.0, 0.0),
                    Quat::IDENTITY,
                    Vec3::splat(10.0),
                ),
                (
                    Vec3::new(50.0, -100.0, 0.0),
                    Quat::IDENTITY,
                    Vec3::splat(10.0),
                ),
            ],
            slot: 0,
            bottom: Some(-110.0),
        };
        let ball = (
            Vec3::new(50.0, -95.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(2.0),
        );
        // Frame 0 tests the first cube (no), frame 1 the second (yes), frame 2
        // restarts the group, frame 3 the first again.
        assert_eq!(
            (0..5).map(|_| d.check(ball)).collect::<Vec<_>>(),
            [false, true, false, false, true]
        );
    }
}
