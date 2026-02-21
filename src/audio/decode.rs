//! Multi-format audio file decoding to f32 samples.
//!
//! Uses symphonia to decode WAV, FLAC, MP3, OGG, and other formats
//! into interleaved f32 sample buffers.

use std::path::Path;

use anyhow::{Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded audio data as interleaved f32 samples.
pub struct DecodedAudio {
    /// Interleaved f32 samples (L R L R L R for stereo).
    pub samples: Vec<f32>,
    /// Number of audio channels.
    pub channels: usize,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Total number of frames (samples.len() / channels).
    pub total_frames: usize,
}

/// Decode an audio file to interleaved f32 samples.
///
/// Supports WAV, FLAC, MP3, OGG, and other formats via symphonia.
/// Returns interleaved f32 samples with the file's native sample rate and channel count.
pub fn decode_audio_file(path: &Path) -> Result<DecodedAudio> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to open audio file: {}", path.display()))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("failed to probe audio format: {}", path.display()))?;

    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("no audio track found in {}", path.display()))?;

    let track_id = track.id;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .with_context(|| "failed to create audio decoder")?;

    let mut all_samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(e).with_context(|| "error reading audio packet"),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder
            .decode(&packet)
            .with_context(|| "error decoding audio packet")?;

        let spec = *decoded.spec();
        let duration = decoded.capacity() as u64;

        let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
        sample_buf.copy_interleaved_ref(decoded);
        all_samples.extend_from_slice(sample_buf.samples());
    }

    let total_frames = if channels > 0 {
        all_samples.len() / channels
    } else {
        0
    };

    Ok(DecodedAudio {
        samples: all_samples,
        channels,
        sample_rate,
        total_frames,
    })
}
