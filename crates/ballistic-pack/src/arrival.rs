//! Extracts the ball's arrival and the screen's fade (see the research repo,
//! findings 30):
//! - `Balls.nmo`'s `Ball_LightningSphere`: the sphere it shows, the
//!   textures its *Parameter Selector* cycles through, the turn (*Per
//!   Second*) and how long (*Timer*), the growth (*Bezier Progression* in
//!   `Scale Lighting Sphere`), and the Delayer before it starts
//!   `BallParticle_Frame script`, whose particle system's entity and *Exit
//!   On* → *Off* link delay make the smoke puff;
//! - `Gameplay.nmo`'s *Überblendung*: the *Interpolator*'s colours and the
//!   *Bezier Progression* of its *Planar Filter*.

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckscript::ScriptObject;
use ckscript::extract::{Extractor, NodeParams};

use crate::hud::texture_key;
use crate::install::Install;
use crate::physics::{Loaded, load};
use crate::transformers::{float, input, progression, target, text, vector};

pub fn arrival(install: &Install) -> Result<bb::BallArrival> {
    let b = load(install, "3d entities/balls.nmo")?;
    let ex = Extractor::new(&b.file, &b.scripts);
    let all = ex.nodes(&|_| true);
    let script = |name: &str| -> Vec<&NodeParams> {
        all.iter()
            .filter(|n| n.path.first().map(String::as_str) == Some(name))
            .collect()
    };
    let sphere = script("Ball_LightningSphere");
    let find = |bb_name: &str, graph: Option<&str>| {
        sphere
            .iter()
            .copied()
            .find(|n| {
                n.building_block == bb_name
                    && graph.map_or(n.path.len() == 1, |g| {
                        n.path.last().map(String::as_str) == Some(g)
                    })
            })
            .with_context(|| format!("Ball_LightningSphere {graph:?}: {bb_name} not found"))
    };
    let num = |n: &NodeParams, pin: &str| {
        float(input(n, pin)).with_context(|| format!("{}: {pin}", n.building_block))
    };
    let object = target(find("Show", None)?).context("Ball_LightningSphere: Show's target")?;
    let textures = find("Parameter Selector", Some("Rotate Lighting Sphere"))?
        .inputs
        .iter()
        .filter_map(|i| text(Some(i)))
        .map(|name| texture_key(install, &b.file, &name))
        .collect::<Result<Vec<_>>>()?;
    if textures.is_empty() {
        bail!("Ball_LightningSphere: no textures");
    }
    let rotate = find("Rotate", Some("Rotate Lighting Sphere"))?;
    if vector(input(rotate, "Axis of Rotation")).as_deref() != Some(&[0.0, 1.0, 0.0]) {
        bail!("Ball_LightningSphere: not turning about Y");
    }

    // The smoke puff: the particle system of the script the Delayer starts.
    let puff = script("BallParticle_Frame script");
    let system = puff
        .iter()
        .copied()
        .find(|n| n.building_block.ends_with("ParticleSystem"))
        .context("BallParticle_Frame script: no particle system")?;
    let puff_object = system
        .owner
        .clone()
        .context("BallParticle_Frame script: no owner")?;

    Ok(bb::BallArrival {
        sphere: object,
        textures,
        spin: num(find("Per Second", Some("Rotate Lighting Sphere"))?, "X")?,
        shown_ms: num(find("Timer", Some("Rotate Lighting Sphere"))?, "Duration")?,
        grow: progression(find("Bezier Progression", Some("Scale Lighting Sphere"))?)?,
        puff_object,
        puff_ms: num(find("Delayer", None)?, "Time to Wait")?,
        puff_frames: u32::from(
            link_delay(&b, system, "Exit On", "Off")
                .context("BallParticle_Frame script: Exit On → Off")?,
        ),
    })
}

/// The delay of the link from `node`'s output `from` to its own input `to`.
fn link_delay(g: &Loaded, node: &NodeParams, from: &str, to: &str) -> Option<u16> {
    let s = &g.scripts;
    let b = s.behavior(node.node)?;
    let named = |list: &[usize], name: &str| {
        list.iter()
            .copied()
            .find(|&o| g.file.objects.get(o).and_then(|o| o.name.as_deref()) == Some(name))
    };
    let (out, inp) = (named(&b.outputs, from)?, named(&b.inputs, to)?);
    let parent = *s.owner.get(&node.node)?;
    s.behavior(parent)?
        .links
        .iter()
        .find_map(|&l| match s.objects.get(l) {
            Some(Some(ScriptObject::Link(k))) if k.source == Some(out) && k.target == Some(inp) => {
                Some(k.delay)
            }
            _ => None,
        })
}

