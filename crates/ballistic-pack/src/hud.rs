//! Extracts the in-game HUD (see the research repo, findings 18):
//! - `Gameplay.nmo`'s `Gameplay_Energy`: the font (`TT CreateFontEx`, *Set
//!   Font Properties*), the points text (*2D Text*), the life balls' step,
//!   fades and slides, and the glow. Its objects are found by name at runtime
//!   (an *Op* "get object by name" writes each one into a local); those names
//!   are followed here.
//! - `Camera.nmo`'s 2D entities (rectangles, texture parts, z-order,
//!   materials) and its `HUD_sprites` group, which *Init Ingame* shows.
//! - `Menu.nmo`'s font texture and glyph table.

use anyhow::{Context, Result, bail};
use ballistic_bundle as bb;
use ckfile::{CkFile, ClassId};
use ckscene::{Decoded, decode_object};
use ckscript::extract::{Extractor, NodeParams};
use ckscript::{ObjRef, ScriptObject, Scripts};

use crate::install::Install;
use crate::physics::{Loaded, load};
use crate::transformers::{float, input, progression, text, vector};

#[allow(
    clippy::too_many_lines,
    reason = "one pass over the script, in HUD order"
)]
pub fn extract(install: &Install) -> Result<bb::Hud> {
    let g = load(install, "3d entities/gameplay.nmo")?;
    let ex = Extractor::new(&g.file, &g.scripts);
    let nodes = ex.nodes(&|_| true);
    let energy: Vec<&NodeParams> = nodes
        .iter()
        .filter(|n| n.path.first().map(String::as_str) == Some("Gameplay_Energy"))
        .collect();
    let find = |graph: &str, bb_name: &str| {
        energy
            .iter()
            .copied()
            .find(|n| {
                n.building_block == bb_name && n.path.last().map(String::as_str) == Some(graph)
            })
            .with_context(|| format!("Gameplay_Energy: {graph}'s {bb_name} not found"))
    };
    let name_of = |n: &NodeParams, pin: Option<&str>| {
        let param = match pin {
            Some(pin) => pin_param(&g.file, &g.scripts, n.node, pin),
            None => g.scripts.behavior(n.node).and_then(|b| b.target_parameter),
        };
        param
            .and_then(|p| named_by(&g, &nodes, p))
            .with_context(|| format!("{}: {pin:?} isn't looked up by name", n.building_block))
    };

    // The font.
    let create = find("Init", "TT_CreateFontEx")?;
    let font_texture = name_of(create, Some("Font Texture"))?;
    let font_data = name_of(create, Some("FontCoordinatesData"))?;
    let props = find("Init", "Set Font Properties")?;
    let pair = |n: &NodeParams, pin: &str| -> Result<[f32; 2]> {
        match vector(input(n, pin)).as_deref() {
            Some([x, y]) => Ok([*x, *y]),
            other => bail!("{}: {pin} {other:?}", n.building_block),
        }
    };
    let quad = |n: &NodeParams, pin: &str| -> Result<[f32; 4]> {
        match vector(input(n, pin)).as_deref() {
            Some([a, b, c, d]) => Ok([*a, *b, *c, *d]),
            other => bail!("{}: {pin} {other:?}", n.building_block),
        }
    };
    let menu = load(install, "3d entities/menu.nmo")?;
    let font = bb::HudFont {
        texture: texture_key(install, &menu.file, &font_texture)?,
        glyphs: glyphs(&menu, &font_data)?,
        scale: pair(props, "Scale")?,
        space: pair(props, "Space")?,
        color: quad(props, "Color")?,
    };

    // The points text.
    let text2d = find("Gameplay_Energy", "2D Text")?;
    let align = float(input(text2d, "Alignment")).context("2D Text: Alignment")?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "flag bits"
    )]
    let align = match align as u32 {
        a if a & 1 != 0 => bb::TextAlign::Left,
        a if a & 2 != 0 => bb::TextAlign::Right,
        _ => bb::TextAlign::Center,
    };
    let points = bb::HudText {
        sprite: name_of(text2d, None)?,
        align,
        margins: quad(text2d, "Margins")?,
    };

    // The life balls.
    let step = float(input(find("Init Startlifes", "Calculator")?, "c"))
        .context("Init Startlifes: the balls' step")?;
    let copy = find("Init Startlifes", "Object Copy")?;
    let end_sprite = find("Init Startlifes", "TT Set_2DSprite")
        .ok()
        .and_then(|_| {
            energy
                .iter()
                .filter(|n| {
                    n.building_block == "TT Set_2DSprite"
                        && n.path.last().map(String::as_str) == Some("Init Startlifes")
                })
                .find_map(|n| name_of(n, None).ok())
        })
        .context("Init Startlifes: the end sprite")?;
    let lives = bb::HudLives {
        ball: name_of(copy, Some("Original"))?,
        end: end_sprite,
        step,
        fade_in: progression(find("FadeIn Lifeball", "Bezier Progression")?)?,
        fade_out: progression(find("FadeOut  Lifeball", "Bezier Progression")?)?,
        slide: progression(find("Move LifeEnd", "Bezier Progression")?)?,
        lost_delay_ms: float(input(find("Gameplay_Energy", "Delayer")?, "Time to Wait"))
            .context("Gameplay_Energy: Delayer")?,
    };

    // The glow.
    let glow = bb::HudGlow {
        sprite: name_of(find("Gameplay_Energy", "Show")?, None)?,
        fade: progression(find("Gameplay_Energy", "Bezier Progression")?)?,
    };

    // The HUD group *Init Ingame* shows (among other things it shows).
    let camera = CkFile::open(
        install
            .get("3d entities/camera.nmo")
            .context("Camera.nmo missing")?,
    )
    .context("reading Camera.nmo")?;
    let shown = nodes
        .iter()
        .filter(|n| n.building_block == "Show" && n.path.iter().any(|p| p == "Init Ingame"))
        .filter_map(|n| name_of(n, None).ok())
        .find_map(|name| group_members(&camera, &name).ok())
        .context("Init Ingame: the HUD group")?;
    let mut wanted: Vec<String> = shown.clone();
    for n in [&lives.ball, &lives.end, &glow.sprite, &points.sprite] {
        if !wanted.contains(n) {
            wanted.push(n.clone());
        }
    }
    let mut sprites = wanted
        .iter()
        .map(|name| sprite(install, &camera, name, shown.contains(name)))
        .collect::<Result<Vec<_>>>()?;
    sprites.sort_by_key(|s| s.z_order);

    Ok(bb::Hud {
        sprites,
        font,
        points,
        lives,
        glow,
    })
}

