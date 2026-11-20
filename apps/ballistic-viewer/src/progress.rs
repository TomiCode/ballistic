//! Level progress as the original's scripts run it (research findings 12):
//! checkpoints, reset points and the sector manager.
//!
//! - Checkpoint `PC_TwoFlames_NN` and reset point `PR_Resetpoint_NN` are row
//!   NN − 1 of their tables. The ball starts at reset point 0, and checkpoint 0
//!   is the first one armed.
//! - A checkpoint is reached when the ball comes within 6.5 of its big flame;
//!   that test only runs while the ball is within 70 (on X/Z) of the
//!   checkpoint. Both are *TT Scaleable Proximity* tests, checked every few
//!   frames depending on distance.
//! - Reaching checkpoint N − 1 (`ActCheckpoint` = N): the reset point becomes
//!   row N, sector N is deactivated, `ActCheckpoint` becomes N + 1, sector N + 1
//!   is activated 4 frames later, and 2 frames after the switch the next
//!   checkpoint is armed.
//! - Level start activates sector 1. A respawn deactivates the current sector
//!   and activates it again 4 frames later, so its props go back to their
//!   placeholders.
//! - The level end (`PE_Balloon`) belongs to the last sector. Its script starts
//!   when that sector activates: within 70 (X/Z) of the platform it switches
//!   on a second test, and the level is finished when the ball is within 1 of
//!   the platform's origin (`Level_Finish`).
//!
//! "Frames" are calls to [`Progress::frame`], once per rendered frame like the
//! original's behaviour graphs.

use ballistic_bundle::Scene;
use ballistic_physics::{Mat4, Vec3};

use crate::modules::LevelEnd;

/// Frames between deactivating one sector and activating the next
/// (`Gameplay_SectorManager`: a 4-frame link delay).
const SECTOR_DELAY: u32 = 4;
/// Frames from the sector switch to arming the next checkpoint.
const NEXT_CHECKPOINT_DELAY: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Enter,
    Exit,
}

/// One check of a proximity test: in range, and Enter/Exit if it changed.
/// In range without an edge is the block's `InRange` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub inside: bool,
    pub edge: Option<Edge>,
}

/// *TT Scaleable Proximity* (`TT_Toolbox_RT`): a distance test between two
/// entities, evaluated every `delay` frames. The delay is `min_delay` within
/// `near`, `max_delay` beyond `far`, interpolated between.
#[derive(Debug, Clone)]
pub struct Proximity {
    distance: f32,
    near: f32,
    far: f32,
    min_delay: i32,
    max_delay: i32,
    /// Only X and Z count (else X, Y and Z).
    xz: bool,
    on: bool,
    countdown: i32,
    /// 0 out, 1 in, 2 unknown (just switched on).
    state: u8,
}

impl Proximity {
    pub fn new(distance: f32, near: f32, far: f32, delays: (i32, i32), xz: bool) -> Self {
        Self {
            distance,
            near,
            far,
            min_delay: delays.0,
            max_delay: delays.1,
            xz,
            on: false,
            // The block's "Last Check" starts at 1: the first frame checks.
            countdown: 1,
            state: 2,
        }
    }

    /// "In": the next check reports Enter or Exit, whichever applies.
    pub fn switch_on(&mut self) {
        self.on = true;
        self.state = 2;
    }

    pub fn switch_off(&mut self) {
        self.on = false;
    }

    /// One frame: Enter or Exit, if the state changed at a check.
    pub fn frame(&mut self, a: Vec3, b: Vec3) -> Option<Edge> {
        self.check(a, b).and_then(|r| r.edge)
    }

