//! Extracts the highscores (see the research repo, findings 24):
//! - `base.cmo`'s tables: each level's default table (`DB_Highscore_LvNN`),
//!   the levels open at first (`DB_Levelfreischaltung`), the last player's
//!   name (`DB_Options`);
//! - `Menu_HighscoreEntry` and `Menu_Highscore`: their 2D entities (shown by
//!   their *Init*s, the places from `Menu_Highscore_ShowHide`), their *2D
//!   Text*s (fixed texts followed to the `Language.nmo` rows `Menu_Init`
//!   reads), the name's length and caret, and their keys.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckfile::{CkFile, ClassId};
use ckscene::{Decoded, decode_object};
use ckscript::extract::{Extractor, NodeParams};
use ckscript::{Cell, DataArray, ObjRef, ScriptObject};

use crate::hud::{glyphs, object, sprite, texture_key, texture_key_at};
use crate::install::Install;
use crate::physics::{Loaded, load};
use crate::score::{keys, language_row, source_name};
use crate::transformers::{float, input, target, text, vector, writer_of};

#[allow(
    clippy::too_many_lines,
    reason = "the tables, then each screen in turn"
)]
pub fn extract(install: &Install) -> Result<bb::Highscores> {
    let base = load(install, "base.cmo")?;
    let mut defaults = Vec::new();
    for n in 1.. {
        let Ok(t) = base.array(&format!("DB_Highscore_Lv{n:02}")) else {
            break;
        };
        #[expect(clippy::cast_possible_truncation, reason = "table integers")]
        let rows = t
            .rows
            .iter()
            .map(|r| {
                let row = base.row(t, r);
                Ok(bb::HighscoreRow {
                    name: row.string("Playername")?,
                    score: row.f32("Points")? as i32,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        defaults.push(rows);
    }
    anyhow::ensure!(!defaults.is_empty(), "no DB_Highscore_LvNN tables");
    let open = base.array("DB_Levelfreischaltung")?;
    let unlocked = open
        .rows
        .iter()
        .map(|r| Ok(base.row(open, r).f32("Freigeschaltet?")? != 0.0))
        .collect::<Result<_>>()?;
    let options = base.array("DB_Options")?;
    let default_name = base
        .row(
            options,
            options.rows.first().context("DB_Options is empty")?,
        )
        .string("LastPlayer")?;

    let menu = load(install, "3d entities/menu.nmo")?;
    let ex = Extractor::new(&menu.file, &menu.scripts);
    let all = ex.nodes(&|_| true);
    let language = load(install, "3d entities/language.nmo")?;
    let table = language.array("language")?;
    let cx = Cx {
        install,
        menu: &menu,
        all: &all,
        language: table,
    };

    // The name entry.
    let in_script = |script: &str| -> Vec<&NodeParams> {
        all.iter()
            .filter(|n| n.path.first().map(String::as_str) == Some(script))
            .collect()
    };
    let entry_nodes = in_script("Menu_HighscoreEntry");
    let name_node = by_target(&entry_nodes, "2D Text", "_NameEntry")?;
    let create = find(&entry_nodes, "Create String")?;
    let mut score = cx.text(by_target(&entry_nodes, "2D Text", "_Score")?)?;
    score.text = cx.localised(create.node, "Pin 2")?;
    let input_string = find(&entry_nodes, "TT InputString")?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a character count"
    )]
    let max_len = input_string
        .locals
        .iter()
        .find(|l| l.name == "Max Size")
        .and_then(|l| float(Some(l)))
        .context("TT InputString: Max Size")? as u32;
    let entry = bb::HighscoreEntryScreen {
        sprites: cx.shown_sprites(&entry_nodes, &[])?,
        title: cx.text(by_target(&entry_nodes, "2D Text", "_Title")?)?,
        score,
        name: cx.text(name_node)?,
        caret_size: float(input(name_node, "Caret Size")).context("2D Text: Caret Size")?,
        max_len,
        ok: cx.text(by_target(&entry_nodes, "2D Text", "_But_1")?)?,
        keys: keys(&entry_nodes, "Secure Key"),
    };

    // The table.
    let table_nodes = in_script("Menu_Highscore");
    let mut title = cx.text(by_target(&table_nodes, "2D Text", "_Title")?)?;
    // The Op joining a language text and the level's number.
    title.text = table_nodes
        .iter()
        .filter(|n| {
            n.building_block == "Op" && n.path.last().map(String::as_str) == Some("Highscore")
        })
        .find_map(|n| cx.localised(n.node, "p1").ok())
        .context("Highscore: the headline's text")?;
    let mut back = cx.text(by_target(&table_nodes, "2D Text", "_But_Back")?)?;
    // After a level the button says *Next* (the selector's first input).
    let selector = find(&table_nodes, "Parameter Selector")?;
    back.text = cx.localised(selector.node, "pIn 0")?;
    let back_text = cx.localised(selector.node, "pIn 1")?;
    let mut places: Vec<(String, &NodeParams)> = table_nodes
        .iter()
        .copied()
        .filter(|n| n.building_block == "2D Text" && n.path.iter().any(|p| p == "Text Display"))
        .filter_map(|n| Some((target(n)?, n)))
        .collect();
    places.sort_by(|a, b| a.0.cmp(&b.0));
    let mut names = Vec::new();
    let mut scores = Vec::new();
    for (_, n) in &places {
        let t = cx.text(n)?;
        if t.align == bb::TextAlign::Left {
            names.push(t);
        } else {
            scores.push(t);
        }
    }
    let name_offset = vector(input(
        find(&table_nodes, "TT ConvertPixel-Homogen")?,
        "Position",
    ))
    .and_then(|v| v.first().copied())
    .context("TT ConvertPixel-Homogen: Position")?;
    let counter = find(&table_nodes, "TT_Counter")?;
    #[expect(clippy::cast_sign_loss, reason = "scan codes")]
    let key_into = |pin: &str| -> Vec<u32> {
        linked_from(&menu, counter.node, pin)
            .into_iter()
            .filter_map(|src| {
                all.iter()
                    .find(|n| n.node == src && n.building_block == "Key Event")
            })
            .filter_map(|n| float(input(n, "Key Waited")))
            .map(|k| k as u32)
            .collect()
    };
    let previous_keys = key_into("Count Down");
    let next_keys = key_into("Count Up");
    let previous_button = linked_from(&menu, counter.node, "Count Down")
        .into_iter()
        .filter_map(|src| {
            all.iter()
                .find(|n| n.node == src && n.building_block == "TT PushButton2")
        })
        .find_map(target)
        .context("TT_Counter: the previous button")?;
    let next_button = linked_from(&menu, counter.node, "Count Up")
        .into_iter()
        .filter_map(|src| {
            all.iter()
                .find(|n| n.node == src && n.building_block == "TT PushButton2")
        })
        .find_map(target)
        .context("TT_Counter: the next button")?;
    let pressed = table_nodes
        .iter()
        .filter(|n| {
            n.building_block == "Set 2D Material" && n.path.iter().any(|p| p == "Highscore")
        })
        .filter_map(|n| text(input(n, "Material")))
        .find(|m| m.contains("Over"))
        .context("Highscore: the pressed arrows' material")?;
    let arrow_pressed = material(install, &menu.file, &pressed)?;
    let showhide = menu.array("Menu_Highscore_ShowHide")?;
    let listed = object_cells(&menu, showhide);
    let mut sprites = cx.shown_sprites(&table_nodes, &listed)?;
    // The entry's black screen stays behind the table.
    for s in &entry.sprites {
        if s.name.contains("BlackScreen") && !sprites.iter().any(|t| t.name == s.name) {
            sprites.push(s.clone());
        }
    }
    sprites.sort_by_key(|s| s.z_order);
    let leave_keys = {
        let top: Vec<&NodeParams> = table_nodes
            .iter()
            .copied()
            .filter(|n| n.path.len() == 2 && n.path[1] == "Secure Key")
            .collect();
        let mut k = keys(&top, "Secure Key");
        k.sort_unstable();
        k.dedup();
        k
    };
    let table = bb::HighscoreTableScreen {
        sprites,
        title,
        back,
        back_text,
        names,
        scores,
        name_offset,
        arrows: [previous_button, next_button],
        arrow_pressed,
        previous_keys,
        next_keys,
        leave_keys,
    };

    Ok(bb::Highscores {
        defaults,
        unlocked,
        default_name,
        entry,
        table,
    })
}

/// The `bb_name` node whose target's name ends in `suffix`.
pub(crate) fn by_target<'a>(
    nodes: &[&'a NodeParams],
    bb_name: &str,
    suffix: &str,
) -> Result<&'a NodeParams> {
    nodes
        .iter()
        .copied()
        .find(|n| n.building_block == bb_name && target(n).is_some_and(|t| t.ends_with(suffix)))
        .with_context(|| format!("{bb_name} on *{suffix} not found"))
}

pub(crate) fn find<'a>(nodes: &[&'a NodeParams], bb_name: &str) -> Result<&'a NodeParams> {
    nodes
        .iter()
        .copied()
        .find(|n| n.building_block == bb_name)
        .with_context(|| format!("{bb_name} not found"))
}

