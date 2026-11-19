//! Extra lives and extra points (research findings 16).
//!
//! Both are placed at their placeholders and, like the original's activation
//! type 1, only there while their sector is active. Until their particle
//! effects exist, the level's placeholder markers stand in for them.
//!
//! - **Extra life** (`P_Extra_Life`): the ball's centre within 4.5 (its
//!   proximity test, with the frame delays) is one life more. It comes back
//!   when its sector is entered again.
//! - **Extra point** (`P_Extra_Point`, *TT Extra*): the ball within 3 of it is
//!   100 points. Its six small balls then fly off for 1 s and chase the ball;
//!   each that comes within 4 is 20 more (220 in all). Once taken it doesn't
//!   come back (it clears its row's *Set?*).
//!
//! The small balls' flight is approximated: *TT Extra*'s fly-away and chase
//! aren't traced in detail. Their points are exact.

use ballistic_bundle::{EnergyParams, Scene};
use ballistic_physics::{Mat4, Vec3, props};

use crate::follow::TICK_HZ;
use crate::progress::{Edge, Proximity};

/// What the pickups do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    LifeUp,
    Points(i32),
}

struct Life {
    sector: u32,
    at: Vec3,
    /// Level object standing in for it.
    marker: usize,
    proximity: Proximity,
    taken: bool,
    active: bool,
}

#[derive(Debug, Clone, Copy)]
struct SmallBall {
    pos: Vec3,
    prev: Vec3,
    /// Its share of the chase force (`1 ± force_width`).
    pull: f32,
    hit: bool,
}

struct Point {
    sector: u32,
    at: Vec3,
    marker: usize,
    /// Activated: its small balls are flying (time since, in ms).
    flying: Option<f32>,
    balls: Vec<SmallBall>,
    tick_acc: f32,
    /// Taken for the rest of the level.
    used: bool,
    active: bool,
}

pub struct Pickups {
    params: EnergyParams,
    lives: Vec<Life>,
    points: Vec<Point>,
    /// Level objects to show or hide (taken by the window).
    visibility: Vec<(usize, bool)>,
}

impl Pickups {
    pub fn new(params: &EnergyParams, level: &Scene) -> Self {
        let r = &params.extra_life.reach;
        let lives = props::placeholders(level, &params.extra_life.group)
            .into_iter()
            .map(|s| Life {
                sector: s.sector,
                at: s.transform.transform_point3(Vec3::ZERO),
                marker: s.placeholder as usize,
                proximity: Proximity::new(
                    r.distance,
                    r.exactness[0],
                    r.exactness[1],
                    (r.frame_delay[0], r.frame_delay[1]),
                    r.xz,
                ),
                taken: false,
                active: false,
            })
            .collect::<Vec<_>>();
        let points = props::placeholders(level, &params.extra_point.group)
            .into_iter()
            .map(|s| Point {
                sector: s.sector,
                at: s.transform.transform_point3(Vec3::ZERO),
                marker: s.placeholder as usize,
                flying: None,
                balls: Vec::new(),
                tick_acc: 0.0,
                used: false,
                active: false,
            })
            .collect::<Vec<_>>();
        // Placed at level init, hidden until their sector activates.
        let visibility = lives
            .iter()
            .map(|l| (l.marker, false))
            .chain(points.iter().map(|p| (p.marker, false)))
            .collect();
        Self {
            params: params.clone(),
            lives,
            points,
            visibility,
        }
    }

    pub fn count(&self) -> (usize, usize) {
        (self.lives.len(), self.points.len())
    }

    /// Sector activation: the scripts start afresh (an extra point already
    /// taken stays away); deactivation hides them.
    pub fn set_sector_active(&mut self, sector: u32, on: bool) {
        for l in self.lives.iter_mut().filter(|l| l.sector == sector) {
            l.active = on;
            l.taken = false;
            l.proximity.switch_off();
            if on {
                l.proximity.switch_on();
            }
            self.visibility.push((l.marker, on));
        }
        for p in self.points.iter_mut().filter(|p| p.sector == sector) {
            if p.used {
                continue;
            }
            p.active = on;
            p.flying = None;
            self.visibility.push((p.marker, on));
        }
    }

