//! The push-stone puzzle (`P_Modul_34`), headless: a stone on a vertical
//! slider over a crate. Pushed by a stone ball, the crate slides out and the
//! stone stays on its line; with the crate gone, the stone drops into its
//! place and rests there. And the pusher (`P_Modul_01`) stays in its channel. Needs `BALLISTIC_BUNDLE` and a GPU adapter; skipped
//! otherwise.

use ballistic_bundle::BundleReader;
use ballistic_physics::{BodyDesc, Shape, Vec3};
use ballistic_render::{Gpu, OFFSCREEN_FORMAT};

use crate::play::{Bundle, Play};
use crate::save::SaveFile;

fn step(p: &mut Play, seconds: f32) {
    let dt = 1.0 / 60.0;
    #[expect(clippy::cast_sign_loss, reason = "positive durations")]
    let frames = (seconds / dt).round() as u32;
    for _ in 0..frames {
        p.update(dt);
        p.take_level_visibility();
        p.take_sounds();
        p.take_music();
        p.loops();
    }
}

/// Levels with the puzzle, and the axis its first crate slides out along.
const LEVELS: [(u32, Vec3); 2] = [(7, Vec3::X), (10, Vec3::Z)];

#[test]
fn the_push_stone_drops_into_the_crates_place() {
    let Some(path) = std::env::var_os("BALLISTIC_BUNDLE") else {
        eprintln!("skipping: BALLISTIC_BUNDLE not set");
        return;
    };
    let Ok(gpu) = pollster::block_on(Gpu::headless()) else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut bundle: Bundle = BundleReader::open(path).unwrap();
    let physics = bundle.physics().unwrap();
    let stone = physics
        .balls
        .iter()
        .find(|b| b.name == "Ball_Stone")
        .unwrap()
        .clone();
    let save = SaveFile::in_memory(&physics.highscores).shared();
    for (level, along) in LEVELS {
        let key = format!("level/level_{level:02}");
        let scene = bundle.scene(&key).unwrap();
        for dir in [along, -along] {
            let mut p = Play::new(
                &gpu,
                &mut bundle,
                &key,
                &scene,
                OFFSCREEN_FORMAT,
                save.clone(),
            )
            .unwrap();
            step(&mut p, 0.1);
            let (sector, ..) = p.sliders_of("P_Modul_34")[0];
            p.activate_sector_for_test(sector);
            let (_, _, origin, axis) = p.sliders_of("P_Modul_34")[0];
            let bodies = p.module_bodies("P_Modul_34")[0].clone();
            let (crate_, stone_id) = (bodies[0], bodies[1]);
            let c0 = p.world.body(crate_).unwrap().position;
            let s0 = p.world.body(stone_id).unwrap().position;
            p.world.wake(crate_);
            p.world.wake(stone_id);
            step(&mut p, 1.0);
            // A stone ball against the crate's side, pushing.
            let r = stone.shape.radius();
            let ball = p.world.add_body(BodyDesc {
                position: c0 - dir * (2.05 + r + 0.5),
                shape: Shape::Sphere { radius: r },
                mass: stone.mass,
                friction: stone.friction,
                elasticity: stone.elasticity,
                linear_damping: stone.linear_damping,
                rotational_damping: stone.rotational_damping,
                group: stone.collision_group.clone(),
                ..BodyDesc::default()
            });
            p.world.add_push(ball, dir, stone.force);
            let axis = axis.normalize();
            for _ in 0..30 {
                step(&mut p, 0.5);
                let s = p.world.body(stone_id).unwrap().position;
                let off = s - origin;
                let across = (off - axis * off.dot(axis)).length();
                assert!(
                    across < 0.05,
                    "level {level}: the stone left its line by {across}"
                );
            }
            let c = p.world.body(crate_).unwrap().position;
            assert!(
                (c - c0).dot(dir) > 3.0,
                "level {level} {dir}: the crate didn't slide out ({:?})",
                c - c0
            );
            // The crate gone, the stone drops 4.8 into its place and stays.
            p.world.remove_body(crate_);
            step(&mut p, 3.0);
            let drop = p.world.body(stone_id).unwrap().position.y - s0.y;
            assert!(
                (drop + 4.8).abs() < 0.2,
                "level {level} {dir}: the stone dropped {drop}"
            );
        }
    }
}

/// Levels with the pusher (`P_Modul_01`) whose every pusher a stone ball
/// can reach from where the test puts it.
const PUSHER_LEVELS: [u32; 4] = [7, 9, 10, 11];