pub(crate) struct Cx<'a> {
    pub(crate) install: &'a Install,
    pub(crate) menu: &'a Loaded,
    pub(crate) all: &'a [NodeParams],
    pub(crate) language: &'a DataArray,
}

impl Cx<'_> {
    /// A *2D Text*'s box, font, alignment and (when fixed) text.
    pub(crate) fn text(&self, n: &NodeParams) -> Result<bb::MenuText> {
        let name = target(n).context("2D Text without a target")?;
        let rect = sprite(self.install, &self.menu.file, &name, true)?.rect;
        let font = source_name(self.menu, n.node, "Font").context("2D Text: Font")?;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "flag bits"
        )]
        let flags = float(input(n, "Alignment")).context("2D Text: Alignment")? as u32;
        let align = match flags {
            a if a & 1 != 0 => bb::TextAlign::Left,
            a if a & 2 != 0 => bb::TextAlign::Right,
            _ => bb::TextAlign::Center,
        };
        let margins = match vector(input(n, "Margins")).as_deref() {
            Some(&[a, b, c, d]) => [a, b, c, d],
            other => bail!("2D Text: Margins {other:?}"),
        };
        // *Text Properties*: bit 5 is *WordWrap*.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "flag bits"
        )]
        let wrap = n
            .locals
            .iter()
            .find(|l| l.name == "Text Properties")
            .and_then(|l| float(Some(l)))
            .is_some_and(|f| f as u32 & 32 != 0);
        Ok(bb::MenuText {
            rect,
            font,
            align,
            margins,
            text: self.localised(n.node, "Text").unwrap_or_default(),
            wrap,
            // Alignment bit 3: top.
            top: flags & 4 != 0,
        })
    }

    /// The texts by language that `node`'s input `pin` shows (a local
    /// `Menu_Init` reads from the language table).
    pub(crate) fn localised(&self, node: ObjRef, pin: &str) -> Result<BTreeMap<String, String>> {
        let row = language_row(self.menu, self.all, node, pin)?;
        let cells = self
            .language
            .rows
            .get(row)
            .with_context(|| format!("language row {row}"))?;
        Ok(self
            .language
            .columns
            .iter()
            .zip(cells)
            .filter(|(c, _)| c.name != "description")
            .filter_map(|(c, cell)| match cell {
                Cell::String(s) => Some((c.name.clone(), s.clone())),
                _ => None,
            })
            .collect())
    }

    /// The 2D entities with a material that the screen's *Init* shows (by
    /// name), and `listed` ones.
    fn shown_sprites(
        &self,
        nodes: &[&NodeParams],
        listed: &[String],
    ) -> Result<Vec<bb::HudSprite>> {
        let mut names: Vec<String> = nodes
            .iter()
            .filter(|n| n.building_block == "Show" && n.path.iter().any(|p| p == "Init"))
            .filter_map(|n| target(n))
            .collect();
        names.extend(listed.iter().cloned());
        let mut out = Vec::new();
        for name in names {
            if out.iter().any(|s: &bb::HudSprite| s.name == name)
                || !has_material(&self.menu.file, &name)
            {
                continue;
            }
            out.push(sprite(self.install, &self.menu.file, &name, true)?);
        }
        out.sort_by_key(|s| s.z_order);
        Ok(out)
    }
}

