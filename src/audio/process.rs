//! Block-based offline audio rendering through a VST3 plugin.
//!
//! Processes decoded audio through a PluginInstance in fixed-size blocks,
//! with tail handling for effects (reverb, delay fade-out).

use anyhow::{Context, Result};
use tracing::debug;

use crate::audio::buffers;
use crate::audio::decode::DecodedAudio;
use crate::hosting::plugin::PluginInstance;

/// Default block size for processing if not specified by the plugin.
const DEFAULT_BLOCK_SIZE: usize = 4096;

/// Maximum tail duration in seconds for plugins reporting infinite tail.
const MAX_TAIL_SECONDS: f64 = 30.0;

/// Render audio offline through a VST3 plugin.
///
/// 1. Deinterleaves the input audio to planar format.
/// 2. Processes in fixed-size blocks through the plugin.
/// 3. Handles tail (silence processing after input ends for effects).
/// 4. Interleaves output back for WAV encoding.
///
/// The plugin must be in the Processing state before calling this function.
pub fn render_offline(plugin: &mut PluginInstance, decoded: &DecodedAudio) -> Result<Vec<f32>> {
    let channels = decoded.channels;
    let total_frames = decoded.total_frames;
    let sample_rate = decoded.sample_rate;

    if channels == 0 || total_frames == 0 {
        return Ok(Vec::new());
    }

    // 1. Deinterleave input audio to planar format
    let input_planar = buffers::deinterleave(&decoded.samples, channels);

    // 2. Determine block size
    let max_block_size = DEFAULT_BLOCK_SIZE;

    // 3. Query tail length from plugin
    let tail_samples = plugin.get_tail_samples();
    let tail_frames = if tail_samples == u32::MAX {
        // kInfiniteTail -- use configurable max
        (MAX_TAIL_SECONDS * sample_rate as f64) as usize
    } else {
        tail_samples as usize
    };

    debug!(
        "render_offline: {} frames, {} channels, {} tail frames, block size {}",
        total_frames, channels, tail_frames, max_block_size
    );

    // 4. Pre-allocate output buffers (input frames + tail)
    let total_output_frames = total_frames + tail_frames;
    let mut output_planar: Vec<Vec<f32>> = (0..channels)
        .map(|_| vec![0.0f32; total_output_frames])
        .collect();

    // Pre-allocate silence buffer for tail processing
    let silence: Vec<f32> = vec![0.0f32; max_block_size];

    // 5. Process input audio in blocks
    let mut offset = 0;
    while offset < total_frames {
        let block_size = (total_frames - offset).min(max_block_size);

        // Build per-channel input slices
        let input_slices: Vec<&[f32]> = input_planar
            .iter()
            .map(|ch| &ch[offset..offset + block_size])
            .collect();

        // Build per-channel output slices
        let mut output_vecs: Vec<&mut [f32]> = output_planar
            .iter_mut()
            .map(|ch| &mut ch[offset..offset + block_size])
            .collect();

        plugin
            .process(&input_slices, &mut output_vecs, block_size as i32)
            .with_context(|| format!("plugin process failed at frame offset {}", offset))?;

        offset += block_size;
    }

    // 6. Process tail (feed silence to capture reverb/delay fade-out)
    if tail_frames > 0 {
        let mut tail_offset = 0;
        while tail_offset < tail_frames {
            let block_size = (tail_frames - tail_offset).min(max_block_size);
            let output_offset = total_frames + tail_offset;

            // Build per-channel silence input slices
            let input_slices: Vec<&[f32]> = (0..channels)
                .map(|_| &silence[..block_size])
                .collect();

            // Build per-channel output slices into the tail region
            let mut output_vecs: Vec<&mut [f32]> = output_planar
                .iter_mut()
                .map(|ch| &mut ch[output_offset..output_offset + block_size])
                .collect();

            plugin
                .process(&input_slices, &mut output_vecs, block_size as i32)
                .with_context(|| {
                    format!("plugin process failed during tail at offset {}", tail_offset)
                })?;

            tail_offset += block_size;
        }
    }

    // 7. Interleave output channels
    let interleaved = buffers::interleave(&output_planar);

    debug!(
        "render_offline complete: {} output frames ({} input + {} tail)",
        total_output_frames, total_frames, tail_frames
    );

    Ok(interleaved)
}
