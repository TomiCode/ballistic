//! Extracts level 1's tutorial (see the research repo, findings 29) from
//! `Gameplay.nmo`'s `Gameplay_Tutorial`, `Tutorial.nmo` and the install's
//! `Text/Tutorial<n>.txt`:
//! - the texts: each language's file split at `*` (`TT_SplitString`), the
//!   language number being its column in `Language.nmo`'s table;
//! - the font (`Font_Tutorial`: `TT CreateFontEx` on the texture and glyph
//!   table that `Op`s look up by name, and the *Set Font Properties* it
//!   feeds), the text's *2D Text* box and the dark box behind it;
//! - the timings: the first text's Delayer, the fades, the Delayer between
//!   chapters, the intro's time-out;
//! - the chapters, by the two *Switch On Parameter*s on the chapter number:
//!   `Kapitel Wait`'s graphs (the object and distance of each *TT Scaleable
//!   Proximity*, the ball type the ramp's test skips) and `Kapitel
//!   Aktion`'s (the arrows each shows, the distances that end them).

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckscript::extract::{Extractor, NodeParams};
use ckscript::{ObjRef, ScriptObject};

use crate::hud::{glyphs, group_members, pin_param, sprite, texture_key};
use crate::install::Install;
use crate::physics::{Loaded, load};
use crate::score::source_name;
use crate::transformers::{float, input, text, vector};

const SCRIPT: &str = "Gameplay_Tutorial";

#[expect(clippy::too_many_lines, reason = "reads each part in turn")]
pub fn extract(install: &Install) -> Result<bb::Tutorial> {
    let g = load(install, "3d entities/gameplay.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let all = ex.nodes(&|_| true);
    let nodes: Vec<&NodeParams> = all
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some(SCRIPT))
        .collect();
    let t = Tut {
        g: &g,
        nodes: &nodes,
    };
    let tutorial = load(install, "3d entities/tutorial.nmo")?;
    let menu = load(install, "3d entities/menu.nmo")?;

    // The font: the texture and glyph table are looked up by name.
    let create = t.one("TT_CreateFontEx", &[])?;
    let looked_up = |pin: &str| -> Result<String> {
        let op = t
            .local_writer(create.node, pin)
            .with_context(|| format!("TT CreateFontEx: nothing writes {pin}"))?;
        // Looked up by another script (`Gameplay_Ingame`'s init).
        let op = all
            .iter()
            .find(|n| n.node == op)
            .context("TT CreateFontEx: the lookup isn't in a script")?;
        text(input(op, "p1")).with_context(|| format!("TT CreateFontEx: {pin}'s name"))
    };
    let props = t.one("Set Font Properties", &["Init"])?;
    let pair = |pin: &str| -> Result<[f32; 2]> {
        match vector(input(props, pin)).as_deref() {
            Some(&[x, y]) => Ok([x, y]),
            other => bail!("Font_Tutorial: {pin} {other:?}"),
        }
    };
    let font = bb::HudFont {
        texture: texture_key(install, &menu.file, &looked_up("Font Texture")?)?,
        glyphs: glyphs(&menu, &looked_up("FontCoordinatesData")?)?,
        scale: pair("Scale")?,
        space: pair("Space")?,
        color: color(props, "Color")?,
    };

    // The text's box: the *2D Text* in `Tutorial Text`.
    let text_node = t.one("2D Text", &["Tutorial Text"])?;
    let text_entity = t.target_lookup(text_node)?;
    let margins = match vector(input(text_node, "Margins")).as_deref() {
        Some(&[a, b, c, d]) => [a, b, c, d],
        other => bail!("2D Text: Margins {other:?}"),
    };
    let text_box = bb::MenuText {
        rect: sprite(install, &tutorial.file, &text_entity, false)?.rect,
        font: "Font_Tutorial".into(),
        // *Top-Left*, *WordWrap*.
        align: bb::TextAlign::Left,
        margins,
        text: BTreeMap::new(),
        wrap: true,
        top: true,
    };
    let back_fade = t.one("Interpolator", &["show Interface_Back"])?;
    let mut back = sprite(install, &tutorial.file, "Tutorial_Interface_Back", false)?;
    back.color = color(back_fade, "B")?;

    // The intro's *wait for continue* Timer.
    let quit_after_ms = num(t.one("Timer", &["wait for continue"])?, "Duration")?;
    let delay = |path: &[&str]| -> Result<f32> {
        let n = nodes
            .iter()
            .find(|n| n.building_block == "Delayer" && path_is(n, path))
            .with_context(|| format!("{path:?}: Delayer not found"))?;
        num(n, "Time to Wait")
    };

    let chapters = t.chapters(&tutorial)?;
    let grow = t.one("Interpolator", &["MoveKeys", "max Scale"])?;
    let grow = match vector(input(grow, "B")).as_deref() {
        Some(&[x, y, z]) => [x, y, z],
        other => bail!("max Scale: B {other:?}"),
    };
    // The Wave Player's target: a local named after the sound.
    let wave = t.one("Wave Player", &["Kapitel Aktion", "Physic on/off"])?;
    let wave = t
        .target_local(wave)
        .and_then(|l| g.file.objects.get(l)?.name.clone())
        .context("Physic on/off: Wave Player's sound")?;

    Ok(bb::Tutorial {
        level: 1,
        scene: "tutorial".into(),
        texts: texts(install)?,
        font,
        text: text_box,
        back,
        fade_ms: num(
            t.one("Linear Progression", &["Tutorial Text", "FadeIn"])?,
            "Duration",
        )?,
        back_delay_ms: delay(&["Kapitel Aktion", "hide Interface_Back"])?,
        start_ms: delay(&[])?,
        next_ms: delay(&["Kapitel Aktion"])?,
        arrow_fade_ms: num(
            t.one("Linear Progression", &["Kapitel Aktion", "FadeIn"])?,
            "Duration",
        )?,
        quit_after_ms,
        ball_arrows: t.lookup_in(&["Init"], "Tut_Pfeile")?,
        pointer: t.lookup_in(&["Kapitel Aktion"], "Pfeil_Runter")?,
        grow,
        grow_ms: num(
            t.one("Linear Progression", &["MoveKeys", "max Scale"])?,
            "Duration",
        )?,
        freeze_sound: crate::score::wave_key(install, &wave)?,
        continue_key: t.key_in(&["Kapitel Aktion", "Tut continue/exit"], 0)?,
        quit_key: t.key_in(&["Kapitel Aktion", "Tut continue/exit"], 1)?,
        chapters,
    })
}

