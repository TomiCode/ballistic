//! Hulls, hinges and sleeping bodies.

use std::sync::Arc;

use ballistic_physics::*;

fn floor() -> StaticMesh {
    let mut m = StaticMesh::default();
    let mat = m.add_material(SurfaceMaterial {
        friction: 0.7,
        elasticity: 0.3,
        group: "Floor".into(),
    });
    let s = 100.0;
    m.add_triangle(
        Vec3::new(-s, 0.0, -s),
        Vec3::new(-s, 0.0, s),
        Vec3::new(s, 0.0, s),
        mat,
    );
    m.add_triangle(
        Vec3::new(-s, 0.0, -s),
        Vec3::new(s, 0.0, s),
        Vec3::new(s, 0.0, -s),
        mat,
    );
    m.finish();
    m
}

fn cuboid(half: Vec3, offset: Vec3) -> Hull {
    let pts: Vec<Vec3> = (0..8)
        .map(|i| {
            offset
                + Vec3::new(
                    if i & 1 == 0 { -half.x } else { half.x },
                    if i & 2 == 0 { -half.y } else { half.y },
                    if i & 4 == 0 { -half.z } else { half.z },
                )
        })
        .collect();
    Hull::new(&pts).unwrap()
}

fn hull_body(position: Vec3, hulls: Vec<Hull>, mass: f32) -> BodyDesc {
    BodyDesc {
        position,
        shape: Shape::Hulls(Arc::from(hulls)),
        mass,
        friction: 0.7,
        elasticity: 0.3,
        ..BodyDesc::default()
    }
}

fn no_damping() -> WorldConfig {
    WorldConfig::default()
}

#[test]
fn hull_box_rests_like_a_box() {
    let mut w = World::new(no_damping(), floor());
    let b = w.add_body(hull_body(
        Vec3::new(0.0, 3.0, 0.0),
        vec![cuboid(Vec3::splat(2.5), Vec3::ZERO)],
        1.0,
    ));
    for _ in 0..660 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        (body.position.y - 2.5).abs() < 0.05,
        "y = {}",
        body.position.y
    );
    assert!(body.velocity.length() < 0.05);
    assert!((body.orientation * Vec3::Y).y > 0.999);
}

/// A rod hanging from a world hinge swings with the period of a physical
/// pendulum, `T = 2π √(I_pivot / (m g d))`, and its pivot stays put.
#[test]
fn hinged_rod_is_a_pendulum() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    // A 1 × 10 × 1 rod, its centre 5 below the pivot at the origin.
    let half = Vec3::new(0.5, 5.0, 0.5);
    let start = Quat::from_rotation_z(0.2);
    let mut d = hull_body(
        start * Vec3::new(0.0, -5.0, 0.0),
        vec![cuboid(half, Vec3::ZERO)],
        2.0,
    );
    d.orientation = start;
    let rod = w.add_body(d);
    w.add_hinge(rod, None, Vec3::ZERO, Vec3::Z).unwrap();

    let angle = |w: &World| {
        let b = w.body(rod).unwrap();
        b.position.x.atan2(-b.position.y)
    };
    let mut crossings = Vec::new();
    let mut last = angle(&w);
    for step in 0..2000 {
        w.step();
        let a = angle(&w);
        if last > 0.0 && a <= 0.0 {
            crossings.push(step);
        }
        last = a;
        let b = w.body(rod).unwrap();
        let pivot = b.position + b.orientation * Vec3::new(0.0, 5.0, 0.0);
        assert!(
            pivot.length() < 0.05,
            "pivot drifted to {pivot} at step {step}"
        );
        assert!(b.position.z.abs() < 1e-3, "left the hinge plane");
    }
    assert!(crossings.len() >= 2, "{crossings:?}");
    let period = (crossings[1] - crossings[0]) as f32 * STEP_DT;
    // I about the pivot: m (h² + w²)/12 + m d² with h = 10, w = 1, d = 5.
    let i_pivot: f32 = 2.0 * (100.0 + 1.0) / 12.0 + 2.0 * 25.0;
    let expected = std::f32::consts::TAU * (i_pivot / (2.0 * 20.0 * 5.0)).sqrt();
    assert!(
        (period - expected).abs() < 0.05 * expected,
        "period {period}, expected {expected}"
    );
}

