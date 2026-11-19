use ckfile::{Result, StateChunk};

use crate::{ObjRef, is_hidden, missing, obj_ref};

const SECTION_MESHES: u32 = 0x4000;
const SECTION_TRANSFORM: u32 = 0x10_0000;
/// Bit in the first transform word: a parent reference follows the matrix.
const HAS_PARENT: u32 = 0x2_0000;

/// A 3D entity or 3D object (`3dEntity` / `3dObject`, and the entity part of
/// lights and cameras).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Entity3d {
    /// Mesh currently shown, if any.
    pub current_mesh: Option<ObjRef>,
    /// All meshes attached (alternatives the scripts can switch between).
    pub meshes: Vec<ObjRef>,
    /// World matrix, row-major, row-vector convention: rows 0–2 are the
    /// (scaled) X/Y/Z axes, row 3 is the translation.
    pub world_matrix: [[f32; 4]; 4],
    /// Raw entity flags (`0x42000b` on most level geometry).
    pub flags: u32,
    pub parent: Option<ObjRef>,
    pub hidden: bool,
}

impl Entity3d {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let (current_mesh, meshes) = match chunk.section(SECTION_MESHES)? {
            Some(mut r) => {
                let current = obj_ref(r.u32()?);
                let n = r.u32()? as usize;
                let list = (0..n)
                    .map(|_| r.u32().map(obj_ref))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .flatten()
                    .collect();
                (current, list)
            }
            None => (None, Vec::new()),
        };

        let mut r = chunk
            .section(SECTION_TRANSFORM)?
            .ok_or_else(|| missing("3dEntity", SECTION_TRANSFORM))?;
        let head = r.u32()?;
        let flags = r.u32()?;
        let mut m = [[0.0f32; 4]; 4];
        for (i, row) in m.iter_mut().enumerate() {
            for v in row.iter_mut().take(3) {
                *v = r.f32()?;
            }
            row[3] = if i == 3 { 1.0 } else { 0.0 };
        }
        let parent = if head & HAS_PARENT != 0 {
            obj_ref(r.u32()?)
        } else {
            None
        };

        Ok(Self {
            current_mesh,
            meshes,
            world_matrix: m,
            flags,
            parent,
            hidden: is_hidden(chunk)?,
        })
    }

    /// Transforms a point from local to world space.
    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.world_matrix;
        std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c])
    }

    /// Transforms a direction (no translation). Not normalised.
    pub fn transform_vector(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.world_matrix;
        std::array::from_fn(|c| v[0] * m[0][c] + v[1] * m[1][c] + v[2] * m[2][c])
    }

    pub fn position(&self) -> [f32; 3] {
        let t = self.world_matrix[3];
        [t[0], t[1], t[2]]
    }
}
