# Level viewer (`ballistic-viewer`)

Flies a free camera around any scene in your bundle: levels, prefabs, ball
models, the menu scene. It's the first piece of the engine (winit + wgpu)
and is used to check rendering against the original.

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
  them with prefabs at load time
- Debug overlay UI