#[test]
fn chain_of_hinges_stays_connected() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    // Two planks between two fixed pivots at x = 0 and x = 8 (like P_Modul_29).
    let half = Vec3::new(2.0, 0.25, 1.0);
    let a = w.add_body(hull_body(
        Vec3::new(2.0, 0.0, 0.0),
        vec![cuboid(half, Vec3::ZERO)],
        0.5,
    ));
    let b = w.add_body(hull_body(
        Vec3::new(6.0, 0.0, 0.0),
        vec![cuboid(half, Vec3::ZERO)],
        0.5,
    ));
    w.add_hinge(a, None, Vec3::ZERO, Vec3::Z).unwrap();
    w.add_hinge(b, Some(a), Vec3::new(4.0, 0.0, 0.0), Vec3::Z)
        .unwrap();
    w.add_hinge(b, None, Vec3::new(8.0, 0.0, 0.0), Vec3::Z)
        .unwrap();
    for _ in 0..660 {
        w.step();
    }
    let (ba, bb) = (w.body(a).unwrap(), w.body(b).unwrap());
    let joint_a = ba.position + ba.orientation * Vec3::new(2.0, 0.0, 0.0);
    let joint_b = bb.position + bb.orientation * Vec3::new(-2.0, 0.0, 0.0);
    assert!(joint_a.distance(joint_b) < 0.1, "{joint_a} vs {joint_b}");
    // It sags under gravity but hangs on its ends.
    assert!(joint_a.y < -0.01 && joint_a.y > -2.0, "middle at {joint_a}");
    let end_a = ba.position + ba.orientation * Vec3::new(-2.0, 0.0, 0.0);
    assert!(end_a.length() < 0.1, "{end_a}");
}

#[test]
fn frozen_body_waits_until_touched() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    let mut d = hull_body(
        Vec3::new(0.0, 0.0, 0.0),
        vec![cuboid(Vec3::ONE, Vec3::ZERO)],
        1.0,
    );
    d.asleep = true;
    let frozen = w.add_body(d);
    for _ in 0..66 {
        w.step();
    }
    assert_eq!(
        w.body(frozen).unwrap().position,
        Vec3::ZERO,
        "fell while asleep"
    );
    // A ball dropped on it wakes it up.
    let ball = w.add_body(BodyDesc {
        position: Vec3::new(0.0, 4.0, 0.0),
        shape: Shape::Sphere { radius: 1.0 },
        mass: 1.0,
        ..BodyDesc::default()
    });
    for _ in 0..66 {
        w.step();
    }
    assert!(!w.body(frozen).unwrap().asleep);
    assert!(w.body(frozen).unwrap().position.y < -0.5);
    let _ = ball;
}

#[test]
fn explicit_wake_starts_simulation() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    let mut d = hull_body(Vec3::ZERO, vec![cuboid(Vec3::ONE, Vec3::ZERO)], 1.0);
    d.asleep = true;
    let b = w.add_body(d);
    w.step();
    assert_eq!(w.body(b).unwrap().velocity, Vec3::ZERO);
    w.wake(b);
    w.step();
    assert!(w.body(b).unwrap().velocity.y < 0.0);
}

#[test]
fn ball_hits_hinged_flap_and_turns_it() {
    let mut w = World::new(no_damping(), floor());
    // An upright flap hinged at its bottom edge, mass centre above the hinge.
    let half = Vec3::new(0.25, 3.0, 2.0);
    let flap = w.add_body(hull_body(
        Vec3::new(0.0, 3.5, 0.0),
        vec![cuboid(half, Vec3::ZERO)],
        3.0,
    ));
    w.add_hinge(flap, None, Vec3::new(0.0, 0.5, 0.0), Vec3::Z)
        .unwrap();
    let ball = w.add_body(BodyDesc {
        position: Vec3::new(-6.0, 2.0, 0.0),
        shape: Shape::Sphere { radius: 2.0 },
        mass: 2.0,
        friction: 0.6,
        ..BodyDesc::default()
    });
    w.body_mut(ball).unwrap().velocity = Vec3::new(10.0, 0.0, 0.0);
    for _ in 0..264 {
        w.step();
    }
    let f = w.body(flap).unwrap();
    let up = f.orientation * Vec3::Y;
    assert!(up.x > 0.5, "flap didn't fall forward: up {up}");
    let pivot = f.position + f.orientation * Vec3::new(0.0, -3.0, 0.0);
    assert!(
        pivot.distance(Vec3::new(0.0, 0.5, 0.0)) < 0.1,
        "pivot at {pivot}"
    );
}

