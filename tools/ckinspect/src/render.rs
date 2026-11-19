//! Minimal software rasteriser used to eyeball decoded levels. Dev-only: the
//! images it writes are derived from game data and must stay out of the repo.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::similar_names,
    clippy::too_many_lines
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ckfile::CkFile;
use ckscene::{Decoded, Entity3d, Material, Mesh, Texture, decode_object};
use image::{Rgba, RgbaImage};

pub struct View {
    /// Rotation around the vertical axis, degrees.
    pub azimuth: f32,
    /// Angle above the horizon, degrees (90 = straight down).
    pub elevation: f32,
    pub size: u32,
    /// Also draw objects saved as hidden (prefabs are shown by scripts at runtime).
    pub include_hidden: bool,
}

struct Tri {
    p: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    material: Option<usize>,
}

pub fn render(file: &CkFile, textures_dir: Option<&Path>, view: &View, out: &Path) -> Result<()> {
    let decoded: Vec<Option<Decoded>> = file
        .objects
        .iter()
        .map(|o| decode_object(o).transpose())
        .collect::<Result<_, _>>()?;

    let mut tris = Vec::new();
    for d in &decoded {
        let Some(Decoded::Entity(e)) = d else {
            continue;
        };
        if e.hidden && !view.include_hidden {
            continue;
        }
        let Some(Decoded::Mesh(mesh)) = e
            .current_mesh
            .and_then(|m| decoded.get(m))
            .and_then(Option::as_ref)
        else {
            continue;
        };
        push_mesh(&mut tris, e, mesh);
    }
    anyhow::ensure!(!tris.is_empty(), "nothing visible to render");

    // Camera basis: forward looks down at `elevation`, rotated by `azimuth`.
    let (az, el) = (view.azimuth.to_radians(), view.elevation.to_radians());
    let fwd = [-el.cos() * az.sin(), -el.sin(), -el.cos() * az.cos()];
    let right = normalize(cross([0.0, 1.0, 0.0], fwd));
    let right = if right.iter().all(|v| v.abs() < 1e-6) {
        [1.0, 0.0, 0.0]
    } else {
        right
    };
    let up = cross(fwd, right);
    let project = |p: [f32; 3]| [dot(p, right), dot(p, up), dot(p, fwd)];

    let projected: Vec<[[f32; 3]; 3]> = tris.iter().map(|t| t.p.map(project)).collect();
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for t in &projected {
        for p in t {
            for k in 0..2 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
    }
    let size = view.size as f32;
    let scale = (size - 20.0) / (max[0] - min[0]).max(max[1] - min[1]);
    let to_px = |p: [f32; 3]| {
        [
            10.0 + (p[0] - min[0]) * scale,
            size - 10.0 - (p[1] - min[1]) * scale,
            p[2],
        ]
    };

    let materials: HashMap<usize, Material> = decoded
        .iter()
        .enumerate()
        .filter_map(|(i, d)| match d {
            Some(Decoded::Material(m)) => Some((i, m.clone())),
            _ => None,
        })
        .collect();
    let mut cache = TextureCache::new(textures_dir);

    let (w, h) = (view.size, view.size);
    let mut img = RgbaImage::from_pixel(w, h, Rgba([24, 26, 32, 255]));
    let mut depth = vec![f32::MAX; (w * h) as usize];
    let light = normalize([0.4, 1.0, 0.25]);

    for (t, p) in tris.iter().zip(&projected) {
        let n = normalize(cross(sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0])));
        let shade = 0.35 + 0.65 * dot(n, light).abs();
        let mat = t.material.and_then(|m| materials.get(&m));
        let tex = mat
            .and_then(|m| m.texture)
            .and_then(|ti| match decoded.get(ti) {
                Some(Some(Decoded::Texture(tx))) => cache.get(tx),
                _ => None,
            });
        let base = mat.map_or([0.8, 0.8, 0.8, 1.0], |m| ckscene::argb_to_rgba(m.diffuse));
        let cut_alpha = mat.is_some_and(|m| m.alpha_test() || m.alpha_blend());
        raster(&mut img, &mut depth, p.map(to_px), |b| {
            let mut c = base;
            if let Some(tex) = tex {
                let u = b[0] * t.uv[0][0] + b[1] * t.uv[1][0] + b[2] * t.uv[2][0];
                let v = b[0] * t.uv[0][1] + b[1] * t.uv[1][1] + b[2] * t.uv[2][1];
                let s = sample(tex, u, v);
                if cut_alpha && s[3] < 0.5 {
                    return None;
                }
                c = [s[0], s[1], s[2], 1.0];
            }
            Some(Rgba([
                to_u8(c[0] * shade),
                to_u8(c[1] * shade),
                to_u8(c[2] * shade),
                255,
            ]))
        });
    }
    img.save(out)
        .with_context(|| format!("writing {}", out.display()))?;
    eprintln!(
        "rendered {} triangles, {} textures loaded, {} missing",
        tris.len(),
        cache.loaded(),
        cache.missing
    );
    Ok(())
}

