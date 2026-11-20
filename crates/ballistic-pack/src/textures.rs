//! Image → mip chain → GPU block compression → KTX2.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use ballistic_bundle::{TextureEncoding, TextureEntry, TextureFormat};
use image::RgbaImage;

pub struct EncodedTexture {
    pub entry: TextureEntry,
    pub ktx2: Vec<u8>,
}

/// `color_key`: black pixels become transparent (a texture with colour
/// keying on, research findings 07).
pub fn encode_file(
    path: &Path,
    key: &str,
    encoding: TextureEncoding,
    color_key: bool,
) -> Result<EncodedTexture> {
    let mut img = image::open(path)
        .with_context(|| format!("decoding {}", path.display()))?
        .to_rgba8();
    if color_key {
        for p in img.pixels_mut() {
            if p.0[..3] == [0, 0, 0] {
                p.0[3] = 0;
            }
        }
    }
    Ok(encode(&img, key, encoding))
}

pub fn encode(img: &RgbaImage, key: &str, encoding: TextureEncoding) -> EncodedTexture {
    let has_alpha = img.pixels().any(|p| p.0[3] != 255);
    let format = match (encoding, has_alpha) {
        (TextureEncoding::Rgba8, _) => TextureFormat::Rgba8Srgb,
        (TextureEncoding::Bc, false) => TextureFormat::Bc1Srgb,
        (TextureEncoding::Bc, true) => TextureFormat::Bc3Srgb,
    };
    let chain = mip_chain(img);
    let levels: Vec<Vec<u8>> = chain.iter().map(|m| compress(m, format)).collect();
    EncodedTexture {
        entry: TextureEntry {
            key: key.to_owned(),
            width: img.width(),
            height: img.height(),
            format,
            mip_levels: levels.len() as u32,
            has_alpha,
        },
        ktx2: crate::ktx2::write(format, img.width(), img.height(), &levels),
    }
}

fn compress(img: &RgbaImage, format: TextureFormat) -> Vec<u8> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let bc = match format {
        TextureFormat::Rgba8Srgb => return img.as_raw().clone(),
        TextureFormat::Bc1Srgb => texpresso::Format::Bc1,
        TextureFormat::Bc3Srgb => texpresso::Format::Bc3,
    };
    let mut out = vec![0u8; bc.compressed_size(w, h)];
    bc.compress(img.as_raw(), w, h, texpresso::Params::default(), &mut out);
    out
}

/// Full chain down to 1×1, box-filtered in linear light (alpha linearly).
pub fn mip_chain(base: &RgbaImage) -> Vec<RgbaImage> {
    let mut chain = vec![base.clone()];
    while let Some(prev) = chain.last() {
        let (w, h) = (prev.width(), prev.height());
        if w == 1 && h == 1 {
            break;
        }
        chain.push(downsample(prev));
    }
    chain
}

fn downsample(src: &RgbaImage) -> RgbaImage {
    let (sw, sh) = (src.width(), src.height());
    let (dw, dh) = ((sw / 2).max(1), (sh / 2).max(1));
    let lut = srgb_to_linear_lut();
    RgbaImage::from_fn(dw, dh, |x, y| {
        let mut acc = [0f32; 4];
        let mut n = 0f32;
        for sy in (y * 2)..((y * 2 + 2).min(sh)) {
            for sx in (x * 2)..((x * 2 + 2).min(sw)) {
                let p = src.get_pixel(sx, sy).0;
                for c in 0..3 {
                    acc[c] += lut[usize::from(p[c])];
                }
                acc[3] += f32::from(p[3]) / 255.0;
                n += 1.0;
            }
        }
        let px: [u8; 4] = std::array::from_fn(|c| {
            let v = acc[c] / n;
            let v = if c == 3 { v } else { linear_to_srgb(v) };
            #[allow(clippy::cast_sign_loss)] // clamped to 0..=255
            let b = (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            b
        });
        image::Rgba(px)
    })
}

fn srgb_to_linear_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        std::array::from_fn(|i| {
            let c = i as f32 / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
    })
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_sizes_and_linear_averaging() {
        // 4x2 image: left half black, right half white.
        let img = RgbaImage::from_fn(4, 2, |x, _| {
            image::Rgba(if x < 2 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            })
        });
        let chain = mip_chain(&img);
        let sizes: Vec<_> = chain.iter().map(|m| (m.width(), m.height())).collect();
        assert_eq!(sizes, vec![(4, 2), (2, 1), (1, 1)]);
        // 50% black/white in linear light is ~188 in sRGB, not 128.
        let mid = chain[2].get_pixel(0, 0).0;
        assert!((185..=190).contains(&mid[0]), "{mid:?}");
    }

    #[test]
    fn picks_bc3_only_with_alpha() {
        let opaque = RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
        let mut alpha = opaque.clone();
        alpha.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let a = encode(&opaque, "a", TextureEncoding::Bc);
        let b = encode(&alpha, "b", TextureEncoding::Bc);
        assert_eq!(a.entry.format, TextureFormat::Bc1Srgb);
        assert_eq!(b.entry.format, TextureFormat::Bc3Srgb);
        assert_eq!(a.entry.mip_levels, 4);
        assert!(ktx2::Reader::new(&a.ktx2[..]).is_ok());
    }
}
