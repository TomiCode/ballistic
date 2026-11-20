//! Extracts the prefabs' particle systems (see the research repo, findings
//! 20): every *Point*, *Planar* and *Spherical Particle System* in the
//! prefab files (`PH/*.nmo`) and `Balls.nmo`, with its inputs and its
//! settings (rendering, blend, which variances and evolutions are on).

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckscript::extract::{Extractor, NamedValue, NodeParams, Resolved, Value};

use crate::install::Install;
use crate::physics::load;
use crate::transformers::{float, input, text, vector};

/// *Variances* bits.
mod variance {
    pub const SPEED: u32 = 0x1;
    pub const LIFESPAN: u32 = 0x4;
    pub const EMISSION: u32 = 0x8;
    pub const SIZE: u32 = 0x10;
    pub const END_SIZE: u32 = 0x20;
    pub const COLOR: u32 = 0x200;
    pub const END_COLOR: u32 = 0x400;
}

/// *Evolutions* bits.
mod evolution {
    pub const SIZE: u32 = 0x1;
    pub const COLOR: u32 = 0x2;
}

pub fn extract(install: &Install) -> Result<Vec<bb::ParticleEmitter>> {
    let mut files: Vec<(String, String)> = install
        .under("3d entities/ph/")
        .filter(|(k, _)| {
            std::path::Path::new(k)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("nmo"))
        })
        .map(|(k, _)| {
            let stem = k["3d entities/".len()..]
                .trim_end_matches(".nmo")
                .to_owned();
            (k.to_owned(), stem)
        })
        .collect();
    files.push(("3d entities/balls.nmo".into(), "balls".into()));
    files.sort();
    let mut out = Vec::new();
    for (key, scene) in files {
        let g = load(install, &key)?;
        let nodes = Extractor::new(&g.file, &g.scripts).nodes(&|_| true);
        for n in &nodes {
            let shape = match n.building_block.as_str() {
                // The time-dependent one emits the same way (the extra
                // point's trails).
                "Point Particle System" | "TT_TimedependentPointParticlesystem" => {
                    bb::EmitterShape::Point
                }
                "PlanarParticleSystem" => bb::EmitterShape::Planar,
                "SphericalParticleSystem" => bb::EmitterShape::Spherical,
                _ => continue,
            };
            let e = emitter(install, &g.file, &scene, n, shape)
                .with_context(|| format!("{key}: {}", n.name))?;
            out.push(e);
        }
    }
    Ok(out)
}

