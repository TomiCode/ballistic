//! Extracts the menus (see the research repo, findings 25) from `Menu.nmo`:
//! - the fonts `Menu_Init` makes (each `TT CreateFontEx` and the *Set Font
//!   Properties* it feeds), and the buttons' materials;
//! - each button menu: its buttons (its `Menu_*_ShowHide` table, or the
//!   buttons it shows), their texts (a *2D Text* on the button, or the level
//!   select's *Button* sub-graphs), the row active at first (the table's
//!   roll-over column), the row Escape picks (the *Identity* a *Secure Key*
//!   feeds in its *Keyboard* graph), the sprites it shows and its keys;
//! - `Menu_YesNo`, and the questions the other menus give it (*Set Row* on
//!   `Menu_ScriptParameters`).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckscript::extract::{Extractor, NodeParams};
use ckscript::{Cell, ObjRef, ParamValue, ScriptObject};

use crate::highscores::{Cx, font, has_material, linked_from, material};
use crate::hud::{pin_param, sprite};
use crate::install::Install;
use crate::physics::{Loaded, load};
use crate::score::{keys, language_row, source_name};
use crate::transformers::{float, input, target};

/// The fonts the menus use.
const FONTS: [&str; 5] = [
    "GameFont_01",
    "GameFont_02",
    "GameFont_03",
    "GameFont_03a",
    "GameFont_04",
];