/// A font as `Menu_Init` makes it: `TT CreateFontEx` `name`, then the *Set
/// Font Properties* it feeds.
pub(crate) fn font(
    install: &Install,
    menu: &Loaded,
    all: &[NodeParams],
    name: &str,
) -> Result<bb::HudFont> {
    let create = all
        .iter()
        .find(|n| {
            n.building_block == "TT_CreateFontEx"
                && text(input(n, "Font Name")).as_deref() == Some(name)
        })
        .with_context(|| format!("{name}'s TT_CreateFontEx"))?;
    // The *Set Font Properties* the creation feeds, else one on the font's
    // local (the credits' fonts are set where they're shown).
    let props = all
        .iter()
        .find(|n| {
            n.building_block == "Set Font Properties"
                && writer_of(&menu.file, &menu.scripts, n.node, "Font") == Some(create.node)
        })
        .or_else(|| {
            all.iter().find(|n| {
                n.building_block == "Set Font Properties"
                    && crate::score::source_name(menu, n.node, "Font").as_deref() == Some(name)
            })
        })
        .with_context(|| format!("{name}'s Set Font Properties"))?;
    let pair = |pin: &str| -> Result<[f32; 2]> {
        match vector(input(props, pin)).as_deref() {
            Some(&[x, y]) => Ok([x, y]),
            other => bail!("{name}: {pin} {other:?}"),
        }
    };
    // A colour that fades (an *Interpolator* feeds it) is its end colour.
    let faded = || {
        let w = writer_of(&menu.file, &menu.scripts, props.node, "Color")?;
        let n = all.iter().find(|n| n.node == w)?;
        vector(input(n, "B"))
    };
    let color = match vector(input(props, "Color")).or_else(faded).as_deref() {
        Some(&[r, g, b, a]) => [r, g, b, a],
        other => bail!("{name}: Color {other:?}"),
    };
    Ok(bb::HudFont {
        texture: texture_key(
            install,
            &menu.file,
            &text(input(create, "Font Texture")).context("Font Texture")?,
        )?,
        glyphs: glyphs(
            menu,
            &text(input(create, "FontCoordinatesData")).context("FontCoordinatesData")?,
        )?,
        scale: pair("Scale")?,
        space: pair("Space")?,
        color,
    })
}