fn push_mesh(tris: &mut Vec<Tri>, e: &Entity3d, mesh: &Mesh) {
    let world: Vec<[f32; 3]> = mesh
        .positions
        .iter()
        .map(|&p| e.transform_point(p))
        .collect();
    for f in &mesh.faces {
        let idx = f.indices.map(usize::from);
        tris.push(Tri {
            p: idx.map(|i| world[i]),
            uv: idx.map(|i| mesh.uvs[i]),
            material: mesh
                .material_groups
                .get(usize::from(f.group))
                .copied()
                .flatten(),
        });
    }
}

fn raster(
    img: &mut RgbaImage,
    depth: &mut [f32],
    p: [[f32; 3]; 3],
    mut shade: impl FnMut([f32; 3]) -> Option<Rgba<u8>>,
) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let area = edge(p[0], p[1], p[2]);
    if area.abs() < 1e-9 {
        return;
    }
    let minx = p
        .iter()
        .map(|v| v[0])
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as i32;
    let maxx = p
        .iter()
        .map(|v| v[0])
        .fold(f32::MIN, f32::max)
        .ceil()
        .min((w - 1) as f32) as i32;
    let miny = p
        .iter()
        .map(|v| v[1])
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as i32;
    let maxy = p
        .iter()
        .map(|v| v[1])
        .fold(f32::MIN, f32::max)
        .ceil()
        .min((h - 1) as f32) as i32;
    for y in miny..=maxy {
        for x in minx..=maxx {
            let q = [x as f32 + 0.5, y as f32 + 0.5, 0.0];
            let b = [
                edge(p[1], p[2], q) / area,
                edge(p[2], p[0], q) / area,
                edge(p[0], p[1], q) / area,
            ];
            if b.iter().any(|&v| v < 0.0) {
                continue;
            }
            let z = b[0] * p[0][2] + b[1] * p[1][2] + b[2] * p[2][2];
            let i = (y * w + x) as usize;
            if z < depth[i]
                && let Some(c) = shade(b)
            {
                depth[i] = z;
                img.put_pixel(x as u32, y as u32, c);
            }
        }
    }
}

struct TextureCache {
    dir: Option<PathBuf>,
    index: HashMap<String, PathBuf>,
    images: HashMap<String, Option<RgbaImage>>,
    missing: usize,
}

impl TextureCache {
    fn new(dir: Option<&Path>) -> Self {
        let mut index = HashMap::new();
        if let Some(d) = dir
            && let Ok(rd) = std::fs::read_dir(d)
        {
            for e in rd.flatten() {
                index.insert(e.file_name().to_string_lossy().to_lowercase(), e.path());
            }
        }
        Self {
            dir: dir.map(Path::to_path_buf),
            index,
            images: HashMap::new(),
            missing: 0,
        }
    }

    fn get(&mut self, t: &Texture) -> Option<&RgbaImage> {
        self.dir.as_ref()?;
        let name = t.file()?.to_lowercase();
        if !self.images.contains_key(&name) {
            let img = self
                .index
                .get(&name)
                .and_then(|p| image::open(p).ok())
                .map(|i| i.to_rgba8());
            if img.is_none() {
                self.missing += 1;
            }
            self.images.insert(name.clone(), img);
        }
        self.images.get(&name).and_then(Option::as_ref)
    }

    fn loaded(&self) -> usize {
        self.images.values().filter(|v| v.is_some()).count()
    }
}

fn sample(img: &RgbaImage, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (img.width(), img.height());
    let x = ((u.rem_euclid(1.0) * w as f32) as u32).min(w - 1);
    let y = ((v.rem_euclid(1.0) * h as f32) as u32).min(h - 1);
    let p = img.get_pixel(x, y).0;
    p.map(|c| f32::from(c) / 255.0)
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0) as u8
}

fn edge(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = dot(v, v).sqrt();
    if l > 0.0 { v.map(|x| x / l) } else { v }
}
