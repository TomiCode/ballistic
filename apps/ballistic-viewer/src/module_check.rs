//! The push-stone puzzle (`P_Modul_34`), headless: a stone on a vertical
//! slider over a crate. Pushed by a stone ball, the crate slides out and the
//! stone stays on its line; with the crate gone, the stone drops into its
//! place and rests there. Needs `BALLISTIC_BUNDLE` and a GPU adapter; skipped
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
