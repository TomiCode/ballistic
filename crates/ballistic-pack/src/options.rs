//! Extracts the options' screens (see the research repo, findings 27) from
//! `Menu.nmo`'s `Menu_Opt_Graphics`, `Menu_Opt_Keys` and `Menu_Opt_Sound`.
//!
//! Each screen's rows are found by their 2D entities' names (the scripts
//! address them directly); their texts are the scripts' *2D Text*s on them.
//! The screen sizes come from `Dummy_ScreenModes`, the default keys from
//! `Keyboard`, the volume's step from the *Volume* graph.

use anyhow::{Context, Result};
use ballistic_bundle as bb;
use ckscript::extract::NodeParams;

use crate::highscores::{Cx, material};
use crate::hud::sprite;
use crate::transformers::{float, input, target};

/// A row's setting and its sprites (by name): the field, then the kind's.
enum Row {
    Choice(&'static str, [&'static str; 4]),
    YesNo(&'static str, [&'static str; 3]),
    Key(&'static str, [&'static str; 2]),
}

#[allow(
    clippy::too_many_lines,
    reason = "the three screens' rows, then the tables"
)]
pub fn extract(cx: &Cx<'_>) -> Result<bb::OptionsScreens> {
    use Row::{Choice, Key, YesNo};
    let graphics = screen(
        cx,
        "Menu_Opt_Graphics",
        "M_Opt_Graph_Title",
        "M_Opt_Gra_Back",
        &[
            Choice(
                "resolution",
                [
                    "M_Opt_Gra_ResField",
                    "M_Opt_Gra_ResText",
                    "M_Opt_Gra_ResButLeft",
                    "M_Opt_Gra_ResButRight",
                ],
            ),
            YesNo(
                "vsync",
                [
                    "M_Opt_Graph_SynchField",
                    "M_Opt_Gra_SynchYes",
                    "M_Opt_Gra_SynchNo",
                ],
            ),
            YesNo(
                "clouds",
                [
                    "M_Opt_Gra_CloudsField",
                    "M_Opt_Gra_CloudsYes",
                    "M_Opt_Gra_CloudsNo",
                ],
            ),
        ],
    )?;
    let controls = screen(
        cx,
        "Menu_Opt_Keys",
        "M_Opt_Keys_Title",
        "M_Opt_Keys_But_Back",
        &[
            Key("key_forward", ["M_Opt_Keys_Field1", "M_Opt_Keys_Key1"]),
            Key("key_backward", ["M_Opt_Keys_Field2", "M_Opt_Keys_Key2"]),
            Key("key_left", ["M_Opt_Keys_Field3", "M_Opt_Keys_Key3"]),
            Key("key_right", ["M_Opt_Keys_Field4", "M_Opt_Keys_Key4"]),
            Key("key_rotate", ["M_Opt_Keys_Field5", "M_Opt_Keys_Key5"]),
            Key("key_lift", ["M_Opt_Keys_Field6", "M_Opt_Keys_Key6"]),
            YesNo(
                "invert_rotation",
                [
                    "M_Opt_Keys_Inv_Field",
                    "M_Opt_Keys_Inv_Yes",
                    "M_Opt_Keys_Inv_No",
                ],
            ),
        ],
    )?;
    let sound = screen(
        cx,
        "Menu_Opt_Sound",
        "M_Opt_Sound_Title",
        "M_Opt_Sound_But_Back",
        &[Choice(
            "volume",
            [
                "M_Opt_Sound_VolField",
                "M_Opt_Sound_VolText",
                "M_Opt_Sound_VolButLeft",
                "M_Opt_Sound_VolButRight",
            ],
        )],
    )?;

    let modes = cx.menu.array("Dummy_ScreenModes")?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "pixel sizes"
    )]
    let resolutions = modes
        .rows
        .iter()
        .map(|r| {
            let row = cx.menu.row(modes, r);
            Ok([row.f32("Width")? as u32, row.f32("Height")? as u32])
        })
        .collect::<Result<_>>()?;
    let keyboard = cx.menu.array("Keyboard")?;
    let first = keyboard.rows.first().context("Keyboard is empty")?;
    #[expect(clippy::cast_sign_loss, reason = "scan codes")]
    let default_keys = ["up", "down", "left", "right", "rotation", "camup"]
        .iter()
        .map(|c| Ok(cx.menu.row(keyboard, first).int(c)? as u32))
        .collect::<Result<_>>()?;
    let step = cx
        .all
        .iter()
        .find(|n| {
            n.path.first().map(String::as_str) == Some("Menu_Opt_Sound")
                && n.building_block == "Op"
                && n.path.last().map(String::as_str) == Some("Volume")
                && input(n, "p2").is_some_and(|v| v.name == "p2")
                && float(input(n, "p2")).is_some_and(|f| f > 1.0)
        })
        .and_then(|n| float(input(n, "p2")))
        .context("Menu_Opt_Sound: the volume's step")?;
    Ok(bb::OptionsScreens {
        graphics,
        controls,
        sound,
        field_active: material(cx.install, &cx.menu.file, "M_FieldActive")?,
        field_inactive: material(cx.install, &cx.menu.file, "M_FieldInActive")?,
        key_waiting: material(cx.install, &cx.menu.file, "M_Keys_Highlight")?,
        resolutions,
        default_keys,
        // The volume's steps are percents.
        volume_step: step / 100.0,
    })
}

