//! Extracts the music and the storm (see the research repo, findings 26):
//! - `Sound.nmo`'s `Music_Manager`: the ambient and theme graphs' random
//!   waits and sounds, the first theme's delay, the fades, *Last Stage* (its
//!   loop, fades and the distance to the level end), the finish music; the
//!   highscore and thunder sounds in `Sound_Manager`; the wooden flaps' hit
//!   sound (`HitSound Woodenflaps`).
//! - `Levelinit.nmo`'s `AllLevel` *Music* column: each level's theme set
//!   (`Music_Theme_<set>_1` … `_3`).
//! - `Gameplay.nmo`'s `Gameplay_Blitz`: the last level's lightning.
//!
//! The music's wave sounds are looked up by name at runtime (`Music_Init`);
//! their names are the locals' names without " Sound", and their files are
//! the same names.

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckscript::extract::{Extractor, NodeParams};
use ckscript::{ObjRef, ScriptObject};

use crate::install::Install;
use crate::physics::{Loaded, load};
use crate::transformers::{float, input, progression, target, text, vector};

#[allow(clippy::too_many_lines, reason = "one pass over Music_Manager")]
pub fn extract(install: &Install) -> Result<bb::Music> {
    let s = load(install, "3d entities/sound.nmo")?;
    let ex = Extractor::new(&s.file, &s.scripts);
    let all = ex.nodes(&|_| true);
    let in_graph = |graph: &str, bb_name: &str| -> Vec<&NodeParams> {
        all.iter()
            .filter(|n| n.building_block == bb_name && n.path.iter().any(|p| p == graph))
            .collect()
    };
    let one = |graph: &str, bb_name: &str| {
        in_graph(graph, bb_name)
            .into_iter()
            .next()
            .with_context(|| format!("{graph}: {bb_name} not found"))
    };
    let num = |n: &NodeParams, pin: &str| {
        float(input(n, pin)).with_context(|| format!("{}: {pin}", n.building_block))
    };
    // A Wave Player's (or Parameter Selector input's) sound: its local's
    // name without " Sound".
    let sound = |name: Option<String>| -> Result<String> {
        let name = name.context("a sound isn't named")?;
        let key = name.trim_end_matches(" Sound").to_lowercase();
        anyhow::ensure!(
            install.get(&format!("sounds/{key}.wav")).is_some(),
            "sound {key} not in Sounds/"
        );
        Ok(key)
    };
    let player = |graph: &str| -> Result<Vec<String>> {
        in_graph(graph, "Wave Player")
            .into_iter()
            .map(|n| sound(target_local(&s, n.node)))
            .collect()
    };
    let choices = |graph: &str| -> Result<Vec<String>> {
        let sel = one(graph, "Parameter Selector")?;
        (0..3)
            .map(|i| sound(input_local(&s, sel.node, &format!("pIn {i}"))))
            .collect()
    };

    // Each level's theme set.
    let li = load(install, "3d entities/levelinit.nmo")?;
    let levels = li.array("AllLevel")?;
    let mut themes = Vec::new();
    for r in &levels.rows {
        let set = li.row(levels, r).f32("Music")?;
        themes.push(
            (1..=3)
                .map(|n| sound(Some(format!("Music_Theme_{set}_{n}"))))
                .collect::<Result<Vec<_>>>()?,
        );
    }

    // *Last Stage*'s fades: the sting (in 0, out long), near the end (in
    // long), the finish (out short), each an *Identity* of two floats.
    let fades: Vec<[f32; 2]> = in_graph("Last Stage", "Identity")
        .into_iter()
        .filter_map(|n| Some([float(input(n, "pIn 0"))?, float(input(n, "Pin 1"))?]))
        .collect();
    let sting = fades
        .iter()
        .find(|f| f[0] == 0.0 && f[1] >= 1000.0)
        .context("Last Stage: the sting's fades")?;
    let near = fades
        .iter()
        .find(|f| f[0] > 0.0)
        .context("Last Stage: the near-end fades")?;
    let finish = fades
        .iter()
        .find(|f| f[0] == 0.0 && f[1] < 1000.0)
        .context("Last Stage: the finish fade")?;
    let end_music = player("Play EndMusic")?;
    let [finish_music, last_finish] = end_music.as_slice() else {
        bail!("Play EndMusic: expected two sounds, got {end_music:?}");
    };
    let manager_delay = all
        .iter()
        .find(|n| {
            n.building_block == "Delayer"
                && n.path.last().map(String::as_str) == Some("Music_Manager")
        })
        .context("Music_Manager: the themes' Delayer")?;
    let highscore = all
        .iter()
        .filter(|n| n.building_block == "Wave Player")
        .filter_map(|n| target_local(&s, n.node))
        .find(|name| name.starts_with("Music_Highscore"))
        .context("Sound_Manager: the highscore music")?;

    Ok(bb::Music {
        fade_ms: num(one("Fade In Music", "Linear Progression")?, "Duration")?,
        atmo: choices("Music_Atmo")?,
        atmo_gap_ms: num(one("Music_Atmo", "Random")?, "Max")?,
        themes,
        theme_gap_ms: num(one("Music_Theme", "Random")?, "Max")?,
        theme_start_ms: num(manager_delay, "Time to Wait")?,
        last_stage: player("Last Stage")?
            .into_iter()
            .find(|k| k.contains("endcheckpoint"))
            .context("Last Stage: the loop")?,
        last_stage_fade_out_ms: sting[1],
        near_end_distance: num(one("Last Stage", "TT Scaleable Proximity")?, "Distance")?,
        near_end_fades_ms: *near,
        finish_fade_out_ms: finish[1],
        finish: finish_music.clone(),
        last_finish: last_finish.clone(),
        highscore: sound(Some(highscore))?,
        thunder: player("Donner")?
            .into_iter()
            .next()
            .context("Donner: no sound")?,
        storm: storm(install)?,
    })
}

