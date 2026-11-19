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
