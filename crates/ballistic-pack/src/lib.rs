//! Library side of `ballistic-pack`: converts a user's Ballance installation
//! into a Ballistic bundle. The binary in `main.rs` is a thin CLI over [`pack`].

pub mod audio;
pub mod install;
mod ktx2;
pub mod pack;
pub mod physics;
pub mod scenes;
pub mod textures;
