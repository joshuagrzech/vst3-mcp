//! WAV file encoding from f32 samples.
//!
//! Uses hound to write interleaved f32 samples to WAV files
//! with full 32-bit float precision.

use std::path::Path;

use anyhow::{Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};

/// Write interleaved f32 samples to a WAV file.
///
/// Preserves full 32-bit float precision. No dithering or bit depth conversion.
pub fn write_wav(path: &Path, samples: &[f32], channels: u16, sample_rate: u32) -> Result<()> {
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };

    let mut writer = WavWriter::create(path, spec)
        .with_context(|| format!("failed to create WAV file: {}", path.display()))?;

    for &sample in samples {
        writer
            .write_sample(sample)
            .with_context(|| "error writing WAV sample")?;
    }

    writer
        .finalize()
        .with_context(|| format!("error finalizing WAV file: {}", path.display()))?;

    Ok(())
}