fn screen(
    cx: &Cx<'_>,
    script: &str,
    title: &str,
    ok: &str,
    rows: &[Row],
) -> Result<bb::OptionsScreen> {
    let nodes: Vec<&NodeParams> = cx
        .all
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some(script))
        .collect();
    let text_on = |name: &str| -> Result<bb::MenuText> {
        let n = nodes
            .iter()
            .find(|n| n.building_block == "2D Text" && target(n).as_deref() == Some(name))
            .with_context(|| format!("{script}: no text on {name}"))?;
        cx.text(n)
    };
    let spr = |name: &str| sprite(cx.install, &cx.menu.file, name, true);
    let button = |name: &str| -> Result<bb::MenuButton> {
        Ok(bb::MenuButton {
            sprite: spr(name)?,
            text: text_on(name)?,
        })
    };
    let rows = rows
        .iter()
        .map(|r| {
            let (setting, field) = match r {
                Row::Choice(s, n) => (s, n[0]),
                Row::YesNo(s, n) => (s, n[0]),
                Row::Key(s, n) => (s, n[0]),
            };
            let kind = match r {
                Row::Choice(_, [_, value, left, right]) => bb::OptionKind::Choice {
                    value: text_on(value)?,
                    left: spr(left)?,
                    right: spr(right)?,
                },
                Row::YesNo(_, [_, yes, no]) => bb::OptionKind::YesNo {
                    yes: button(yes)?,
                    no: button(no)?,
                },
                Row::Key(_, [_, key]) => bb::OptionKind::Key { key: text_on(key)? },
            };
            Ok(bb::OptionRow {
                setting: (*setting).to_owned(),
                field: spr(field)?,
                label: text_on(field)?,
                kind,
            })
        })
        .collect::<Result<_>>()?;
    Ok(bb::OptionsScreen {
        title: text_on(title)?,
        rows,
        ok: button(ok)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::install::Install;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_options() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let o = crate::menus::extract(&install).unwrap().settings;
        let label = |r: &ballistic_bundle::OptionRow| r.label.text["english"].trim().to_owned();
        assert_eq!(o.graphics.rows.len(), 3);
        assert_eq!(label(&o.graphics.rows[0]), "Screen Resolution");
        assert!(o.graphics.rows[0].label.top);
        assert_eq!(o.controls.rows.len(), 7);
        assert_eq!(label(&o.controls.rows[0]), "Forward");
        assert_eq!(o.controls.rows[5].setting, "key_lift");
        assert_eq!(o.sound.rows.len(), 1);
        assert_eq!(o.sound.rows[0].setting, "volume");
        assert_eq!(o.graphics.ok.text.text["english"], "OK");
        assert_eq!(o.resolutions.first(), Some(&[640, 480]));
        assert_eq!(o.resolutions.len(), 6);
        assert_eq!(o.default_keys, [200, 208, 203, 205, 42, 57]);
        assert!((o.volume_step - 0.1).abs() < 1e-6);
        assert!(!o.graphics.title.text.is_empty());
    }
}
