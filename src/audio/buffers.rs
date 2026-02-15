//! Audio buffer conversion between interleaved and planar (deinterleaved) formats.
//!
//! VST3 plugins expect planar audio (one buffer per channel), while audio files
//! are typically stored interleaved. These functions handle the conversion.

/// Convert interleaved samples to per-channel (planar/deinterleaved) vectors.
///
/// Input: `[L0, R0, L1, R1, L2, R2]` (interleaved stereo)
/// Output: `[[L0, L1, L2], [R0, R1, R2]]` (per-channel)
pub fn deinterleave(interleaved: &[f32], channels: usize) -> Vec<Vec<f32>> {
    if channels == 0 || interleaved.is_empty() {
        return vec![Vec::new(); channels];
    }

    let total_frames = interleaved.len() / channels;
    let mut planar: Vec<Vec<f32>> = (0..channels)
        .map(|_| Vec::with_capacity(total_frames))
        .collect();

    for frame in 0..total_frames {
        for ch in 0..channels {
            planar[ch].push(interleaved[frame * channels + ch]);
        }
    }

    planar
}

/// Convert per-channel (planar/deinterleaved) vectors to interleaved samples.
///
/// Input: `[[L0, L1, L2], [R0, R1, R2]]` (per-channel)
/// Output: `[L0, R0, L1, R1, L2, R2]` (interleaved stereo)
pub fn interleave(planar: &[Vec<f32>]) -> Vec<f32> {
    if planar.is_empty() {
        return Vec::new();
    }

    let total_frames = planar[0].len();
    let mut interleaved = Vec::with_capacity(total_frames * planar.len());

    for frame in 0..total_frames {
        for channel in planar {
            interleaved.push(channel[frame]);
        }
    }

    interleaved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip() {
        let original = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let planar = deinterleave(&original, 2);
        let result = interleave(&planar);
        assert_eq!(original, result);
    }

    #[test]
    fn test_known_signal_stereo() {
        let interleaved = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let planar = deinterleave(&interleaved, 2);
        assert_eq!(planar.len(), 2);
        assert_eq!(planar[0], vec![1.0, 3.0, 5.0]); // Left channel
        assert_eq!(planar[1], vec![2.0, 4.0, 6.0]); // Right channel
    }

    #[test]
    fn test_single_channel() {
        let mono = vec![1.0, 2.0, 3.0, 4.0];
        let planar = deinterleave(&mono, 1);
        assert_eq!(planar.len(), 1);
        assert_eq!(planar[0], vec![1.0, 2.0, 3.0, 4.0]);

        let result = interleave(&planar);
        assert_eq!(mono, result);
    }

    #[test]
    fn test_empty_input() {
        let empty: Vec<f32> = Vec::new();
        let planar = deinterleave(&empty, 2);
        assert_eq!(planar.len(), 2);
        assert!(planar[0].is_empty());
        assert!(planar[1].is_empty());

        let result = interleave(&planar);
        assert!(result.is_empty());
    }

    #[test]
    fn test_zero_channels() {
        let data = vec![1.0, 2.0, 3.0];
        let planar = deinterleave(&data, 0);
        assert!(planar.is_empty());
    }

    #[test]
    fn test_three_channels() {
        let interleaved = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let planar = deinterleave(&interleaved, 3);
        assert_eq!(planar.len(), 3);
        assert_eq!(planar[0], vec![1.0, 4.0, 7.0]);
        assert_eq!(planar[1], vec![2.0, 5.0, 8.0]);
        assert_eq!(planar[2], vec![3.0, 6.0, 9.0]);

        let result = interleave(&planar);
        assert_eq!(interleaved, result);
    }

    #[test]
    fn test_interleave_empty() {
        let empty: Vec<Vec<f32>> = Vec::new();
        let result = interleave(&empty);
        assert!(result.is_empty());
    }
}