pub fn extract(install: &Install) -> Result<bb::Menus> {
    let menu = load(install, "3d entities/menu.nmo")?;
    let ex = Extractor::new(&menu.file, &menu.scripts);
    let all = ex.nodes(&|_| true);
    let language = load(install, "3d entities/language.nmo")?;
    let cx = Cx {
        install,
        menu: &menu,
        all: &all,
        language: language.array("language")?,
    };
    let fonts = FONTS
        .iter()
        .map(|&name| Ok((name.to_owned(), font(install, &menu, &all, name)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let dead_buttons = ["M_Dead_But_1", "M_Dead_But_2"].map(str::to_owned);
    Ok(bb::Menus {
        fonts,
        button_up: material(install, &menu.file, "M_Button_Up")?,
        button_over: material(install, &menu.file, "M_Button_Over")?,
        button_inactive: material(install, &menu.file, "M_Button_Inactive")?,
        main: button_menu(&cx, "Menu_Main", Buttons::Table("Menu_Main_ShowHide"))?,
        start: button_menu(&cx, "Menu_Start", Buttons::Table("Menu_Start_ShowHide"))?,
        pause: button_menu(&cx, "Menu_Pause", Buttons::Table("Menu_Pause_ShowHide"))?,
        end: button_menu(&cx, "Menu_End", Buttons::Table("Menu_End_ShowHide"))?,
        dead: button_menu(&cx, "Menu_Dead", Buttons::Named(&dead_buttons))?,
        options: button_menu(&cx, "Menu_Options", Buttons::Table("Menu_Options_ShowHide"))?,
        yes_no: yes_no(&cx)?,
        settings: crate::options::extract(&cx)?,
    })
}

#[derive(Clone, Copy)]
enum Buttons<'a> {
    /// A `Menu_*_ShowHide` table: buttons in its first column, the active
    /// one marked in its roll-over column.
    Table(&'a str),
    Named(&'a [String]),
}

fn script_nodes<'a>(cx: &Cx<'a>, script: &str) -> Vec<&'a NodeParams> {
    cx.all
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some(script))
        .collect()
}

fn button_menu(cx: &Cx<'_>, script: &str, buttons: Buttons<'_>) -> Result<bb::ButtonMenu> {
    let nodes = script_nodes(cx, script);
    let (names, first) = match buttons {
        Buttons::Table(t) => {
            let table = cx.menu.array(t)?;
            let names = table
                .rows
                .iter()
                .filter_map(|r| match r.first() {
                    Some(Cell::Object(Some(o))) => cx.menu.file.objects.get(*o)?.name.clone(),
                    Some(Cell::Parameter(Some(p))) => object_value(cx.menu, *p),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let roll = table
                .columns
                .iter()
                .position(|c| c.name.starts_with("RollOver"))
                .with_context(|| format!("{t}: no roll-over column"))?;
            let first = table
                .rows
                .iter()
                .position(|r| matches!(r.get(roll), Some(Cell::Int(1))))
                .unwrap_or(0);
            (names, first)
        }
        Buttons::Named(n) => (n.to_vec(), 0),
    };
    anyhow::ensure!(!names.is_empty(), "{script}: no buttons");
    // The level select's unlocked and locked fonts (a *Parameter Selector*
    // between two fonts).
    let fonts = nodes.iter().find_map(|n| {
        if n.building_block != "Parameter Selector" {
            return None;
        }
        let locked = source_name(cx.menu, n.node, "pIn 0")?;
        let open = source_name(cx.menu, n.node, "pIn 1")?;
        (locked.starts_with("GameFont") && open.starts_with("GameFont")).then_some((open, locked))
    });
    let buttons = names
        .iter()
        .map(|name| button(cx, &nodes, name, fonts.as_ref().map(|f| f.0.as_str())))
        .collect::<Result<Vec<_>>>()?;
    // Sprites it shows that aren't its buttons; a fixed text that isn't on
    // a button is its title.
    let mut sprites = Vec::new();
    for n in &nodes {
        let Some(t) = target(n) else { continue };
        if n.building_block != "Show"
            || names.contains(&t)
            || sprites.iter().any(|s: &bb::HudSprite| s.name == t)
            || !has_material(&cx.menu.file, &t)
        {
            continue;
        }
        sprites.push(sprite(cx.install, &cx.menu.file, &t, true)?);
    }
    sprites.sort_by_key(|s| s.z_order);
    let title = nodes
        .iter()
        .filter(|n| n.building_block == "2D Text")
        .filter(|n| target(n).is_some_and(|t| !names.contains(&t)))
        .find_map(|n| cx.text(n).ok().filter(|t| !t.text.is_empty()));
    Ok(bb::ButtonMenu {
        sprites,
        title,
        buttons,
        first,
        escape: escape_row(cx, &nodes),
        locked_font: fonts.map(|f| f.1),
        keys: keys(&nodes, script),
    })
}

/// A button's sprite and text: a *2D Text* on it, or a *Button* sub-graph
/// given it (`Pin 0`) and its text (`Pin 1`).
fn button(
    cx: &Cx<'_>,
    nodes: &[&NodeParams],
    name: &str,
    open_font: Option<&str>,
) -> Result<bb::MenuButton> {
    let sprite = sprite(cx.install, &cx.menu.file, name, true)?;
    // (A *2D Text* in a sub-graph that shares the graph's input names the
    // button too; its font and text then come from the graph.)
    if let Some(n) = nodes
        .iter()
        .find(|n| n.building_block == "2D Text" && target(n).as_deref() == Some(name))
        && let Ok(text) = cx.text(n)
        && !text.text.is_empty()
        && text.font.starts_with("GameFont")
    {
        return Ok(bb::MenuButton { text, sprite });
    }
    // The *2D Text* inside the *Button* graph whose `Pin 0` is this button.
    let (inner, graph) = nodes
        .iter()
        .filter(|n| n.building_block == "2D Text")
        .find_map(|n| {
            let graph = *cx.menu.scripts.owner.get(&n.node)?;
            let pin = pin_param(&cx.menu.file, &cx.menu.scripts, graph, "Pin 0")?;
            let Some(Some(ScriptObject::In(i))) = cx.menu.scripts.objects.get(pin) else {
                return None;
            };
            (object_value(cx.menu, i.source?).as_deref() == Some(name)).then_some((*n, graph))
        })
        .with_context(|| format!("{name}: no text"))?;
    let font = source_name(cx.menu, inner.node, "Font")
        .filter(|f| f.starts_with("GameFont"))
        .or_else(|| open_font.map(str::to_owned))
        .with_context(|| format!("{name}: no font"))?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "flag bits"
    )]
    let align = match float(input(inner, "Alignment")).unwrap_or(0.0) as u32 {
        a if a & 1 != 0 => bb::TextAlign::Left,
        a if a & 2 != 0 => bb::TextAlign::Right,
        _ => bb::TextAlign::Center,
    };
    Ok(bb::MenuButton {
        text: bb::MenuText {
            rect: sprite.rect,
            font,
            align,
            margins: [2.0; 4],
            text: cx.localised(graph, "Pin 1")?,
            wrap: false,
            top: false,
        },
        sprite,
    })
}

/// The object a parameter (a local, or an output) holds.
fn object_value(g: &Loaded, p: ObjRef) -> Option<String> {
    let value = match g.scripts.objects.get(p)? {
        Some(ScriptObject::Local(l) | ScriptObject::Parameter(l)) => &l.value,
        Some(ScriptObject::Out(o)) => &o.value,
        _ => return None,
    };
    match value {
        ParamValue::Object(Some(r)) => g.file.objects.get(*r)?.name.clone(),
        _ => None,
    }
}

/// The row Escape picks: in the *Keyboard* graph, the *Identity* a *Secure
/// Key* feeds sets the active row (Enter's *Secure Key* leaves the graph).
fn escape_row(cx: &Cx<'_>, nodes: &[&NodeParams]) -> Option<usize> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a row"
    )]
    nodes
        .iter()
        .filter(|n| n.building_block == "Identity" && n.path.iter().any(|p| p == "Keyboard"))
        .find(|n| {
            linked_from(cx.menu, n.node, "In").iter().any(|&src| {
                cx.menu
                    .file
                    .objects
                    .get(src)
                    .and_then(|o| o.name.as_deref())
                    == Some("Secure Key")
            })
        })
        .and_then(|n| float(input(n, "pIn 0")))
        .map(|r| r as usize)
}