/// Each language's texts: `Text/Tutorial<n>.txt` (Latin-1) split at `*`,
/// `n` being the language's column in `Language.nmo`'s table.
fn texts(install: &Install) -> Result<BTreeMap<String, Vec<String>>> {
    let language = load(install, "3d entities/language.nmo")?;
    let table = language.array("language")?;
    let mut out = BTreeMap::new();
    for (i, c) in table
        .columns
        .iter()
        .filter(|c| c.name != "description")
        .enumerate()
    {
        let key = format!("text/tutorial{}.txt", i + 1);
        let Some(path) = install.get(&key) else {
            continue;
        };
        let bytes = std::fs::read(path).with_context(|| format!("reading {key}"))?;
        let s: String = bytes.iter().map(|&b| char::from(b)).collect();
        let parts = s
            .replace("\r\n", "\n")
            .split('*')
            .map(|p| p.trim().to_owned())
            .collect();
        out.insert(c.name.clone(), parts);
    }
    if out.is_empty() {
        bail!("no Text/Tutorial<n>.txt in the install");
    }
    Ok(out)
}

fn color(n: &NodeParams, pin: &str) -> Result<[f32; 4]> {
    match vector(input(n, pin)).as_deref() {
        Some(&[r, g, b, a]) => Ok([r, g, b, a]),
        other => bail!("{}: {pin} {other:?}", n.building_block),
    }
}

struct Tut<'a> {
    g: &'a Loaded,
    nodes: &'a [&'a NodeParams],
}

