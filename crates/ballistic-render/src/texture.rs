//! KTX2 → GPU texture upload.
//!
//! Textures are created with **non-sRGB** formats on purpose: the original
//! renderer did all shading in gamma space, so sampling must return the stored
//! values unchanged.

use crate::{Error, Gpu, Result};

pub struct GpuTexture {
    pub view: wgpu::TextureView,
}

enum Source {
    Bc1,
    Bc3,
    Rgba8,
}

fn err(key: &str, reason: impl Into<String>) -> Error {
    Error::Texture {
        key: key.to_owned(),
        reason: reason.into(),
    }
}

pub fn upload_ktx2(gpu: &Gpu, key: &str, bytes: &[u8]) -> Result<GpuTexture> {
    let reader = ktx2::Reader::new(bytes).map_err(|e| err(key, e.to_string()))?;
    let header = reader.header();
    let (width, height) = (header.pixel_width, header.pixel_height);
    let source = match header.format {
        Some(ktx2::Format::BC1_RGB_SRGB_BLOCK | ktx2::Format::BC1_RGB_UNORM_BLOCK) => Source::Bc1,
        Some(ktx2::Format::BC3_SRGB_BLOCK | ktx2::Format::BC3_UNORM_BLOCK) => Source::Bc3,
        Some(ktx2::Format::R8G8B8A8_SRGB | ktx2::Format::R8G8B8A8_UNORM) => Source::Rgba8,
        other => return Err(err(key, format!("unsupported format {other:?}"))),
    };
    let levels: Vec<&[u8]> = reader.levels().map(|l| l.data).collect();

    // BC needs device support and block-aligned top-level dimensions.
    let native_bc = gpu.bc_textures
        && width.is_multiple_of(4)
        && height.is_multiple_of(4)
        && !matches!(source, Source::Rgba8);
    let format = match (&source, native_bc) {
        (Source::Bc1, true) => wgpu::TextureFormat::Bc1RgbaUnorm,
        (Source::Bc3, true) => wgpu::TextureFormat::Bc3RgbaUnorm,
        _ => wgpu::TextureFormat::Rgba8Unorm,
    };

    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(key),
        size,
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (level, data) in levels.iter().enumerate() {
        let mip = size.mip_level_size(level as u32, wgpu::TextureDimension::D2);
        let decoded;
        let (bytes, bytes_per_row, rows, extent) = if format == wgpu::TextureFormat::Rgba8Unorm {
            let pixels: &[u8] = match source {
                Source::Rgba8 => data,
                Source::Bc1 | Source::Bc3 => {
                    let f = if matches!(source, Source::Bc1) {
                        texpresso::Format::Bc1
                    } else {
                        texpresso::Format::Bc3
                    };
                    let mut out = vec![0u8; (mip.width * mip.height * 4) as usize];
                    f.decompress(data, mip.width as usize, mip.height as usize, &mut out);
                    decoded = out;
                    &decoded
                }
            };
            (pixels, mip.width * 4, mip.height, mip)
        } else {
            let phys = mip.physical_size(format);
            let block_bytes = format.block_copy_size(None).unwrap_or(16);
            (*data, (phys.width / 4) * block_bytes, phys.height / 4, phys)
        };
        let expected = (bytes_per_row * rows) as usize;
        if bytes.len() < expected {
            return Err(err(
                key,
                format!("level {level}: {} bytes, expected {expected}", bytes.len()),
            ));
        }
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes[..expected],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows),
            },
            extent,
        );
    }

    Ok(GpuTexture {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
    })
}

/// 1×1 white texture for untextured materials.
pub fn white(gpu: &Gpu) -> GpuTexture {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("white"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255; 4],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        texture.size(),
    );
    GpuTexture {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
    }
}