fn yes_no(cx: &Cx<'_>) -> Result<bb::YesNoMenu> {
    let nodes = script_nodes(cx, "Menu_YesNo");
    let on = |suffix: &str| {
        nodes
            .iter()
            .copied()
            .find(|n| {
                n.building_block == "2D Text" && target(n).is_some_and(|t| t.ends_with(suffix))
            })
            .with_context(|| format!("Menu_YesNo: 2D Text on *{suffix}"))
    };
    let yes_no_button = |suffix: &str| -> Result<bb::MenuButton> {
        let n = on(suffix)?;
        let name = target(n).context("2D Text target")?;
        Ok(bb::MenuButton {
            sprite: sprite(cx.install, &cx.menu.file, &name, true)?,
            text: cx.text(n)?,
        })
    };
    let mut sprites = Vec::new();
    for n in nodes.iter().filter(|n| n.building_block == "Show") {
        let Some(t) = target(n) else { continue };
        if t.contains("_But_") || !has_material(&cx.menu.file, &t) {
            continue;
        }
        sprites.push(sprite(cx.install, &cx.menu.file, &t, true)?);
    }
    // The questions: each menu's *Set Row* on the parameters table.
    let mut questions = BTreeMap::new();
    for n in cx.all.iter().filter(|n| n.building_block == "Set Row") {
        let Some(local) = source_name(cx.menu, n.node, "Menu Text") else {
            continue;
        };
        if language_row(cx.menu, cx.all, n.node, "Menu Text").is_err() {
            continue;
        }
        questions.insert(local, cx.localised(n.node, "Menu Text")?);
    }
    Ok(bb::YesNoMenu {
        sprites,
        text: cx.text(on("_TextSprite")?)?,
        yes: yes_no_button("_But_Yes")?,
        no: yes_no_button("_But_No")?,
        questions,
        keys: keys(&nodes, "Menu_YesNo"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_menus() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let m = extract(&install).unwrap();
        assert_eq!(m.fonts.len(), 5);
        assert_eq!(m.fonts["GameFont_01"].scale, [0.45, 0.55]);
        assert!(m.button_over.0.is_some());
        let text = |b: &bb::MenuButton| b.text.text["english"].clone();
        let main: Vec<String> = m.main.buttons.iter().map(text).collect();
        assert_eq!(main.len(), 5, "{main:?}");
        assert_eq!(main[0], "Start");
        assert_eq!(m.main.first, 0);
        assert_eq!(m.main.escape, Some(4));
        assert!(m.main.keys.contains(&200) && m.main.keys.contains(&208));

        assert_eq!(m.start.buttons.len(), 13);
        assert_eq!(text(&m.start.buttons[0]), "Level 1");
        assert_eq!(m.start.buttons[0].text.font, "GameFont_03");
        assert_eq!(m.start.locked_font.as_deref(), Some("GameFont_03a"));
        assert_eq!(m.start.buttons[12].text.font, "GameFont_01");

        assert_eq!(m.pause.buttons.len(), 5);
        assert_eq!(m.pause.first, 4);
        assert_eq!(m.end.buttons.len(), 5);
        assert_eq!(m.dead.buttons.len(), 2);
        assert_eq!(m.dead.escape, None);
        assert_eq!(m.options.buttons.len(), 4);
        assert!(m.options.title.is_some());
        let names: Vec<&str> = m.main.sprites.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"M_BlackScreen"), "{names:?}");

        let y = &m.yes_no;
        assert!(
            y.questions.contains_key("Exit Game?"),
            "{:?}",
            y.questions.keys()
        );
        assert!(y.questions.contains_key("Restart Level?"));
        assert!(y.keys.contains(&203) && y.keys.contains(&205));
        assert!(!text(&y.yes).is_empty() && !text(&y.no).is_empty());
    }
}
