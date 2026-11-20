//! Ballistic renderer: draws bundle scenes with wgpu.
//!
//! Shading imitates the original's fixed-function pipeline, including doing all
//! colour math in **gamma space**: textures are sampled through non-sRGB views
//! and the render target is non-sRGB, exactly like a 2004 Direct3D frame buffer.

mod camera;
mod gpu;
mod overlay;
mod particles;
mod scene;
mod sky;
mod texture;

pub use camera::Camera;
pub use gpu::{Gpu, RenderTarget, depth_view as gpu_depth_view};
pub use overlay::{Overlay, Quad};
pub use particles::{ParticleBatch, ParticleRenderer, ParticleVertex};
pub use scene::{CullMode, RenderOptions, SceneRenderer, Shadow};
pub use sky::{SkyRenderer, sky_mesh};
pub use texture::{GpuTexture, upload_ktx2, white};

/// Colour format used for off-screen targets (gamma space, like the original).
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no suitable GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("could not create GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("texture {key}: {reason}")]
    Texture { key: String, reason: String },
    #[error("GPU readback failed: {0}")]
    Readback(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