/// `HitSound Woodenflaps` in `Sound.nmo`'s `Sound_Manager`.
pub fn flaps(install: &Install) -> Result<bb::GroupHitSound> {
    let s = load(install, "3d entities/sound.nmo")?;
    let ex = Extractor::new(&s.file, &s.scripts);
    let all = ex.nodes(&|_| true);
    let find = |bb_name: &str| {
        all.iter()
            .find(|n| {
                n.building_block == bb_name && n.path.iter().any(|p| p == "HitSound Woodenflaps")
            })
            .with_context(|| format!("HitSound Woodenflaps: {bb_name}"))
    };
    let detect = find("PhysicsCollDetection")?;
    let num = |pin: &str| {
        float(input(detect, pin)).with_context(|| format!("PhysicsCollDetection: {pin}"))
    };
    let key = target(find("Play Sound Instance")?)
        .context("Play Sound Instance: no sound")?
        .to_lowercase();
    anyhow::ensure!(
        install.get(&format!("sounds/{key}.wav")).is_some(),
        "sound {key} not in Sounds/"
    );
    Ok(bb::GroupHitSound {
        group: text(input(find("Op")?, "p1")).context("the flaps' group")?,
        sound: key,
        min_speed: num("Min Speed m/s")?,
        max_speed: num("Max Speed m/s")?,
        sleep_s: num("Sleep afterwards")?,
    })
}

