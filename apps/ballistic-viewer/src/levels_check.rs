//! Every level, headless: the checkpoints in turn to the finish, and a ball
//! lost at the last checkpoint coming back there. Needs `BALLISTIC_BUNDLE`
//! and a GPU adapter (play mode loads its renderers); skipped otherwise.
//!
//! This checks the level flow, not the routes between checkpoints: the ball
//! is warped (as the `next` play-script key does).

use crate::play::{AfterScore, Bundle, Play};
use crate::save::{SaveFile, SharedSave};
use ballistic_bundle::BundleReader;
use ballistic_render::{Gpu, OFFSCREEN_FORMAT};

fn run(p: &mut Play, seconds: f32) {
    let dt = 1.0 / 60.0;
    #[expect(clippy::cast_sign_loss, reason = "positive durations")]
    let frames = (seconds / dt).round() as u32;
    for _ in 0..frames {
        p.update(dt);
        p.take_level_visibility();
        p.take_sounds();
        p.loops();
    }
}

#[test]
fn every_level_flows_to_the_finish() {
    let Some(path) = std::env::var_os("BALLISTIC_BUNDLE") else {
        eprintln!("skipping: BALLISTIC_BUNDLE not set");
        return;
    };
    let gpu = match pollster::block_on(Gpu::headless()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let mut bundle: Bundle = BundleReader::open(path).unwrap();
    // One save through all the levels (in memory).
    let defaults = bundle.physics().unwrap().highscores;
    let save = SaveFile::in_memory(&defaults).shared();
    for level in 1..=12 {
        let key = format!("level/level_{level:02}");
        let scene = bundle.scene(&key).unwrap();
        let mut p = Play::new(
            &gpu,
            &mut bundle,
            &key,
            &scene,
            OFFSCREEN_FORMAT,
            save.clone(),
        )
        .unwrap();
        // The ball comes 3 s after the start.
        run(&mut p, 4.0);
        assert!(!p.status().contains("[new ball]"), "level {level}: no ball");
        let (_, _, checkpoints) = p.progress();
        for n in 1..=checkpoints {
            // The next checkpoint arms a while after the last one.
            p.warp_to_next_checkpoint();
            run(&mut p, 3.0);
            assert_eq!(p.progress().1, n, "level {level}: checkpoint {n}");
        }
        // Lost at the last checkpoint: back there 4 s later, and staying.
        p.respawn();
        run(&mut p, 8.0);
        let status = p.status();
        assert_eq!(p.respawns, 1, "level {level}: {status}");
        assert!(
            status.contains("lives 2") && !status.contains('['),
            "level {level}: {status}"
        );
        assert_eq!(p.progress().1, checkpoints, "level {level}");
        // Then the level end.
        p.warp_to_next_checkpoint();
        run(&mut p, 3.0);
        let status = p.status();
        assert!(status.contains("[finished"), "level {level}: {status}");
        assert!(status.contains("[level end]"), "level {level}: {status}");
        assert!(
            p.fallen_modules().is_empty(),
            "level {level}: {:?}",
            p.fallen_modules()
        );
        eprintln!("level {level}: {status}");
        level_end(&mut p, level, &save);
    }
}

/// The level end of `level`, its score screen and highscores, and what
/// comes next.
fn level_end(p: &mut Play, level: usize, save: &SharedSave) {
    // The wait cut short, fast scoring, the score screen left; the lives
    // go one by one first.
    assert!(p.menu_key(57), "level {level}: the wait's key");
    // Faded in, the level bonus still to come.
    run(p, 1.0);
    assert!(
        p.status().contains("[score screen 0]"),
        "level {level}: {}",
        p.status()
    );
    assert!(p.menu_key(57), "level {level}: fast scoring");
    assert!(p.menu_key(28), "level {level}: leaving");
    assert_eq!(
        p.take_next_level(),
        None,
        "level {level}: lives still going"
    );
    run(p, 3.0);
    // A score for the table: the name entry, then the table; the next
    // level opens either way.
    let before = save.borrow().data.highscores[level - 1].clone();
    if p.status().contains("[highscore entry]") {
        assert_eq!(p.take_next_level(), None, "level {level}: the name first");
        p.menu_text("Check");
        assert!(p.menu_key(28), "level {level}: the name");
        run(p, 0.1);
        assert!(
            p.status().contains("[highscores]"),
            "level {level}: {}",
            p.status()
        );
        assert!(p.menu_key(205), "level {level}: the next table");
        assert!(p.menu_key(1), "level {level}: leaving the table");
        run(p, 0.1);
        let s = save.borrow();
        assert_ne!(s.data.highscores[level - 1], before, "level {level}");
        assert!(
            s.data.highscores[level - 1]
                .iter()
                .any(|h| h.name.ends_with("Check"))
        );
    } else {
        assert_eq!(
            save.borrow().data.highscores[level - 1],
            before,
            "level {level}"
        );
    }
    if level < 12 {
        assert!(
            save.borrow().data.is_unlocked(level),
            "level {level}: the next one opens"
        );
    }
    let next = if level < 12 {
        AfterScore::Level(format!("level/level_{:02}", level + 1))
    } else {
        AfterScore::LastLevel
    };
    assert_eq!(p.take_next_level(), Some(next), "level {level}");
    assert!(
        p.status().contains("lives 0"),
        "level {level}: {}",
        p.status()
    );
}
