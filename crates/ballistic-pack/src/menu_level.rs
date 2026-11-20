//! Extracts the scene behind the main menu (see the research repo,
//! findings 28) from `MenuLevel.nmo`'s `MenuLevel_Init`: the camera's curve
//! (*Position On Curve*: the curve's points, decoded here) and its *Bezier
//! Progression*, the camera's target, the sky (`Load Sky-Textures`: an
//! `AllLevel` row), the light's colour, the fog and the ambient sound.

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckfile::{CkFile, ClassId};
use ckscene::Entity3d;
use ckscript::extract::{Extractor, NodeParams};

use crate::install::Install;
use crate::physics::load;
use crate::transformers::{float, input, progression, target, text, vector};

/// A curve's chunk section listing its points.
const SECTION_CURVE_POINTS: u32 = 0xFFC0_0000;

pub fn extract(install: &Install) -> Result<bb::MenuLevel> {
    let m = load(install, "3d entities/menulevel.nmo")?;
    let ex = Extractor::new(&m.file, &m.scripts);
    let all = ex.nodes(&|_| true);
    let init: Vec<&NodeParams> = all
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some("MenuLevel_Init"))
        .collect();
    let find = |bb_name: &str| {
        init.iter()
            .copied()
            .find(|n| n.building_block == bb_name)
            .with_context(|| format!("MenuLevel_Init: {bb_name} not found"))
    };
    let num = |n: &NodeParams, pin: &str| {
        float(input(n, pin)).with_context(|| format!("{}: {pin}", n.building_block))
    };
    let color = |n: &NodeParams, pin: &str| -> Result<[f32; 4]> {
        match vector(input(n, pin)).as_deref() {
            Some(&[r, g, b, a]) => Ok([r, g, b, a]),
            other => bail!("{}: {pin} {other:?}", n.building_block),
        }
    };

    let on_curve = find("Position On Curve")?;
    let camera = target(on_curve).context("Position On Curve: no camera")?;
    let curve = text(input(on_curve, "Curve")).context("Position On Curve: no curve")?;
    let (path, closed) = curve_points(&m.file, &curve)?;
    // The target camera's target, by the original's naming.
    let target_name = format!("{camera}_Target");
    anyhow::ensure!(
        m.file
            .objects
            .iter()
            .any(|o| o.name.as_deref() == Some(target_name.as_str())),
        "{target_name} not found"
    );

    // `Load Sky-Textures` reads `AllLevel`'s row `Pin 0`.
    let sky_row = init
        .iter()
        .find(|n| n.building_block == "Get Cell" && n.path.iter().any(|p| p == "Load Sky-Textures"))
        .and_then(|n| float(input(n, "Row Index")))
        .context("Load Sky-Textures: the sky's row")?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a row index"
    )]
    let sky_level = format!("level/level_{:02}", sky_row as usize + 1);

    let fog = find("Set Fog")?;
    let wave = find("Wave Player")?;
    let atmo = target(wave)
        .context("MenuLevel_Init: the ambient sound")?
        .to_lowercase();
    anyhow::ensure!(
        install.get(&format!("sounds/{atmo}.wav")).is_some(),
        "sound {atmo} not in Sounds/"
    );
    let random = find("Random")?;
    Ok(bb::MenuLevel {
        scene: "menulevel".into(),
        path,
        closed,
        progression: progression(find("Bezier Progression")?)?,
        target: target_name,
        sky_level,
        light_color: color(find("Set Light Color")?, "Color")?,
        fog: [num(fog, "Fog Start")?, num(fog, "Fog End")?],
        fog_color: color(fog, "Fog Color")?,
        atmo,
        atmo_gap_ms: [num(random, "Min")?, num(random, "Max")?],
    })
}

/// A curve's points (world positions, bundle space), and whether it closes:
/// the curve's section `0xFFC00000` holds the point count, the points, then
/// a word that is 0 for the menu's closed loop.
fn curve_points(file: &CkFile, name: &str) -> Result<(Vec<[f32; 3]>, bool)> {
    let curve = file
        .objects
        .iter()
        .find(|o| o.class_id == ClassId::CURVE && o.name.as_deref() == Some(name))
        .with_context(|| format!("curve {name} not found"))?;
    let chunk = curve.chunk.as_ref().context("curve without data")?;
    let mut r = chunk
        .section(SECTION_CURVE_POINTS)?
        .context("curve without points")?;
    let n = r.u32()? as usize;
    let refs = (0..n)
        .map(|_| r.u32())
        .collect::<ckfile::Result<Vec<_>>>()?;
    let closed = r.u32()? == 0;
    let points = refs
        .iter()
        .map(|&i| {
            let o = file
                .objects
                .get(i as usize)
                .filter(|o| o.class_id == ClassId::CURVE_POINT)
                .with_context(|| format!("{name}: point #{i} isn't a curve point"))?;
            let e = Entity3d::decode(o.chunk.as_ref().context("curve point without data")?)?;
            let p = e.world_matrix[3];
            // Left-handed → right-handed: Z flips.
            Ok([p[0], p[1], -p[2]])
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((points, closed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_menu_level() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let m = extract(&install).unwrap();
        assert_eq!(m.path.len(), 4);
        assert!(m.closed);
        // The first point is where the camera is stored.
        let p = m.path[0];
        assert!(
            (p[1] - 40.0).abs() < 0.1 && (p[2] - 95.0).abs() < 1.0,
            "{p:?}"
        );
        assert_eq!(m.target, "Cam_MenuLevel_Target");
        assert!((m.progression.duration_ms - 44000.0).abs() < 1e-3);
        assert_eq!(m.sky_level, "level/level_05");
        assert_eq!(m.fog, [100.0, 800.0]);
        assert_eq!(m.atmo, "menu_atmo");
        assert_eq!(m.atmo_gap_ms, [1000.0, 10000.0]);
    }
}
