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
| `--game` | The game: starts at the main menu over the menu level; levels are played from its level select, with pause (Escape), end and game over menus. Implies play mode |
| `--menu-keys <codes>` | With `--game --screenshot`: menu keys to press first, as `DirectInput` scan codes (`28,208`: Enter, Down); `--play-script wait:N` lets N seconds pass (the menu level's flight, the credits). Development aid |
| `--save <path>` | Play mode's save file (open levels, highscores, the last name). In a window it defaults to the platform's data folder (`%APPDATA%\Ballistic\save.json`, `~/Library/Application Support/Ballistic/save.json`, `$XDG_DATA_HOME/ballistic/save.json`); headless runs keep it in memory unless given. A file that can't be read is left alone and the session isn't saved |
| `--play-script <keys:seconds,...>` | With `--play --screenshot`: inputs to simulate before the frame, e.g. `wait:4,up:2,rotate+left:0.3` (the ball comes 3 s after the start). Keys: `up`, `down`, `left`, `right`, `rotate`, `lift`, `wait`; `next` warps to the next checkpoint (then the level end), `wake` wakes every active moving element as if the ball came near (its wake-up targets), `trafo` puts the ball on the nearest transformer of another type (shown ones first), `pickup` on the nearest extra life or extra point, `fan` 2 above the nearest fan in an active sector, `near` 8 from the nearest pickup (to look at it without taking it), `lose` loses the ball (as **R**), `skip` presses Space (cuts the level end's wait short, scores fast, leaves the score screen), and `enter` presses Enter (goes on in level 1's tutorial, confirms the highscore name, leaves the table), all at the start of their segment. A checkpoint arms a while after the last one, so leave 2 s or more between `next`s. The summary then also prints the points, lives and score, and any moving element that fell below the level's depth cubes. With `--camera` too, the camera stays at that view instead of following the ball |

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
- The ball comes 3 s after the level start or a loss, in a lightning
  (research findings 30): a sphere at its place grows over 1.5 s, turning
  and flickering, with the lightning sound; a smoke puff just before the
  ball appears (also when a transformer bursts the ball). Until then you can
  turn the camera but not push.
- The ball is lost when it falls into one of the level's depth cubes
  (`DepthTestCubes`, one tested per frame). Control, the camera and the points
  countdown stop at once; 1 s later the ball is taken away, a life is used,
  the sector is reset, and a new ball comes. Meanwhile the screen goes white
  and clears again over 2 s. (A level without depth cubes loses it 50 units
  below its lowest geometry.)
- The reset points' and the level start's placeholders aren't drawn (the
  level's init deletes them).

Level 1 starts facing away from the path, as in the original (its tutorial
teaches turning first): press Shift + Left twice.

**Level 1's tutorial** (research findings 29) runs at its every start. Its
texts come in a dark box at the bottom; the first ball waits for the
intro: **Return** goes on, **Q** (or 25 s without either) ends the
tutorial. Then: turning the view and lifting it (arrows round the ball;
Return goes on), rolling (the physics runs; each move arrow grows while
its key is held, until all four were used or the ball reaches the end of
the first track), and as the ball comes near the extra life, the stone
and wood transformers (a wooden ball skips the latter), the extra point and
the first checkpoint: the physics stops with a sound and an arrow points
at the place; Return lets the ball roll on. 4 s after the checkpoint, the
hints; Return ends it.

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
checkpoints reached.

**Flames** (research findings 20), as the original's particle systems:
- the level start's four flames burn while the ball is within 70 (on X and
  Z);
- the next checkpoint's big flame burns while the ball is within 70.
  Reaching it lights the two small flames instead;
- only the next checkpoint and the last one reached have flames.

Each flame emits a particle every 20 ms that rises, shrinks and fades over
about a second. It's drawn as a camera-facing triangle of the flame texture,
blended additively. The summary of a headless run counts the flames burning.

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
    only strong enough for the paper ball. While the ball is within 80, a
    fan's rotor turns and wind lines and smoke rise from it (research
    findings 20).
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
- The last level's UFO isn't drawn.

**The level end and the score screen**, like the original's (research
findings 23):
- At the finish the camera stops where it is and watches the balloon carry
  the ball off; the clouds fade to black.
- 10 s later (23 s after the last level), or on Space, Enter or Escape, the
  score screen fades in over the level: the level bonus, then the points
  counted over (the HUD counting down, with the counter's clicks rising in
  pitch), then each life left (the HUD taking a ball each time), then the
  total.
- Space, Enter or Escape while it counts shows everything at once; on the
  total they leave (it leaves by itself after 4 s).
- Then the next level opens in the save. A score above a level's tenth
  place brings the **name entry** (research findings 24): type the name
  (starting as the last one entered; up to 9 characters, Backspace to
  correct), Enter to confirm. The score goes into the table, which shows
  next: Left and Right go through the levels' tables, Enter or Escape goes
  on. The save is written then.