    /// One frame; `Some` when the block checked this frame. With "Squared
    /// Distance?" set, as in Ballance, the block compares squared distances
    /// with squared thresholds: the same test.
    pub fn check(&mut self, a: Vec3, b: Vec3) -> Option<Reading> {
        if !self.on {
            return None;
        }
        self.countdown -= 1;
        if self.countdown >= 1 {
            return None;
        }
        let mut d = a - b;
        if self.xz {
            d.y = 0.0;
        }
        let d2 = d.length_squared();
        let (near2, far2) = (self.near * self.near, self.far * self.far);
        let delay = if d2 >= far2 {
            self.max_delay
        } else if d2 <= near2 {
            self.min_delay
        } else {
            let t = (d2 - near2) / (far2 - near2);
            #[expect(clippy::cast_possible_truncation, reason = "a frame count")]
            let extra = ((self.max_delay - self.min_delay) as f32 * t) as i32;
            self.min_delay + extra
        };
        self.countdown = delay.max(1);
        let inside = d2 < self.distance * self.distance;
        let edge = match (inside, self.state) {
            (true, 0 | 2) => Some(Edge::Enter),
            (false, 1 | 2) => Some(Edge::Exit),
            _ => None,
        };
        self.state = u8::from(inside);
        Some(Reading { inside, edge })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Checkpoint {
    /// The checkpoint's frame (`PC_TwoFlames_MF` at the placeholder).
    pub frame: Vec3,
    /// Its big flame, the target of the "reached" test.
    pub flame: Vec3,
}

/// Checkpoints and reset points of a level, in table order.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub checkpoints: Vec<Checkpoint>,
    pub reset_points: Vec<Mat4>,
}

/// The last sector: `Sector_01`, `Sector_02`, … counted up to the first one
/// missing (at most 9), as level init's *Set Sector Attribute* does.
pub fn last_sector(level: &Scene) -> u32 {
    (1..=9)
        .take_while(|n| {
            level
                .groups
                .iter()
                .any(|g| g.name == format!("Sector_{n:02}"))
        })
        .last()
        .unwrap_or(1)
}

/// The number after the last `_` of a name (`PR_Resetpoint_03` → 3).
fn number(name: &str) -> Option<usize> {
    name.rsplit_once('_')?.1.parse().ok()
}

/// Members of a group as (row, world matrix, name), ordered by row. Row =
/// name number − 1; members without a number are skipped.
pub(crate) fn numbered(level: &Scene, group: &str) -> Vec<(usize, Mat4, String)> {
    let Some(g) = level.groups.iter().find(|g| g.name == group) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = g
        .members
        .iter()
        .filter_map(|&i| {
            let o = level.objects.get(i as usize)?;
            let row = number(&o.name)?.checked_sub(1)?;
            Some((row, Mat4::from_cols_array_2d(&o.transform), o.name.clone()))
        })
        .collect();
    rows.sort_by_key(|r| r.0);
    rows
}

/// Reads the level's checkpoints (with the flame offset from their prefab,
/// `ph/<name without _NN>`) and reset points.
pub fn layout(level: &Scene, mut prefab: impl FnMut(&str) -> Option<Scene>) -> Layout {
    let reset_points = numbered(level, "PR_Resetpoints")
        .into_iter()
        .map(|(_, m, _)| m)
        .collect();
    let rows = numbered(level, "PC_Checkpoints");
    // Level init loads the prefab named like the first checkpoint minus "_NN".
    let flame_local = rows
        .first()
        .and_then(|(_, _, name)| {
            let base = name.rsplit_once('_')?.0;
            let scene = prefab(&format!("ph/{}", base.to_lowercase()))?;
            let find = |n: String| scene.objects.iter().find(|o| o.name == n);
            let root = Mat4::from_cols_array_2d(&find(format!("{base}_MF"))?.transform);
            let flame = Mat4::from_cols_array_2d(&find(format!("{base}_Flame_Big"))?.transform);
            Some((root.inverse() * flame).transform_point3(Vec3::ZERO))
        })
        .unwrap_or_else(|| {
            eprintln!("warning: checkpoint prefab not found; using the checkpoint frame");
            Vec3::ZERO
        });
    let checkpoints = rows
        .into_iter()
        .map(|(_, m, _)| Checkpoint {
            frame: m.transform_point3(Vec3::ZERO),
            flame: m.transform_point3(flame_local),
        })
        .collect();
    Layout {
        checkpoints,
        reset_points,
    }
}

/// The armed checkpoint's script (`PC_TwoFlames_MF Script`).
#[derive(Debug, Clone)]
struct Armed {
    index: usize,
    /// Ball near the checkpoint (70 on X/Z): arms `reach`.
    near: Proximity,
    /// Ball at the big flame (6.5): reached.
    reach: Proximity,
}

impl Armed {
    fn new(index: usize) -> Self {
        let mut near = Proximity::new(70.0, 75.0, 150.0, (10, 100), true);
        near.switch_on();
        Self {
            index,
            near,
            reach: Proximity::new(6.5, 20.0, 30.0, (1, 15), false),
        }
    }

