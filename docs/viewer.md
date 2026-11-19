# Level viewer (`ballistic-viewer`)

Flies a free camera around any scene in your bundle: levels, prefabs, ball
models, the menu scene. It's the first piece of the engine (winit + wgpu)
and is used to check rendering against the original. With `--play` it puts a
physics ball on the level (see [Play mode](#play-mode)).

## Usage

```bash
ballistic-viewer ballance.bpk                          # opens level/level_01
ballistic-viewer ballance.bpk --scene level/level_08
ballistic-viewer ballance.bpk --list                   # all scene keys
ballistic-viewer ballance.bpk --scene ph/pe_balloon --show-hidden
```

| Option | Meaning |
|---|---|
| `--scene <key>` | Scene to open (default `level/level_01`) |
| `--camera x,y,z,yaw,pitch` | Initial camera (degrees). Press **P** in the viewer to print the current one |
| `--show-hidden` | Draw objects the level hides at load (invisible collision, prefab parts) |
| `--fullscreen` | Start in borderless fullscreen |
| `--no-vsync` | Start with vsync off |
| `--cull back\|none\|front` | Face culling (debugging) |
| `--screenshot out.png [--size 1920x1080]` | Render one frame off-screen and exit, no window |
| `--exit-after <frames>` | Quit after N frames and print the average frame rate |
| `--play` | Play mode: roll a ball on the level |
| `--play-script <keys:seconds,...>` | With `--play --screenshot`: inputs to simulate before the frame, e.g. `wait:1,up:2,rotate+left:0.3`. Keys: `up`, `down`, `left`, `right`, `rotate`, `lift`, `wait`; `next` warps to the next checkpoint (then the level end) and `wake` wakes every active moving element as if the ball came near (its wake-up targets), both at the start of their segment. With `--camera` too, the camera stays at that view instead of following the ball |

The window is resizable, any size and aspect ratio work, and HiDPI is handled.

## Controls

| Input | Action |
|---|---|
| Right mouse (hold) or **Tab** | Look around |
| **W A S D** / arrows | Move |
| **Space** / **E**, **Q** / **C** | Up, down |
| **Shift** | Move faster |
| **PageUp / PageDown**, **[ ]** | Previous / next scene |
| **Home** | Reset camera |
| **H** | Toggle hidden objects |
| **B** | Cycle face culling |
| **V** | Toggle vsync |
| **F11** | Fullscreen |
| **P** | Print camera pose |
| **Esc** | Release mouse, or quit |

## Play mode

```bash
ballistic-viewer ballance.bpk --play --scene level/level_03
```

A wooden ball spawns at the level's first reset point and the camera starts
behind it, both as the original sets them. Control works like the original:

| Input | Action |
|---|---|
| Arrows | Push the ball forward / back / left / right, relative to the camera |
| **Shift** + **Left / Right** | Turn the camera 90° |
| **Space** (hold) | Lift view: the camera rises 50 units for a look from above |
| **R** | Respawn (at the current reset point) |
| **N** | Warp to the next checkpoint, then to the level end (development aid) |
| **1** | Next ball type (wood, stone, paper) |

Details carried over from the original's scripts:

- Holding **Shift** switches ball control off. You can't push while turning.
- A push's direction is fixed when you press the key. After a camera turn,
  directions you're holding are re-aimed one frame after the turn finishes.
- The ball respawns when it falls 50 units below the lowest level geometry.

Level 1 starts facing away from the path, as in the original (its tutorial
teaches turning first): press Shift + Left twice.

Physics is described in [physics.md](physics.md).

**Props.** Boxes, loose balls and domes stand where the level's placeholders
are (the placeholders themselves are hidden, as the game replaces them), drawn
with their prefabs. You can push boxes and balls around; domes don't move.

**Checkpoints and sectors** work like the original's scripts. Props belong to
the sector they're placed in, and only the current sector's props are there.
Checkpoints must be passed in order. Rolling through the next one (within
6.5 of its flame) does three things:
- moves the respawn point on;
- removes the previous sector's props;
- brings in the next sector's, back at their places, 4 frames later.

A respawn resets the current sector's props. The window title shows the
checkpoints reached. The checkpoint flames aren't drawn yet (they're
particles).

**Moving elements** (`P_Modul_*`) are placed and drawn like the original's.
They're physical while their sector is active. Frozen parts stay still until
the ball comes near (each module's own wake-up distance), and then fall,
swing or tip by themselves.
- All of them are simulated:
  - the hinged ones (bridges that fall over, flaps, seesaws, the plank bridge);
  - the rotating arms (hinge and spring);
  - the lift (slider and spring);
  - the sliding stone (slider) and the pusher;
  - the swing, pushed one way for 0.5 s, resting 0.5 s, pushed back, resting;
  - the punching bags (ball joints), pushed each way in turn for 1.5 s;
  - the fans, which push the ball up while it's in their air column. That's
    only strong enough for the paper ball.
- The lift doesn't rise: with the original's values it can't carry the walls
  standing on it (research findings 13).

**The level end** is the balloon, part of the last sector. The level is
finished when the ball settles in the middle of its platform (within 1 of the
platform's centre, as in the original). Ball control then switches off for
good, and the window title shows the time since the level started.
- The balloon is simulated like a moving element. Its platform hangs from
  four balloons on ropes, on a slider and spring under a hidden slide box, and
  a plank bridge leads to it.
- When the ball comes within 70, the platform wakes and the balloons pull up
  and out.
- At the finish, the bridge's last hinge breaks and the slide box is pushed
  along its slider: the balloon drifts away with the ball.
- There's no end sequence or score screen yet, and the last level's UFO isn't
  drawn.
A prop that falls out of the level is gone until its sector is entered again.

The camera follows like the original: a point chases the ball, and the camera
chases a spot 22 units behind and 35 above that point, with the original's pull
and momentum per axis. It reacts slowly in height, so it stays high when the
ball drops. Field of view is the original's at 4:3 (45° vertical), kept at
every aspect ratio. Holding Space lifts the camera's goal 50 units and
strengthens its vertical pull, as the original's lift view does; it changes
nothing else. In the original this filter ran once per rendered frame,
so its feel depended on frame rate. Here it runs at a fixed 60 Hz and is
interpolated, so it feels the same at any refresh rate. The turn's easing
curve is still an approximation.

Headless check of the same thing:

```bash
ballistic-viewer ballance.bpk --play --scene level/level_03   --play-script "wait:1,up:1.5" --screenshot play.png
```

## How it renders

- Fixed-function-style shading: one directional light plus ambient, material
  diffuse/ambient/emissive, texture blend modes, alpha test, alpha blending
  (sorted back to front), per-material depth state, two-sided materials, and
  sphere-mapped environment textures.
- All colour maths happens in **gamma space** (non-sRGB targets and texture
  views), like the original Direct3D renderer, so colours match its look.
- Textures are uploaded as BC1/BC3 directly when the GPU supports it. Otherwise
  they're decoded on the CPU.

## Not yet

- Sky boxes, per-level fog and lighting (they're set by the game's scripts)
- Placeholder markers (`P_*` objects) show as simple shapes. The game replaces
  them with prefabs at load time (play mode does this for props)
- Debug overlay UI
- In play mode: pickups, transformers, checkpoint flames and fan particles,
  the end sequence (and the last level's UFO) and score
