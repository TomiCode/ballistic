//! Extracts the play environment (see the research repo, findings 19):
//! - `base.cmo`: the background colour (*Set Background Color*), the scene's
//!   ambient light (the `Level` object's scene), the cloud-layer option;
//! - `Gameplay.nmo`'s `Gameplay_Sky`: *TT Sky*'s settings and which table
//!   column feeds each face, and the light it colours (`Light_Ingame`, whose
//!   placement gives the direction);
//! - `Levelinit.nmo`: `AllLevel`'s sky, light colour and cloud scroll per
//!   level, and `init SkyLayer`'s cloud-layer material settings.

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckfile::{ClassId, StateChunk};
use ckscene::{Entity3d, Light};
use ckscript::extract::{Extractor, NamedValue, NodeParams, Resolved, Value};

use crate::install::Install;
use crate::physics::load;
use crate::transformers::{float, input, target, text, vector};

/// The `Level` scene's settings section: background, ambient, fog.
const SECTION_SCENE_SETTINGS: u32 = 0x8_0000;

pub fn extract(install: &Install) -> Result<bb::Environment> {
    let base = load(install, "base.cmo")?;
    let base_nodes = Extractor::new(&base.file, &base.scripts).nodes(&|_| true);
    let background = base_nodes
        .iter()
        .find(|n| n.building_block == "Set Background Color")
        .and_then(|n| vector(input(n, "Background Color")))
        .and_then(|v| <[f32; 4]>::try_from(v).ok())
        .context("base.cmo: Set Background Color")?;
    let ambient = scene_ambient(&base.file)?;
    let options = base.array("DB_Options")?;
    let enabled = options
        .rows
        .first()
        .map(|r| base.row(options, r).f32("CloudLayer?"))
        .transpose()?
        .is_some_and(|v| v != 0.0);

    let g = load(install, "3d entities/gameplay.nmo")?;
    let nodes = Extractor::new(&g.file, &g.scripts).nodes(&|_| true);
    let in_sky = |bb_name: &str| {
        nodes
            .iter()
            .find(|n| n.building_block == bb_name && n.path.iter().any(|p| p == "Gameplay_Sky"))
            .with_context(|| format!("Gameplay_Sky: {bb_name} not found"))
    };
    let light_name = target(in_sky("Set Light Color")?).context("Set Light Color: target")?;
    let light_direction = light_direction(&g.file, &light_name)?;
    let sky = sky_params(in_sky("TT Sky")?)?;

    let li = load(install, "3d entities/levelinit.nmo")?;
    let table = li.array("AllLevel")?;
    let levels = table
        .rows
        .iter()
        .map(|r| {
            let row = li.row(table, r);
            let file = row.string("Levelfile")?.to_lowercase();
            let stem = file.strip_suffix(".nmo").unwrap_or(&file).to_owned();
            let sky_key = format!("sky/{}", row.string("Sky")?.to_lowercase());
            for face in sky.sides.iter().chain(&sky.top).chain(&sky.bottom) {
                anyhow::ensure!(
                    install
                        .get(&format!("textures/{sky_key}{face}.bmp"))
                        .is_some(),
                    "{stem}: sky texture {sky_key}{face} missing"
                );
            }
            let light = row.floats("Light")?;
            let scroll = row.floats("Skytranslation")?;
            Ok(bb::LevelEnvironment {
                level: format!("level/{stem}"),
                sky: sky_key,
                light: <[f32; 4]>::try_from(light)
                    .ok()
                    .context("Light: not a colour")?,
                cloud_scroll: <[f32; 2]>::try_from(scroll)
                    .ok()
                    .context("Skytranslation: not a 2D vector")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let li_nodes = Extractor::new(&li.file, &li.scripts).nodes(&|_| true);
    let in_layer = |bb_name: &str| {
        li_nodes
            .iter()
            .find(|n| n.building_block == bb_name && n.path.iter().any(|p| p == "init SkyLayer"))
            .with_context(|| format!("init SkyLayer: {bb_name} not found"))
    };
    let object = li_nodes
        .iter()
        .filter(|n| n.building_block == "Op" && n.path.iter().any(|p| p == "init SkyLayer"))
        .find_map(|n| text(input(n, "p1")))
        .context("init SkyLayer: the object's name")?;
    let blend = in_layer("Set Blend Modes")?;
    let one = |pin| float(input(blend, pin)).is_some_and(|v| (v - 2.0).abs() < 0.5);
    let cloud_layer = bb::CloudLayer {
        object,
        color: vector(input(in_layer("Set Prelit Color")?, "Filtering Color"))
            .and_then(|v| <[f32; 4]>::try_from(v).ok())
            .context("Set Prelit Color: Filtering Color")?,
        additive: boolean(input(blend, "Blend")).unwrap_or(false)
            && one("Source Blend")
            && one("Destination Blend"),
        z_write: boolean(input(in_layer("Set Material Z Buffer")?, "Z Buffer Write"))
            .context("Set Material Z Buffer: Z Buffer Write")?,
        enabled,
    };

    Ok(bb::Environment {
        background,
        ambient,
        light_direction,
        sky,
        cloud_layer,
        levels,
    })
}

fn boolean(v: Option<&NamedValue>) -> Option<bool> {
    match &v?.value {
        Resolved::Constant {
            value: Value::Bool(b),
        } => Some(*b),
        Resolved::Constant {
            value: Value::Int(i),
        } => Some(*i != 0),
        _ => None,
    }
}

/// The ambient colour of the `Level` object's scene (`base.cmo`).
fn scene_ambient(file: &ckfile::CkFile) -> Result<[f32; 3]> {
    let level = file
        .objects
        .iter()
        .find(|o| o.class_id == ClassId::LEVEL)
        .and_then(|o| o.chunk.as_ref())
        .context("base.cmo: no Level object")?;
    for scene in level.sub_chunks()? {
        if let Some(mut r) = scene.section(SECTION_SCENE_SETTINGS)? {
            let _background = r.u32()?;
            let [red, green, blue, _] = ckscene::argb_to_rgba(r.u32()?);
            return Ok([red, green, blue]);
        }
    }
    bail!("base.cmo: the level's scene has no settings")
}

/// The direction the light `name` shines: its Z axis, in bundle space.
fn light_direction(file: &ckfile::CkFile, name: &str) -> Result<[f32; 3]> {
    let chunk: &StateChunk = file
        .objects
        .iter()
        .find(|o| {
            o.name.as_deref() == Some(name)
                && matches!(o.class_id, ClassId::LIGHT | ClassId::TARGET_LIGHT)
        })
        .and_then(|o| o.chunk.as_ref())
        .with_context(|| format!("light {name} not found"))?;
    let light = Light::decode(chunk)?;
    anyhow::ensure!(
        light.kind == Light::DIRECTIONAL,
        "{name}: not a directional light"
    );
    let m = Entity3d::decode(chunk)?.world_matrix;
    // Row vectors: row 2 is the Z axis. The bundle mirrors Z.
    let d = [m[2][0], m[2][1], -m[2][2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    anyhow::ensure!(len > 0.0, "{name}: no orientation");
    Ok(d.map(|v| v / len))
}

/// *TT Sky*'s inputs, and the table column feeding each material input
/// (`Back`, `Right`, … become texture suffixes `_back`, `_right`, …).
fn sky_params(n: &NodeParams) -> Result<bb::SkyParams> {
    let num = |pin: &str| float(input(n, pin)).with_context(|| format!("TT Sky: {pin}"));
    let square = boolean(input(n, "Quadratic SideFaces?")).unwrap_or(true);
    let suffix = |pin: &NamedValue| match &pin.value {
        Resolved::Dynamic { from, .. } => from.rsplit(['.', ' ']).next().map(|c| {
            format!(
                "_{}",
                c.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            )
        }),
        _ => None,
    };
    let mut sides = Vec::new();
    let (mut top, mut bottom) = (None, None);
    for i in &n.inputs {
        if i.name.ends_with("Side-Mat") {
            sides.push(suffix(i).with_context(|| format!("TT Sky: {} {:?}", i.name, i.value))?);
        } else if i.name == "Top Mat" {
            top = suffix(i);
        } else if i.name == "Bottom Mat" {
            bottom = suffix(i);
        }
    }
    anyhow::ensure!(sides.len() >= 3, "TT Sky: {} side faces", sides.len());
    Ok(bb::SkyParams {
        radius: num("Radius")?,
        distortion: num("Distortion")?,
        height: if square {
            None
        } else {
            Some(num("or SideFace-Heigth")?)
        },
        y: num("Y-Position of Sky")?,
        color: vector(input(n, "Vertex Color"))
            .and_then(|v| <[f32; 4]>::try_from(v).ok())
            .context("TT Sky: Vertex Color")?,
        sides,
        top,
        bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_environment() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let e = extract(&install).unwrap();
        assert_eq!(e.background, [0.0, 0.0, 0.0, 1.0]);
        assert!(
            (e.ambient[0] - 15.0 / 255.0).abs() < 1e-6,
            "{:?}",
            e.ambient
        );
        // The light shines downwards.
        assert!(e.light_direction[1] < -0.5, "{:?}", e.light_direction);
        assert_eq!(e.sky.sides, ["_back", "_right", "_front", "_left"]);
        assert_eq!(e.sky.bottom.as_deref(), Some("_down"));
        assert_eq!(e.sky.top, None);
        assert!((e.sky.radius - 100.0).abs() < 1e-4);
        assert!((e.sky.distortion - 0.15).abs() < 1e-6);
        assert_eq!(e.levels.len(), 12);
        assert_eq!(e.levels[0].level, "level/level_01");
        assert_eq!(e.levels[0].sky, "sky/sky_l");
        assert!((e.levels[11].light[0] - 0.5882).abs() < 1e-3);
        assert_eq!(e.cloud_layer.object, "SkyLayer");
        assert!(e.cloud_layer.additive && !e.cloud_layer.z_write && e.cloud_layer.enabled);
    }
}
