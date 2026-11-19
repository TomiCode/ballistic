//! Points, lives and score, like the original's `Gameplay_Energy` and score
//! screen (research findings 16).
//!
//! - A level starts with 1000 points and 3 lives.
//! - While the counter runs, one point goes every 500 ms of real time (at
//!   most one per frame), down to 0. It stops when the ball is lost (until the
//!   new ball) and at the finish.
//! - Losing the ball with lives left costs one; with none left the game is
//!   over.
//! - Extra lives and extra points add to them ([`crate::pickups`]).
//! - The score at the end: the level's bonus (100 × its number) + the points
//!   left + the lives left × 200.

use ballistic_bundle::EnergyParams;

#[derive(Debug, Clone)]
pub struct Energy {
    params: EnergyParams,
    /// 0-based level number (for its bonus).
    level: usize,
    pub points: i32,
    pub lives: i32,
    counting: bool,
    /// Real time since the last point went (the original's `real Time −
    /// verifyTime`).
    since_tick_ms: f32,
    pub game_over: bool,
}

/// The score screen's parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Score {
    pub level_bonus: i32,
    pub points: i32,
    pub life_bonus: i32,
    pub total: i32,
}

impl Energy {
    pub fn new(params: &EnergyParams, level: usize) -> Self {
        Self {
            points: params.start_points,
            lives: params.start_lives,
            params: params.clone(),
            level,
            counting: true,
            since_tick_ms: 0.0,
            game_over: false,
        }
    }

    /// *Counter active* / *Counter inactive*.
    pub fn set_counting(&mut self, on: bool) {
        self.counting = on;
    }

    pub fn frame(&mut self, real_dt: f32) {
        if !self.counting {
            return;
        }
        self.since_tick_ms += real_dt * 1000.0;
        if self.since_tick_ms >= self.params.tick_ms {
            self.since_tick_ms -= self.params.tick_ms;
            if self.points != 0 {
                self.points -= 1;
            }
        }
    }

    pub fn add_points(&mut self, n: i32) {
        self.points += n;
    }

    pub fn add_life(&mut self) {
        self.lives += 1;
    }

    /// The ball is lost: `true` if a new ball comes (a life is used), `false`
    /// if the game is over.
    pub fn ball_lost(&mut self) -> bool {
        if self.lives > 0 {
            self.lives -= 1;
            true
        } else {
            self.game_over = true;
            self.counting = false;
            false
        }
    }

    pub fn score(&self) -> Score {
        let level_bonus = self
            .params
            .level_bonus
            .get(self.level)
            .copied()
            .unwrap_or(0);
        let life_bonus = self.lives * self.params.life_bonus;
        Score {
            level_bonus,
            points: self.points,
            life_bonus,
            total: level_bonus + self.points + life_bonus,
        }
    }
}

/// The 0-based level number of a level scene key (`level/level_03` → 2).
pub fn level_index(key: &str) -> Option<usize> {
    let digits: String = key
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse::<usize>().ok()?.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballistic_bundle::{ExtraLife, ExtraPoint, WakeParams};

    fn params() -> EnergyParams {
        EnergyParams {
            start_points: 1000,
            start_lives: 3,
            tick_ms: 500.0,
            life_bonus: 200,
            level_bonus: (1..=12).map(|l| l * 100).collect(),
            extra_life: ExtraLife {
                group: "P_Extra_Life".into(),
                reach: WakeParams {
                    object: String::new(),
                    distance: 4.5,
                    exactness: [6.0, 20.0],
                    frame_delay: [1, 10],
                    xz: false,
                    bodies: Vec::new(),
                },
            },
            extra_point: ExtraPoint {
                group: "P_Extra_Point".into(),
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
            },
        }
    }

    #[test]
    fn counts_down_and_scores() {
        let mut e = Energy::new(&params(), 2);
        // Just over 10 s (frame times add up in floats): 20 points.
        for _ in 0..601 {
            e.frame(1.0 / 60.0);
        }
        assert_eq!(e.points, 980);
        e.set_counting(false);
        e.frame(5.0);
        assert_eq!(e.points, 980);
        e.add_points(220);
        e.add_life();
        assert_eq!(
            e.score(),
            Score {
                level_bonus: 300,
                points: 1200,
                life_bonus: 800,
                total: 2300
            }
        );
    }

    #[test]
    fn the_fourth_loss_with_three_lives_is_game_over() {
        let mut e = Energy::new(&params(), 0);
        assert!(e.ball_lost() && e.ball_lost() && e.ball_lost());
        assert_eq!(e.lives, 0);
        assert!(!e.ball_lost());
        assert!(e.game_over);
    }

    #[test]
    fn reads_level_numbers() {
        assert_eq!(level_index("level/level_03"), Some(2));
        assert_eq!(level_index("level/level_12"), Some(11));
        assert_eq!(level_index("balls"), None);
    }
}