/// The behaviours linked into `node`'s input `pin` in its parent graph.
pub(crate) fn linked_from(g: &Loaded, node: ObjRef, pin: &str) -> Vec<ObjRef> {
    let s = &g.scripts;
    let (Some(&parent), Some(b)) = (s.owner.get(&node), s.behavior(node)) else {
        return Vec::new();
    };
    let Some(input) = b
        .inputs
        .iter()
        .copied()
        .find(|&i| g.file.objects.get(i).and_then(|o| o.name.as_deref()) == Some(pin))
    else {
        return Vec::new();
    };
    let Some(pb) = s.behavior(parent) else {
        return Vec::new();
    };
    pb.links
        .iter()
        .filter_map(|&l| match s.objects.get(l) {
            Some(Some(ScriptObject::Link(k))) => Some(k),
            _ => None,
        })
        .filter(|k| k.target == Some(input))
        .filter_map(|k| s.owner.get(&k.source?).copied())
        .collect()
}

/// Every object a table's cells name, row by row.
fn object_cells(g: &Loaded, t: &DataArray) -> Vec<String> {
    t.rows
        .iter()
        .flatten()
        .filter_map(|c| {
            let r = match c {
                Cell::Object(Some(r)) => *r,
                // An object cell kept as a parameter.
                Cell::Parameter(Some(p)) => {
                    let value = match g.scripts.objects.get(*p)? {
                        Some(ScriptObject::Local(l) | ScriptObject::Parameter(l)) => &l.value,
                        Some(ScriptObject::Out(o)) => &o.value,
                        _ => return None,
                    };
                    match value {
                        ckscript::ParamValue::Object(Some(r)) => *r,
                        _ => return None,
                    }
                }
                _ => return None,
            };
            g.file.objects.get(r)?.name.clone()
        })
        .collect()
}