/// The input parameter `pin` of behaviour `node`.
fn pin_param(file: &CkFile, scripts: &Scripts, node: ObjRef, pin: &str) -> Option<ObjRef> {
    scripts
        .behavior(node)?
        .input_params
        .iter()
        .copied()
        .find(|&p| file.objects.get(p).and_then(|o| o.name.as_deref()) == Some(pin))
}

/// The name an *Op* "get object by name" gives the local feeding input `param`.
fn named_by(g: &Loaded, nodes: &[NodeParams], param: ObjRef) -> Option<String> {
    let Some(Some(ScriptObject::In(i))) = g.scripts.objects.get(param) else {
        return None;
    };
    let local = i.source?;
    g.scripts.objects.iter().enumerate().find_map(|(idx, o)| {
        let Some(ScriptObject::Out(out)) = o else {
            return None;
        };
        if !out.destinations.contains(&local) {
            return None;
        }
        let owner = g.scripts.owner.get(&idx)?;
        let op = nodes
            .iter()
            .find(|n| n.node == *owner && n.building_block == "Op")?;
        text(input(op, "p1"))
    })
}

/// The object `name` of `class` (names repeat across classes: a sprite and
/// its material, say).
fn object(file: &CkFile, name: &str, class: ClassId) -> Result<usize> {
    file.objects
        .iter()
        .position(|o| o.name.as_deref() == Some(name) && o.class_id == class)
        .with_context(|| format!("{class:?} {name} not found"))
}

fn group_members(file: &CkFile, name: &str) -> Result<Vec<String>> {
    let i = object(file, name, ClassId::GROUP)?;
    let Some(Ok(Decoded::Group(g))) = decode_object(&file.objects[i]) else {
        bail!("{name} isn't a group");
    };
    Ok(g.members
        .iter()
        .filter_map(|&m| file.objects.get(m).and_then(|o| o.name.clone()))
        .collect())
}

