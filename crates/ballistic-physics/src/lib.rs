//! Ballistic physics: a small, deterministic rigid-body simulation built to
//! reproduce Ballance's feel.
//!
//! What's modelled on the original (research repo, findings 09–11):
//!
//! - **Fixed step** of [`STEP_HZ`] = 66 Hz of *simulated* time, with simulated
//!   time running at `time_factor` × real time (2 in Ballance).
//! - **Pushes** (the original's "forces") are impulses applied *every step*
//!   (`Δv = F / m`), with a world-space direction fixed when created.
//! - Bodies, materials and damping values come from the bundle's physics data.
//!
//! Where the original's exact behaviour is still unknown, the choice is
//! explicit and tunable in [`ContactModel`] (damping law, material
//! combination, collision groups) so it can be calibrated against recorded
//! traces later.
//!
//! Coordinates: bundle space (right-handed, Y up, original units).

mod bvh;
mod collide;
mod hull;
mod level;
mod mesh;
pub mod modules;
pub mod props;
mod shape;
mod world;

pub use glam::{Mat4, Quat, Vec3};
pub use hull::Hull;
pub use level::{LevelBuildReport, build_level};
pub use mesh::{StaticMesh, SurfaceMaterial};
pub use shape::Shape;
pub use world::{Body, BodyDesc, BodyId, JointId, PushId, World};

/// Whether two oriented boxes (centre, orientation, half extents) overlap,
/// like the original's *Box Box Intersection* on two entities' boxes.
pub fn boxes_overlap(a: (Vec3, Quat, Vec3), b: (Vec3, Quat, Vec3)) -> bool {
    collide::poly_poly(
        &collide::Poly::cuboid(a.0, a.1, a.2),
        &collide::Poly::cuboid(b.0, b.1, b.2),
    )
    .is_some()
}

/// Integration steps per second of simulated time (`physics_RT.dll`: 1/66 s).
pub const STEP_HZ: f32 = 66.0;
/// Simulated seconds per step.
pub const STEP_DT: f32 = 1.0 / STEP_HZ;

/// How a pair of materials combines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combine {
    Multiply,
    Average,
    Min,
    Max,
    GeometricMean,
}

impl Combine {
    pub fn apply(self, a: f32, b: f32) -> f32 {
        match self {
            Self::Multiply => a * b,
            Self::Average => f32::midpoint(a, b),
            Self::Min => a.min(b),
            Self::Max => a.max(b),
            Self::GeometricMean => (a * b).max(0.0).sqrt(),
        }
    }
}

/// Velocity damping applied once per step with factor `k`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DampingLaw {
    /// `v ·= max(0, 1 − k·Δt)`
    Linear,
    /// `v ·= e^(−k·Δt)`
    Exponential,
}

impl DampingLaw {
    pub fn factor(self, k: f32, dt: f32) -> f32 {
        match self {
            Self::Linear => (1.0 - k * dt).max(0.0),
            Self::Exponential => (-k * dt).exp(),
        }
    }
}

/// Choices where the original's behaviour isn't known yet. Defaults are
/// reasonable guesses, to be calibrated against traces of the original.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactModel {
    pub friction: Combine,
    pub restitution: Combine,
    pub damping: DampingLaw,
    /// Below this normal approach speed (units/s), contacts don't bounce.
    pub bounce_threshold: f32,
    /// Fraction of penetration corrected per step (position projection).
    pub penetration_correction: f32,
    /// Contact passes per step (detect, push apart, solve velocities).
    pub iterations: u32,
    /// Velocity solver iterations per contact pass.
    pub velocity_iterations: u32,
    /// Bodies and surfaces sharing a non-empty collision group don't collide.
    /// (Inferred: `Phys_FloorStopper` uses group "Ball", like the player ball.)
    pub same_group_ignores: bool,
}

impl Default for ContactModel {
    fn default() -> Self {
        Self {
            friction: Combine::GeometricMean,
            restitution: Combine::Multiply,
            damping: DampingLaw::Linear,
            bounce_threshold: 1.0,
            penetration_correction: 0.8,
            iterations: 4,
            velocity_iterations: 8,
            same_group_ignores: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldConfig {
    pub gravity: Vec3,
    /// Simulated seconds per real second.
    pub time_factor: f32,
    pub contact: ContactModel,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -20.0, 0.0),
            time_factor: 2.0,
            contact: ContactModel::default(),
        }
    }
}
