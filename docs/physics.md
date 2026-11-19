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
- **Moving elements** (`ballistic_physics::modules`, from the bundle's
  `modules`): bodies made of the **convex hulls** of their collision meshes,
  with **hinges** to the world or to each other. Mass properties come from
  the hulls' volume (uniform density). With automatic mass centre off, the
  mass centre is the object's origin plus *Shift Mass Center*, in the object's
  frame, as the original's Physicalize builds it (with it on, the hulls'
  centroid). Modules also use **sliders**, **springs** and **ball joints**
  (below), and pushes switched on and off by their scripts.
- **Sleeping**: a body can start asleep (the original's "Start Frozen"). It
  doesn't move, and others collide with it as if it were fixed, until it's
  woken: explicitly (the scripts' Physics WakeUp), by a joint or spring to a
  moving body, or by an awake body touching it. These are the original's
  rules (research findings 13).
- **Contacts between bodies**: sphere–sphere, sphere–box, sphere–hull and
  between boxes and hulls.
- **Pushes at a point**: a push can act at a point on the body (fixed on it
  when created), so off the mass centre it also turns the body. Directions are
  fixed in world space when created, as for the ball.
- **Bodies without collision** (the original's "Enable Collision" off) move
  and take part in joints, but touch nothing (the punching bag's rope).
- Ball transformers aren't physics: the play mode takes the ball out of the
  world while it's pulled in, and adds the new ball as a new body (viewer
  docs).
- **One-off impulses** (the original's *Physics Impulse*): the velocity change
  of one push step, once, at a world point. The burst gives each piece of the
  old ball one. The pieces are convex bodies built like the moving elements'
  (`modules::convex_shape`).
- **Collision reports**: a body can report its collisions (for sounds). A
  collision is a contact pair, with another body or static geometry, that
  wasn't touching at the step before. It's reported with the relative speed
  at the contact point and the simulated time.

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

**Hinges** are solved with the contacts:
- In each pass, before the contacts, their positions are corrected (4 sweeps,
  for chains): the axes are aligned, then the pivots joined with a position
  impulse through the full effective mass, so a plank turns about its other
  pivot instead of sliding.
- In the velocity iterations, the pivots' point velocities are made equal
  (3×3 effective mass) and relative rotation across the axis is removed.
- Rotation about the axis is free; the original's limits are off on every
  hinge in Ballance.

**Sliders** keep a body's orientation relative to the other body (or the
world) and let it move only along the axis.
- Position correction restores the relative orientation, then moves the body
  back onto the axis, and inside the limits if set, by inverse mass.
- In the velocity iterations, relative rotation is removed (3×3 effective
  mass), and so is motion across the axis at the axis origin.
- Past a limit, motion further out of range is removed.
- The original's limits bound the other body's travel relative to the slider's
  body, the opposite of its own travel, so the module plan flips their sign
  (research findings 13). Only the balloon's slide box has limits.

**Ball joints** are hinges without the axis: the pivots are joined, and
rotation is free.

**Springs** pull a point on one body towards a point on the other (or the
world), with force `constant · (distance − length)` along the line between
them. Damping is `linear_damping` times the points' relative velocity along
that line, plus `global_damping` times their whole relative velocity. The
forces are applied as impulses each step, with the pushes. A sleeping body
gets none, and a spring doesn't wake it. That's the original's spring, with
absolute values (research findings 13).

**Collision detection.** Spheres use closest points (to triangles, boxes, hulls
and other spheres). Boxes and triangles are convex polyhedra: the separating-axis
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
| Joint position sweeps per pass | 4 | |
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

The joint tests check:
- a hull box resting like a box;
- a hinged rod swinging with the period of a physical pendulum, its pivot
  fixed;
- a chain of planks between two pivots staying connected as it sags;
- a frozen body waiting until it's touched or woken;
- a ball knocking over a hinged flap;
- a slider keeping a kicked body on its axis without turning;
- a spring on a vertical slider oscillating at √(k/m) and, damped, settling
  mg/k below its anchor;
- slider limits stopping a fall;
- a spring turning a hinged arm towards its anchor;
- a ball-joint pendulum swinging in two directions, its pivot fixed;
- a push off the mass centre turning a body;
- a body without collision falling through the floor.

The hull unit tests check hull construction and mass properties (parallel
axes).

The bundle tests drop a wooden ball on Level 1's start pad and roll it, and
place every prop of all 12 levels: after three simulated seconds nearly all
of them rest close to their placeholders. They also place every moving
element of all 12 levels, awake: after three seconds no simulated module has
come apart (hinge pivots within 0.2 of their frames, slider bodies within
0.2 of their axes).