#[test]
fn slider_keeps_a_body_on_its_axis() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    let b = w.add_body(hull_body(
        Vec3::new(0.0, 5.0, 0.0),
        vec![cuboid(Vec3::ONE, Vec3::ZERO)],
        2.0,
    ));
    w.add_slider(b, None, Vec3::new(0.0, 5.0, 0.0), Vec3::X, None)
        .unwrap();
    // A sideways and upward kick and a spin; only the X part survives.
    {
        let body = w.body_mut(b).unwrap();
        body.velocity = Vec3::new(3.0, 4.0, 2.0);
        body.angular_velocity = Vec3::new(1.0, 2.0, 3.0);
    }
    for _ in 0..132 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        (body.position.y - 5.0).abs() < 0.02,
        "fell: {}",
        body.position
    );
    assert!(
        body.position.z.abs() < 0.02,
        "left the axis: {}",
        body.position
    );
    assert!(body.position.x > 3.0, "didn't slide: {}", body.position);
    assert!(
        body.orientation.angle_between(Quat::IDENTITY) < 0.02,
        "turned"
    );
}

#[test]
fn spring_on_a_slider_oscillates_and_settles() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    let (mass, k) = (3.0, 15.0);
    let b = w.add_body(hull_body(
        Vec3::ZERO,
        vec![cuboid(Vec3::ONE, Vec3::ZERO)],
        mass,
    ));
    w.add_slider(b, None, Vec3::ZERO, Vec3::Y, None).unwrap();
    // Rest length 0 to a world point 10 above: equilibrium at 10 − m g / k = 6.
    w.add_spring(
        b,
        None,
        Vec3::ZERO,
        Vec3::new(0.0, 10.0, 0.0),
        0.0,
        k,
        (0.0, 0.0),
    )
    .unwrap();
    let mut peaks = Vec::new();
    let mut last_v = 0.0;
    for step in 0..400 {
        w.step();
        let v = w.body(b).unwrap().velocity.y;
        if last_v > 0.0 && v <= 0.0 {
            peaks.push(step);
        }
        last_v = v;
    }
    assert!(peaks.len() >= 2, "{peaks:?}");
    let period = (peaks[1] - peaks[0]) as f32 * STEP_DT;
    let expected = std::f32::consts::TAU * (mass / k).sqrt();
    assert!(
        (period - expected).abs() < 0.05 * expected,
        "period {period}, expected {expected}"
    );

    // With global damping it comes to rest at the equilibrium.
    let mut w = World::new(no_damping(), StaticMesh::default());
    let b = w.add_body(hull_body(
        Vec3::ZERO,
        vec![cuboid(Vec3::ONE, Vec3::ZERO)],
        mass,
    ));
    w.add_slider(b, None, Vec3::ZERO, Vec3::Y, None).unwrap();
    w.add_spring(
        b,
        None,
        Vec3::ZERO,
        Vec3::new(0.0, 10.0, 0.0),
        0.0,
        k,
        (0.1, 3.0),
    )
    .unwrap();
    for _ in 0..1320 {
        w.step();
    }
    let y = w.body(b).unwrap().position.y;
    let rest = 10.0 - mass * 20.0 / k;
    assert!((y - rest).abs() < 0.05, "settled at {y}, expected {rest}");
}

#[test]
fn slider_limits_stop_it() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    let b = w.add_body(hull_body(
        Vec3::ZERO,
        vec![cuboid(Vec3::ONE, Vec3::ZERO)],
        1.0,
    ));
    // Down by gravity, at most 2.
    w.add_slider(b, None, Vec3::ZERO, Vec3::Y, Some((-2.0, 0.0)))
        .unwrap();
    for _ in 0..132 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        (body.position.y + 2.0).abs() < 0.05,
        "y = {}",
        body.position.y
    );
    assert!(body.velocity.length() < 0.2, "v = {}", body.velocity);
}

#[test]
fn spring_turns_a_hinged_arm_back() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    // An arm hinged at the origin about Z, with a spring from its tip to a
    // world point: it swings towards that point and settles there.
    let arm = w.add_body(hull_body(
        Vec3::new(2.0, 0.0, 0.0),
        vec![cuboid(Vec3::new(2.0, 0.25, 0.25), Vec3::ZERO)],
        1.0,
    ));
    w.add_hinge(arm, None, Vec3::ZERO, Vec3::Z).unwrap();
    let target = Vec3::new(0.0, 4.0, 0.0);
    w.add_spring(
        arm,
        None,
        Vec3::new(4.0, 0.0, 0.0),
        target,
        0.0,
        200.0,
        (0.0, 10.0),
    )
    .unwrap();
    for _ in 0..1320 {
        w.step();
    }
    let b = w.body(arm).unwrap();
    let tip = b.position + b.orientation * Vec3::new(2.0, 0.0, 0.0);
    assert!(tip.distance(target) < 0.6, "tip at {tip}");
}

