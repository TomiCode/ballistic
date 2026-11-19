//! Physics on real level geometry. Needs `BALLISTIC_BUNDLE` (a bundle built by
//! ballistic-pack); skipped otherwise.

use ballistic_bundle::BundleReader;
use ballistic_physics::*;

fn bundle() -> Option<BundleReader<std::io::BufReader<std::fs::File>>> {
    let path = std::env::var_os("BALLISTIC_BUNDLE")?;
    Some(BundleReader::open(path).expect("open bundle"))
}

fn start_position(scene: &ballistic_bundle::Scene) -> Vec3 {
    let g = scene
        .groups
        .iter()
        .find(|g| g.name == "PS_Levelstart")
        .expect("PS_Levelstart");
    let t = scene.objects[g.members[0] as usize].transform[3];
    Vec3::new(t[0], t[1], t[2])
}

#[test]
fn ball_settles_and_rolls_on_level_one() {
    let Some(mut b) = bundle() else {
        eprintln!("skipping: BALLISTIC_BUNDLE not set");
        return;
    };
    let physics = b.physics().unwrap();
    let scene = b.scene("level/level_01").unwrap();
    let (statics, report) = build_level(&scene, &physics);
    assert!(statics.triangle_count() > 1000, "{report:?}");

    let wood = physics
        .balls
        .iter()
        .find(|x| x.name == "Ball_Wood")
        .unwrap();
    let cfg = WorldConfig {
        gravity: Vec3::from(physics.gravity),
        time_factor: physics.time_factor,
        ..WorldConfig::default()
    };
    let mut w = World::new(cfg, statics);
    let start = start_position(&scene);
    let id = w.add_body(BodyDesc {
        position: start + Vec3::Y * 3.0,
        radius: wood.shape.radius(),
        mass: wood.mass,
        friction: wood.friction,
        elasticity: wood.elasticity,
        linear_damping: wood.linear_damping,
        rotational_damping: wood.rotational_damping,
        group: wood.collision_group.clone(),
    });

    // Settle for 2 simulated seconds: the ball should come to rest on the start pad.
    for _ in 0..132 {
        w.step();
    }
    let rest = w.body(id).unwrap().clone();
    assert!(rest.grounded, "ball not on the ground at {}", rest.position);
    assert!(
        rest.velocity.length() < 0.5,
        "still moving: {}",
        rest.velocity
    );
    assert!(
        (rest.position.y - start.y).abs() < 5.0,
        "settled at {}",
        rest.position
    );

    // Push in every horizontal direction until one makes progress without falling off.
    let mut moved = false;
    for dir in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
        let mut trial = w.clone();
        let p = trial.add_push(id, dir, wood.force);
        for _ in 0..66 {
            trial.step();
        }
        trial.remove_push(p);
        let after = trial.body(id).unwrap();
        let travelled = (after.position - rest.position).length();
        if travelled > 3.0 && after.position.y > rest.position.y - 5.0 {
            moved = true;
            // Rolling, not sliding: the ball spins.
            assert!(
                after.angular_velocity.length() > 0.5,
                "no spin: {}",
                after.angular_velocity
            );
            break;
        }
    }
    assert!(moved, "no direction moved the ball on the start pad");
}