/// *Überblendung* in `Gameplay.nmo`'s `BallManager`.
pub fn fade(nodes: &[NodeParams]) -> Result<bb::ScreenFade> {
    let find = |bb_name: &str| {
        nodes
            .iter()
            .find(|n| {
                n.building_block == bb_name
                    && n.path.last().map(String::as_str) == Some("Überblendung")
            })
            .with_context(|| format!("Überblendung: {bb_name} not found"))
    };
    let color = |pin: &str| -> Result<[f32; 4]> {
        match vector(input(find("Interpolator")?, pin)).as_deref() {
            Some(&[r, g, b, a]) => Ok([r, g, b, a]),
            other => bail!("Überblendung: {pin} {other:?}"),
        }
    };
    find("Planar Filter")?;
    Ok(bb::ScreenFade {
        from: color("A")?,
        to: color("B")?,
        progression: progression(find("Bezier Progression")?)?,
    })
}

/// The ball's shadow: `Balls.nmo`'s `Ball_Shadow` (*TT Simple Shadow*: its
/// texture, size scale and maximum height) and the group `Levelinit.nmo`'s
/// *set Floor* gives the floor attribute (the shadow's receivers).
pub fn shadow(install: &Install) -> Result<bb::BallShadow> {
    let b = load(install, "3d entities/balls.nmo")?;
    let nodes = Extractor::new(&b.file, &b.scripts).nodes(&|_| true);
    let n = nodes
        .iter()
        .find(|n| {
            n.building_block == "TT Simple Shadow"
                && n.path.first().map(String::as_str) == Some("Ball_Shadow")
        })
        .context("Ball_Shadow: TT Simple Shadow not found")?;
    let num = |pin: &str| float(input(n, pin)).with_context(|| format!("TT Simple Shadow: {pin}"));
    let texture = text(input(n, "Texture")).context("TT Simple Shadow: Texture")?;
    let li = load(install, "3d entities/levelinit.nmo")?;
    let group = Extractor::new(&li.file, &li.scripts)
        .nodes(&|_| true)
        .iter()
        .filter(|n| n.path.last().map(String::as_str) == Some("set Floor"))
        .find_map(|n| text(input(n, "p1")))
        .context("Levelinit: set Floor's group")?;
    Ok(bb::BallShadow {
        texture: texture_key(install, &b.file, &texture)?,
        size_scale: num("Size Scale")?,
        max_height: num("Maximum Height")?,
        group,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_arrival_and_the_fade() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let a = arrival(&install).unwrap();
        assert_eq!(a.sphere, "Ball_LightningSphere");
        assert_eq!(a.textures.len(), 3);
        assert!((a.spin - std::f32::consts::TAU).abs() < 1e-3);
        assert_eq!((a.shown_ms, a.puff_ms, a.puff_frames), (3000.0, 2500.0, 6));
        assert_eq!(a.grow.duration_ms, 1500.0);
        assert_eq!(a.puff_object, "Ball_Particle_Frame");
        let s = shadow(&install).unwrap();
        assert_eq!(
            (
                s.texture.as_str(),
                s.size_scale,
                s.max_height,
                s.group.as_str()
            ),
            ("hardshadow", 1.3, 20.0, "Shadow")
        );
        let g = crate::physics::load(&install, "3d entities/gameplay.nmo").unwrap();
        let nodes = Extractor::new(&g.file, &g.scripts).nodes(&|_| true);
        let f = fade(&nodes).unwrap();
        assert_eq!(f.progression.duration_ms, 2000.0);
        assert_eq!(f.to, [1.0, 1.0, 1.0, 1.0]);
        // White in by 0.8 s, then out by 2 s.
        assert!((f.progression.value(800.0) - 1.0).abs() < 1e-3);
        assert!(f.progression.value(2000.0).abs() < 1e-3);
    }
}
