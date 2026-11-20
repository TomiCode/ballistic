//! The player's wooden ball pushing props from rest (no run-up): a box
//! (`P_Box`) and a wooden ball (`P_Ball_Wood`) on a flat floor.

use ballistic_physics::*;

fn floor() -> StaticMesh {
    let mut m = StaticMesh::default();
    let mat = m.add_material(SurfaceMaterial {
        friction: 0.7,
        elasticity: 0.3,
        group: "Floor".into(),
        ..SurfaceMaterial::default()
    });
    let s = 200.0;
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

/// The player's wooden ball (`Physicalize_GameBall`).
fn wood_ball(at: Vec3) -> BodyDesc {
    BodyDesc {
        position: at,
        shape: Shape::Sphere { radius: 2.0 },
        mass: 1.9,
        friction: 0.8,
        elasticity: 0.2,
        linear_damping: 0.9,
        rotational_damping: 0.1,
        group: "Ball".into(),
        ..BodyDesc::default()
    }
}

/// Pushes the ball along +X against `prop` for `seconds` (real time);
/// returns how far the prop moved.
fn push(prop: BodyDesc, seconds: f32) -> f32 {
    let mut w = World::new(WorldConfig::default(), floor());
    let start = prop.position;
    let prop = w.add_body(prop);
    let ball = w.add_body(wood_ball(Vec3::new(start.x - 4.6, 2.0, 0.0)));
    w.add_push(ball, Vec3::X, 0.43);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a few seconds"
    )]
    let frames = (seconds * 60.0) as u32;
    for _ in 0..frames {
        w.advance(1.0 / 60.0);
    }
    let end = w.body(prop).unwrap().position;
    eprintln!(
        "moved {:?} in {seconds} s, ball at {:?}",
        end - start,
        w.body(ball).unwrap().position
    );
    (end - start).x
}

#[test]
fn a_box_moves_under_a_push_from_rest() {
    let moved = push(
        BodyDesc {
            position: Vec3::new(0.0, 2.5, 0.0),
            shape: Shape::Box {
                half: Vec3::splat(2.5),
            },
            mass: 1.0,
            friction: 0.7,
            elasticity: 0.3,
            linear_damping: 0.1,
            rotational_damping: 0.1,
            asleep: true,
            ..BodyDesc::default()
        },
        2.0,
    );
    assert!(moved > 1.0, "the box moved {moved}");
}

#[test]
fn a_wooden_ball_moves_under_a_push_from_rest() {
    let moved = push(
        BodyDesc {
            position: Vec3::new(0.0, 2.0, 0.0),
            shape: Shape::Sphere { radius: 2.0 },
            mass: 2.0,
            friction: 0.6,
            elasticity: 0.2,
            linear_damping: 0.6,
            rotational_damping: 0.1,
            asleep: true,
            ..BodyDesc::default()
        },
        2.0,
    );
    assert!(moved > 1.0, "the ball moved {moved}");
}
