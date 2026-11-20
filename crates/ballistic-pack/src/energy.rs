//! Extracts points, lives and score (see the research repo, findings 16):
//! - the `Energy` table (`Gameplay.nmo`): start points and lives, the
//!   countdown step, the life bonus;
//! - `AllLevel` (`Levelinit.nmo`): each level's bonus;
//! - the pickups' scripts: `P_Extra_Life` (its pick-up proximity test) and
//!   `P_Extra_Point` (its *TT Extra* and the points per hit).

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckscript::extract::{Extractor, NodeParams};

use crate::install::Install;
use crate::physics::load;
use crate::transformers::{float, input, progression, target, text, vector, writer_of};

pub fn extract(install: &Install) -> Result<bb::EnergyParams> {
    let g = load(install, "3d entities/gameplay.nmo")?;
    let table = g.array("Energy")?;
    let cells = table.rows.first().context("Energy: no row")?;
    let row = g.row(table, cells);
    let base = load(install, "3d entities/levelinit.nmo")?;
    let levels = base.array("AllLevel")?;
    let level_bonus = levels
        .rows
        .iter()
        .map(|r| base.row(levels, r).f32("LevelBonus").map(|v| v as i32))
        .collect::<Result<_>>()?;
    Ok(bb::EnergyParams {
        start_points: row.f32("StartPoints")? as i32,
        start_lives: row.f32("StartLifes")? as i32,
        tick_ms: row.f32("Timefactor")?,
        life_bonus: row.f32("LifeBonus")? as i32,
        level_bonus,
        extra_life: extra_life(install)?,
        extra_point: extra_point(install)?,
    })
}

fn nodes(install: &Install, key: &str) -> Result<Vec<NodeParams>> {
    let g = load(install, key)?;
    let ex = Extractor::new(&g.file, &g.scripts);
    Ok(ex.nodes(&|_| true))
}

/// `P_Extra_Life_MF Script`: the smallest of its proximity tests picks it up.
fn extra_life(install: &Install) -> Result<bb::ExtraLife> {
    let nodes = nodes(install, "3d entities/ph/p_extra_life.nmo")?;
    let reach = nodes
        .iter()
        .filter(|n| n.building_block == "TT Scaleable Proximity")
        .filter_map(|n| crate::modules::proximity(n, Vec::new()))
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .context("P_Extra_Life: proximity test not found")?;
    Ok(bb::ExtraLife {
        group: "P_Extra_Life".into(),
        reach,
        look: extra_life_look(install)?,
    })
}

/// `TT_TextureSine`'s phase spread over the vertices (a constant in
/// `TT_Gravity_RT.dll`, `0x10010274`).
const UV_WAVE_SPREAD: f32 = 4.0;