impl<'a> Tut<'a> {
    /// The first `bb_name` in the graph `path` or below.
    fn one(&self, bb_name: &str, path: &[&str]) -> Result<&'a NodeParams> {
        self.nodes
            .iter()
            .copied()
            .find(|n| n.building_block == bb_name && path_in(n, path))
            .with_context(|| format!("{SCRIPT} {path:?}: {bb_name} not found"))
    }

    /// The behaviour whose output writes the local feeding `node`'s `pin`.
    fn local_writer(&self, node: ObjRef, pin: &str) -> Option<ObjRef> {
        let s = &self.g.scripts;
        let param = pin_param(&self.g.file, s, node, pin)?;
        let Some(Some(ScriptObject::In(i))) = s.objects.get(param) else {
            return None;
        };
        let local = i.source?;
        s.objects.iter().enumerate().find_map(|(idx, o)| match o {
            Some(ScriptObject::Out(out)) if out.destinations.contains(&local) => {
                s.owner.get(&idx).copied()
            }
            _ => None,
        })
    }

    /// The name an `Op` in `path` looks up, checked to be `name`.
    fn lookup_in(&self, path: &[&str], name: &str) -> Result<String> {
        self.nodes
            .iter()
            .filter(|n| n.building_block == "Op" && path_in(n, path))
            .find_map(|n| text(input(n, "p1")).filter(|t| t == name))
            .with_context(|| format!("{SCRIPT} {path:?}: no lookup of {name}"))
    }

    /// The local a node's target comes from.
    fn target_local(&self, n: &NodeParams) -> Option<ObjRef> {
        let s = &self.g.scripts;
        let param = s.behavior(n.node)?.target_parameter?;
        let Some(Some(ScriptObject::In(i))) = s.objects.get(param) else {
            return None;
        };
        i.source
    }

    /// The 2D entity a node's target is: the name an `Op` looks up into
    /// the target's local.
    fn target_lookup(&self, n: &NodeParams) -> Result<String> {
        let s = &self.g.scripts;
        let local = self.target_local(n).context("target without a source")?;
        let writer = s
            .objects
            .iter()
            .enumerate()
            .find_map(|(idx, o)| match o {
                Some(ScriptObject::Out(out)) if out.destinations.contains(&local) => {
                    s.owner.get(&idx).copied()
                }
                _ => None,
            })
            .context("nothing writes the target")?;
        self.nodes
            .iter()
            .find(|m| m.node == writer)
            .and_then(|m| text(input(m, "p1")))
            .context("the target isn't looked up by name")
    }

    /// The `i`th *Keyboard Key* a `Key` graph in `path` waits for.
    fn key_in(&self, path: &[&str], i: usize) -> Result<u32> {
        let mut keys: Vec<u32> = Vec::new();
        for n in self.nodes.iter().filter(|n| {
            n.building_block == "Key Event"
                && path_in(n, path)
                && n.path.last().map(String::as_str) == Some("Key")
        }) {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a scan code"
            )]
            if let Some(k) = float(input(n, "Key Waited")).map(|f| f as u32)
                && k != 1
                && !keys.contains(&k)
            {
                keys.push(k);
            }
        }
        keys.get(i)
            .copied()
            .with_context(|| format!("{path:?}: key {i} not found (have {keys:?})"))
    }

    /// The proximity in graph `path`: its object (looked up by an `Op` in
    /// the graph) and distance.
    fn near(&self, path: &[&str]) -> Result<(String, f32)> {
        let p = self.one("TT Scaleable Proximity", path)?;
        let object = self
            .nodes
            .iter()
            .filter(|n| n.building_block == "Op" && path_is(n, path))
            .find_map(|n| text(input(n, "p1")).filter(|t| t.starts_with("Tut_")))
            .with_context(|| format!("{path:?}: the proximity's object"))?;
        Ok((object, num(p, "Distance")?))
    }

    /// The distance of the first proximity in `path`.
    fn distance(&self, path: &[&str]) -> Result<f32> {
        num(self.one("TT Scaleable Proximity", path)?, "Distance")
    }

    /// The chapters, in the order of the *Switch On Parameter*s' outputs.
    #[expect(clippy::too_many_lines, reason = "the ten chapters in turn")]
    fn chapters(&self, tutorial: &Loaded) -> Result<Vec<bb::TutorialChapter>> {
        let wait = |graph: &str| self.near(&["Kapitel Wait", graph]);
        let action = |graph: &str| self.distance(&["Kapitel Aktion", graph]);
        // An arrow, or a group's arrows, an `Op` in the actions looks up.
        let arrows_of = |name: &str| -> Result<Vec<String>> {
            self.lookup_in(&["Kapitel Aktion"], name)?;
            Ok(group_members(&tutorial.file, name).unwrap_or_else(|_| vec![name.to_owned()]))
        };
        let (rampe, rampe_d) = wait("Wait for Rampe")?;
        let wood = self
            .nodes
            .iter()
            .filter(|n| n.building_block == "Op" && path_in(n, &["Kapitel Wait", "Wait for Rampe"]))
            .find_map(|n| text(input(n, "p1")).filter(|t| t.starts_with("Ball_")))
            .context("Wait for Rampe: the ball type")?;
        let near = |(object, distance): (String, f32)| bb::TutorialWait::Near {
            object,
            distance,
            unless_ball: None,
        };
        let reach = |object: &str, graph: &str| -> Result<bb::TutorialAction> {
            self.lookup_in(&["Kapitel Aktion"], object)?;
            Ok(bb::TutorialAction::Reach {
                object: object.to_owned(),
                distance: action(graph)?,
                away: None,
            })
        };
        // The move arrows, by the keys that grow them.
        let mut arrows: Vec<(String, String)> = Vec::new();
        for n in self.nodes.iter().filter(|n| {
            n.building_block == "Key Event" && path_is(n, &["Kapitel Aktion", "MoveKeys"])
        }) {
            // The key's local (`Up`, `Down`, …) names the direction.
            let Some(dir) = source_name(self.g, n.node, "Key Waited") else {
                continue;
            };
            let Some(arrow) = self.grown_by(n) else {
                continue;
            };
            arrows.push((dir, arrow));
        }
        let arrow = |dir: &str| -> Result<String> {
            arrows
                .iter()
                .find(|(d, _)| d == dir)
                .map(|(_, a)| a.clone())
                .with_context(|| format!("MoveKeys: no arrow for {dir} (have {arrows:?})"))
        };
        let (key_end, key_end_d) = self.near(&["Kapitel Aktion", "MoveKeys"])?;
        let extra_point = {
            let p: Vec<f32> = self
                .nodes
                .iter()
                .filter(|n| {
                    n.building_block == "TT Scaleable Proximity"
                        && path_is(n, &["Kapitel Aktion", "Extrapoint"])
                })
                .filter_map(|n| float(input(n, "Distance")))
                .collect();
            match p.as_slice() {
                &[a, b] => (a.min(b), a.max(b)),
                other => bail!("Extrapoint: proximities {other:?}"),
            }
        };
        let hints_ms = num(
            self.nodes
                .iter()
                .find(|n| n.building_block == "Delayer" && path_is(n, &["Kapitel Wait"]))
                .context("Kapitel Wait: Delayer not found")?,
            "Time to Wait",
        )?;
        Ok(vec![
            bb::TutorialChapter {
                wait: bb::TutorialWait::Now,
                action: bb::TutorialAction::Intro,
            },
            bb::TutorialChapter {
                wait: bb::TutorialWait::Now,
                action: bb::TutorialAction::Arrows {
                    arrows: arrows_of("Pfeile_Orientation")?,
                    camera: true,
                },
            },
            bb::TutorialChapter {
                wait: bb::TutorialWait::Now,
                action: bb::TutorialAction::Arrows {
                    arrows: arrows_of("Pfeil_Hoch")?,
                    camera: false,
                },
            },
            bb::TutorialChapter {
                wait: bb::TutorialWait::Now,
                action: bb::TutorialAction::Move {
                    arrows: [
                        arrow("Up")?,
                        arrow("Down")?,
                        arrow("Left")?,
                        arrow("Right")?,
                    ],
                    end: key_end,
                    distance: key_end_d,
                },
            },
            bb::TutorialChapter {
                wait: near(wait("Wait for ExtraLife")?),
                action: reach("Tut_ExtraLife", "ExtraLife")?,
            },
            bb::TutorialChapter {
                wait: near(wait("Wait for SteinTrafo")?),
                action: reach("Tut_SteinTrafo", "SteinTrafo")?,
            },
            bb::TutorialChapter {
                wait: bb::TutorialWait::Near {
                    object: rampe,
                    distance: rampe_d,
                    unless_ball: Some(wood),
                },
                action: reach("Tut_HolzTrafo", "HolzTrafo")?,
            },
            bb::TutorialChapter {
                wait: near(wait("Wait for Extrapoint")?),
                action: bb::TutorialAction::Reach {
                    object: self.lookup_in(&["Kapitel Aktion"], "Tut_ExtraPoint")?,
                    distance: extra_point.0,
                    away: Some(extra_point.1),
                },
            },
            bb::TutorialChapter {
                wait: near(wait("Wait for Checkpoint")?),
                action: reach("Tut_Checkpoint", "Checkpoint")?,
            },
            bb::TutorialChapter {
                wait: bb::TutorialWait::Delay { ms: hints_ms },
                action: bb::TutorialAction::Pause,
            },
        ])
    }

    /// The arrow a key's *Pressed* grows: the *max Scale* graph it is
    /// linked to, and the arrow name that graph is given.
    fn grown_by(&self, key: &NodeParams) -> Option<String> {
        let s = &self.g.scripts;
        let b = s.behavior(key.node)?;
        let pressed = b.outputs.iter().copied().find(|&o| {
            self.g.file.objects.get(o).and_then(|o| o.name.as_deref()) == Some("Pressed")
        })?;
        let parent = *s.owner.get(&key.node)?;
        s.behavior(parent)?.links.iter().find_map(|&l| {
            let Some(Some(ScriptObject::Link(link))) = s.objects.get(l) else {
                return None;
            };
            if link.source != Some(pressed) {
                return None;
            }
            // The graph it starts; the `Op` in it names the arrow.
            let graph = *s.owner.get(&link.target?)?;
            self.nodes
                .iter()
                .find(|n| n.building_block == "Op" && s.owner.get(&n.node) == Some(&graph))
                .and_then(|n| text(input(n, "p1")))
        })
    }
}

