//! The in-game camera's follow behaviour (research findings 11).
//!
//! The original moves two rig frames every frame with *TT Set Dynamic
//! Position*: each one is pulled towards its target by `force · Δt` of the
//! remaining distance (per axis, Δt in seconds), and keeps `damping` times its
//! own motion of the previous frame. "Damping" is really carried-over momentum.
//!
//! - `Cam_Target` chases the ball: force 10, no momentum.
//! - The camera chases `Cam_Pos` (22 behind and 35 above `Cam_Target`, along the
//!   current view heading): force 5 / 0.8 / 5, momentum 0.5 / 0.3 / 0.5.
//!   The weak vertical pull is the original's lazy follow when the ball drops.
//!
//! The camera always looks at `Cam_Target`.
//!
//! The lift view (`CamUp`, Space by default) changes only the camera's chase,
//! while the key is held: its world-space offset becomes (0, −50, 0), so it
//! settles 50 units higher, and its vertical force becomes 2. Releasing the
//! key restores offset 0 and force 0.8. Nothing physical changes.
//!
//! The filter is per frame in the original, so its feel depends on the frame
//! rate. Here it runs at a fixed [`TICK_HZ`] and is interpolated for display.

use ballistic_physics::{Quat, Vec3};

/// Fixed update rate of the follow filter. Assumed: the original's frame rate
/// at the time isn't known; 60 Hz is the common case.
pub const TICK_HZ: f32 = 60.0;
const TICK: f32 = 1.0 / TICK_HZ;

/// `Cam_Pos` relative to `Cam_Target`, in the heading frame (from `Camera.nmo`).
const CAM_BACK: f32 = 22.0;
const CAM_UP: f32 = 35.0;

const TARGET_FORCE: Vec3 = Vec3::splat(10.0);
const TARGET_DAMPING: Vec3 = Vec3::ZERO;
const EYE_FORCE: Vec3 = Vec3::new(5.0, 0.8, 5.0);
const EYE_DAMPING: Vec3 = Vec3::new(0.5, 0.3, 0.5);
/// Lift view: the camera chase's *Offset* and vertical *Force* while `CamUp` is held.
const LIFT_OFFSET: Vec3 = Vec3::new(0.0, -50.0, 0.0);
const LIFT_FORCE_Y: f32 = 2.0;

/// One *TT Set Dynamic Position* in world space with zero offset.
#[derive(Debug, Clone, Copy)]
pub struct Chase {
    pub pos: Vec3,
    prev: Vec3,
    force: Vec3,
    damping: Vec3,
}

impl Chase {
    pub fn new(pos: Vec3, force: Vec3, damping: Vec3) -> Self {
        Self {
            pos,
            prev: pos,
            force,
            damping,
        }
    }

    /// One frame of `dt` seconds towards `target − offset`.
    pub fn tick(&mut self, target: Vec3, offset: Vec3, dt: f32) {
        let motion = self.pos - self.prev;
        self.prev = self.pos;
        self.pos += self.damping * motion + self.force * dt * (target - self.pos - offset);
    }
}

/// `Cam_Pos` for a target position and view heading (radians about +Y).
fn eye_goal(target: Vec3, heading: f32) -> Vec3 {
    let forward = Quat::from_rotation_y(heading) * Vec3::NEG_Z;
    target - forward * CAM_BACK + Vec3::Y * CAM_UP
}

/// The camera rig: `Cam_Target` and `InGameCam`.
#[derive(Debug, Clone)]
pub struct Follow {
    target: Chase,
    eye: Chase,
    /// Poses before the last tick (for interpolation).
    prev_target: Vec3,
    prev_eye: Vec3,
    accumulator: f32,
}

impl Follow {
    pub fn new(ball: Vec3, heading: f32) -> Self {
        let eye = eye_goal(ball, heading);
        Self {
            target: Chase::new(ball, TARGET_FORCE, TARGET_DAMPING),
            eye: Chase::new(eye, EYE_FORCE, EYE_DAMPING),
            prev_target: ball,
            prev_eye: eye,
            accumulator: 0.0,
        }
    }

    /// Snap to the ball (spawn and respawn: the rig is reset with the ball).
    pub fn reset(&mut self, ball: Vec3, heading: f32) {
        *self = Self::new(ball, heading);
    }

