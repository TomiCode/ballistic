//! WAV → Ogg Vorbis (or normalised 16-bit WAV).

use std::io::Cursor;
use std::num::{NonZeroU8, NonZeroU32};
use std::path::Path;

use anyhow::{Context, Result, bail};
use ballistic_bundle::{AudioEncoding, SoundEntry};
use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

pub struct EncodedSound {
    pub entry: SoundEntry,
    pub bytes: Vec<u8>,
}

/// Decoded PCM: planar `f32` channels in `-1.0..=1.0`.
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

pub fn read_wav(path: &Path) -> Result<Pcm> {
    let mut r =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = r.spec();
    let n_ch = usize::from(spec.channels);
    if n_ch == 0 {
        bail!("{}: no channels", path.display());
    }
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            // 8-bit WAV is unsigned; hound already re-centres it to signed.
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mut channels = vec![Vec::with_capacity(interleaved.len() / n_ch); n_ch];
    for frame in interleaved.chunks_exact(n_ch) {
        for (c, &s) in frame.iter().enumerate() {
            channels[c].push(s);
        }
    }
    Ok(Pcm {
        sample_rate: spec.sample_rate,
        channels,
    })
}

pub fn encode_file(
    path: &Path,
    key: &str,
    encoding: AudioEncoding,
    quality: f32,
) -> Result<EncodedSound> {
    let pcm = read_wav(path)?;
    encode(&pcm, key, encoding, quality).with_context(|| format!("encoding {}", path.display()))
}

pub fn encode(pcm: &Pcm, key: &str, encoding: AudioEncoding, quality: f32) -> Result<EncodedSound> {
    let frames = pcm.channels[0].len();
    let bytes = match encoding {
        AudioEncoding::Vorbis => vorbis(pcm, key, quality)?,
        AudioEncoding::Wav => wav16(pcm)?,
    };
    Ok(EncodedSound {
        entry: SoundEntry {
            key: key.to_owned(),
            sample_rate: pcm.sample_rate,
            channels: pcm.channels.len() as u16,
            frames: frames as u64,
            format: encoding,
        },
        bytes,
    })
}

fn vorbis(pcm: &Pcm, key: &str, quality: f32) -> Result<Vec<u8>> {
    let rate = NonZeroU32::new(pcm.sample_rate).context("sample rate 0")?;
    let ch = NonZeroU8::new(u8::try_from(pcm.channels.len())?).context("no channels")?;
    // A serial derived from the key keeps the output reproducible.
    let mut builder = VorbisEncoderBuilder::new_with_serial(rate, ch, Vec::new(), serial(key));
    builder.bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
        target_quality: quality,
    });
    let mut enc = builder.build()?;
    let frames = pcm.channels[0].len();
    let block = 1024;
    let mut start = 0;
    while start < frames {
        let end = (start + block).min(frames);
        let chunk: Vec<&[f32]> = pcm.channels.iter().map(|c| &c[start..end]).collect();
        enc.encode_audio_block(&chunk)?;
        start = end;
    }
    Ok(enc.finish()?)
}

fn wav16(pcm: &Pcm) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: pcm.channels.len() as u16,
        sample_rate: pcm.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec)?;
        for i in 0..pcm.channels[0].len() {
            for c in &pcm.channels {
                w.write_sample((c[i].clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
            }
        }
        w.finalize()?;
    }
    Ok(cur.into_inner())
}

/// FNV-1a of the key, as a non-negative Ogg stream serial.
fn serial(key: &str) -> i32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in key.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    (h & 0x7FFF_FFFF).cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> Pcm {
        let n = 22_050;
        let s: Vec<f32> = (0..n)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 22_050.0).sin() * 0.5)
            .collect();
        Pcm {
            sample_rate: 22_050,
            channels: vec![s.clone(), s],
        }
    }

    #[test]
    fn vorbis_is_ogg_and_reproducible() {
        let a = encode(&tone(), "music_atmo_1", AudioEncoding::Vorbis, 0.6).unwrap();
        let b = encode(&tone(), "music_atmo_1", AudioEncoding::Vorbis, 0.6).unwrap();
        assert_eq!(&a.bytes[..4], b"OggS");
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.entry.frames, 22_050);
        assert_eq!(a.entry.channels, 2);
    }

    #[test]
    fn wav_round_trip_length() {
        let e = encode(&tone(), "x", AudioEncoding::Wav, 0.0).unwrap();
        let r = hound::WavReader::new(Cursor::new(e.bytes)).unwrap();
        assert_eq!(r.duration(), 22_050);
        assert_eq!(r.spec().channels, 2);
    }
}