- Then the next level loads. After the last level the view stays (the
  menus aren't there yet).
- While the level end or its screens are on, Escape doesn't close the
  viewer, and during the name entry every key types.

**The game** (`--game`, research findings 25) wraps the levels in the
original's menus:
- The **main menu** over the menu level: Start, Highscore, Options, Credits,
  Exit (asks first). Up and Down choose (round), Enter or a click picks,
  Escape picks Exit; pointing at a button makes it the active one.
- **Start** lists the levels: the ones the save has open can be played, the
  others are grey.
- In a level, **Escape pauses**: the level waits under the pause menu
  (Restart and Exit level ask first; Back or Escape goes on).
- After the score (and a highscore's name and table) comes the **end
  menu**: Restart, Highscore, Options, Home, Next level (Escape).
- With the last life gone, the **game over** menu: Restart, Home.
- A highscore table opened from a menu starts at the current level and
  goes back to it.
- Behind the main menu the **menu level** (research findings 28): the
  camera goes round it every 44 s under level 5's sky, in fog, with its four
  flames burning and its ambient sound now and then.
- **Credits**: the two logos, then the pages fading in and out, round and
  round; Escape, Enter or *Back* leave. Typing "gravity" there opens every
  level.
- **Options** (research findings 27): *Graphics* (the window's size from
  the original's list, or as it opened; vsync; the cloud layer), *Controls*
  (the six keys: Enter on one, then press the new key; a key in use
  elsewhere swaps; invert rotation) and *Sound* (the music's volume in
  tens). Up and Down choose a row, Left and Right change it, OK or Escape
  goes back. Changes apply at once (the clouds with the next level) and
  are saved with the rest (`--save`).

**Points and lives**, as the original counts them (research findings 16):
- A level starts with 1000 points and 3 lives.
- One point goes every 500 ms until the finish.
- Losing the ball costs a life. With none left the game is over: the ball
  stays lost and control is off.
- **Extra lives** (+1) and **extra points** (+100, then +20 for each of its
  six small balls as they reach the ball: 220 in all) are only there while
  their sector is active.
  - An **extra life** looks like the original's (research findings 21): an
    oily bubble with a silver ball inside, squashing, its texture swirling.
    Over a floor it bounces, with a glowing shadow below. It shows while the
    ball is within 60 on X and Y. Taking it bursts its particles (sparks and
    fizz lines).
  - An **extra point** looks like the original's (research findings 22): a
    silver centre ball with six small ones circling it, trailing sparkles,
    and a glow on the floor below. It shows while the ball is within 80.
    Taking it bursts sparks from the centre; the small balls fly off, then
    home in on the ball, each bursting in sparks when it arrives.
  - An extra life comes back when its sector is entered again; an extra
    point doesn't.
- At the finish the score is printed: the level's bonus (100 × its number) +
  the points left + 200 per life left.
- The status line shows the points and lives, `[new ball]` or `[ball lost]`
  while one is coming or going (`[lightning]`, `[puff N]` with the smoke's
  particles), `[tutorial N]` in level 1's tutorial, then `[level end]` and
  `[score screen N]`.

**The HUD** is the original's (research findings 18), from its own sprites
and font:
- bottom left, the points in their frame;
- bottom right, a ball per life. An extra life slides the end cap left and
  fades a ball in. A lost life fades the last ball out 2 s after the loss,
  then the cap slides back;
- an extra point makes the points' frame glow and fade.

The original stretched the HUD with the window. Here it keeps its 4:3
proportions: on a wide window the left part stays at the left edge and the
lives at the right. Screenshots (`--screenshot`) include it in play mode.

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
  317 ms later its jingle), an extra point and its small balls;
- the **wooden flaps** (`Phys_FloorStopper`): the ball hitting them faster
  than 0.3, louder up to 10, at most every 0.5 s.

**Music** (research findings 26): it fades in at the level start. Now and
then (waits up to 15 s) one of three ambient pieces plays; from 7 s in, the
level's themes the same way (waits up to 50 s). At the last checkpoint the
themes stop and a loop sounds and fades out; near the level end it fades
back in, and at the finish the finish music plays (another after the last
level). A new highscore plays its tune. Leaving a level fades its music out.
On the last level a storm flashes now and then (10 to 90 s apart), with
thunder.

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

- In play mode, the level's **sky** first, like the original's *TT Sky*
  (research findings 19): a cube of five textures around the camera's
  position, turning with the camera, with a field of view widened 15 % of
  the way to 180° and no depth. The level then draws over it, lit by the
  original's light (its direction, the level's colour) and the scene's 6 %
  ambient. There's no fog in play. Level 12's cloud layer is shown,
  additive, and scrolls. Outside play mode, a default light, no sky.
- Fixed-function-style shading: one directional light plus ambient, material
  diffuse/ambient/emissive, texture blend modes, alpha test, alpha blending
  (sorted back to front), per-material depth state, two-sided materials, and
  sphere-mapped environment textures.
- All colour maths happens in **gamma space** (non-sRGB targets and texture
  views), like the original Direct3D renderer, so colours match its look.
- Textures are uploaded as BC1/BC3 directly when the GPU supports it. Otherwise
  they're decoded on the CPU.

## Not yet

- The sky and the original's lighting outside play mode
- Placeholder markers (`P_*` objects) show as simple shapes. The game replaces
  them with prefabs at load time (play mode does this for props)
- Debug overlay UI
- In play mode: clicking the highscore buttons, explosions, the screen fade, the
  paper pieces' wind, the last level's UFO