    /// Advances the rig; `lifted` is the lift view (`CamUp` held).
    pub fn update(&mut self, real_dt: f32, ball: Vec3, heading: f32, lifted: bool) {
        let (offset, force_y) = if lifted {
            (LIFT_OFFSET, LIFT_FORCE_Y)
        } else {
            (Vec3::ZERO, EYE_FORCE.y)
        };
        self.eye.force.y = force_y;
        self.accumulator = (self.accumulator + real_dt.max(0.0)).min(0.25);
        while self.accumulator >= TICK {
            self.prev_target = self.target.pos;
            self.prev_eye = self.eye.pos;
            self.target.tick(ball, Vec3::ZERO, TICK);
            self.eye
                .tick(eye_goal(self.target.pos, heading), offset, TICK);
            self.accumulator -= TICK;
        }
    }

    /// Interpolated (eye, look-at target).
    pub fn pose(&self) -> (Vec3, Vec3) {
        let t = self.accumulator / TICK;
        (
            self.prev_eye.lerp(self.eye.pos, t),
            self.prev_target.lerp(self.target.pos, t),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chase_without_momentum_is_exponential() {
        let mut c = Chase::new(Vec3::ZERO, Vec3::splat(10.0), Vec3::ZERO);
        c.tick(Vec3::X, Vec3::ZERO, 0.01);
        // 10 · 0.01 = 10 % of the distance per frame.
        assert!((c.pos.x - 0.1).abs() < 1e-6);
        c.tick(Vec3::X, Vec3::ZERO, 0.01);
        assert!((c.pos.x - 0.19).abs() < 1e-6);
    }

    #[test]
    fn momentum_carries_last_motion() {
        let mut c = Chase::new(Vec3::ZERO, Vec3::splat(5.0), Vec3::splat(0.5));
        c.tick(Vec3::X, Vec3::ZERO, 0.02); // pull 0.1
        c.tick(Vec3::X, Vec3::ZERO, 0.02); // 0.5 · 0.1 + 0.1 · 0.9
        assert!((c.pos.x - (0.1 + 0.05 + 0.09)).abs() < 1e-6);
    }

    #[test]
    fn rig_settles_behind_and_above_the_ball() {
        let ball = Vec3::new(10.0, 0.0, 5.0);
        let mut f = Follow::new(Vec3::ZERO, 0.0);
        for _ in 0..600 {
            f.update(1.0 / 60.0, ball, 0.0, false);
        }
        let (eye, target) = f.pose();
        assert!(target.abs_diff_eq(ball, 1e-3), "{target}");
        assert!(
            eye.abs_diff_eq(ball + Vec3::new(0.0, 35.0, 22.0), 1e-2),
            "{eye}"
        );
    }

    #[test]
    fn same_result_at_any_frame_rate() {
        let run = |rate: u16| {
            let fps = f32::from(rate);
            let mut f = Follow::new(Vec3::ZERO, 0.0);
            for i in 0..2 * rate {
                let t = f32::from(i) / fps;
                f.update(1.0 / fps, Vec3::new(t * 20.0, -t * 5.0, 0.0), 0.0, false);
            }
            f.pose().0
        };
        // Only the ball samples differ between frame rates, so states stay close.
        assert!(run(60).abs_diff_eq(run(120), 0.5));
    }

    #[test]
    fn lift_raises_the_camera_and_release_lowers_it() {
        let ball = Vec3::new(3.0, 1.0, -4.0);
        let mut f = Follow::new(ball, 0.0);
        for _ in 0..600 {
            f.update(1.0 / 60.0, ball, 0.0, true);
        }
        let (eye, _) = f.pose();
        assert!(
            eye.abs_diff_eq(ball + Vec3::new(0.0, 85.0, 22.0), 1e-2),
            "{eye}"
        );
        for _ in 0..1200 {
            f.update(1.0 / 60.0, ball, 0.0, false);
        }
        let (eye, _) = f.pose();
        assert!(
            eye.abs_diff_eq(ball + Vec3::new(0.0, 35.0, 22.0), 1e-2),
            "{eye}"
        );
    }
}
