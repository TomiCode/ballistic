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
| `--play-script <keys:seconds,...>` | With `--play --screenshot`: inputs to simulate before the frame, e.g. `wait:4,up:2,rotate+left:0.3` (the ball comes 3 s after the start). Keys: `up`, `down`, `left`, `right`, `rotate`, `lift`, `wait`; `next` warps to the next checkpoint (then the level end), `wake` wakes every active moving element as if the ball came near (its wake-up targets), `trafo` puts the ball on the nearest transformer of another type (shown ones first), `pickup` on the nearest extra life or extra point, and `lose` loses the ball (as **R**), all at the start of their segment. A checkpoint arms a while after the last one, so leave 2 s or more between `next`s. The summary then also prints the points, lives and score, and any moving element that fell below the level's depth cubes. With `--camera` too, the camera stays at that view instead of following the ball |

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
| **R** | Lose the ball (it comes back at the current reset point) |
| **N** | Warp to the next checkpoint, then to the level end (development aid) |
| **1** | Next ball type (wood, stone, paper) |

Details carried over from the original's scripts:

- Holding **Shift** switches ball control off. You can't push while turning.
- A push's direction is fixed when you press the key. After a camera turn,
  directions you're holding are re-aimed one frame after the turn finishes.
- The ball comes 3 s after the level start or a loss, with the lightning
  sound. Until then you can turn the camera but not push.
- The ball is lost when it falls into one of the level's depth cubes
  (`DepthTestCubes`, one tested per frame). Control, the camera and the points
  countdown stop at once; 1 s later the ball is taken away, a life is used,
  the sector is reset, and a new ball comes. (A level without depth cubes
  loses it 50 units below its lowest geometry.)

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

**Points and lives**, as the original counts them (research findings 16):
- A level starts with 1000 points and 3 lives.
- One point goes every 500 ms until the finish.
- Losing the ball costs a life. With none left the game is over: the ball
  stays lost and control is off.
- **Extra lives** (+1) and **extra points** (+100, then +20 for each of its
  six small balls as they reach the ball: 220 in all) are only there while
  their sector is active. The level's placeholder markers stand in for them
  until their particle effects exist. An extra life comes back when its
  sector is entered again; an extra point doesn't.
- At the finish the score is printed: the level's bonus (100 × its number) +
  the points left + 200 per life left.
- The status line shows the points and lives, and `[new ball]` or `[ball
  lost]` while one is coming or going. There's no HUD or score screen yet.

**Transformers** (`P_Trafo_Paper`, `_Stone`, `_Wood`) stand at their
placeholders, shown while their sector is active. They work like the
original's script (research findings 14):
- Every frame it takes the transformer nearest to the ball, shown or not.
- If the ball's centre is within 4.3 of it and the ball is of another type,
  ball control goes off.
- Two frames later the ball leaves the simulation and is pulled into the
  ring for 1.35 s, to 3 above the transformer's origin.
- It hangs there for 1 s; 0.15 s later the new ball appears in its place.
- Two frames later the new ball is simulated, at rest, and control comes back.

Meanwhile the transformer plays its animation:
- The transformer is replaced by the animation's copy, in its colour.
- A frame later the ring springs open (0.35 s).
- The bars rise and sink again (2 s), the flash field flickering between them.
  Meanwhile the shadows of that transformer type fade and come back.
- The ring closes (0.2 s) and the transformer is back.

At the burst the old ball falls apart into its pieces (16 wood, 17 stone or 18
paper). They fly outwards and land on the level, but never touch the ball.
They fade out 20 s after the transformation, or at once if the ball
transforms from the same type again, or at a respawn.

The ball keeps its new type when it respawns. The status line counts the
pieces out.

**Sounds** (play mode in a window, through kira on the default output device;
silent if there's none):
- a transformation plays `Misc_Trafo` a frame after it starts;
- paper pieces play `Pieces_Paper` once at the burst;
- for 3 s after a wood or stone burst, six of the pieces play their sound
  when they hit the level, louder for harder hits (the original's collision
  sound rules);
- the ball's **hits**: hitting stone, wood, metal or a dome faster than 2
  (1 for domes) plays the ball type's hit sound for it, louder for harder
  hits, at most once a second per surface kind (metal: every 2 s);
- the ball's **rolls**: after 0.3 s on stone, wood or metal, that roll sound
  loops until 0.3 s after the ball leaves it. Louder and higher the faster the
  ball goes; the paper ball has one roll sound;
- **events**, as the sound scripts play them: the level start, the
  lightning before each ball, a lost ball, a checkpoint, an extra life (and
  317 ms later its jingle), an extra point and its small balls.

Hits and rolls only sound while ball control is on: not during a
transformation, nor after the finish. Surfaces get their kind from the level's
sound groups, props and moving elements from their prefabs.

With `--screenshot` nothing is played; the summary counts the sounds.
A prop that falls 200 below the lowest depth cube is gone until its sector is
entered again.

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
ballistic-viewer ballance.bpk --play --scene level/level_03   --play-script "wait:4,up:1.5" --screenshot play.png
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
- In play mode: the HUD and score screen, the pickups' looks and sounds,
  checkpoint flames and fan particles, the other
  sounds (the wooden flaps, the level finish, music), the start flames,
  the screen fade, the paper pieces' wind, the end
  sequence (and the last level's UFO) and score
