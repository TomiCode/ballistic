//! Physics data extracted from the game's scripts and tables (`data/physics.json`).
//! Bundle space: right-handed, Y up, original units.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsData {
    /// World gravity during play.
    pub gravity: [f32; 3],
    /// Simulated seconds per real second during play.
    pub time_factor: f32,
    /// Player ball types (`Physicalize_GameBall`).
    pub balls: Vec<BallParams>,
    /// Static level surfaces by level group (`Physicalize_Floors`).
    pub surfaces: Vec<SurfaceParams>,
    /// Movable level props by prefab group (`Physicalize_Convex`, `Physicalize_Balls`).
    pub props: Vec<PropParams>,
    /// Moving elements (`P_Modul_*` prefabs): bodies and joints from their scripts.
    pub modules: Vec<ModuleSetup>,
}

/// One moving element's physics, as its prefab script sets it up. Object and
/// mesh names refer to the prefab scene (`ph/<group>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleSetup {
    /// Prefab group, e.g. `P_Modul_25`.
    pub group: String,
    pub bodies: Vec<ModuleBody>,
    pub hinges: Vec<HingeParams>,
    /// Proximity test that wakes the frozen bodies (`TT Scaleable Proximity`
    /// into `Physics WakeUp`), if the script has one.
    pub wake: Option<WakeParams>,
    /// Physics building blocks the script also uses that aren't in this data
    /// yet (`Set Physics Slider`, `SetPhysicsForce`, …).
    pub unsupported: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the original Physicalize inputs"
)]
pub struct ModuleBody {
    /// The physicalized object.
    pub object: String,
    pub fixed: bool,
    pub friction: f32,
    pub elasticity: f32,
    pub mass: f32,
    pub collision_group: String,
    pub start_frozen: bool,
    pub collision: bool,
    pub auto_mass_center: bool,
    /// Mass centre offset in the object's frame (`Shift Mass Center`).
    pub shift_mass_center: [f32; 3],
    pub linear_damping: f32,
    pub rotational_damping: f32,
    /// Meshes whose convex hulls make up the body, in the object's frame.
    pub convex: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HingeParams {
    pub body: String,
    /// The other body; `None` is the world (the level's fixed `FixCube`).
    pub other: Option<String>,
    /// The joint frame: its position is the pivot, its Z axis the hinge axis.
    pub frame: String,
    /// Angle limits in degrees, if enabled.
    pub limits: Option<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WakeParams {
    /// The object the ball's distance is measured to.
    pub object: String,
    pub distance: f32,
    pub exactness: [f32; 2],
    pub frame_delay: [i32; 2],
    /// Only X and Z count.
    pub xz: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BallParams {
    /// Ball object name, e.g. `Ball_Wood`.
    pub name: String,
    pub friction: f32,
    pub elasticity: f32,
    pub mass: f32,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    /// Control push per physics step (see the physics docs).
    pub force: f32,
    pub collision_group: String,
    pub shape: BallShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BallShape {
    /// Exact sphere primitive.
    Sphere { radius: f32 },
    /// Convex hull of the ball's mesh; `radius` is its bounding radius.
    Convex { radius: f32 },
}

impl BallShape {
    pub fn radius(self) -> f32 {
        match self {
            Self::Sphere { radius } | Self::Convex { radius } => radius,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceParams {
    /// Level group, e.g. `Phys_Floors`.
    pub group: String,
    pub friction: f32,
    pub elasticity: f32,
    pub collision_group: String,
    pub collision: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the original Physicalize table columns"
)]
pub struct PropParams {
    /// Prefab group, e.g. `P_Box`.
    pub group: String,
    pub fixed: bool,
    pub friction: f32,
    pub elasticity: f32,
    pub mass: f32,
    pub collision_group: String,
    pub start_frozen: bool,
    pub collision: bool,
    pub auto_mass_center: bool,
    pub linear_damping: f32,
    pub rotational_damping: f32,
    /// Set for props physicalized as a sphere primitive.
    pub sphere_radius: Option<f32>,
}
