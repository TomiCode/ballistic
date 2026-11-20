//! Extracts the level's end and the score screen (see the research repo,
//! findings 23):
//! - `Gameplay.nmo`'s `Gameplay_Events` after `Level_Finish`: the wait before
//!   the score screen (*Wait*'s *Parameter Selector*: the usual one and the
//!   last level's), its keys, *fadeout Sky*, *Set Clipping Planes*;
//! - `Menu.nmo`'s `Menu_Score`: its sprites (`M_Score_Sprites`) and the
//!   colours they fade to (*Einblendung*), the highlight's places (each
//!   step's `TT Set_2DSprite`), the texts (*Text*'s *2D Text*s; the labels
//!   are `Language.nmo` rows, picked in `Menu_Init`), the font
//!   (`GameFont_04`), the delays, `Menu_Score_CountSpeed` and the counter's
//!   pitch, and the counter sound (`Sound.nmo`'s wave sound).

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckfile::{CkFile, ClassId};
use ckscript::ObjRef;
use ckscript::ScriptObject;
use ckscript::extract::{Extractor, NodeParams};

use crate::hud::{glyphs, group_members, named_by, pin_param, sprite, texture_key};
use crate::install::Install;
use crate::physics::{Loaded, load};
use crate::transformers::{float, input, target, text, vector, writer_of};

pub fn level_end(install: &Install) -> Result<bb::LevelEnd> {
    let g = load(install, "3d entities/gameplay.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let nodes = ex.nodes(&|_| true);
    let events: Vec<&NodeParams> = nodes
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some("Gameplay_Events"))
        .collect();
    let find = |graph: &str, bb_name: &str| {
        events
            .iter()
            .copied()
            .find(|n| {
                n.building_block == bb_name && n.path.last().map(String::as_str) == Some(graph)
            })
            .with_context(|| format!("Gameplay_Events: {graph}'s {bb_name} not found"))
    };
    let select = find("Wait", "Parameter Selector")?;
    let num = |n: &NodeParams, pin: &str| {
        float(input(n, pin)).with_context(|| format!("{}: {pin}", n.building_block))
    };
    let fade = find("fadeout Sky", "Interpolator")?;
    let colour = |pin: &str| -> Result<[f32; 4]> {
        match vector(input(fade, pin)).as_deref() {
            Some(&[r, g, b, a]) => Ok([r, g, b, a]),
            other => bail!("fadeout Sky: {pin} {other:?}"),
        }
    };
    Ok(bb::LevelEnd {
        wait_ms: num(select, "pIn 0")?,
        last_wait_ms: num(select, "pIn 1")?,
        sky_fade_ms: num(find("fadeout Sky", "Linear Progression")?, "Duration")?,
        sky_fade: [colour("A")?, colour("B")?],
        far_clip: num(find("Gameplay_Events", "Set Clipping Planes")?, "Far Clip")?,
        skip_keys: keys(&events, "3 keys"),
    })
}

/// The keys of the *Key Event*s in `graph`, in order, once each.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "scan codes"
)]
pub(crate) fn keys(nodes: &[&NodeParams], graph: &str) -> Vec<u32> {
    let mut k: Vec<u32> = nodes
        .iter()
        .filter(|n| n.building_block == "Key Event" && n.path.iter().any(|p| p == graph))
        .filter_map(|n| float(input(n, "Key Waited")))
        .map(|k| k as u32)
        .collect();
    k.sort_unstable();
    k.dedup();
    k
}