    /// One frame; true when the ball reaches the flame.
    fn frame(&mut self, ball: Vec3, cp: &Checkpoint) -> bool {
        match self.near.frame(ball, cp.frame) {
            Some(Edge::Enter) => self.reach.switch_on(),
            Some(Edge::Exit) => self.reach.switch_off(),
            None => {}
        }
        if self.reach.frame(ball, cp.flame) == Some(Edge::Enter) {
            self.reach.switch_off();
            self.near.switch_off();
            return true;
        }
        false
    }
}

/// The level end's script (`PE_Balloon Script`), once its sector is active.
#[derive(Debug, Clone)]
struct EndScript {
    /// Ball near the platform (70 on X/Z): wakes it and arms `reach`.
    near: Proximity,
    /// Ball at the platform's origin (1): `Level_Finish`.
    reach: Proximity,
}

impl EndScript {
    fn new() -> Self {
        let mut near = Proximity::new(70.0, 75.0, 100.0, (10, 60), true);
        near.switch_on();
        Self {
            near,
            reach: Proximity::new(1.0, 5.0, 10.0, (1, 10), false),
        }
    }

    fn frame(&mut self, ball: Vec3, platform: Vec3) -> bool {
        if self.near.frame(ball, platform) == Some(Edge::Enter) {
            self.near.switch_off();
            self.reach.switch_on();
        }
        if self.reach.frame(ball, platform) == Some(Edge::Enter) {
            self.reach.switch_off();
            return true;
        }
        false
    }
}

/// Something that happened in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Checkpoint (index) reached.
    Checkpoint(usize),
    /// The ball reached the level end's platform.
    Finished,
}

#[derive(Debug, Clone)]
pub struct Progress {
    layout: Layout,
    /// The original's `ActCheckpoint`: 1 + checkpoints reached; also the
    /// active sector.
    act: u32,
    /// Current reset point (row of `layout.reset_points`).
    reset: usize,
    /// Sector to activate, and frames to wait.
    pending_sector: Option<(u32, u32)>,
    /// Frames until the next checkpoint is armed.
    pending_checkpoint: Option<u32>,
    armed: Option<Armed>,
    /// Sector switches not yet applied: (sector, active).
    changes: Vec<(u32, bool)>,
    level_end: Option<LevelEnd>,
    end_script: Option<EndScript>,
    finished: bool,
}

impl Progress {
    /// Level start: reset point 0, checkpoint 0 armed, sector 1 switched off
    /// and on (after the usual 4 frames), as `Init Ingame` writes 1 to both.
    pub fn start(layout: Layout, level_end: Option<LevelEnd>) -> Self {
        let armed = (!layout.checkpoints.is_empty()).then(|| Armed::new(0));
        let mut p = Self {
            layout,
            act: 1,
            reset: 0,
            pending_sector: None,
            pending_checkpoint: None,
            armed,
            changes: Vec::new(),
            level_end,
            end_script: None,
            finished: false,
        };
        p.switch(1, 1);
        p
    }

    /// The sector manager: deactivate now, activate a few frames later.
    fn switch(&mut self, deactivate: u32, activate: u32) {
        if deactivate != 0 {
            self.changes.push((deactivate, false));
            if self.level_end.is_some_and(|e| e.sector == deactivate) {
                self.end_script = None;
            }
        }
        self.pending_sector = Some((activate, SECTOR_DELAY));
    }

    /// Ball lost: the current sector is switched off and on again.
    pub fn respawn(&mut self) {
        self.switch(self.act, self.act);
    }

    /// Sector switches since the last call, in order: (sector, active).
    pub fn take_changes(&mut self) -> Vec<(u32, bool)> {
        std::mem::take(&mut self.changes)
    }

    /// Where the ball respawns.
    pub fn reset_point(&self) -> Option<Mat4> {
        self.layout.reset_points.get(self.reset).copied()
    }

    /// Where the level end's platform is now (it hangs from the balloon).
    pub fn set_level_end_target(&mut self, platform: Vec3) {
        if let Some(e) = self.level_end.as_mut() {
            e.platform = platform;
        }
    }

