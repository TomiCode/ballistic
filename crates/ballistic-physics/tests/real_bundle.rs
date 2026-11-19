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
        shape: Shape::Sphere {
            radius: wood.shape.radius(),
        },
        mass: wood.mass,
        friction: wood.friction,
        elasticity: wood.elasticity,
        linear_damping: wood.linear_damping,
        rotational_damping: wood.rotational_damping,
        group: wood.collision_group.clone(),
        ..BodyDesc::default()
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

#[test]
fn props_settle_where_they_are_placed() {
    let Some(mut b) = bundle() else {
        eprintln!("skipping: BALLISTIC_BUNDLE not set");
        return;
    };
    let physics = b.physics().unwrap();
    let mut prefabs = std::collections::HashMap::new();
    for p in &physics.props {
        let key = format!("ph/{}", p.group.to_lowercase());
        let prefab = b.scene(&key).unwrap();
        prefabs.insert(p.group.clone(), props::collider(p, &prefab).expect(&key));
    }
    let (mut total, mut fallen, mut moving) = (0, Vec::new(), Vec::new());
    for level in 1..=12 {
        let scene = b.scene(&format!("level/level_{level:02}")).unwrap();
        let (mut statics, _) = build_level(&scene, &physics);
        let spawns = props::find_props(&scene, &physics);
        assert!(
            spawns.iter().all(|s| (1..=9).contains(&s.sector)),
            "level {level}: sector outside Sector_01..09"
        );
        let params = |g: &str| physics.props.iter().find(|p| p.group == g).unwrap();
        for s in &spawns {
            props::add_fixed(
                &mut statics,
                params(&s.group),
                &prefabs[&s.group],
                s.transform,
            );
        }
        statics.finish();
        let cfg = WorldConfig {
            gravity: Vec3::from(physics.gravity),
            time_factor: physics.time_factor,
            ..WorldConfig::default()
        };
        let mut w = World::new(cfg, statics);
        let bodies: Vec<_> = spawns
            .iter()
            .filter_map(|s| {
                let d = props::body_desc(params(&s.group), &prefabs[&s.group], s.transform)?;
                let start = d.position;
                Some((s, start, w.add_body(d)))
            })
            .collect();
        // Three simulated seconds.
        for _ in 0..198 {
            w.step();
        }
        for (s, start, id) in &bodies {
            let body = w.body(*id).unwrap();
            let moved = body.position - *start;
            if std::env::var_os("PROPS_DEBUG").is_some() {
                eprintln!(
                    "level {level}: {} #{} at {start} moved {moved} v {}",
                    s.group, s.placeholder, body.velocity
                );
            }
            // Placeholders hover a little above the floor; props drop onto it.
            if moved.y < -25.0 {
                fallen.push(format!("level {level} {} #{}", s.group, s.placeholder));
            }
            if body.velocity.length() > 1.5 {
                moving.push(format!(
                    "level {level} {} #{} v {}",
                    s.group, s.placeholder, body.velocity
                ));
            }
        }
        total += spawns.len();
    }
    assert!(total > 100, "only {total} props found");
    // Props that rest on moving elements (not simulated yet) may fall.
    assert!(fallen.len() <= 2, "fell out of the level: {fallen:#?}");
    assert!(
        moving.len() * 20 <= total,
        "still moving after 3 s: {moving:#?}"
    );
}
