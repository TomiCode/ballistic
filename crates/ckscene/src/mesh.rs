use ckfile::{Result, StateChunk};

use crate::{ObjRef, missing, obj_ref};

const SECTION_FLAGS: u32 = 0x2000;
const SECTION_FACES: u32 = 0x1_0000;
const SECTION_VERTICES: u32 = 0x2_0000;
const SECTION_MATERIAL_GROUPS: u32 = 0x10_0000;
const SECTION_CHANNELS: u32 = 0x4000;

/// Vertex-array flags: arrays stored as a single shared value, or omitted.
const SINGLE_DIFFUSE: u32 = 0x1;
const SINGLE_SPECULAR: u32 = 0x2;
const NO_NORMALS: u32 = 0x4;
const SINGLE_UV: u32 = 0x8;

/// A triangle. `group` indexes [`Mesh::material_groups`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Face {
    pub indices: [u16; 3],
    pub group: u16,
}

/// Triangle mesh with per-vertex attributes, all arrays of equal length.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Mesh {
    /// Raw mesh flags.
    pub flags: u32,
    pub positions: Vec<[f32; 3]>,
    /// `None` when the file omits normals (they must be generated).
    pub normals: Option<Vec<[f32; 3]>>,
    pub uvs: Vec<[f32; 2]>,
    /// `0xAARRGGBB`.
    pub diffuse: Vec<u32>,
    pub specular: Vec<u32>,
    pub faces: Vec<Face>,
    /// Material per group (`None` = untextured default material).
    pub material_groups: Vec<Option<ObjRef>>,
    /// `true` if the mesh has extra texture channels (multi-texturing).
    /// They're not decoded yet.
    pub has_extra_channels: bool,
}

impl Mesh {
    pub fn decode(chunk: &StateChunk) -> Result<Self> {
        let flags = match chunk.section(SECTION_FLAGS)? {
            Some(mut r) => r.u32()?,
            None => 0,
        };

        let mut r = chunk
            .section(SECTION_MATERIAL_GROUPS)?
            .ok_or_else(|| missing("Mesh", SECTION_MATERIAL_GROUPS))?;
        let n_groups = r.u32()? as usize;
        let mut material_groups = Vec::with_capacity(n_groups);
        for _ in 0..n_groups {
            material_groups.push(obj_ref(r.u32()?));
            let _reserved = r.u32()?;
        }

        let mut r = chunk
            .section(SECTION_FACES)?
            .ok_or_else(|| missing("Mesh", SECTION_FACES))?;
        let n_faces = r.u32()? as usize;
        let mut faces = Vec::with_capacity(n_faces);
        for _ in 0..n_faces {
            let a = r.u32()?;
            let b = r.u32()?;
            faces.push(Face {
                indices: [a as u16, (a >> 16) as u16, b as u16],
                group: (b >> 16) as u16,
            });
        }

        let mut r = chunk
            .section(SECTION_VERTICES)?
            .ok_or_else(|| missing("Mesh", SECTION_VERTICES))?;
        let n = r.u32()? as usize;
        let vflags = r.u32()?;
        let _data_size = r.u32()?;

        let mut positions = Vec::with_capacity(n);
        for _ in 0..n {
            positions.push([r.f32()?, r.f32()?, r.f32()?]);
        }
        let mut colors = |single: bool| -> Result<Vec<u32>> {
            if single {
                Ok(vec![r.u32()?; n])
            } else {
                Ok(r.dwords(n)?.to_vec())
            }
        };
        let diffuse = colors(vflags & SINGLE_DIFFUSE != 0)?;
        let specular = colors(vflags & SINGLE_SPECULAR != 0)?;
        let normals = if vflags & NO_NORMALS != 0 {
            None
        } else {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push([r.f32()?, r.f32()?, r.f32()?]);
            }
            Some(v)
        };
        let uvs = if vflags & SINGLE_UV != 0 {
            vec![[r.f32()?, r.f32()?]; n]
        } else {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push([r.f32()?, r.f32()?]);
            }
            v
        };
        if !r.is_empty() {
            return Err(ckfile::Error::BadChunk(format!(
                "Mesh: {} unread words in vertex section",
                r.remaining()
            )));
        }

        let mesh = Self {
            flags,
            positions,
            normals,
            uvs,
            diffuse,
            specular,
            faces,
            material_groups,
            has_extra_channels: chunk.section(SECTION_CHANNELS)?.is_some(),
        };
        mesh.validate()?;
        Ok(mesh)
    }

    fn validate(&self) -> Result<()> {
        let n = self.positions.len();
        for (i, f) in self.faces.iter().enumerate() {
            if f.indices.iter().any(|&v| usize::from(v) >= n) {
                return Err(ckfile::Error::BadChunk(format!(
                    "Mesh: face {i} references vertex beyond {n}"
                )));
            }
            if usize::from(f.group) >= self.material_groups.len().max(1) {
                return Err(ckfile::Error::BadChunk(format!(
                    "Mesh: face {i} uses material group {} of {}",
                    f.group,
                    self.material_groups.len()
                )));
            }
        }
        Ok(())
    }

    /// Face normals, area-weighted and averaged per vertex. Used when the file has none.
    pub fn generate_normals(&self) -> Vec<[f32; 3]> {
        let mut acc = vec![[0.0f32; 3]; self.positions.len()];
        for f in &self.faces {
            let [a, b, c] = f.indices.map(|i| self.positions[usize::from(i)]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            for i in f.indices {
                let t = &mut acc[usize::from(i)];
                for k in 0..3 {
                    t[k] += n[k];
                }
            }
        }
        for n in &mut acc {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if l > 0.0 {
                for x in n.iter_mut() {
                    *x /= l;
                }
            }
        }
        acc
    }
}