#[test]
fn ball_joint_lets_a_pendulum_swing_any_way() {
    let mut w = World::new(no_damping(), StaticMesh::default());
    // A weight hanging 5 below a world pivot, pushed sideways in X and Z.
    let bob = w.add_body(hull_body(
        Vec3::new(0.0, -5.0, 0.0),
        vec![cuboid(Vec3::splat(0.5), Vec3::ZERO)],
        1.0,
    ));
    w.add_ball_joint(bob, None, Vec3::ZERO).unwrap();
    w.body_mut(bob).unwrap().velocity = Vec3::new(3.0, 0.0, 2.0);
    let mut max_x: f32 = 0.0;
    let mut max_z: f32 = 0.0;
    for step in 0..400 {
        w.step();
        let b = w.body(bob).unwrap();
        let pivot = b.position + b.orientation * Vec3::new(0.0, 5.0, 0.0);
        assert!(
            pivot.length() < 0.05,
            "pivot drifted to {pivot} at step {step}"
        );
        max_x = max_x.max(b.position.x.abs());
        max_z = max_z.max(b.position.z.abs());
    }
    assert!(
        max_x > 1.0 && max_z > 0.5,
        "didn't swing both ways: {max_x} {max_z}"
    );
}

#[test]
fn push_off_centre_turns_a_body() {
    let cfg = WorldConfig {
        gravity: Vec3::ZERO,
        ..WorldConfig::default()
    };
    let mut w = World::new(cfg, StaticMesh::default());
    let b = w.add_body(hull_body(
        Vec3::ZERO,
        vec![cuboid(Vec3::ONE, Vec3::ZERO)],
        1.0,
    ));
    w.add_push_at(b, Vec3::Z, 0.1, Vec3::new(1.0, 0.0, 0.0));
    for _ in 0..10 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        (body.velocity.z - 1.0).abs() < 1e-4,
        "v = {}",
        body.velocity
    );
    // r × F = X × Z = −Y.
    assert!(
        body.angular_velocity.y < -0.1,
        "w = {}",
        body.angular_velocity
    );
}

#[test]
fn body_without_collision_falls_through() {
    let mut w = World::new(no_damping(), floor());
    let mut d = hull_body(
        Vec3::new(0.0, 3.0, 0.0),
        vec![cuboid(Vec3::ONE, Vec3::ZERO)],
        1.0,
    );
    d.collides = false;
    let b = w.add_body(d);
    for _ in 0..66 {
        w.step();
    }
    assert!(w.body(b).unwrap().position.y < -1.0);
}

#[test]
fn a_landing_is_reported_once() {
    let mut w = World::new(no_damping(), floor());
    let a = w.add_body(hull_body(
        Vec3::new(0.0, 5.0, 0.0),
        vec![cuboid(Vec3::splat(0.5), Vec3::ZERO)],
        1.0,
    ));
    // Not reporting: nothing.
    let quiet = w.add_body(hull_body(
        Vec3::new(5.0, 5.0, 0.0),
        vec![cuboid(Vec3::splat(0.5), Vec3::ZERO)],
        1.0,
    ));
    w.report_collisions(a, true);
    let mut hits = Vec::new();
    for _ in 0..200 {
        let before = w.body(a).unwrap().velocity.length();
        w.step();
        for c in w.take_collisions() {
            hits.push((c, before));
        }
    }
    assert!(!hits.is_empty(), "no landing reported");
    let (first, before) = hits[0];
    assert_eq!(first.body, a);
    assert_eq!(first.other, None);
    assert!(hits.iter().all(|(c, _)| c.body == a && c.body != quiet));
    // At the falling speed (4.5 units, gravity 20 by default config).
    assert!(
        (first.speed - before).abs() < 0.2 * before,
        "{} vs {}",
        first.speed,
        before
    );
    assert!(first.time > 0.0);
    // Resting afterwards: no stream of reports (a bounce or two at most).
    assert!(hits.len() <= 3, "{} reports", hits.len());
}
