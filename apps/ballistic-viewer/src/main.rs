//! `ballistic-viewer`: fly around the scenes of a Ballistic bundle.

mod audio;
mod ballsounds;
mod effects;
mod energy;
mod flames;
mod flow;
mod follow;
mod game;
mod highscore_screens;
mod hud;
#[cfg(test)]
mod levels_check;
mod life_look;
mod menus;
mod modules;
mod music;
mod particles;
mod pickups;
mod pieces;
mod play;
mod point_look;
mod progress;
mod props;
mod save;
mod score_screen;
mod transformers;
mod window;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use ballistic_bundle::BundleReader;
use ballistic_render::{Camera, CullMode, Gpu, RenderOptions, RenderTarget, SceneRenderer};
use clap::{Parser, ValueEnum};
use glam::Vec3;

#[derive(Parser)]
#[command(version, about)]
#[allow(clippy::struct_excessive_bools)] // independent CLI flags
pub struct Args {
    /// Bundle built by ballistic-pack.
    #[arg(default_value = "ballance.bpk")]
    bundle: PathBuf,
    /// Scene key to open (see --list).
    #[arg(long, default_value = "level/level_01")]
    scene: String,
    /// List the bundle's scenes and exit.
    #[arg(long)]
    list: bool,
    /// Render one frame off-screen to this PNG and exit (no window).
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Off-screen image size, `WIDTHxHEIGHT`.
    #[arg(long, default_value = "1600x900")]
    size: String,
    /// Initial camera: `x,y,z,yaw_deg,pitch_deg`.
    #[arg(long, allow_hyphen_values = true)]
    camera: Option<String>,
    /// Also draw objects that the level hides at load time.
    #[arg(long)]
    show_hidden: bool,
    /// Face culling (debugging).
    #[arg(long, value_enum, default_value_t = CullArg::Back)]
    cull: CullArg,
    /// Start in borderless fullscreen.
    #[arg(long)]
    fullscreen: bool,
    /// Quit after this many frames and print the average frame rate (testing).
    #[arg(long)]
    exit_after: Option<u32>,
    /// Start with vsync off.
    #[arg(long)]
    no_vsync: bool,
    /// Play mode: a physics ball on the level (arrows move, Shift + Left/Right turns).
    #[arg(long)]
    play: bool,
    /// Scripted input for play mode in screenshots: `up:2,rotate+left:0.4,wait:1`
    /// (keys held for seconds; keys: up down left right rotate lift wait, next = warp to the next checkpoint first, wake = wake all moving elements first).
    #[arg(long)]
    play_script: Option<String>,
    /// Save file for play mode (progress, highscores, the name). Default in
    /// a window: the platform's data folder (`%APPDATA%\\Ballistic\\save.json`,
    /// …); headless: none (kept in memory).
    #[arg(long)]
    save: Option<PathBuf>,
    /// The game: start at the main menu over the menu level, and play the
    /// levels from its menus (implies `--play`).
    #[arg(long)]
    game: bool,
    /// With `--game --screenshot`: menu keys to press before the frame, as
    /// `DirectInput` scan codes (`208,28`: Down, Enter). Development aid.
    #[arg(long, value_delimiter = ',')]
    menu_keys: Vec<u32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum CullArg {
    Back,
    None,
    Front,
}

impl From<CullArg> for CullMode {
    fn from(c: CullArg) -> Self {
        match c {
            CullArg::Back => CullMode::Back,
            CullArg::None => CullMode::None,
            CullArg::Front => CullMode::Front,
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let bundle = BundleReader::open(&args.bundle)
        .with_context(|| format!("opening {}", args.bundle.display()))?;
    if args.list {
        for s in &bundle.manifest().scenes {
            println!(
                "{:<28} {:>5} objects {:>5} meshes",
                s.key, s.objects, s.meshes
            );
        }
        return Ok(());
    }
    if let Some(path) = args.screenshot.clone() {
        return screenshot(&args, bundle, &path);
    }
    window::run(args, bundle)
}

/// Loaded scene plus the state the viewer keeps per scene.
pub struct LoadedScene {
    pub key: String,
    pub renderer: SceneRenderer,
    pub home: Camera,
}

pub fn load_scene(
    gpu: &Gpu,
    bundle: &mut BundleReader<std::io::BufReader<std::fs::File>>,
    key: &str,
    format: wgpu::TextureFormat,
) -> Result<LoadedScene> {
    let scene = bundle
        .scene(key)
        .with_context(|| format!("loading scene {key}"))?;
    let (renderer, warnings) =
        SceneRenderer::new(gpu, &scene, format, &mut |k| bundle.texture(k).ok())?;
    for w in warnings {
        eprintln!("warning: {w}");
    }
    let home = home_camera(&renderer);
    Ok(LoadedScene {
        key: key.to_owned(),
        renderer,
        home,
    })
}

/// Starting view: behind and above the level start, or overlooking the bounds.
fn home_camera(r: &SceneRenderer) -> Camera {
    let (lo, hi) = r.bounds;
    let size = (hi - lo).length().max(10.0);
    let mut cam = Camera {
        far: (size * 3.0).max(2000.0),
        ..Camera::default()
    };
    if let Some(start) = r.start_position {
        cam.position = start + Vec3::new(0.0, 18.0, 30.0);
        cam.look_at(start);
    } else {
        let center = (lo + hi) * 0.5;
        cam.position = center + Vec3::new(0.0, size * 0.35, size * 0.6);
        cam.look_at(center);
    }
    cam
}

pub fn parse_camera(s: &str, base: Camera) -> Result<Camera> {
    let v: Vec<f32> = s
        .split(',')
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .context("--camera expects x,y,z,yaw_deg,pitch_deg")?;
    if v.len() != 5 {
        bail!("--camera expects 5 numbers, got {}", v.len());
    }
    Ok(Camera {
        position: Vec3::new(v[0], v[1], v[2]),
        yaw: v[3].to_radians(),
        pitch: v[4].to_radians(),
        ..base
    })
}

pub fn format_camera(c: &Camera) -> String {
    format!(
        "{:.2},{:.2},{:.2},{:.1},{:.1}",
        c.position.x,
        c.position.y,
        c.position.z,
        c.yaw.to_degrees(),
        c.pitch.to_degrees()
    )
}

pub fn options(args: &Args) -> RenderOptions {
    RenderOptions {
        show_hidden: args.show_hidden,
        cull: args.cull.into(),
        ..RenderOptions::default()
    }
}

/// One segment of a play script: keys held for some seconds, after an
/// optional warp to the next checkpoint (`next`).
#[derive(Debug, Clone, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent development aids, each at the start of a segment"
)]
pub struct Segment {
    pub keys: Vec<play::PlayKey>,
    pub seconds: f32,
    pub warp: bool,
    /// Wake every active moving element first (`wake`).
    pub wake: bool,
    /// Put the ball on the nearest transformer of another type first.
    pub trafo: bool,
    /// Put the ball on the nearest pickup first.
    pub pickup: bool,
    /// Put the ball at the nearest fan in an active sector first.
    pub fan: bool,
    /// Put the ball 8 from the nearest pickup first.
    pub near: bool,
    /// Lose the ball first (as **R**).
    pub lose: bool,
    /// Press Space first: cuts the level end's wait short, scores fast,
    /// leaves the score screen (`skip`).
    pub skip: bool,
    /// Press Enter first: confirms the highscore name, leaves the table
    /// (`enter`).
    pub enter: bool,
}

/// Parses `up:2,rotate+left:0.4,wait:1,next:1` into segments.
pub fn parse_play_script(s: &str) -> Result<Vec<Segment>> {
    use play::PlayKey as K;
    s.split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|part| {
            let (keys, secs) = part
                .split_once(':')
                .with_context(|| format!("segment {part:?}: expected keys:seconds"))?;
            let seconds: f32 = secs
                .trim()
                .parse()
                .with_context(|| format!("segment {part:?}"))?;
            anyhow::ensure!(seconds >= 0.0, "segment {part:?}: negative duration");
            let warp = keys.split('+').any(|k| k.trim() == "next");
            let wake = keys.split('+').any(|k| k.trim() == "wake");
            let trafo = keys.split('+').any(|k| k.trim() == "trafo");
            let pickup = keys.split('+').any(|k| k.trim() == "pickup");
            let fan = keys.split('+').any(|k| k.trim() == "fan");
            let near = keys.split('+').any(|k| k.trim() == "near");
            let lose = keys.split('+').any(|k| k.trim() == "lose");
            let skip = keys.split('+').any(|k| k.trim() == "skip");
            let enter = keys.split('+').any(|k| k.trim() == "enter");
            let keys = keys
                .split('+')
                .filter_map(|k| match k.trim() {
                    "up" => Some(Ok(K::Up)),
                    "down" => Some(Ok(K::Down)),
                    "left" => Some(Ok(K::Left)),
                    "right" => Some(Ok(K::Right)),
                    "rotate" => Some(Ok(K::Rotate)),
                    "lift" => Some(Ok(K::Lift)),
                    "wait" | "next" | "wake" | "trafo" | "pickup" | "fan" | "near" | "lose"
                    | "skip" | "enter" => None,
                    other => Some(Err(anyhow::anyhow!("unknown key {other:?}"))),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Segment {
                keys,
                seconds,
                warp,
                wake,
                trafo,
                pickup,
                fan,
                near,
                lose,
                skip,
                enter,
            })
        })
        .collect()
}

#[allow(clippy::too_many_lines, reason = "linear: load, script, render, save")]
fn screenshot(
    args: &Args,
    mut bundle: BundleReader<std::io::BufReader<std::fs::File>>,
    path: &std::path::Path,
) -> Result<()> {
    let (w, h) = args
        .size
        .split_once('x')
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .context("--size expects WIDTHxHEIGHT")?;
    let gpu = pollster::block_on(Gpu::headless())?;
    eprintln!(
        "adapter: {} (BC textures: {})",
        gpu.adapter_name(),
        gpu.bc_textures
    );
    let format = ballistic_render::OFFSCREEN_FORMAT;
    let first = if args.game {
        game::MENU_LEVEL
    } else {
        args.scene.as_str()
    };
    let mut scene = load_scene(&gpu, &mut bundle, first, format)?;
    let mut camera = match &args.camera {
        Some(s) => parse_camera(s, scene.home)?,
        None => scene.home,
    };
    // The main menu over the menu level, after `--menu-keys`.
    let mut menus = if args.game {
        camera = game::menu_camera(&bundle.scene(first)?, camera);
        let defaults = bundle.physics()?.highscores;
        let save = save::SaveFile::open(args.save.clone(), &defaults).shared();
        let mut g = game::Game::create(&gpu, &mut bundle, format, save.clone())?;
        for &k in &args.menu_keys {
            g.key(k);
        }
        for a in g.take_actions() {
            eprintln!("menu action (not carried out headless): {a:?}");
        }
        Some((g, save))
    } else {
        None
    };
    let mut play = if args.play {
        let level = bundle.scene(&args.scene)?;
        let defaults = bundle.physics()?.highscores;
        let save = save::SaveFile::open(args.save.clone(), &defaults).shared();
        let mut p = play::Play::new(&gpu, &mut bundle, &args.scene, &level, format, save)?;
        for i in p.placeholders() {
            scene.renderer.set_object_hidden(i, true);
        }
        let dt = 1.0 / 60.0;
        for seg in parse_play_script(args.play_script.as_deref().unwrap_or("wait:1"))? {
            let (keys, seconds) = (seg.keys, seg.seconds);
            if seg.warp {
                p.warp_to_next_checkpoint();
            }
            if seg.wake {
                p.wake_modules();
            }
            if seg.trafo {
                p.warp_to_transformer();
            }
            if seg.fan {
                p.warp_to_fan();
            }
            if seg.near {
                p.warp_near_pickup();
            }
            if seg.pickup {
                p.warp_to_pickup();
            }
            if seg.lose {
                p.respawn();
            }
            if seg.skip {
                p.menu_key(57);
            }
            if seg.enter {
                p.menu_key(28);
            }
            for &k in &keys {
                p.input(k, true);
            }
            #[expect(
                clippy::cast_sign_loss,
                reason = "durations are checked to be non-negative"
            )]
            let frames = (seconds / dt).round() as u32;
            for _ in 0..frames {
                p.update(dt);
                for (i, show) in p.take_level_visibility() {
                    scene.renderer.set_object_hidden(i, !show);
                }
                p.apply_level(&gpu, &mut scene.renderer);
                match p.take_next_level() {
                    Some(play::AfterScore::Level(key)) => eprintln!("next level: {key}"),
                    Some(play::AfterScore::LastLevel) => eprintln!("that was the last level"),
                    None => {}
                }
                // Headless: counted, not played.
                p.take_sounds();
                p.take_music();
                p.loops();
                // An explicit --camera stays put (to look at something else).
                if args.camera.is_none() {
                    camera = p.camera(camera);
                }
            }
            for &k in &keys {
                p.input(k, false);
            }
        }
        eprintln!(
            "play: {} after {} steps, {} respawns, {} props ({} active), {} modules ({} simulated), {} transformers, checkpoints {}/{}",
            p.status(),
            p.world.steps,
            p.respawns,
            p.prop_count(),
            p.progress().0,
            p.module_count().0,
            p.module_count().1,
            p.transformer_count(),
            p.progress().1,
            p.progress().2
        );
        eprintln!("{}", p.energy_summary());
        eprintln!(
            "flames burning: {}, fans running: {}, extra lives shown: {}, extra points shown: {}",
            p.flames_burning(),
            p.fans_running(),
            p.lives_shown(),
            p.points_shown()
        );
        let fallen = p.fallen_modules();
        if !fallen.is_empty() {
            eprintln!("fallen modules: {}", fallen.join(", "));
        }
        if !p.sound_counts.is_empty() {
            let s: Vec<String> = p
                .sound_counts
                .iter()
                .map(|(k, n)| format!("{k} ×{n}"))
                .collect();
            eprintln!("sounds: {}", s.join(", "));
        }
        Some(p)
    } else {
        None
    };
    let target = RenderTarget::new(&gpu, w, h, format);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    let opts = match play.as_ref() {
        Some(p) => {
            p.render_sky(
                &gpu,
                &mut encoder,
                &target.color_view,
                &camera,
                w as f32 / h as f32,
            );
            p.scene_options(&options(args))
        }
        None => options(args),
    };
    scene.renderer.render(
        &gpu,
        &mut encoder,
        &target.color_view,
        &target.depth_view,
        &camera,
        w as f32 / h as f32,
        &opts,
    );
    if let Some(p) = play.as_mut() {
        p.render(
            &gpu,
            &mut encoder,
            &target.color_view,
            &target.depth_view,
            &camera,
            w as f32 / h as f32,
            &opts,
        );
        p.render_hud(&gpu, &mut encoder, &target.color_view, (w, h));
    }
    if let Some((g, save)) = menus.as_mut() {
        g.menus
            .render(&gpu, &mut encoder, &target.color_view, (w, h));
        let save = save.borrow();
        g.tables
            .render(&gpu, &mut encoder, &target.color_view, (w, h), &save.data);
    }
    gpu.queue.submit([encoder.finish()]);
    let rgba = target.read_rgba(&gpu)?;
    image::save_buffer(path, &rgba, w, h, image::ExtendedColorType::Rgba8)
        .with_context(|| format!("writing {}", path.display()))?;
    eprintln!(
        "wrote {} ({} triangles, camera {})",
        path.display(),
        scene.renderer.triangle_count,
        format_camera(&camera)
    );
    Ok(())
}