/// A texture object's bundle key: its file's name under `Textures/`.
pub(crate) fn texture_key(install: &Install, file: &CkFile, name: &str) -> Result<String> {
    let i = object(file, name, ClassId::TEXTURE)?;
    texture_key_at(install, file, i)
}

fn texture_key_at(install: &Install, file: &CkFile, i: usize) -> Result<String> {
    let Some(Ok(Decoded::Texture(t))) = decode_object(&file.objects[i]) else {
        bail!("object #{i} isn't a texture");
    };
    let f = t.file().context("texture without a file")?.to_lowercase();
    anyhow::ensure!(
        install.get(&format!("textures/{f}")).is_some(),
        "texture {f} not in Textures/"
    );
    Ok(f.rsplit_once('.')
        .map_or(f.clone(), |(stem, _)| stem.to_owned()))
}

fn sprite(install: &Install, file: &CkFile, name: &str, shown: bool) -> Result<bb::HudSprite> {
    let i = object(file, name, ClassId::ENTITY_2D)?;
    let Some(Ok(Decoded::Entity2d(e))) = decode_object(&file.objects[i]) else {
        bail!("{name} isn't a 2D entity");
    };
    let (texture, color) = match e.material {
        Some(m) => {
            let Some(Ok(Decoded::Material(mat))) = decode_object(&file.objects[m]) else {
                bail!("{name}: material isn't decodable");
            };
            let texture = mat
                .texture
                .map(|t| texture_key_at(install, file, t))
                .transpose()?;
            (texture, ckscene::argb_to_rgba(mat.diffuse))
        }
        None => (None, [1.0; 4]),
    };
    Ok(bb::HudSprite {
        name: name.to_owned(),
        texture,
        rect: e.rect,
        source: e.source,
        z_order: e.z_order,
        color,
        shown,
    })
}

/// The font's glyph table, by character code.
fn glyphs(menu: &Loaded, name: &str) -> Result<Vec<bb::Glyph>> {
    let table = menu.array(name)?;
    table
        .rows
        .iter()
        .map(|r| {
            let row = menu.row(table, r);
            Ok(bb::Glyph {
                u: row.f32("ustart")?,
                v: row.f32("vstart")?,
                width: row.f32("uwidth")?,
                before: row.f32("uprewidth")?,
                after: row.f32("upostwidth")?,
                height: row.f32("vwidth")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `BALLANCE_DIR`; skipped otherwise.
    #[test]
    fn extracts_the_hud() {
        let Some(dir) = std::env::var_os("BALLANCE_DIR") else {
            eprintln!("skipping: BALLANCE_DIR not set");
            return;
        };
        let install = Install::index(std::path::Path::new(&dir)).unwrap();
        let hud = extract(&install).unwrap();
        let names: Vec<&str> = hud.sprites.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Interface_Life_Startbogen"), "{names:?}");
        // Drawn in z-order.
        assert!(hud.sprites.windows(2).all(|w| w[0].z_order <= w[1].z_order));
        let shown = hud.sprites.iter().filter(|s| s.shown).count();
        assert_eq!(shown, 4, "the HUD_sprites group");
        assert_eq!(hud.font.texture, "font_1");
        assert_eq!(hud.font.glyphs.len(), 255);
        assert_eq!(hud.font.scale, [0.8, 0.9]);
        assert_eq!(hud.points.sprite, "Interface_Points_digits");
        assert_eq!(hud.points.align, bb::TextAlign::Right);
        assert_eq!(hud.lives.ball, "Interface_Life_Kugel");
        assert_eq!(hud.lives.end, "Interface_Life_End");
        assert!((hud.lives.step - 0.0387).abs() < 1e-6);
        assert!((hud.lives.lost_delay_ms - 2000.0).abs() < 1e-3);
        assert!((hud.lives.slide.duration_ms - 300.0).abs() < 1e-3);
        assert_eq!(hud.glow.sprite, "Interface_Points_glow");
        // The glow starts bright and fades out.
        assert!((hud.glow.fade.value(0.0) - 1.0).abs() < 1e-4);
        assert!(hud.glow.fade.value(500.0).abs() < 1e-4);
    }
}