pub(crate) fn has_material(file: &CkFile, name: &str) -> bool {
    object(file, name, ClassId::ENTITY_2D)
        .ok()
        .and_then(|i| match decode_object(&file.objects[i]) {
            Some(Ok(Decoded::Entity2d(e))) => Some(e.material.is_some()),
            _ => None,
        })
        .unwrap_or(false)
}

/// A material's texture key and diffuse colour.
pub(crate) fn material(
    install: &Install,
    file: &CkFile,
    name: &str,
) -> Result<(Option<String>, [f32; 4])> {
    let i = object(file, name, ClassId::MATERIAL)?;
    let Some(Ok(Decoded::Material(m))) = decode_object(&file.objects[i]) else {
        bail!("{name} isn't a material");
    };
    let texture = m
        .texture
        .map(|t| texture_key_at(install, file, t))
        .transpose()?;
    Ok((texture, ckscene::argb_to_rgba(m.diffuse)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_highscores() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let h = extract(&install).unwrap();
        assert_eq!(h.defaults.len(), 12);
        assert_eq!(h.defaults[0].len(), 10);
        assert_eq!(h.defaults[0][0].score, 4000);
        assert_eq!(h.defaults[11][0].name, "Mrs. Default");
        assert_eq!(h.unlocked.iter().filter(|&&u| u).count(), 1);
        assert!(h.unlocked[0]);

        let e = &h.entry;
        assert_eq!(e.title.text["english"], "New highscore entry!");
        assert_eq!(e.title.font, "GameFont_01");
        assert!(e.title.wrap && !e.score.wrap);
        assert_eq!(e.name.font, "GameFont_02");
        assert_eq!(e.name.align, bb::TextAlign::Left);
        assert_eq!(e.max_len, 9);
        assert_eq!(e.keys, [28]);
        assert!(!e.score.text.is_empty());
        assert!(!e.ok.text.is_empty());
        let names: Vec<&str> = e.sprites.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"M_BlackScreen"), "{names:?}");

        let t = &h.table;
        assert_eq!(t.title.text["english"].trim(), "Highscore Level");
        assert_eq!(t.back.text["english"], "Next");
        assert_eq!(t.back_text["english"].trim(), "Back");
        assert_eq!(t.names.len(), 10);
        assert_eq!(t.scores.len(), 10);
        assert_eq!(t.scores[0].align, bb::TextAlign::Right);
        assert!(t.names[0].rect[1] < t.names[9].rect[1]);
        assert!((t.name_offset - 0.035).abs() < 1e-6);
        assert_eq!(t.arrows, ["M_Highscore_But_1", "M_Highscore_But_2"]);
        assert_eq!(t.previous_keys, [203]);
        assert_eq!(t.next_keys, [205]);
        assert_eq!(t.leave_keys, [1, 28]);
        let names: Vec<&str> = t.sprites.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"M_Highscore_Number10"), "{names:?}");
        assert!(names.contains(&"M_BlackScreen"), "{names:?}");
    }
}