/// `P_Extra_Life_MF Script` and `P_Extra_Life Anim Script`: which objects
/// show, and the animation, followed from its targets back through the
/// interpolators to their progressions.
fn extra_life_look(install: &Install) -> Result<bb::ExtraLifeLook> {
    let g = load(install, "3d entities/ph/p_extra_life.nmo")?;
    let nodes = Extractor::new(&g.file, &g.scripts).nodes(&|_| true);
    let anim = |bb_name: &str| {
        nodes
            .iter()
            .find(|n| {
                n.building_block == bb_name
                    && n.path.iter().any(|p| p == "P_Extra_Life Anim Script")
            })
            .with_context(|| format!("P_Extra_Life Anim Script: {bb_name} not found"))
    };
    let node = |id| nodes.iter().find(|n| n.node == id);
    // The interpolator feeding `pin` of `n`, and the progression feeding it.
    let driven = |n: &NodeParams, pin: &str| -> Result<(&NodeParams, &NodeParams)> {
        let interp = writer_of(&g.file, &g.scripts, n.node, pin)
            .and_then(node)
            .with_context(|| format!("{}: {pin} isn't interpolated", n.building_block))?;
        let prog = writer_of(&g.file, &g.scripts, interp.node, "Value")
            .and_then(node)
            .with_context(|| format!("{}: no progression", n.building_block))?;
        Ok((interp, prog))
    };
    let vec3 = |n: &NodeParams, pin: &str| -> Result<[f32; 3]> {
        vector(input(n, pin))
            .and_then(|v| <[f32; 3]>::try_from(v).ok())
            .with_context(|| format!("{}: {pin}", n.building_block))
    };
    let color = |n: &NodeParams, pin: &str| -> Result<[f32; 4]> {
        vector(input(n, pin))
            .and_then(|v| <[f32; 4]>::try_from(v).ok())
            .with_context(|| format!("{}: {pin}", n.building_block))
    };
    let mirror = |v: [f32; 3]| [v[0], v[1], -v[2]];

    let set_position = anim("Set Position")?;
    let (bounce_i, bounce_p) = driven(set_position, "Position")?;
    let scale = anim("Scale")?;
    let (squash_i, squash_p) = driven(scale, "Scaling Vector")?;
    let emissive = anim("Set Emissive")?;
    let (shadow_i, _) = driven(emissive, "Emissive Color")?;
    let sphere = target(scale).context("Scale: target")?;
    let shadow = target(anim("Show")?).context("Show: target")?;
    let sine = anim("TT_TextureSine")?;

    let main = |bb_name: &str| {
        nodes
            .iter()
            .filter(|n| {
                n.building_block == bb_name && n.path.iter().any(|p| p == "P_Extra_Life_MF Script")
            })
            .collect::<Vec<_>>()
    };
    let ball = main("Show")
        .into_iter()
        .filter_map(target)
        .find(|t| *t != sphere && *t != shadow)
        .context("P_Extra_Life_MF Script: the ball sprite")?;
    let show_distance = main("TT Scaleable Proximity")
        .into_iter()
        .filter_map(|n| float(input(n, "Distance")))
        .fold(0.0_f32, f32::max);
    let ray = main("Ray Intersection")
        .into_iter()
        .next()
        .context("P_Extra_Life_MF Script: Ray Intersection")?;
    let floor_group = writer_of(&g.file, &g.scripts, ray.node, "Filter")
        .and_then(node)
        .and_then(|op| text(input(op, "p1")))
        .context("Ray Intersection: the floor group")?;

    Ok(bb::ExtraLifeLook {
        show_distance,
        sphere,
        ball,
        shadow,
        shadow_material: target(emissive).context("Set Emissive: target")?,
        bounce_from: mirror(vec3(bounce_i, "A")?),
        bounce_to: mirror(vec3(bounce_i, "B")?),
        bounce: progression(bounce_p)?,
        shadow_from: color(shadow_i, "A")?,
        shadow_to: color(shadow_i, "B")?,
        squash_from: vec3(squash_i, "A")?,
        squash_to: vec3(squash_i, "B")?,
        squash: progression(squash_p)?,
        floor_ray_origin: mirror(vec3(ray, "Ray Origin")?),
        floor_ray_depth: float(input(ray, "Depth")).context("Ray Intersection: Depth")?,
        floor_group,
        uv_wave: bb::UvWave {
            amplitude: [
                float(input(sine, "X Amplitude")).context("TT_TextureSine: X Amplitude")?,
                float(input(sine, "Y Amplitude")).context("TT_TextureSine: Y Amplitude")?,
            ],
            velocity: float(input(sine, "Velocity")).context("TT_TextureSine: Velocity")?,
            spread: UV_WAVE_SPREAD,
        },
    })
}

/// `P_Extra_Point_MF Script`: *TT Extra*'s inputs, and the two values its
/// Parameter Selector multiplies by the hits (activation, then each small
/// ball).
fn extra_point(install: &Install) -> Result<bb::ExtraPoint> {
    let nodes = nodes(install, "3d entities/ph/p_extra_point.nmo")?;
    let extra = nodes
        .iter()
        .find(|n| n.building_block == "TT Extra")
        .context("P_Extra_Point: TT Extra not found")?;
    let f = |pin: &str| float(input(extra, pin)).with_context(|| format!("TT Extra: {pin}"));
    let points = nodes
        .iter()
        .filter(|n| n.building_block == "Parameter Selector")
        .find_map(|n| {
            let a = float(input(n, "pIn 0"))?;
            let b = float(input(n, "pIn 1"))?;
            Some([a as i32, b as i32])
        })
        .context("P_Extra_Point: points selector not found")?;
    let hit_frames = text(input(extra, "Hitframegroup")).unwrap_or_default();
    Ok(bb::ExtraPoint {
        group: "P_Extra_Point".into(),
        small_balls: u32::try_from(f("Number of Smallballs")?.round() as i64)
            .context("TT Extra: Number of Smallballs")?,
        activation_distance: f("Activationdistance")?,
        hit_distance: f("Extra_Points CollDistance")?,
        away_force: f("Awayforce")?,
        away_damping: f("Awaydamping")?,
        away_ms: f("Flyawaytime")?,
        force: f("Force")?,
        damping: f("Damping")?,
        force_width: f("Forcewidth")?,
        activation_points: points[0],
        hit_points: points[1],
        hit_frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_extra_life_look() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let look = extra_life_look(&install).unwrap();
        assert_eq!(look.sphere, "P_Extra_Life_Sphere");
        assert_eq!(look.ball, "P_Extra_Life_SilverBall");
        assert_eq!(look.shadow, "P_Extra_Life_Shadow");
        assert_eq!(look.shadow_material, "P_Extra_Life_Schatten");
        assert!((look.show_distance - 60.0).abs() < 1e-4);
        assert_eq!(look.bounce_to[1], 1.2);
        assert_eq!(look.squash_from, [1.0, 1.2, 1.0]);
        assert!((look.bounce.duration_ms - 2000.0).abs() < 1e-3);
        assert_eq!(look.floor_group, "Phys_Floors");
        assert!((look.uv_wave.amplitude[0] - 0.2).abs() < 1e-6);
    }
}