/// `n`'s graphs below the script are `path` (from the top).
fn path_is(n: &NodeParams, path: &[&str]) -> bool {
    n.path.len() == path.len() + 1 && n.path[1..].iter().zip(path).all(|(a, b)| a == b)
}

/// `n` lies in a graph `path` (graph names in a row, anywhere below
/// the script) or below it.
fn path_in(n: &NodeParams, path: &[&str]) -> bool {
    let below = &n.path[1.min(n.path.len())..];
    path.is_empty()
        || below
            .windows(path.len())
            .any(|w| w.iter().zip(path).all(|(a, b)| a == b))
}

fn num(n: &NodeParams, pin: &str) -> Result<f32> {
    float(input(n, pin)).with_context(|| format!("{}: {pin}", n.building_block))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_tutorial() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let t = extract(&install).unwrap();
        assert_eq!(t.chapters.len(), 10);
        assert_eq!(t.texts["english"].len(), 10);
        assert!(t.texts["english"][1].starts_with("View Rotation"));
        assert_eq!((t.continue_key, t.quit_key), (28, 16));
        assert_eq!(t.start_ms, 100.0);
        assert_eq!(t.next_ms, 510.0);
        assert_eq!(t.fade_ms, 200.0);
        assert_eq!(t.quit_after_ms, 25000.0);
        assert_eq!(t.pointer, "Pfeil_Runter");
        assert_eq!(t.freeze_sound, "hit_stone_kuppel");
        assert_eq!(t.font.scale, [0.4, 0.5]);
        assert!((t.back.color[3] - 0.4706).abs() < 1e-3);
        assert_eq!(
            t.chapters[3].action,
            bb::TutorialAction::Move {
                arrows: [
                    "Tut_Richt_Pfeil03".into(),
                    "Tut_Richt_Pfeil04".into(),
                    "Tut_Richt_Pfeil02".into(),
                    "Tut_Richt_Pfeil01".into()
                ],
                end: "Tut_KeyEnd".into(),
                distance: 4.0,
            }
        );
        assert_eq!(
            t.chapters[6].wait,
            bb::TutorialWait::Near {
                object: "Tut_Rampe".into(),
                distance: 4.5,
                unless_ball: Some("Ball_Wood".into()),
            }
        );
        assert_eq!(
            t.chapters[7].action,
            bb::TutorialAction::Reach {
                object: "Tut_ExtraPoint".into(),
                distance: 3.0,
                away: Some(40.0),
            }
        );
        assert_eq!(t.chapters[9].wait, bb::TutorialWait::Delay { ms: 4000.0 });
    }
}