    /// One frame with the ball at `ball`.
    pub fn frame(&mut self, ball: Vec3, real_dt: f32) -> Vec<Event> {
        let mut out = Vec::new();
        for l in self.lives.iter_mut().filter(|l| l.active && !l.taken) {
            if l.proximity.frame(ball, l.at) == Some(Edge::Enter) {
                l.taken = true;
                l.proximity.switch_off();
                self.visibility.push((l.marker, false));
                out.push(Event::LifeUp);
            }
        }
        let xp = &self.params.extra_point;
        let tick = 1.0 / TICK_HZ;
        for p in self.points.iter_mut().filter(|p| p.active) {
            let Some(t) = p.flying.as_mut() else {
                // TT Extra, idle: activated when the ball comes this near.
                if !p.used && ball.distance_squared(p.at) <= xp.activation_distance.powi(2) {
                    p.used = true;
                    p.flying = Some(0.0);
                    p.balls = small_balls(p.at, xp.small_balls, xp.force_width);
                    self.visibility.push((p.marker, false));
                    out.push(Event::Points(xp.activation_points));
                }
                continue;
            };
            *t += real_dt * 1000.0;
            let away = *t < xp.away_ms;
            p.tick_acc += real_dt;
            let mut hits = 0;
            while p.tick_acc >= tick {
                p.tick_acc -= tick;
                for b in p.balls.iter_mut().filter(|b| !b.hit) {
                    let motion = b.pos - b.prev;
                    b.prev = b.pos;
                    if away {
                        let out = (b.pos - p.at).normalize_or_zero();
                        b.pos += motion * xp.away_damping + out * xp.away_force * tick * 10.0;
                    } else {
                        b.pos += motion * xp.damping + (ball - b.pos) * xp.force * b.pull;
                        if b.pos.distance(ball) < xp.hit_distance {
                            b.hit = true;
                            hits += 1;
                        }
                    }
                }
            }
            if hits > 0 {
                out.push(Event::Points(xp.hit_points * hits));
            }
            if p.balls.iter().all(|b| b.hit) {
                p.flying = None;
                p.active = false;
            }
        }
        out
    }

    /// Development aid: the nearest pickup still there (in an active sector).
    pub fn nearest(&self, ball: Vec3) -> Option<Vec3> {
        self.lives
            .iter()
            .filter(|l| l.active && !l.taken)
            .map(|l| l.at)
            .chain(
                self.points
                    .iter()
                    .filter(|p| p.active && !p.used)
                    .map(|p| p.at),
            )
            .min_by(|a, b| a.distance(ball).total_cmp(&b.distance(ball)))
    }

    /// Level objects to show (`true`) or hide since the last call.
    pub fn take_visibility(&mut self) -> Vec<(usize, bool)> {
        std::mem::take(&mut self.visibility)
    }
}

/// The small balls around an extra point, each with its share of the force.
fn small_balls(at: Vec3, n: u32, width: f32) -> Vec<SmallBall> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let pos = at + Mat4::from_rotation_y(a).transform_vector3(Vec3::X * 2.0);
            let spread = if n > 1 {
                i as f32 / (n - 1) as f32 * 2.0 - 1.0
            } else {
                0.0
            };
            SmallBall {
                pos,
                prev: pos,
                pull: 1.0 + width * spread,
                hit: false,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_balls_all_reach_a_resting_ball() {
        let xp = ballistic_bundle::ExtraPoint {
            group: String::new(),
            small_balls: 6,
            activation_distance: 3.0,
            hit_distance: 4.0,
            away_force: 1.0,
            away_damping: 0.3,
            away_ms: 1000.0,
            force: 0.12,
            damping: 0.95,
            force_width: 0.08,
            activation_points: 100,
            hit_points: 20,
            hit_frames: String::new(),
        };
        let mut p = Pickups {
            params: ballistic_bundle::EnergyParams {
                start_points: 1000,
                start_lives: 3,
                tick_ms: 500.0,
                life_bonus: 200,
                level_bonus: Vec::new(),
                extra_life: ballistic_bundle::ExtraLife {
                    group: String::new(),
                    reach: ballistic_bundle::WakeParams {
                        object: String::new(),
                        distance: 4.5,
                        exactness: [6.0, 20.0],
                        frame_delay: [1, 10],
                        xz: false,
                        bodies: Vec::new(),
                    },
                },
                extra_point: xp,
            },
            lives: Vec::new(),
            points: vec![Point {
                sector: 1,
                at: Vec3::ZERO,
                marker: 0,
                flying: None,
                balls: Vec::new(),
                tick_acc: 0.0,
                used: false,
                active: true,
            }],
            visibility: Vec::new(),
        };
        let mut total = 0;
        // The ball rolls through it and stops 10 further on.
        for frame in 0..900 {
            let ball = Vec3::new((frame as f32 * 0.2).min(10.0) - 2.0, 0.0, 0.0);
            for e in p.frame(ball, 1.0 / 60.0) {
                if let Event::Points(n) = e {
                    total += n;
                }
            }
        }
        assert_eq!(total, 220);
        // Taken for good: re-entering the sector doesn't bring it back.
        p.set_sector_active(1, false);
        p.set_sector_active(1, true);
        assert_eq!(p.frame(Vec3::ZERO, 1.0 / 60.0), Vec::new());
    }
}
