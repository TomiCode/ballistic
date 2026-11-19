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

#[test]
#[allow(clippy::too_many_lines, reason = "one scenario per level")]
fn modules_hold_together_on_every_level() {
    let Some(mut b) = bundle() else {
        eprintln!("skipping: BALLISTIC_BUNDLE not set");
        return;
    };
    let physics = b.physics().unwrap();
    assert!(
        physics.modules.len() >= 10,
        "{} modules",
        physics.modules.len()
    );
    let mut placed = 0;
    let mut simulated = 0;
    for level in 1..=12 {
        let scene = b.scene(&format!("level/level_{level:02}")).unwrap();
        let (mut statics, _) = build_level(&scene, &physics);
        statics.finish();
        let cfg = WorldConfig {
            gravity: Vec3::from(physics.gravity),
            time_factor: physics.time_factor,
            ..WorldConfig::default()
        };
        let mut w = World::new(cfg, statics);
        let mut pivots = Vec::new();
        let mut sliding = Vec::new();
        for setup in &physics.modules {
            let spawns = props::placeholders(&scene, &setup.group);
            if spawns.is_empty() {
                continue;
            }
            let prefab = b
                .scene(&format!("ph/{}", setup.group.to_lowercase()))
                .unwrap();
            for s in &spawns {
                let plan = modules::plan(setup, &prefab, s.transform)
                    .unwrap_or_else(|e| panic!("level {level} {}: {e:#}", setup.group));
                placed += 1;
                if !modules::simulated(setup) {
                    continue;
                }
                simulated += 1;
                let ids: Vec<_> = plan
                    .bodies
                    .iter()
                    .map(|pb| {
                        let mut d = pb.desc.clone();
                        d.asleep = false; // as if the ball came near
                        w.add_body(d)
                    })
                    .collect();
                for sl in &plan.sliders {
                    w.add_slider(
                        ids[sl.body],
                        sl.other.map(|o| ids[o]),
                        sl.origin,
                        sl.axis,
                        sl.limits,
                    )
                    .unwrap();
                    let body = w.body(ids[sl.body]).unwrap();
                    sliding.push((setup.group.clone(), ids[sl.body], body.position, sl.axis));
                }
                for j in &plan.ball_joints {
                    w.add_ball_joint(ids[j.body], j.other.map(|o| ids[o]), j.pivot)
                        .unwrap();
                    let body = w.body(ids[j.body]).unwrap();
                    let local = body.orientation.inverse() * (j.pivot - body.position);
                    pivots.push((
                        setup.group.clone(),
                        ids[j.body],
                        local,
                        j.pivot,
                        j.other.is_none(),
                    ));
                }
                for sp in &plan.springs {
                    if std::env::var_os("PROPS_DEBUG").is_some() {
                        let body = w.body(ids[sp.body]).unwrap();
                        eprintln!(
                            "level {level} {}: spring {} -> {} (body at {}, asleep {})",
                            setup.group, sp.point, sp.other_point, body.position, body.asleep
                        );
                    }
                    w.add_spring(
                        ids[sp.body],
                        sp.other.map(|o| ids[o]),
                        sp.point,
                        sp.other_point,
                        sp.length,
                        sp.constant,
                        sp.damping,
                    )
                    .unwrap();
                }
                for h in &plan.hinges {
                    w.add_hinge(ids[h.body], h.other.map(|o| ids[o]), h.pivot, h.axis)
                        .unwrap();
                    let body = w.body(ids[h.body]).unwrap();
                    let local = body.orientation.inverse() * (h.pivot - body.position);
                    pivots.push((
                        setup.group.clone(),
                        ids[h.body],
                        local,
                        h.pivot,
                        h.other.is_none(),
                    ));
                }
            }
        }
        for _ in 0..198 {
            w.step();
        }
        for (group, id, start, axis) in &sliding {
            let body = w.body(*id).unwrap();
            assert!(body.position.is_finite(), "level {level} {group}: exploded");
            let off = body.position - *start;
            let across = (off - *axis * off.dot(*axis)).length();
            assert!(
                across < 0.2,
                "level {level} {group}: left its slider by {across}"
            );
            if std::env::var_os("PROPS_DEBUG").is_some() {
                eprintln!(
                    "level {level} {group}: slid {:.2} along the axis",
                    off.dot(*axis)
                );
            }
        }
        for (group, id, local, pivot, to_world) in &pivots {
            let body = w.body(*id).unwrap();
            if std::env::var_os("PROPS_DEBUG").is_some() {
                eprintln!(
                    "level {level} {group}: speed {:.3} spin {:.3} up {}",
                    body.velocity.length(),
                    body.angular_velocity.length(),
                    body.orientation * Vec3::Y
                );
            }
            assert!(body.position.is_finite(), "level {level} {group}: exploded");
            if *to_world {
                let now = body.position + body.orientation * *local;
                assert!(
                    now.distance(*pivot) < 0.2,
                    "level {level} {group}: pivot moved {} ({now} vs {pivot})",
                    now.distance(*pivot)
                );
            }
        }
    }
    assert!(
        placed > 10 && simulated > 5,
        "placed {placed}, simulated {simulated}"
    );
}
