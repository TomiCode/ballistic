//! Extracts the credits screen (see the research repo, findings 28) from
//! `Menu.nmo`'s `Menu_Credits`: the logos and their fades (*Fade Sprite In
//! and Out*), the pages (`Menu_Credits_Strings`), their box, fonts and
//! timing (*Text Fader*: the hold's *Calculator* expression, *1st shorter*),
//! the cheat (*Cheat*'s *Test*) and the keys.

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckscript::ObjRef;
use ckscript::extract::NodeParams;

use crate::highscores::Cx;
use crate::hud::sprite;
use crate::score::{keys, source_name};
use crate::transformers::{float, input, target, text};

#[allow(clippy::too_many_lines, reason = "one pass over Menu_Credits")]
pub fn extract(cx: &Cx<'_>) -> Result<bb::CreditsScreen> {
    let nodes: Vec<&NodeParams> = cx
        .all
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some("Menu_Credits"))
        .collect();
    let owner = |n: &NodeParams| cx.menu.scripts.owner.get(&n.node).copied();
    let in_graph = |graph: Option<ObjRef>, bb_name: &str| -> Vec<&NodeParams> {
        nodes
            .iter()
            .copied()
            .filter(|n| n.building_block == bb_name && owner(n) == graph)
            .collect()
    };
    let num = |n: &NodeParams, pin: &str| {
        float(input(n, pin)).with_context(|| format!("{}: {pin}", n.building_block))
    };

    // Each logo: *Show*, a fade in, a *Delayer*, a fade out (A 1 → B 0).
    let mut logos = Vec::new();
    let mut logo_fades_ms = Vec::new();
    for show in nodes.iter().filter(|n| {
        n.building_block == "Show" && n.path.iter().any(|p| p == "Fade Sprite In and Out")
    }) {
        let graph = owner(show);
        let name = target(show).context("Fade Sprite In and Out: no logo")?;
        let mut fade_in = None;
        let mut fade_out = None;
        for p in in_graph(graph, "Linear Progression") {
            let d = num(p, "Duration")?;
            if num(p, "A")? < num(p, "B")? {
                fade_in = Some(d);
            } else {
                fade_out = Some(d);
            }
        }
        let hold = in_graph(graph, "Delayer")
            .first()
            .and_then(|d| float(input(d, "Time to Wait")))
            .context("Fade Sprite In and Out: the hold")?;
        logo_fades_ms.push([
            fade_in.context("a logo's fade in")?,
            hold,
            fade_out.context("a logo's fade out")?,
        ]);
        logos.push(sprite(cx.install, &cx.menu.file, &name, true)?);
    }
    anyhow::ensure!(!logos.is_empty(), "Menu_Credits: no logos");

    // The pages.
    let table = cx.menu.array("Menu_Credits_Strings")?;
    let pages = table
        .rows
        .iter()
        .map(|r| {
            let row = cx.menu.row(table, r);
            Ok([row.string("Title Strings")?, row.string("Text Strings")?])
        })
        .collect::<Result<Vec<_>>>()?;
    let texts: Vec<&NodeParams> = nodes
        .iter()
        .copied()
        .filter(|n| {
            n.building_block == "2D Text" && target(n).is_some_and(|t| t.contains("Credits_Text"))
        })
        .collect();
    let font_of = |n: &NodeParams| source_name(cx.menu, n.node, "Font");
    let title_font = texts
        .iter()
        .filter_map(|n| font_of(n))
        .find(|f| f.contains("Big"))
        .context("Menu_Credits: the title font")?;
    let text_font = texts
        .iter()
        .filter_map(|n| font_of(n))
        .find(|f| f.contains("Small"))
        .context("Menu_Credits: the text font")?;
    let text_box = cx.text(texts.first().context("Menu_Credits: no text")?)?;

    // The timing: the hold is `a*X+Y` (a: the text's length); the first page
    // is `a-Z` (Z ms) shorter.
    let calc = |pred: &dyn Fn(&str) -> bool| {
        nodes
            .iter()
            .filter(|n| n.building_block == "Calculator")
            .filter_map(|n| text(input(n, "expression")))
            .find(|e| pred(e))
    };
    let hold = calc(&|e| e.contains('*')).context("Text Fader: the hold's expression")?;
    let Some((per_char, base)) = hold
        .strip_prefix("a*")
        .and_then(|r| r.split_once('+'))
        .and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)))
    else {
        bail!("Text Fader: unexpected hold {hold:?}");
    };
    let shorter = calc(&|e| e.starts_with("a-"))
        .and_then(|e| e[2..].parse::<f32>().ok())
        .context("1st shorter: the expression")?;
    let fade_ms = nodes
        .iter()
        .find(|n| {
            n.building_block == "Linear Progression"
                && n.path.iter().any(|p| p == "Fade In and Out")
        })
        .and_then(|n| float(input(n, "Duration")))
        .context("Fade In and Out: the fade")?;
    let pause_ms = nodes
        .iter()
        .find(|n| {
            n.building_block == "Delayer" && n.path.last().map(String::as_str) == Some("Text Fader")
        })
        .and_then(|n| float(input(n, "Time to Wait")))
        .context("Text Fader: the pause")?;
    let cheat = nodes
        .iter()
        .find(|n| n.building_block == "Test" && n.path.iter().any(|p| p == "Cheat"))
        .and_then(|n| text(input(n, "B")))
        .context("Cheat: the word")?;
    let back = nodes
        .iter()
        .find(|n| {
            n.building_block == "2D Text" && target(n).is_some_and(|t| t.ends_with("_But_Back"))
        })
        .context("Menu_Credits: the back button")?;
    let back_name = target(back).context("2D Text target")?;
    Ok(bb::CreditsScreen {
        back: bb::MenuButton {
            sprite: sprite(cx.install, &cx.menu.file, &back_name, true)?,
            text: cx.text(back)?,
        },
        logos,
        logo_fades_ms,
        pause_ms,
        pages,
        text: text_box,
        title_font,
        text_font,
        fade_ms,
        hold_per_char_ms: per_char,
        hold_ms: base,
        first_shorter_ms: shorter,
        cheat,
        keys: keys(&nodes, "Secure Key"),
    })
}

#[cfg(test)]
mod tests {
    use crate::install::Install;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_credits() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let m = crate::menus::extract(&install).unwrap();
        let c = &m.credits;
        assert_eq!(c.logos.len(), 2);
        assert_eq!(c.logo_fades_ms.len(), 2);
        assert!(
            c.logo_fades_ms.iter().any(|f| f[2] > 1500.0),
            "{:?}",
            c.logo_fades_ms
        );
        assert_eq!(c.pages.len(), 23);
        assert!(c.pages[0][0].contains("Ballance"));
        assert_eq!(c.title_font, "GameFont_Credits_Big");
        assert_eq!(c.text_font, "GameFont_Credits_Small");
        assert!(m.fonts.contains_key("GameFont_Credits_Big"));
        assert!((c.fade_ms - 500.0).abs() < 1e-3);
        assert!((c.hold_per_char_ms - 50.0).abs() < 1e-3);
        assert!((c.hold_ms - 1500.0).abs() < 1e-3);
        assert!((c.first_shorter_ms - 2000.0).abs() < 1e-3);
        assert!((c.pause_ms - 1000.0).abs() < 1e-3);
        assert_eq!(c.cheat, "gravity");
        assert_eq!(c.keys, [1, 28]);
        assert!(c.text.top && c.text.wrap);
    }
}