#[allow(
    clippy::too_many_lines,
    reason = "one pass over Menu_Score, step by step"
)]
pub fn score_screen(install: &Install) -> Result<bb::ScoreScreen> {
    let menu = load(install, "3d entities/menu.nmo")?;
    let ex = Extractor::new(&menu.file, &menu.scripts);
    let all = ex.nodes(&|_| true);
    let score: Vec<&NodeParams> = all
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some("Menu_Score"))
        .collect();
    let in_graph = |graph: &str, bb_name: &str| -> Vec<&NodeParams> {
        score
            .iter()
            .copied()
            .filter(|n| {
                n.building_block == bb_name && n.path.last().map(String::as_str) == Some(graph)
            })
            .collect()
    };
    let find = |graph: &str, bb_name: &str| {
        in_graph(graph, bb_name)
            .into_iter()
            .next()
            .with_context(|| format!("Menu_Score: {graph}'s {bb_name} not found"))
    };
    let num = |n: &NodeParams, pin: &str| {
        float(input(n, pin)).with_context(|| format!("{}: {pin}", n.building_block))
    };
    let pair = |n: &NodeParams, pin: &str| -> Result<[f32; 2]> {
        match vector(input(n, pin)).as_deref() {
            Some(&[x, y]) => Ok([x, y]),
            other => bail!("{}: {pin} {other:?}", n.building_block),
        }
    };
    let quad = |n: &NodeParams, pin: &str| -> Result<[f32; 4]> {
        match vector(input(n, pin)).as_deref() {
            Some(&[a, b, c, d]) => Ok([a, b, c, d]),
            other => bail!("{}: {pin} {other:?}", n.building_block),
        }
    };
    // The node writing `node`'s input `pin` (an Interpolator, say).
    let writer = |n: &NodeParams, pin: &str| {
        writer_of(&menu.file, &menu.scripts, n.node, pin)
            .and_then(|w| all.iter().find(|m| m.node == w))
            .with_context(|| format!("{}: nothing writes {pin}", n.building_block))
    };

    // The sprites, at the colours they fade in to.
    let init_group = target_name(find("Init", "Group Iterator")?, "Group")
        .context("Init: the sprites' group")?;
    let mut sprites = Vec::new();
    // (The group also holds the texts' boxes, which have no material.)
    for name in group_members(&menu.file, &init_group)? {
        let Some(set) = in_graph("Einblendung", "Set Diffuse")
            .into_iter()
            .find(|n| target(n).as_deref() == Some(name.as_str()))
        else {
            continue;
        };
        let mut s = sprite(install, &menu.file, &name, true)?;
        s.color = quad(writer(set, "Diffuse Color")?, "B")?;
        sprites.push(s);
    }
    sprites.sort_by_key(|s| s.z_order);
    let fade_ms = num(find("Einblendung", "Linear Progression")?, "Duration")?;

    // The highlight's places: each step moves it.
    let place = |graph: &str| pair(find(graph, "TT Set_2DSprite")?, "Position");
    let highlight =
        target(find("Score 1", "TT Set_2DSprite")?).context("Score 1: the highlight")?;
    let highlight_rows = ["Score 1", "Score 2", "Score 3", "Score 4"]
        .iter()
        .map(|g| place(g))
        .collect::<Result<_>>()?;

    // The font: as Einblendung sets it, at the colours it fades in to.
    let props = find("Einblendung", "Set Font Properties")?;
    let font_name = source_name(&menu, props.node, "Font").context("Einblendung: the font")?;
    let create = all
        .iter()
        .find(|n| {
            n.building_block == "TT_CreateFontEx"
                && text(input(n, "Font Name")).as_deref() == Some(font_name.as_str())
        })
        .with_context(|| format!("{font_name}'s TT_CreateFontEx"))?;
    let font = bb::HudFont {
        texture: texture_key(
            install,
            &menu.file,
            &text(input(create, "Font Texture")).context("Font Texture")?,
        )?,
        glyphs: glyphs(
            &menu,
            &text(input(create, "FontCoordinatesData")).context("FontCoordinatesData")?,
        )?,
        scale: pair(props, "Scale")?,
        space: pair(props, "Space")?,
        color: quad(writer(props, "Color")?, "B")?,
    };
    let shadow = bb::TextShadow {
        color: quad(writer(props, "Shadow Color")?, "B")?,
        angle: num(props, "Shadow Angle")?,
        distance: num(props, "Shadow Distance")?,
        size: pair(props, "Shadow Size")?,
    };

    // The texts: labels in M_Score_TextN, values in M_Score_ScoreN.
    let texts = in_graph("Text", "2D Text");
    let language = load(install, "3d entities/language.nmo")?;
    let table = language.array("language")?;
    let mut rows = Vec::new();
    for i in 1..=4 {
        let by_target = |prefix: &str| {
            texts
                .iter()
                .copied()
                .find(|n| target(n).is_some_and(|t| t == format!("{prefix}{i}")))
                .with_context(|| format!("Text: {prefix}{i}"))
        };
        let label = by_target("M_Score_Text")?;
        let value = by_target("M_Score_Score")?;
        let row = language_row(&menu, &all, label.node, "Text")?;
        let cells = table
            .rows
            .get(row)
            .with_context(|| format!("language row {row}"))?;
        let label_texts: BTreeMap<String, String> = table
            .columns
            .iter()
            .zip(cells)
            .filter(|(c, _)| c.name != "description")
            .filter_map(|(c, cell)| match cell {
                ckscript::Cell::String(s) => Some((c.name.clone(), s.clone())),
                _ => None,
            })
            .collect();
        let rect = |n: &NodeParams| -> Result<[f32; 4]> {
            let name = target(n).context("2D Text target")?;
            Ok(sprite(install, &menu.file, &name, true)?.rect)
        };
        rows.push(bb::ScoreRow {
            label: label_texts,
            label_rect: rect(label)?,
            value_rect: rect(value)?,
        });
    }
    let first = texts.first().context("Text: no 2D Text")?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "flag bits"
    )]
    let align = match num(first, "Alignment")? as u32 {
        a if a & 1 != 0 => bb::TextAlign::Left,
        a if a & 2 != 0 => bb::TextAlign::Right,
        _ => bb::TextAlign::Center,
    };

    // The counting.
    #[expect(clippy::cast_possible_truncation, reason = "table integers")]
    let count_speed = {
        let t = menu.array("Menu_Score_CountSpeed")?;
        t.rows
            .iter()
            .map(|r| {
                let row = menu.row(t, r);
                Ok([row.f32("Limit")? as i32, row.f32("Step")? as i32])
            })
            .collect::<Result<Vec<_>>>()?
    };
    // Row 0 sets the pitch; the others ramp it (Interpolators A → B).
    let first_pitch = in_graph("Sound Pitch", "Pitch Control")
        .into_iter()
        .find_map(|n| float(input(n, "Frequency")))
        .context("Sound Pitch: a fixed pitch")?;
    let mut ramps = in_graph("Sound Pitch", "Interpolator")
        .into_iter()
        .map(|n| Ok([num(n, "A")?, num(n, "B")?]))
        .collect::<Result<Vec<_>>>()?;
    ramps.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let count_pitch = std::iter::once([first_pitch; 2]).chain(ramps).collect();
    let pitch_ramp_ms = num(find("Sound Pitch", "Linear Progression")?, "Duration")?;
    let player = score
        .iter()
        .copied()
        .find(|n| n.building_block == "Wave Player" && n.path.iter().any(|p| p == "Score 2"))
        .context("Score 2: the counter's Wave Player")?;
    let wave = menu
        .scripts
        .behavior(player.node)
        .and_then(|b| b.target_parameter)
        .and_then(|p| named_by(&menu, &all, p))
        .context("Score 2: the counter sound isn't looked up by name")?;

    Ok(bb::ScoreScreen {
        sprites,
        fade_ms,
        highlight,
        highlight_rows,
        highlight_fast: place("Fast Scoring")?,
        font,
        shadow,
        rows,
        align,
        margins: quad(first, "Margins")?,
        step_ms: num(find("Menu_Score", "Delayer")?, "Time to Wait")?,
        count_speed,
        count_pitch,
        pitch_ramp_ms,
        counter_sound: wave_key(install, &wave)?,
        life_ms: num(find("Score 3", "Delayer")?, "Time to Wait")?,
        fast_life_ms: num(find("Subtract Remaining Lives", "Delayer")?, "Time to Wait")?,
        wait_ms: num(find("Wait", "Timer")?, "Duration")?,
        keys: keys(&score, "3 secure keys"),
    })
}

