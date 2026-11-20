//! Interactive window: winit event loop, surface management, fly camera.

use std::collections::HashSet;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use ballistic_bundle::BundleReader;
use ballistic_render::{CullMode, Gpu, RenderOptions};
use glam::Vec3;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

use crate::{Args, LoadedScene, format_camera, load_scene, options, parse_camera};

type Bundle = BundleReader<BufReader<std::fs::File>>;

const HELP: &str = "\
controls:
  right mouse (hold) / Tab   look around
  W A S D / arrows           move        Space / E  up     Q / C  down
  Shift                      move faster
  PageUp / PageDown, [ ]     previous / next scene
  Home                       reset camera
  H  toggle hidden objects   B  cycle face culling   V  toggle vsync
  F11  fullscreen            P  print camera pose    Esc  release mouse / quit
play mode (--play):
  arrows                     roll the ball (relative to the camera)
  Shift + Left / Right       turn the camera 90°
  Space                      lift the camera (hold)
  R  lose the ball           1  next ball type
  N  warp to the next checkpoint or the level end (development aid)";

pub fn run(args: Args, bundle: Bundle) -> Result<()> {
    println!("{HELP}");
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    // Sounds only in play mode (a missing device just means silence).
    let audio = if args.play {
        crate::audio::Audio::new()
    } else {
        None
    };
    let mut app = App {
        args,
        bundle,
        state: None,
        error: None,
        audio,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct App {
    args: Args,
    bundle: Bundle,
    state: Option<State>,
    error: Option<anyhow::Error>,
    audio: Option<crate::audio::Audio>,
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Non-sRGB view format the renderer draws in.
    view_format: wgpu::TextureFormat,
    gpu: Gpu,
    depth: wgpu::TextureView,
    scene: LoadedScene,
    play: Option<crate::play::Play>,
    /// `--play` given: rebuild play mode on every scene switch.
    play_mode: bool,
    scene_keys: Vec<String>,
    camera: ballistic_render::Camera,
    opts: RenderOptions,
    vsync: bool,
    keys: HashSet<KeyCode>,
    looking: bool,
    last_frame: Instant,
    fps_timer: Instant,
    frames: u32,
    fps: f32,
    total_frames: u32,
    started: Instant,
    exit_after: Option<u32>,
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<State> {
        let mut attrs = Window::default_attributes()
            .with_title("Ballistic viewer")
            .with_inner_size(winit::dpi::LogicalSize::new(1600.0, 900.0));
        if self.args.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attrs)?);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance.create_surface(window.clone())?;
        let gpu = pollster::block_on(Gpu::new(instance, Some(&surface)))?;
        println!(
            "adapter: {} (BC textures: {})",
            gpu.adapter_name(),
            gpu.bc_textures
        );

        let size = window.inner_size();
        let caps = surface.get_capabilities(&gpu.adapter);
        let preferred = caps.formats[0];
        let view_format = preferred.remove_srgb_suffix();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow::anyhow!("surface is not supported by the adapter"))?;
        if caps.formats.contains(&view_format) {
            config.format = view_format;
        } else {
            config.format = preferred;
            config.view_formats = vec![view_format];
        }
        let vsync = !self.args.no_vsync;
        config.present_mode = present_mode(&caps, vsync);
        surface.configure(&gpu.device, &config);

        let scene_keys: Vec<String> = self
            .bundle
            .manifest()
            .scenes
            .iter()
            .map(|s| s.key.clone())
            .collect();
        let mut scene = load_scene(&gpu, &mut self.bundle, &self.args.scene, view_format)?;
        let camera = match &self.args.camera {
            Some(s) => parse_camera(s, scene.home)?,
            None => scene.home,
        };
        let depth = ballistic_render::gpu_depth_view(&gpu, config.width, config.height);
        let play = if self.args.play {
            Some(new_play(&gpu, &mut self.bundle, &mut scene, view_format)?)
        } else {
            None
        };
        Ok(State {
            window,
            surface,
            config,
            view_format,
            depth,
            scene,
            play,
            play_mode: self.args.play,
            scene_keys,
            camera,
            opts: options(&self.args),
            vsync,
            keys: HashSet::new(),
            looking: false,
            last_frame: Instant::now(),
            fps_timer: Instant::now(),
            frames: 0,
            fps: 0.0,
            total_frames: 0,
            started: Instant::now(),
            exit_after: self.args.exit_after,
            gpu,
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, e: anyhow::Error) {
        self.error = Some(e);
        event_loop.exit();
    }
}

/// Play mode on a loaded level; hides the placeholders that props replace.
fn new_play(
    gpu: &Gpu,
    bundle: &mut Bundle,
    scene: &mut crate::LoadedScene,
    format: wgpu::TextureFormat,
) -> Result<crate::play::Play> {
    let level = bundle.scene(&scene.key)?;
    let play = crate::play::Play::new(gpu, bundle, &scene.key, &level, format)?;
    for i in play.placeholders() {
        scene.renderer.set_object_hidden(i, true);
    }
    Ok(play)
}

fn present_mode(caps: &wgpu::SurfaceCapabilities, vsync: bool) -> wgpu::PresentMode {
    if vsync {
        return wgpu::PresentMode::Fifo;
    }
    [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
        .into_iter()
        .find(|m| caps.present_modes.contains(m))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

impl State {
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.gpu.device, &self.config);
        self.depth = ballistic_render::gpu_depth_view(&self.gpu, width, height);
    }

    fn set_looking(&mut self, on: bool) {
        self.looking = on;
        let grab = if on {
            self.window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
        } else {
            self.window.set_cursor_grab(CursorGrabMode::None)
        };
        let _ = grab;
        self.window.set_cursor_visible(!on);
    }

    fn switch_scene(&mut self, bundle: &mut Bundle, forward: bool) -> Result<()> {
        let n = self.scene_keys.len();
        if n == 0 {
            return Ok(());
        }
        let cur = self
            .scene_keys
            .iter()
            .position(|k| *k == self.scene.key)
            .unwrap_or(0);
        let next = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        let key = self.scene_keys[next].clone();
        self.open_scene(bundle, &key)
    }

    /// Loads scene `key` (and play mode on it, if on).
    fn open_scene(&mut self, bundle: &mut Bundle, key: &str) -> Result<()> {
        self.scene = load_scene(&self.gpu, bundle, key, self.view_format)?;
        self.camera = self.scene.home;
        if self.play_mode {
            // Levels have a start marker; other scenes fall back to free-fly.
            self.play = new_play(&self.gpu, bundle, &mut self.scene, self.view_format)
                .inspect_err(|e| eprintln!("play mode off for {key}: {e:#}"))
                .ok();
        }
        Ok(())
    }

    fn key(
        &mut self,
        bundle: &mut Bundle,
        event_loop: &ActiveEventLoop,
        ev: &KeyEvent,
    ) -> Result<()> {
        let PhysicalKey::Code(code) = ev.physical_key else {
            return Ok(());
        };
        if let Some(play) = self.play.as_mut()
            && !ev.repeat
        {
            use crate::play::PlayKey as K;
            let pressed = ev.state == ElementState::Pressed;
            // The level end and score screen take Space, Enter and Escape
            // (by DirectInput scan code).
            if play.in_level_end() {
                let scan = match code {
                    KeyCode::Space => Some(57),
                    KeyCode::Enter | KeyCode::NumpadEnter => Some(28),
                    KeyCode::Escape => Some(1),
                    _ => None,
                };
                if let Some(scan) = scan {
                    if pressed {
                        play.menu_key(scan);
                    }
                    return Ok(());
                }
            }
            let k = match code {
                KeyCode::ArrowUp => Some(K::Up),
                KeyCode::ArrowDown => Some(K::Down),
                KeyCode::ArrowLeft => Some(K::Left),
                KeyCode::ArrowRight => Some(K::Right),
                KeyCode::ShiftLeft | KeyCode::ShiftRight => Some(K::Rotate),
                KeyCode::Space => Some(K::Lift),
                _ => None,
            };
            if let Some(k) = k {
                play.input(k, pressed);
                return Ok(());
            }
            if pressed && code == KeyCode::KeyR {
                play.respawn();
                return Ok(());
            }
            if pressed && code == KeyCode::Digit1 {
                play.cycle_ball();
                return Ok(());
            }
            if pressed && code == KeyCode::KeyN {
                play.warp_to_next_checkpoint();
                return Ok(());
            }
        }
        if ev.state == ElementState::Released {
            self.keys.remove(&code);
            return Ok(());
        }
        self.keys.insert(code);
        if ev.repeat {
            return Ok(());
        }
        match code {
            KeyCode::Escape if self.looking => self.set_looking(false),
            KeyCode::Escape => event_loop.exit(),
            KeyCode::Tab => self.set_looking(!self.looking),
            KeyCode::PageDown | KeyCode::BracketRight => self.switch_scene(bundle, true)?,
            KeyCode::PageUp | KeyCode::BracketLeft => self.switch_scene(bundle, false)?,
            KeyCode::Home => self.camera = self.scene.home,
            KeyCode::KeyH => self.opts.show_hidden = !self.opts.show_hidden,
            KeyCode::KeyB => {
                self.opts.cull = match self.opts.cull {
                    CullMode::Back => CullMode::None,
                    CullMode::None => CullMode::Front,
                    CullMode::Front => CullMode::Back,
                };
                println!("culling: {:?}", self.opts.cull);
            }
            KeyCode::KeyV => {
                self.vsync = !self.vsync;
                let caps = self.surface.get_capabilities(&self.gpu.adapter);
                self.config.present_mode = present_mode(&caps, self.vsync);
                self.surface.configure(&self.gpu.device, &self.config);
                println!("vsync: {} ({:?})", self.vsync, self.config.present_mode);
            }
            KeyCode::F11 => {
                let fs = if self.window.fullscreen().is_some() {
                    None
                } else {
                    Some(Fullscreen::Borderless(None))
                };
                self.window.set_fullscreen(fs);
            }
            KeyCode::KeyP => println!(
                "--scene {} --camera {}",
                self.scene.key,
                format_camera(&self.camera)
            ),
            _ => {}
        }
        Ok(())
    }

    fn update(&mut self, dt: f32) {
        if let Some(play) = self.play.as_mut() {
            play.update(dt);
            for (i, show) in play.take_level_visibility() {
                self.scene.renderer.set_object_hidden(i, !show);
            }
            play.apply_level(&self.gpu, &mut self.scene.renderer);
            self.camera = play.camera(self.camera);
            return;
        }
        let held = |k: &[KeyCode]| k.iter().any(|c| self.keys.contains(c));
        let mut v = Vec3::ZERO;
        if held(&[KeyCode::KeyW, KeyCode::ArrowUp]) {
            v.z += 1.0;
        }
        if held(&[KeyCode::KeyS, KeyCode::ArrowDown]) {
            v.z -= 1.0;
        }
        if held(&[KeyCode::KeyD, KeyCode::ArrowRight]) {
            v.x += 1.0;
        }
        if held(&[KeyCode::KeyA, KeyCode::ArrowLeft]) {
            v.x -= 1.0;
        }
        if held(&[KeyCode::Space, KeyCode::KeyE]) {
            v.y += 1.0;
        }
        if held(&[KeyCode::KeyQ, KeyCode::KeyC]) {
            v.y -= 1.0;
        }
        let speed = if held(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            160.0
        } else {
            40.0
        };
        self.camera.fly(v.normalize_or_zero() * speed * dt);
    }

    /// Renders a frame. Returns `false` when the viewer should quit.
    fn frame(&mut self) -> bool {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.update(dt);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                return true;
            }
            _ => return true,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let opts = match self.play.as_ref() {
            Some(p) => {
                p.render_sky(&self.gpu, &mut encoder, &view, &self.camera, aspect);
                p.scene_options(&self.opts)
            }
            None => self.opts,
        };
        self.scene.renderer.render(
            &self.gpu,
            &mut encoder,
            &view,
            &self.depth,
            &self.camera,
            aspect,
            &opts,
        );
        if let Some(play) = self.play.as_mut() {
            play.render(
                &self.gpu,
                &mut encoder,
                &view,
                &self.depth,
                &self.camera,
                aspect,
                &opts,
            );
            play.render_hud(
                &self.gpu,
                &mut encoder,
                &view,
                (self.config.width, self.config.height),
            );
        }
        self.gpu.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);

        self.frames += 1;
        self.total_frames += 1;
        if let Some(n) = self.exit_after
            && self.total_frames >= n
        {
            let secs = self.started.elapsed().as_secs_f32();
            println!(
                "{} frames in {secs:.2}s: {:.0} fps average at {}x{} ({:?})",
                self.total_frames,
                self.total_frames as f32 / secs,
                self.config.width,
                self.config.height,
                self.config.present_mode
            );
            return false;
        }
        let elapsed = self.fps_timer.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.frames as f32 / elapsed;
            self.frames = 0;
            self.fps_timer = Instant::now();
            let play = self
                .play
                .as_ref()
                .map_or(String::new(), |p| format!(" — {}", p.status()));
            self.window.set_title(&format!(
                "Ballistic viewer — {} — {:.0} fps — {}×{} — {} tris{play}",
                self.scene.key,
                self.fps,
                self.config.width,
                self.config.height,
                self.scene.renderer.triangle_count
            ));
        }
        true
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.init(event_loop) {
            Ok(s) => self.state = Some(s),
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                if !state.frame() {
                    event_loop.exit();
                }
                // The score screen is gone: on to the next level.
                match state
                    .play
                    .as_mut()
                    .and_then(crate::play::Play::take_next_level)
                {
                    Some(crate::play::AfterScore::Level(key)) => {
                        eprintln!("next level: {key}");
                        if let Err(e) = state.open_scene(&mut self.bundle, &key) {
                            self.fail(event_loop, e);
                            return;
                        }
                    }
                    Some(crate::play::AfterScore::LastLevel) => {
                        eprintln!("that was the last level");
                    }
                    None => {}
                }
                if let Some(play) = state.play.as_mut() {
                    let sounds = play.take_sounds();
                    let loops = play.loops();
                    if let Some(audio) = self.audio.as_mut() {
                        for (key, volume, pitch) in sounds {
                            audio.play(&mut self.bundle, &key, volume, pitch);
                        }
                        audio.sync_loops(&mut self.bundle, &loops);
                    }
                }
            }
            WindowEvent::Focused(false) => {
                state.keys.clear();
                state.set_looking(false);
            }
            WindowEvent::MouseInput {
                state: s,
                button: MouseButton::Right,
                ..
            } => state.set_looking(s == ElementState::Pressed),
            WindowEvent::KeyboardInput { event, .. } => {
                if let Err(e) = state.key(&mut self.bundle, event_loop, &event) {
                    self.fail(event_loop, e);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (Some(state), DeviceEvent::MouseMotion { delta }) = (self.state.as_mut(), event)
            && state.looking
        {
            let sensitivity = 0.0025;
            state
                .camera
                .rotate(delta.0 as f32 * sensitivity, -delta.1 as f32 * sensitivity);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}
