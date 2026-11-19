//! Behaviour tests of the physics model on synthetic geometry.

use ballistic_physics::*;

fn plane(size: f32, tilt_deg: f32, group: &str) -> StaticMesh {
    let mut m = StaticMesh::default();
    let mat = m.add_material(SurfaceMaterial {
        friction: 0.7,
        elasticity: 0.3,
        group: group.into(),
    });
    let rot = Quat::from_rotation_x(tilt_deg.to_radians());
    let p = |x: f32, z: f32| rot * Vec3::new(x, 0.0, z);
    // Counter-clockwise seen from above: normal +Y.
    m.add_triangle(p(-size, -size), p(-size, size), p(size, size), mat);
    m.add_triangle(p(-size, -size), p(size, size), p(size, -size), mat);
    m.finish();
    m
}

fn ball(y: f32) -> BodyDesc {
    BodyDesc {
        position: Vec3::new(0.0, y, 0.0),
        radius: 2.0,
        mass: 1.9,
        friction: 0.8,
        elasticity: 0.2,
        linear_damping: 0.0,
        rotational_damping: 0.0,
        group: "Ball".into(),
    }
}

#[test]
fn free_fall_matches_gravity() {
    let mut w = World::new(WorldConfig::default(), StaticMesh::default());
    let b = w.add_body(ball(100.0));
    for _ in 0..66 {
        w.step();
    }
    // One simulated second: v = g·t.
    let v = w.body(b).unwrap().velocity.y;
    assert!((v + 20.0).abs() < 1e-3, "v = {v}");
}

#[test]
fn push_is_an_impulse_per_step() {
    let cfg = WorldConfig {
        gravity: Vec3::ZERO,
        ..WorldConfig::default()
    };
    let mut w = World::new(cfg, StaticMesh::default());
    let b = w.add_body(ball(0.0));
    let p = w.add_push(b, Vec3::new(3.0, 0.0, 0.0), 0.43);
    for _ in 0..66 {
        w.step();
    }
    // 66 steps × 0.43 / 1.9 = 14.94 units/s.
    let v = w.body(b).unwrap().velocity.x;
    assert!((v - 66.0 * 0.43 / 1.9).abs() < 1e-3, "v = {v}");
    w.remove_push(p);
    w.step();
    assert!((w.body(b).unwrap().velocity.x - v).abs() < 1e-6);
}

#[test]
fn time_factor_scales_steps() {
    let mut w = World::new(WorldConfig::default(), StaticMesh::default());
    let mut steps = 0;
    for _ in 0..60 {
        steps += w.advance(1.0 / 60.0);
    }
    // One real second at factor 2 = 132 steps (±1 for accumulator rounding).
    assert!((131..=132).contains(&steps), "{steps} steps");
}

#[test]
fn ball_rests_on_floor() {
    let mut w = World::new(WorldConfig::default(), plane(50.0, 0.0, "Floor"));
    let b = w.add_body(ball(2.5));
    for _ in 0..660 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        (body.position.y - 2.0).abs() < 0.05,
        "y = {}",
        body.position.y
    );
    assert!(body.velocity.length() < 0.05, "v = {}", body.velocity);
    assert!(body.grounded);
}

#[test]
fn ball_rolls_down_slope_without_slipping() {
    let mut w = World::new(WorldConfig::default(), plane(200.0, 15.0, "Floor"));
    // Start just above the tilted plane at the origin.
    let normal = Quat::from_rotation_x(15f32.to_radians()) * Vec3::Y;
    let b = w.add_body(BodyDesc {
        position: normal * 2.01,
        ..ball(0.0)
    });
    for _ in 0..132 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        body.velocity.length() > 5.0,
        "not rolling: {}",
        body.velocity
    );
    // Rolling contact: the contact point is (nearly) at rest.
    let r = -normal * body.radius;
    let slip = body.velocity + body.angular_velocity.cross(r);
    assert!(
        slip.length() < 0.2 * body.velocity.length(),
        "slip {slip} at v {}",
        body.velocity
    );
    // Solid sphere on a slope: a = 5/7 g sin θ.
    let expected = 5.0 / 7.0 * 20.0 * 15f32.to_radians().sin() * 2.0;
    let speed = body.velocity.length();
    assert!(
        (speed - expected).abs() < 0.1 * expected,
        "speed {speed}, expected {expected}"
    );
}

#[test]
fn same_collision_group_is_ignored() {
    let mut w = World::new(WorldConfig::default(), plane(50.0, 0.0, "Ball"));
    let b = w.add_body(ball(2.5));
    for _ in 0..66 {
        w.step();
    }
    assert!(
        w.body(b).unwrap().position.y < 0.0,
        "ball should fall through its own group"
    );
}

#[test]
fn simulation_is_deterministic() {
    let run = || {
        let mut w = World::new(WorldConfig::default(), plane(100.0, 8.0, "Floor"));
        let b = w.add_body(ball(5.0));
        let mut push = None;
        for i in 0..500 {
            if i == 50 {
                push = Some(w.add_push(b, Vec3::new(1.0, 0.0, -1.0), 0.43));
            }
            if i == 300 {
                w.remove_push(push.take().unwrap());
            }
            w.step();
        }
        let body = w.body(b).unwrap().clone();
        (
            body.position.to_array().map(f32::to_bits),
            body.velocity.to_array().map(f32::to_bits),
            body.orientation.to_array().map(f32::to_bits),
        )
    };
    assert_eq!(run(), run());
}