    /// Where to go next: the next checkpoint's flame, then the level end's
    /// platform; `None` once finished.
    pub fn next_target(&self) -> Option<Vec3> {
        if self.finished {
            return None;
        }
        match self.layout.checkpoints.get(self.act as usize - 1) {
            Some(cp) => Some(cp.flame),
            None => self.level_end.map(|e| e.platform),
        }
    }

    pub fn reached(&self) -> usize {
        self.act as usize - 1
    }

    pub fn checkpoint_count(&self) -> usize {
        self.layout.checkpoints.len()
    }

    /// One frame.
    pub fn frame(&mut self, ball: Vec3) -> Option<Event> {
        if let Some((sector, n)) = self.pending_sector.as_mut() {
            if *n == 0 {
                let s = *sector;
                self.pending_sector = None;
                if s != 0 {
                    self.changes.push((s, true));
                    if self.level_end.is_some_and(|e| e.sector == s) && !self.finished {
                        self.end_script = Some(EndScript::new());
                    }
                }
            } else {
                *n -= 1;
            }
        }
        if let Some(n) = self.pending_checkpoint.as_mut() {
            if *n == 0 {
                self.pending_checkpoint = None;
                let next = self.act as usize - 1;
                // Past the last checkpoint comes the level end (not handled yet).
                self.armed = (next < self.layout.checkpoints.len()).then(|| Armed::new(next));
            } else {
                *n -= 1;
            }
        }
        if let (Some(script), Some(end)) = (self.end_script.as_mut(), self.level_end)
            && script.frame(ball, end.platform)
        {
            self.end_script = None;
            self.finished = true;
            return Some(Event::Finished);
        }
        let armed = self.armed.as_mut()?;
        let cp = self.layout.checkpoints[armed.index];
        if !armed.frame(ball, &cp) {
            return None;
        }
        let index = armed.index;
        self.armed = None;
        // set Resetpoint, activate Sektor, then (2 frames on) the next checkpoint.
        if (self.act as usize) < self.layout.reset_points.len() {
            self.reset = self.act as usize;
        }
        let old = self.act;
        self.act += 1;
        self.switch(old, self.act);
        self.pending_checkpoint = Some(NEXT_CHECKPOINT_DELAY);
        Some(Event::Checkpoint(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proximity_enters_once_and_checks_less_often_when_far() {
        let mut p = Proximity::new(6.5, 20.0, 30.0, (1, 15), false);
        p.switch_on();
        // Far away: Exit on the first check (state unknown), then every 15 frames.
        assert_eq!(
            p.frame(Vec3::new(100.0, 0.0, 0.0), Vec3::ZERO),
            Some(Edge::Exit)
        );
        for _ in 0..14 {
            assert_eq!(p.frame(Vec3::ZERO, Vec3::ZERO), None, "checked too early");
        }
        assert_eq!(p.frame(Vec3::ZERO, Vec3::ZERO), Some(Edge::Enter));
        // Close: every frame, and no repeated Enter.
        assert_eq!(p.frame(Vec3::X, Vec3::ZERO), None);
        assert_eq!(p.frame(Vec3::X * 7.0, Vec3::ZERO), Some(Edge::Exit));
    }

    #[test]
    fn xz_proximity_ignores_height() {
        let mut p = Proximity::new(70.0, 75.0, 150.0, (10, 100), true);
        p.switch_on();
        assert_eq!(
            p.frame(Vec3::new(10.0, 500.0, 0.0), Vec3::ZERO),
            Some(Edge::Enter)
        );
    }

    #[test]
    fn names_number_rows() {
        assert_eq!(number("PR_Resetpoint_03"), Some(3));
        assert_eq!(number("PC_TwoFlames_10"), Some(10));
        assert_eq!(number("Ball"), None);
    }

    fn line_layout() -> Layout {
        // Checkpoints every 100 units along X; reset points at 0, 100, 200.
        Layout {
            checkpoints: (1..=2)
                .map(|i| {
                    let x = 100.0 * i as f32;
                    Checkpoint {
                        frame: Vec3::new(x, 0.0, 0.0),
                        flame: Vec3::new(x, 1.5, 0.0),
                    }
                })
                .collect(),
            reset_points: (0..3)
                .map(|i| Mat4::from_translation(Vec3::X * (100.0 * i as f32)))
                .collect(),
        }
    }

    #[test]
    fn start_activates_sector_one_after_four_frames() {
        let mut p = Progress::start(line_layout(), None);
        // Init writes 1 to both cells: sector 1 off (a no-op), then on.
        assert_eq!(p.take_changes(), vec![(1, false)]);
        for _ in 0..4 {
            p.frame(Vec3::ZERO);
            assert_eq!(p.take_changes(), vec![]);
        }
        p.frame(Vec3::ZERO);
        assert_eq!(p.take_changes(), vec![(1, true)]);
    }

    #[test]
    fn rolling_through_checkpoints_switches_sectors_and_reset_points() {
        let mut p = Progress::start(line_layout(), None);
        let mut changes = Vec::new();
        let mut reached = Vec::new();
        // Roll along X at 0.5 units per frame.
        for f in 0..500 {
            let x = f as f32 * 0.5;
            if let Some(Event::Checkpoint(i)) = p.frame(Vec3::new(x, 1.0, 0.0)) {
                reached.push((i, x));
            }
            changes.extend(p.take_changes());
        }
        assert_eq!(reached.len(), 2, "{reached:?}");
        assert!((reached[0].1 - 94.0).abs() < 1.0, "{reached:?}");
        assert_eq!(p.reached(), 2);
        assert_eq!(
            changes,
            vec![
                (1, false),
                (1, true),
                (1, false),
                (2, true),
                (2, false),
                (3, true)
            ]
        );
        assert_eq!(
            p.reset_point(),
            Some(Mat4::from_translation(Vec3::X * 200.0))
        );
        // A respawn re-enters the current sector.
        p.respawn();
        assert_eq!(p.take_changes(), vec![(3, false)]);
    }

    #[test]
    fn checkpoints_must_be_reached_in_order() {
        let mut p = Progress::start(line_layout(), None);
        // Sitting on the second checkpoint does nothing.
        for _ in 0..300 {
            assert_eq!(p.frame(Vec3::new(200.0, 1.5, 0.0)), None);
        }
    }

    /// Needs `BALLISTIC_BUNDLE`; skipped otherwise.
    #[test]
    fn every_level_has_a_reset_point_per_sector() {
        let Some(path) = std::env::var_os("BALLISTIC_BUNDLE") else {
            eprintln!("skipping: BALLISTIC_BUNDLE not set");
            return;
        };
        let mut b = ballistic_bundle::BundleReader::open(path).unwrap();
        let physics = b.physics().unwrap();
        for level in 1..=12 {
            let scene = b.scene(&format!("level/level_{level:02}")).unwrap();
            let l = layout(&scene, |k| b.scene(k).ok());
            let n = l.checkpoints.len();
            assert!(n >= 1, "level {level}: no checkpoints");
            assert_eq!(l.reset_points.len(), n + 1, "level {level}");
            for c in &l.checkpoints {
                let up = c.flame - c.frame;
                assert!(
                    (up.length() - 1.495).abs() < 0.01,
                    "level {level}: flame offset {up}"
                );
            }
            // Every prop sector is one the progress reaches.
            let max_sector = ballistic_physics::props::find_props(&scene, &physics)
                .iter()
                .map(|s| s.sector)
                .max()
                .unwrap_or(1);
            assert!(
                max_sector as usize <= n + 1,
                "level {level}: prop in sector {max_sector}"
            );
        }
    }

    #[test]
    fn level_end_is_armed_in_the_last_sector() {
        let end = LevelEnd {
            platform: Vec3::new(300.0, 0.0, 0.0),
            sector: 3,
        };
        let mut p = Progress::start(line_layout(), Some(end));
        let mut events = Vec::new();
        // Straight onto the platform before the checkpoints: nothing.
        for _ in 0..200 {
            assert_eq!(p.frame(end.platform), None);
        }
        // Through both checkpoints, then onto the platform.
        for f in 0..700 {
            let x = (f as f32 * 0.5).min(300.0);
            let y = if x < 300.0 { 1.0 } else { 0.0 };
            if let Some(e) = p.frame(Vec3::new(x, y, 0.0)) {
                events.push(e);
            }
            p.take_changes();
        }
        assert_eq!(
            events,
            vec![Event::Checkpoint(0), Event::Checkpoint(1), Event::Finished]
        );
        assert!(p.finished);
        assert_eq!(p.next_target(), None);
    }
}