/// The pusher: a stone ball against its disc shoves it 5 along its channel,
/// where the disc meets the channel walls' ends. It stays in the channel
/// (it was once thrown sideways through a wall there and fell).
#[test]
fn the_pusher_stops_in_its_channel() {
    let Some(path) = std::env::var_os("BALLISTIC_BUNDLE") else {
        eprintln!("skipping: BALLISTIC_BUNDLE not set");
        return;
    };
    let Ok(gpu) = pollster::block_on(Gpu::headless()) else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut bundle: Bundle = BundleReader::open(path).unwrap();
    let physics = bundle.physics().unwrap();
    let stone = physics
        .balls
        .iter()
        .find(|b| b.name == "Ball_Stone")
        .unwrap()
        .clone();
    let save = SaveFile::in_memory(&physics.highscores).shared();
    for level in PUSHER_LEVELS {
        let key = format!("level/level_{level:02}");
        let scene = bundle.scene(&key).unwrap();
        let sectors = {
            let mut p = Play::new(
                &gpu,
                &mut bundle,
                &key,
                &scene,
                OFFSCREEN_FORMAT,
                save.clone(),
            )
            .unwrap();
            step(&mut p, 0.1);
            p.module_sectors("P_Modul_01")
        };
        for (m, sector) in sectors.into_iter().enumerate() {
            let mut p = Play::new(
                &gpu,
                &mut bundle,
                &key,
                &scene,
                OFFSCREEN_FORMAT,
                save.clone(),
            )
            .unwrap();
            step(&mut p, 0.1);
            p.activate_sector_for_test(sector);
            let pusher = p.module_bodies("P_Modul_01")[m][0];
            let b0 = p.world.body(pusher).unwrap();
            let (c0, q) = (b0.position, b0.orientation);
            p.world.wake(pusher);
            // In front of the disc (the pusher's own −X end), on the floor.
            let r = stone.shape.radius();
            let ball = p.world.add_body(BodyDesc {
                position: c0 + q * Vec3::new(-8.01 - r - 0.3, 1.49 + r + 0.05, 0.0),
                shape: Shape::Sphere { radius: r },
                mass: stone.mass,
                friction: stone.friction,
                elasticity: stone.elasticity,
                linear_damping: stone.linear_damping,
                rotational_damping: stone.rotational_damping,
                group: stone.collision_group.clone(),
                ..BodyDesc::default()
            });
            p.world.add_push(ball, q * Vec3::X, stone.force);
            step(&mut p, 12.0);
            let at = q.inverse() * (p.world.body(pusher).unwrap().position - c0);
            assert!(
                (at.x - 5.0).abs() < 0.05 && at.y.abs() < 0.2 && at.z.abs() < 0.15,
                "level {level} pusher {m}: at {at} in its own frame"
            );
        }
    }
}

/// The bridges (`P_Modul_25`, `P_Modul_37`): knocked over towards the gap,
/// the plank falls onto the shelf of the far bank and lies there still. (It
/// once fell through the shelf, pushed down by the bevel under the shelf's
/// lip, and where it did land it shook against its hinge.)
#[test]
fn the_bridges_land_on_the_far_bank_and_rest() {
    let Some(path) = std::env::var_os("BALLISTIC_BUNDLE") else {
        eprintln!("skipping: BALLISTIC_BUNDLE not set");
        return;
    };
    let Ok(gpu) = pollster::block_on(Gpu::headless()) else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut bundle: Bundle = BundleReader::open(path).unwrap();
    let physics = bundle.physics().unwrap();
    let save = SaveFile::in_memory(&physics.highscores).shared();
    let mut checked = 0;
    for level in 1..=12u32 {
        let key = format!("level/level_{level:02}");
        let scene = bundle.scene(&key).unwrap();
        for prefix in ["P_Modul_25", "P_Modul_37"] {
            let (sectors, hinges) = {
                let mut p = Play::new(
                    &gpu,
                    &mut bundle,
                    &key,
                    &scene,
                    OFFSCREEN_FORMAT,
                    save.clone(),
                )
                .unwrap();
                step(&mut p, 0.1);
                (p.module_sectors(prefix), p.module_hinges(prefix))
            };
            for (m, sector) in sectors.into_iter().enumerate() {
                let (_, pivot, axis) = hinges.iter().find(|h| h.0 == m).copied().unwrap();
                let mut p = Play::new(
                    &gpu,
                    &mut bundle,
                    &key,
                    &scene,
                    OFFSCREEN_FORMAT,
                    save.clone(),
                )
                .unwrap();
                step(&mut p, 0.1);
                p.activate_sector_for_test(sector);
                let id = p.module_bodies(prefix)[m][0];
                p.world.wake(id);
                let b0 = p.world.body(id).unwrap().clone();
                // The plank's tip: the hull corner farthest from the hinge.
                let pivot_local = b0.orientation.inverse() * (pivot - b0.position);
                let ballistic_physics::Shape::Hulls(hulls) = &b0.shape else {
                    panic!("{prefix}: not hulls");
                };
                let tip = hulls
                    .iter()
                    .flat_map(|h| h.verts.iter().copied())
                    .max_by(|a, b| a.distance(pivot_local).total_cmp(&b.distance(pivot_local)))
                    .unwrap();
                // A nudge towards the gap. (Hanging into it, the tip would be
                // 9 or 14 below the hinge; level 12's first bridge 37 lands
                // 1.4 low.)
                p.world.body_mut(id).unwrap().angular_velocity = axis.normalize() * 0.6;
                step(&mut p, 6.0);
                let mut spin = 0.0f32;
                for _ in 0..60 {
                    step(&mut p, 1.0 / 60.0);
                    spin = spin.max(p.world.body(id).unwrap().angular_velocity.length());
                }
                let b = p.world.body(id).unwrap();
                let rise = (b.position + b.orientation * tip).y - pivot.y;
                assert!(
                    rise.abs() < 2.0,
                    "level {level} {prefix} #{m}: the tip is {rise} above the hinge"
                );
                assert!(
                    spin < 0.01,
                    "level {level} {prefix} #{m}: still turning at {spin} rad/s"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 30, "{checked} bridges");
}