/// The object an input names (a group, say).
fn target_name(n: &NodeParams, pin: &str) -> Option<String> {
    text(input(n, pin))
}

/// The name of the local feeding `node`'s input `pin`.
pub(crate) fn source_name(g: &Loaded, node: ObjRef, pin: &str) -> Option<String> {
    let param = pin_param(&g.file, &g.scripts, node, pin)?;
    let Some(Some(ScriptObject::In(i))) = g.scripts.objects.get(param) else {
        return None;
    };
    g.file.objects.get(i.source?)?.name.clone()
}

/// The `Language.nmo` row a label's text comes from: `Menu_Init` reads it
/// (*Get Cell* on the language table) into the local the *2D Text* shows.
pub(crate) fn language_row(
    g: &Loaded,
    nodes: &[NodeParams],
    text_node: ObjRef,
    pin: &str,
) -> Result<usize> {
    let param = pin_param(&g.file, &g.scripts, text_node, pin)
        .with_context(|| format!("no input {pin}"))?;
    let Some(Some(ScriptObject::In(i))) = g.scripts.objects.get(param) else {
        bail!("2D Text: Text isn't an input");
    };
    let local = i.source.context("2D Text: Text has no source")?;
    let getter = g
        .scripts
        .objects
        .iter()
        .enumerate()
        .find_map(|(idx, o)| match o {
            Some(ScriptObject::Out(out)) if out.destinations.contains(&local) => {
                g.scripts.owner.get(&idx).copied()
            }
            _ => None,
        })
        .context("nothing writes the label's local")?;
    let get = nodes
        .iter()
        .find(|n| n.node == getter && n.building_block == "Get Cell")
        .context("the label's local isn't read from a table")?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a row index"
    )]
    float(input(get, "Row Index"))
        .map(|r| r as usize)
        .context("Get Cell: Row Index")
}

