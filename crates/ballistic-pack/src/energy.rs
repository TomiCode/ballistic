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
use crate::transformers::{float, input, text};

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
