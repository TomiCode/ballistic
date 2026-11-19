# Physics (`ballistic-physics`)

A small, deterministic rigid-body simulation written for Ballistic. Its job is
to reproduce how the ball *feels* in the original, because speedrun routes
depend on it. It is a prototype: the parts that are known from the original
are implemented as such, and the parts that aren't known yet are explicit
options so they can be calibrated later.

## What's simulated

- **Balls** as spheres (the paper ball is a convex hull in the original. For
  now it's simulated as a sphere of the hull's bounding radius).
- **Level geometry** as static triangle meshes: every object in the level
  groups `Phys_Floors`, `Phys_FloorRails` and `Phys_FloorStopper`, in world
  space, behind a bounding volume hierarchy.
- Not yet: dynamic props (boxes, domes, loose balls), moving level parts
  (`P_Modul_*`), constraints, ball transformers.

## Time

| | |
|---|---|
| Step | fixed **1/66 s** of simulated time |
| Simulated time | `time_factor` × real time. Ballance uses **2**, so 132 steps per real second |
| Rendering | interpolated between the last two steps, so any refresh rate works |
| Long frames | at most 1 s of simulation is caught up per frame |

Stepping is independent of the frame rate, so the same inputs at the same
step numbers always give bit-identical results (there's a test for it).

## One step

1. **Pushes.** Each active push adds `impulse / mass` to its body's velocity.
   There's no Δt factor: the original's "force" is a per-step impulse. A
   push's direction is fixed in world space when it is created.
2. **Gravity**: `v += g · Δt`, with `g = (0, −20, 0)` from the game's settings.
3. **Damping** of linear and angular velocity (see the options below).
4. **Integration** of position and orientation.
5. **Contacts**, up to 4 passes. Each pass finds the triangles the sphere
   overlaps and handles each distinct contact:
   - pushes the ball out along the contact normal (80 % of the depth),
   - removes the approaching normal velocity, bouncing with the combined
     elasticity if the impact is faster than 1 unit/s,
   - applies Coulomb friction at the contact point, which spins the ball up
     (rolling) instead of letting it slide.

   Contacts on the faces of a tessellated floor are merged by normal, so a
   ball rolling over internal edges doesn't bump. Back faces don't collide.
   A ball doesn't collide with surfaces in its own collision group (that's how
   `Phys_FloorStopper` lets the player ball through).

## Parameters

All values come from the bundle (`data/physics.json`, extracted by
`ballistic-pack` from the original's scripts):

| Ball | Mass | Push | Friction | Elasticity | Linear / rotational damping | Radius |
|---|---|---|---|---|---|---|
| Wood | 1.9 | 0.43 | 0.8 | 0.2 | 0.9 / 0.1 | 2 |
| Stone | 10 | 0.92 | 0.5 | 0.1 | 0.3 / 0.1 | 2 |
| Paper | 0.2 | 0.065 | 0.5 | 0.4 | 1.5 / 0.1 | 2.11 (hull) |

Level surfaces (`Phys_Floors`, `Phys_FloorRails`, `Phys_FloorStopper`): friction
0.7, elasticity 0.3. Values are as packed from the GOG release. Your own
bundle has the authoritative numbers.

## Options still to calibrate

`WorldConfig::contact` (`ContactModel`) holds the choices that aren't known
from the original yet. The defaults are reasonable guesses:

| Option | Default | Alternatives |
|---|---|---|
| Friction combination (ball × surface) | geometric mean | multiply, average, min, max |
| Elasticity combination | multiply | same set |
| Damping law | linear: `v ·= 1 − k·Δt` | exponential: `v ·= e^(−k·Δt)` |
| Bounce threshold | 1 unit/s | |
| Penetration correction | 0.8 per pass | |
| Contact passes | 4 | |
| Same collision group ignores contact | on | |

They will be fixed by comparing against recorded runs of the original.

## Tests

```bash
cargo test -p ballistic-physics
BALLISTIC_BUNDLE=ballance.bpk cargo test -p ballistic-physics --test real_bundle
```

The model tests check free fall, push impulses, time factor, resting on a
floor, rolling down a slope at the textbook rate for a solid sphere
(a = 5/7 · g · sin θ) without slipping, collision groups, and determinism.
The bundle test drops a wooden ball on Level 1's start pad and rolls it.