/// A wave sound's file, as a bundle sound key (`Sound.nmo` holds the menu's
/// sounds; the file name is the chunk's section `0x1000`, after a word).
pub(crate) fn wave_key(install: &Install, name: &str) -> Result<String> {
    let file = CkFile::open(
        install
            .get("3d entities/sound.nmo")
            .context("Sound.nmo missing")?,
    )?;
    let o = file
        .objects
        .iter()
        .find(|o| o.class_id == ClassId::WAVE_SOUND && o.name.as_deref() == Some(name))
        .with_context(|| format!("wave sound {name} not found"))?;
    let chunk = o.chunk.as_ref().context("wave sound without data")?;
    let mut r = chunk
        .section(0x1000)?
        .context("wave sound without a file section")?;
    r.u32()?;
    let path = r.string()?.context("wave sound without a file")?;
    let key = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&path)
        .to_lowercase();
    let key = key.strip_suffix(".wav").unwrap_or(&key).to_owned();
    anyhow::ensure!(
        install.get(&format!("sounds/{key}.wav")).is_some(),
        "sound {key} not in Sounds/"
    );
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_level_end_and_score_screen() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let end = level_end(&install).unwrap();
        assert!((end.wait_ms - 10000.0).abs() < 1e-3);
        assert!((end.last_wait_ms - 23000.0).abs() < 1e-3);
        assert!((end.sky_fade_ms - 3000.0).abs() < 1e-3);
        assert_eq!(end.sky_fade[1], [0.0, 0.0, 0.0, 1.0]);
        let mut k = end.skip_keys.clone();
        k.sort_unstable();
        assert_eq!(k, [1, 28, 57]);

        let s = score_screen(&install).unwrap();
        let names: Vec<&str> = s.sprites.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&"M_Score_Field"), "{names:?}");
        assert_eq!(s.highlight, "M_Score_Highlight");
        assert_eq!(s.highlight_rows.len(), 4);
        assert!((s.highlight_rows[1][1] - 0.36).abs() < 1e-4);
        assert!((s.fade_ms - 200.0).abs() < 1e-3);
        assert_eq!(s.font.texture, "font_1");
        assert_eq!(s.font.scale, [0.6, 0.6]);
        assert_eq!(s.font.color, [1.0; 4]);
        assert_eq!(s.rows.len(), 4);
        assert_eq!(s.rows[0].label["english"], "Level Bonus:");
        assert_eq!(s.rows[3].label["german"], "Gesamtpunkte:");
        assert_eq!(s.align, bb::TextAlign::Right);
        assert_eq!(s.count_speed, [[80, 1], [500, 5], [9999, 25]]);
        assert_eq!(s.count_pitch, [[1.0, 1.0], [1.0, 1.1], [1.1, 1.2]]);
        assert_eq!(s.counter_sound, "menu_counter");
        assert!((s.life_ms - 610.0).abs() < 1e-3);
        assert!((s.fast_life_ms - 700.0).abs() < 1e-3);
        assert!((s.wait_ms - 4000.0).abs() < 1e-3);
        assert!((s.step_ms - 1000.0).abs() < 1e-3);
    }
}
