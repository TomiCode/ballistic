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
        shape: Shape::Sphere { radius: 2.0 },
        mass: 1.9,
        friction: 0.8,
        elasticity: 0.2,
        linear_damping: 0.0,
        rotational_damping: 0.0,
        group: "Ball".into(),
        ..BodyDesc::default()
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
    let r = -normal * body.radius();
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

fn crate_box(position: Vec3) -> BodyDesc {
    BodyDesc {
        position,
        shape: Shape::Box {
            half: Vec3::splat(2.5),
        },
        mass: 1.0,
        friction: 0.7,
        elasticity: 0.3,
        linear_damping: 0.1,
        rotational_damping: 0.1,
        ..BodyDesc::default()
    }
}

#[test]
fn box_rests_flat_on_floor() {
    let mut w = World::new(WorldConfig::default(), plane(50.0, 0.0, "Floor"));
    let b = w.add_body(crate_box(Vec3::new(0.0, 3.0, 0.0)));
    for _ in 0..660 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        (body.position.y - 2.5).abs() < 0.05,
        "y = {}",
        body.position.y
    );
    assert!(body.velocity.length() < 0.05, "v = {}", body.velocity);
    assert!(
        body.angular_velocity.length() < 0.05,
        "w = {}",
        body.angular_velocity
    );
    assert!(
        Vec3::new(body.position.x, 0.0, body.position.z).length() < 0.05,
        "drifted to {}",
        body.position
    );
    // Still upright.
    assert!((body.orientation * Vec3::Y).y > 0.999);
}

#[test]
fn box_tips_onto_a_face() {
    let mut w = World::new(WorldConfig::default(), plane(50.0, 0.0, "Floor"));
    // Balanced on an edge, slightly off: it falls onto a face.
    let rot = Quat::from_rotation_z(40f32.to_radians());
    let b = w.add_body(BodyDesc {
        orientation: rot,
        ..crate_box(Vec3::new(0.0, 3.6, 0.0))
    });
    for _ in 0..1320 {
        w.step();
    }
    let body = w.body(b).unwrap();
    assert!(
        (body.position.y - 2.5).abs() < 0.1,
        "y = {}",
        body.position.y
    );
    let up = body.orientation * Vec3::Y;
    let side = body.orientation * Vec3::X;
    assert!(
        up.y.abs() > 0.99 || side.y.abs() > 0.99,
        "not on a face: up {up}"
    );
    assert!(body.velocity.length() < 0.1, "v = {}", body.velocity);
}

#[test]
fn ball_pushes_a_box() {
    let cfg = WorldConfig::default();
    let mut w = World::new(cfg, plane(200.0, 0.0, "Floor"));
    let bx = w.add_body(crate_box(Vec3::new(0.0, 2.5, 0.0)));
    let ball_id = w.add_body(ball(2.0));
    w.set_pose(ball_id, Vec3::new(-8.0, 2.0, 0.0), Quat::IDENTITY);
    let p = w.add_push(ball_id, Vec3::X, 0.43);
    for _ in 0..264 {
        w.step();
    }
    w.remove_push(p);
    let b = w.body(bx).unwrap();
    assert!(b.position.x > 1.0, "box didn't move: {}", b.position);
    assert!(
        (b.position.y - 2.5).abs() < 0.2,
        "box left the floor: {}",
        b.position
    );
    let ball_x = w.body(ball_id).unwrap().position.x;
    assert!(ball_x < b.position.x - 4.0, "ball passed through: {ball_x}");
}

#[test]
fn spheres_exchange_momentum() {
    let cfg = WorldConfig {
        gravity: Vec3::ZERO,
        ..WorldConfig::default()
    };
    let mut w = World::new(cfg, StaticMesh::default());
    let a = w.add_body(ball(0.0));
    // A loose ball (no collision group), as placed props are.
    let b = w.add_body(BodyDesc {
        position: Vec3::new(10.0, 0.0, 0.0),
        group: String::new(),
        ..ball(0.0)
    });
    w.body_mut(a).unwrap().velocity = Vec3::new(10.0, 0.0, 0.0);
    let before = w.body(a).unwrap().velocity * 1.9;
    for _ in 0..66 {
        w.step();
    }
    let (va, vb) = (w.body(a).unwrap().velocity, w.body(b).unwrap().velocity);
    assert!(vb.x > va.x, "no exchange: {va} {vb}");
    let after = (va + vb) * 1.9;
    assert!(
        after.abs_diff_eq(before, 1e-3),
        "momentum {before} -> {after}"
    );
}

#[test]
fn stacked_boxes_stay_stacked() {
    let mut w = World::new(WorldConfig::default(), plane(50.0, 0.0, "Floor"));
    let lower = w.add_body(crate_box(Vec3::new(0.0, 2.5, 0.0)));
    let upper = w.add_body(crate_box(Vec3::new(0.5, 7.6, 0.0)));
    for _ in 0..660 {
        w.step();
    }
    let (l, u) = (w.body(lower).unwrap(), w.body(upper).unwrap());
    assert!((u.position.y - 7.5).abs() < 0.15, "upper at {}", u.position);
    assert!((l.position.y - 2.5).abs() < 0.1, "lower at {}", l.position);
    assert!(
        u.velocity.length() < 0.1,
        "upper still moving {}",
        u.velocity
    );
}

#[test]
fn switched_off_material_doesnt_collide() {
    let mut floor = plane(50.0, 0.0, "Floor");
    floor.set_material_enabled(0, false);
    let mut w = World::new(WorldConfig::default(), floor);
    let b = w.add_body(ball(2.5));
    for _ in 0..66 {
        w.step();
    }
    assert!(
        w.body(b).unwrap().position.y < 0.0,
        "landed on a disabled floor"
    );
    w.statics.set_material_enabled(0, true);
    w.set_pose(b, Vec3::new(0.0, 2.5, 0.0), Quat::IDENTITY);
    for _ in 0..132 {
        w.step();
    }
    assert!((w.body(b).unwrap().position.y - 2.0).abs() < 0.05);
}
