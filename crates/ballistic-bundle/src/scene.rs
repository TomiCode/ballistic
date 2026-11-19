//! Engine-native scene data. Right-handed, Y up, original world units.

use serde::{Deserialize, Serialize};

/// One interleaved vertex as stored in [`Mesh::vertex_data`] (32 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub objects: Vec<SceneObject>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub name: String,
    /// Raw little-endian [`Vertex`] array, GPU-uploadable as-is.
    #[serde(with = "serde_bytes_vec")]
    pub vertex_data: Vec<u8>,
    /// Raw little-endian `u16` index array (triangle list).
    #[serde(with = "serde_bytes_vec")]
    pub index_data: Vec<u8>,
    /// Index ranges per material, covering `index_data` without gaps.
    pub submeshes: Vec<Submesh>,
    /// Local-space bounds.
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

impl Mesh {
    /// Decodes the vertex buffer (copying, so alignment doesn't matter).
    pub fn vertices(&self) -> Vec<Vertex> {
        bytemuck::pod_collect_to_vec(&self.vertex_data)
    }

    pub fn indices(&self) -> Vec<u16> {
        bytemuck::pod_collect_to_vec(&self.index_data)
    }

    pub fn vertex_count(&self) -> usize {
        self.vertex_data.len() / std::mem::size_of::<Vertex>()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submesh {
    pub first_index: u32,
    pub index_count: u32,
    /// Index into [`Scene::materials`]; `None` = default untextured material.
    pub material: Option<u32>,
}

/// Fixed-function material state, as authored for the original renderer.
#[allow(clippy::struct_excessive_bools)] // mirrors independent render-state switches
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// RGBA in the original's gamma space.
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    pub specular: [f32; 4],
    pub emissive: [f32; 4],
    pub specular_power: f32,
    /// Texture key (`textures/<key>.ktx2`).
    pub texture: Option<String>,
    pub texture_blend: TextureBlend,
    pub address_mode: AddressMode,
    pub linear_filter: bool,
    pub src_blend: BlendFactor,
    pub dest_blend: BlendFactor,
    /// Alpha blending enabled (with `src_blend` / `dest_blend`).
    pub blend: bool,
    /// Discard fragments failing `alpha_func` against `alpha_ref` (0–1).
    pub alpha_test: Option<(CompareFunc, f32)>,
    pub z_write: bool,
    pub z_func: CompareFunc,
    pub two_sided: bool,
    pub flat_shading: bool,
    pub wireframe: bool,
    /// The original applies a texture-coordinate generator (environment map).
    pub environment_map: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TextureBlend {
    Decal,
    Modulate,
    DecalAlpha,
    ModulateAlpha,
    DecalMask,
    ModulateMask,
    Copy,
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressMode {
    Wrap,
    Mirror,
    Clamp,
    Border,
    MirrorOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlendFactor {
    Zero,
    One,
    SrcColor,
    InvSrcColor,
    SrcAlpha,
    InvSrcAlpha,
    DestAlpha,
    InvDestAlpha,
    DestColor,
    InvDestColor,
    SrcAlphaSat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompareFunc {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectKind {
    Entity,
    Light,
    Camera,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneObject {
    pub name: String,
    pub kind: ObjectKind,
    /// World transform, column-major, column vectors (`glam::Mat4::from_cols_array_2d`).
    pub transform: [[f32; 4]; 4],
    /// Index into [`Scene::meshes`].
    pub mesh: Option<u32>,
    /// Index into [`Scene::objects`].
    pub parent: Option<u32>,
    /// Hidden when the scene loads (scripts may show it later).
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// Indices into [`Scene::objects`].
    pub members: Vec<u32>,
}

/// Serialises `Vec<u8>` as a byte string (one length prefix, no per-element overhead).
mod serde_bytes_vec {
    use serde::{Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a byte array")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }
            fn visit_borrowed_bytes<E: de::Error>(self, v: &'de [u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }
            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(v)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut a: A) -> Result<Vec<u8>, A::Error> {
                let mut out = Vec::with_capacity(a.size_hint().unwrap_or(0));
                while let Some(b) = a.next_element()? {
                    out.push(b);
                }
                Ok(out)
            }
        }
        d.deserialize_byte_buf(V)
    }
}
