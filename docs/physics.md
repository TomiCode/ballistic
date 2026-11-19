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
- **Props** placed by the level (`ballistic_physics::props`), from the
  placeholders in the prop groups of the physics data and the prefab's main
  frame `<group>_MF`:
  - `P_Box`: a dynamic **box**. The original uses the mesh's convex hull, and
    that hull is a 5 × 5 × 5 cube.
  - `P_Ball_Wood`, `P_Ball_Stone`: dynamic spheres (the original's ball primitive).
  - `P_Ball_Paper`: a convex hull in the original, a sphere of its bounding
    radius here (like the player's paper ball).
  - `P_Dome`: fixed, so its mesh is added to the static geometry.
- **Contacts between bodies**: sphere–sphere, sphere–box and box–box.
- Not yet: moving level parts (`P_Modul_*`), constraints, ball transformers,
  general convex hulls, sleeping.

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
5. **Contacts**, up to 4 passes. Each pass:
   - finds every contact, against static geometry and between bodies, as
     *manifolds* (one normal, up to four points);
   - pushes the bodies apart along each normal by 80 % of its deepest point,
     split by inverse mass;
   - solves velocities with sequential impulses (8 iterations): at each point a
     normal impulse that stops approach, bouncing with the combined elasticity
     if the impact is faster than 1 unit/s, and Coulomb friction, which spins a
     ball up (rolling) instead of letting it slide. Impulses accumulate per
     point and are clamped (normal ≥ 0, friction within the cone).

   With a single contact point this is exactly one normal impulse followed by
   one friction impulse, as for a ball on a floor.

   A sphere's contacts on the faces of a tessellated floor are merged by
   normal, so a ball rolling over internal edges doesn't bump. A box gets one
   manifold per distinct normal, with the points of every triangle sharing it
   (its four corners on a floor of two triangles). Back faces don't collide.
   Bodies don't collide with surfaces or bodies in their own collision group
   (that's how `Phys_FloorStopper` lets the player ball through but keeps
   props in).

**Collision detection.** Spheres use closest points (to triangles, boxes and
other spheres). Boxes and triangles are convex polyhedra: the separating-axis
test over face normals and edge cross products finds the axis of least
penetration, then the incident face is clipped against the reference face
(or, for two edges, their closest points are taken), keeping at most four
points. Face axes win over edge axes, and a triangle's face over the box's,
when depths are within a few per cent, so a box doesn't catch on a floor's
internal edges. A level triangle only pushes things out of its front side.

## Parameters

All values come from the bundle (`data/physics.json`, extracted by
`ballistic-pack` from the original's scripts):

| Ball | Mass | Push | Friction | Elasticity | Linear / rotational damping | Radius |
|---|---|---|---|---|---|---|
| Wood | 1.9 | 0.43 | 0.8 | 0.2 | 0.9 / 0.1 | 2 |
| Stone | 10 | 0.92 | 0.5 | 0.1 | 0.3 / 0.1 | 2 |
| Paper | 0.2 | 0.065 | 0.5 | 0.4 | 1.5 / 0.1 | 2.11 (hull) |

Level surfaces (`Phys_Floors`, `Phys_FloorRails`, `Phys_FloorStopper`): friction
0.7, elasticity 0.3.

| Prop | Shape | Mass | Friction | Elasticity | Linear / rotational damping |
|---|---|---|---|---|---|
| `P_Box` | box 5 × 5 × 5 | 1 | 0.7 | 0.3 | 0.1 / 0.1 |
| `P_Ball_Wood` | sphere r 2 | 2 | 0.6 | 0.2 | 0.6 / 0.1 |
| `P_Ball_Stone` | sphere r 2 | 10 | 0.7 | 0.1 | 0.2 / 0.1 |
| `P_Ball_Paper` | sphere r ≈ 2 (hull bound) | 0.2 | 0.5 | 0.4 | 1.5 / 0.1 |
| `P_Dome` | fixed mesh | | 0.2 | 0.8 | |

Props have no collision group, so they collide with every surface and body. Values are as packed from the GOG release. Your own
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
| Velocity iterations per pass | 8 | |
| Face over edge preference (box contacts) | 95 % / 98 % of the depth | |
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
For props: a box resting flat without drifting, a box tipping off an edge onto
a face, two stacked boxes, the ball pushing a box, and momentum between
spheres. The unit tests check the box contacts (four points on a floor, two
along an edge, stacking) and that a level triangle never pushes a box down.

The bundle tests drop a wooden ball on Level 1's start pad and roll it, and
place every prop of all 12 levels: after three simulated seconds nearly all
of them rest close to their placeholders.