/// A setting (local) as a number: enums and flags are stored as integers.
fn setting(n: &NodeParams, name: &str) -> Option<u32> {
    let v: &NamedValue = n.locals.iter().find(|l| l.name == name)?;
    match &v.value {
        Resolved::Constant {
            value: Value::Int(i),
        } => u32::try_from(*i).ok(),
        Resolved::Constant {
            value: Value::Other(s),
        } => s
            .rsplit_once('(')
            .and_then(|(_, r)| r.trim_end_matches(')').trim().parse().ok()),
        _ => None,
    }
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

/// A Virtools blend factor number (1 Zero … 11 Src Alpha Sat).
fn blend(v: u32) -> Option<bb::BlendFactor> {
    use bb::BlendFactor as B;
    Some(match v {
        1 => B::Zero,
        2 => B::One,
        3 => B::SrcColor,
        4 => B::InvSrcColor,
        5 => B::SrcAlpha,
        6 => B::InvSrcAlpha,
        7 => B::DestAlpha,
        8 => B::InvDestAlpha,
        9 => B::DestColor,
        10 => B::InvDestColor,
        11 => B::SrcAlphaSat,
        _ => return None,
    })
}

fn emitter(
    install: &Install,
    file: &ckfile::CkFile,
    scene: &str,
    n: &NodeParams,
    shape: bb::EmitterShape,
) -> Result<bb::ParticleEmitter> {
    let num = |pin: &str| float(input(n, pin)).with_context(|| format!("{pin} missing"));
    let variances = setting(n, "Variances").unwrap_or(0);
    let evolutions = setting(n, "Evolutions").unwrap_or(0);
    let varied = |pin: &str, var_pin: &str, bit: u32| -> Result<bb::Varied> {
        Ok(bb::Varied {
            value: num(pin)?,
            variance: if variances & bit == 0 {
                0.0
            } else {
                num(var_pin)?
            },
        })
    };
    let color = |pin: &str| -> Result<[f32; 4]> {
        vector(input(n, pin))
            .and_then(|v| <[f32; 4]>::try_from(v).ok())
            .with_context(|| format!("{pin} missing"))
    };
    // Two inputs are named "Variance": the first follows the initial
    // colour, the second the ending colour.
    let color_variances: Vec<&NamedValue> =
        n.inputs.iter().filter(|i| i.name == "Variance").collect();
    let color_var = |i: usize, bit: u32| -> [f32; 4] {
        if variances & bit == 0 {
            return [0.0; 4];
        }
        color_variances
            .get(i)
            .and_then(|v| vector(Some(v)))
            .and_then(|v| <[f32; 4]>::try_from(v).ok())
            .unwrap_or([0.0; 4])
    };
    let render = match setting(n, "Particle Rendering").unwrap_or(0) {
        2 => bb::ParticleRender::Line,
        3 => bb::ParticleRender::Sprite,
        5 => bb::ParticleRender::OrientableSprite,
        7 => bb::ParticleRender::FastSprite,
        other => bb::ParticleRender::Other(other),
    };
    let texture = text(input(n, "Texture"))
        .map(|name| crate::hud::texture_key(install, file, &name))
        .transpose()?;
    let src = setting(n, "Source Blend")
        .and_then(blend)
        .unwrap_or(bb::BlendFactor::SrcAlpha);
    let dst = setting(n, "Destination Blend")
        .and_then(blend)
        .unwrap_or(bb::BlendFactor::One);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "small counts"
    )]
    let max = num("Maximum Number")?.max(0.0) as u32;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "small counts"
    )]
    let frames = float(input(n, "Texture Frame Count"))
        .unwrap_or(1.0)
        .max(1.0) as u32;
    Ok(bb::ParticleEmitter {
        scene: scene.to_owned(),
        object: n.owner.clone().context("no owner object")?,
        shape,
        render,
        texture,
        blend: (src, dst),
        delay_ms: num("Emission Delay")?,
        yaw_variance: num("Yaw Variance")?,
        pitch_variance: num("Pitch Variance")?,
        speed: varied("Speed", "Speed Variance", variance::SPEED)?,
        lifespan_ms: varied("Lifespan", "Lifespan Variance", variance::LIFESPAN)?,
        max,
        emission: varied("Emission", "Emission Variance", variance::EMISSION)?,
        size: varied("Initial Size", "Initial Size Variance", variance::SIZE)?,
        end_size: if evolutions & evolution::SIZE == 0 {
            None
        } else {
            Some(varied(
                "Ending Size",
                "Ending Size Variance",
                variance::END_SIZE,
            )?)
        },
        color: color("Initial Color and Alpha")?,
        color_variance: color_var(0, variance::COLOR),
        end_color: if evolutions & evolution::COLOR == 0 {
            None
        } else {
            Some(color("Ending Color and Alpha")?)
        },
        end_color_variance: color_var(1, variance::END_COLOR),
        frames,
        real_time: boolean(input(n, "Real-Time Mode")).unwrap_or(true),
        delta_ms: float(input(n, "DeltaTime")).unwrap_or(20.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_flames() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let all = extract(&install).unwrap();
        let big = all
            .iter()
            .find(|e| e.object == "PC_TwoFlames_Flame_Big")
            .unwrap();
        assert_eq!(big.scene, "ph/pc_twoflames");
        assert_eq!(big.shape, bb::EmitterShape::Point);
        assert_eq!(big.render, bb::ParticleRender::FastSprite);
        assert_eq!(big.texture.as_deref(), Some("particle_flames"));
        assert_eq!(big.blend, (bb::BlendFactor::SrcAlpha, bb::BlendFactor::One));
        assert!((big.speed.value - 0.012).abs() < 1e-6);
        assert!((big.speed.variance - 0.004).abs() < 1e-6);
        assert!((big.lifespan_ms.variance - 500.0).abs() < 1e-3);
        assert_eq!(big.max, 50);
        assert!((big.size.value - 5.0).abs() < 1e-6);
        assert!((big.end_size.unwrap().value - 0.1).abs() < 1e-6);
        assert_eq!(big.end_color, Some([0.0; 4]));
        assert!(big.real_time);
        let start = all.iter().filter(|e| e.scene == "ph/ps_fourflames").count();
        assert_eq!(start, 4);
        let fans = all.iter().filter(|e| e.scene == "ph/p_modul_18").count();
        assert_eq!(fans, 2);
    }
}