/// `Gameplay_Blitz`.
fn storm(install: &Install) -> Result<bb::Storm> {
    let g = load(install, "3d entities/gameplay.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let all = ex.nodes(&|_| true);
    let blitz: Vec<&NodeParams> = all
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some("Gameplay_Blitz"))
        .collect();
    let find = |bb_name: &'static str| {
        blitz
            .clone()
            .into_iter()
            .filter(move |n| n.building_block == bb_name)
    };
    let random = find("Random").next().context("Gameplay_Blitz: Random")?;
    let mut delays: Vec<f32> = find("Delayer")
        .filter_map(|n| float(input(n, "Time to Wait")))
        .collect();
    delays.sort_by(f32::total_cmp);
    let [thunder, first] = delays.as_slice() else {
        bail!("Gameplay_Blitz: expected two Delayers, got {delays:?}");
    };
    let color = match find("Interpolator")
        .next()
        .and_then(|n| vector(input(n, "B")))
        .as_deref()
    {
        Some(&[r, g, b, a]) => [r, g, b, a],
        other => bail!("Gameplay_Blitz: the flash's colour {other:?}"),
    };
    Ok(bb::Storm {
        first_ms: *first,
        gap_ms: [
            float(input(random, "Min")).context("Random: Min")?,
            float(input(random, "Max")).context("Random: Max")?,
        ],
        flash: progression(
            find("Bezier Progression")
                .next()
                .context("Gameplay_Blitz: Bezier Progression")?,
        )?,
        color,
        thunder_delay_ms: *thunder,
    })
}

/// The name of the local feeding `node`'s target.
fn target_local(g: &Loaded, node: ObjRef) -> Option<String> {
    let p = g.scripts.behavior(node)?.target_parameter?;
    source_name(g, p)
}

/// The name of the local feeding `node`'s input `pin`.
fn input_local(g: &Loaded, node: ObjRef, pin: &str) -> Option<String> {
    let p = crate::hud::pin_param(&g.file, &g.scripts, node, pin)?;
    source_name(g, p)
}

fn source_name(g: &Loaded, param: ObjRef) -> Option<String> {
    let Some(Some(ScriptObject::In(i))) = g.scripts.objects.get(param) else {
        return None;
    };
    g.file.objects.get(i.source?)?.name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_music() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let m = extract(&install).unwrap();
        assert_eq!(m.atmo, ["music_atmo_1", "music_atmo_2", "music_atmo_3"]);
        assert!((m.atmo_gap_ms - 15000.0).abs() < 1e-3);
        assert_eq!(m.themes.len(), 12);
        assert_eq!(m.themes[0][0], "music_theme_1_1");
        assert_eq!(m.themes[1][2], "music_theme_5_3");
        assert!((m.theme_gap_ms - 50000.0).abs() < 1e-3);
        assert!((m.theme_start_ms - 7000.0).abs() < 1e-3);
        assert!((m.fade_ms - 1000.0).abs() < 1e-3);
        assert_eq!(m.last_stage, "music_endcheckpoint");
        assert!((m.last_stage_fade_out_ms - 10000.0).abs() < 1e-3);
        assert!((m.near_end_distance - 200.0).abs() < 1e-3);
        assert_eq!(m.near_end_fades_ms, [13000.0, 1500.0]);
        assert!((m.finish_fade_out_ms - 400.0).abs() < 1e-3);
        assert_eq!(m.finish, "music_final");
        assert_eq!(m.last_finish, "music_lastfinal");
        assert_eq!(m.highscore, "music_highscore");
        assert_eq!(m.thunder, "music_thunder");
        assert!((m.storm.first_ms - 4000.0).abs() < 1e-3);
        assert_eq!(m.storm.gap_ms, [10000.0, 90000.0]);
        assert!((m.storm.thunder_delay_ms - 150.0).abs() < 1e-3);
        assert!((m.storm.flash.duration_ms - 200.0).abs() < 1e-3);

        let f = flaps(&install).unwrap();
        assert_eq!(f.group, "Phys_FloorStopper");
        assert_eq!(f.sound, "hit_woodenflap");
        assert!((f.min_speed - 0.3).abs() < 1e-6 && (f.max_speed - 10.0).abs() < 1e-6);
        assert!((f.sleep_s - 0.5).abs() < 1e-6);
    }
}
