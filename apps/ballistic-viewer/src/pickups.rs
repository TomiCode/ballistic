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
    /// An extra life taken: its level object (the placeholder).
    LifeUp(usize),
    /// An extra point activated (its points).
    Activated(i32),
    /// Small balls reached the ball (their points).
    Hits(i32),
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
    /// The small balls' places around it while waiting (world offsets).
    offsets: Vec<Vec3>,
    /// Time circling (s).
    orbit_s: f32,
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
    /// `point_prefab`: the extra point's prefab scene, for where its small
    /// balls start.
    pub fn new(params: &EnergyParams, level: &Scene, point_prefab: Option<&Scene>) -> Self {
        let local_offsets =
            point_prefab.map_or_else(Vec::new, |s| ball_offsets(&params.extra_point, s));
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
                offsets: local_offsets
                    .iter()
                    .map(|&o| s.transform.transform_vector3(o))
                    .collect(),
                orbit_s: 0.0,
                flying: None,
                balls: Vec::new(),
                tick_acc: 0.0,
                used: false,
                active: false,
            })
            .collect::<Vec<_>>();
        // Their markers stay hidden: their looks are drawn instead.
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
        }
        for p in self.points.iter_mut().filter(|p| p.sector == sector) {
            if p.used {
                continue;
            }
            p.active = on;
            p.flying = None;
            p.orbit_s = 0.0;
        }
    }

    /// One frame with the ball at `ball`.
    pub fn frame(&mut self, ball: Vec3, real_dt: f32) -> Vec<Event> {
        let mut out = Vec::new();
        for l in self.lives.iter_mut().filter(|l| l.active && !l.taken) {
            if l.proximity.frame(ball, l.at) == Some(Edge::Enter) {
                l.taken = true;
                l.proximity.switch_off();
                out.push(Event::LifeUp(l.marker));
            }
        }
        let xp = &self.params.extra_point;
        let tick = 1.0 / TICK_HZ;
        for p in self.points.iter_mut().filter(|p| p.active) {
            let Some(t) = p.flying.as_mut() else {
                p.orbit_s += real_dt;
                // TT Extra, idle: activated when the ball comes this near.
                if !p.used && ball.distance_squared(p.at) <= xp.activation_distance.powi(2) {
                    p.used = true;
                    p.flying = Some(0.0);
                    let start = orbit(p.at, &p.offsets, p.orbit_s, xp.look.rotation_speed);
                    p.balls = small_balls(&start, xp.small_balls, xp.force_width);
                    out.push(Event::Activated(xp.activation_points));
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
                out.push(Event::Hits(xp.hit_points * hits));
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
    /// Each extra life's marker (level object) and whether it can show:
    /// its sector active and not taken. Their markers stay hidden: their
    /// looks are drawn instead ([`crate::life_look`]).
    pub fn lives(&self) -> Vec<(usize, bool)> {
        self.lives
            .iter()
            .map(|l| (l.marker, l.active && !l.taken))
            .collect()
    }

    /// Each extra point, for its look.
    pub fn points(&self) -> Vec<PointState> {
        let speed = self.params.extra_point.look.rotation_speed;
        self.points
            .iter()
            .map(|p| PointState {
                marker: p.marker,
                active: p.active,
                activated: p.used,
                balls: if p.used {
                    p.balls.iter().map(|b| (b.pos, b.hit)).collect()
                } else {
                    orbit(p.at, &p.offsets, p.orbit_s, speed)
                        .into_iter()
                        .map(|b| (b, false))
                        .collect()
                },
            })
            .collect()
    }

    pub fn take_visibility(&mut self) -> Vec<(usize, bool)> {
        std::mem::take(&mut self.visibility)
    }
}

/// One extra point's state, for drawing it.
#[derive(Debug, Clone)]
pub struct PointState {
    pub marker: usize,
    /// Its sector is active (and it isn't spent).
    pub active: bool,
    /// Taken: its small balls fly.
    pub activated: bool,
    /// Each small ball's position and whether it reached the ball.
    pub balls: Vec<(Vec3, bool)>,
}

/// The small balls' places relative to the extra point's frame: the prefab's
/// ball sprites (`TT Extra`'s `1` ... `6`), relative to its root.
fn ball_offsets(xp: &ballistic_bundle::ExtraPoint, prefab: &Scene) -> Vec<Vec3> {
    let find = |n: &str| prefab.objects.iter().find(|o| o.name == n);
    let root = find(&format!("{}_MF", xp.group))
        .map_or(Mat4::IDENTITY, |o| Mat4::from_cols_array_2d(&o.transform));
    xp.look
        .balls
        .iter()
        .filter_map(|n| find(n))
        .map(|o| {
            (root.inverse() * Mat4::from_cols_array_2d(&o.transform)).transform_point3(Vec3::ZERO)
        })
        .collect()
}

/// While waiting, the small balls circle at `speed` (radians per second),
/// each about the axis across its offset and the vertical (`TT Extra`'s
/// exact axes aren't read; research findings 22).
fn orbit(at: Vec3, offsets: &[Vec3], t: f32, speed: f32) -> Vec<Vec3> {
    offsets
        .iter()
        .map(|&o| {
            let axis = o.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
            at + ballistic_physics::Quat::from_axis_angle(axis, speed * t) * o
        })
        .collect()
}

/// The small balls flying from `start`, each with its share of the force.
fn small_balls(start: &[Vec3], n: u32, width: f32) -> Vec<SmallBall> {
    let n = n.max(1).min(start.len().max(1) as u32);
    start
        .iter()
        .take(n as usize)
        .enumerate()
        .map(|(i, &pos)| {
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
            look: ballistic_bundle::ExtraPointLook::default(),
        };
        let mut p = Pickups {
            params: ballistic_bundle::EnergyParams {
                start_points: 1000,
                start_lives: 3,
                tick_ms: 500.0,
                life_bonus: 200,
                level_bonus: Vec::new(),
                extra_life: ballistic_bundle::ExtraLife {
                    look: ballistic_bundle::ExtraLifeLook::default(),
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
                offsets: (0..6)
                    .map(|i| {
                        Mat4::from_rotation_y(i as f32 / 6.0 * std::f32::consts::TAU)
                            .transform_vector3(Vec3::X * 2.0)
                    })
                    .collect(),
                orbit_s: 0.0,
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
                if let Event::Activated(n) | Event::Hits(n) = e {
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

    #[test]
    fn small_balls_circle_at_their_distance() {
        let offsets = [
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(1.0, 1.0, -1.41),
        ];
        let at = Vec3::new(5.0, 1.0, -3.0);
        let start = orbit(at, &offsets, 0.0, 5.0);
        let later = orbit(at, &offsets, 0.3, 5.0);
        for ((o, a), b) in offsets.iter().zip(&start).zip(&later) {
            assert!((a.distance(at) - o.length()).abs() < 1e-4);
            assert!((b.distance(at) - o.length()).abs() < 1e-4);
            assert!(a.distance(*b) > 0.5, "it moves");
        }
    }
}
