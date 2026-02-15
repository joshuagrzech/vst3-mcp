//! Integration tests for VST3 parameter control (Phase 3).
//!
//! Tests validate:
//! - Parameter enumeration (PARAM-01)
//! - Parameter reading with display strings (PARAM-02, PARAM-04)
//! - Parameter writing with audible changes (PARAM-03, PARAM-06)
//! - Read-only parameter filtering (PARAM-05)
//! - Smooth parameter sweeps without zipper noise (PARAM-06)

use std::f32::consts::PI;
use std::sync::Arc;

use vst3_mcp_host::hosting::host_app::{ComponentHandler, HostApp};
use vst3_mcp_host::hosting::module::VstModule;
use vst3_mcp_host::hosting::plugin::PluginInstance;
use vst3_mcp_host::hosting::scanner;
use vst3_mcp_host::hosting::types::{BusDirection, BusType};

// ---------------------------------------------------------------------------
// Plugin loading helper
// ---------------------------------------------------------------------------

/// Try to find and load a suitable VST3 effect plugin for testing.
///
/// Looks for a plugin with at least 1 audio input bus and 1 audio output bus.
/// Prefers plugins known to have parameters. Respects PLUGIN_PATH env var for CI.
/// Returns None if no suitable plugin is found.
fn load_test_plugin() -> Option<(PluginInstance, String)> {
    // Check for PLUGIN_PATH env var override
    let custom_path = std::env::var("PLUGIN_PATH").ok();

    let plugins = scanner::scan_plugins(custom_path.as_deref())
        .unwrap_or_else(|e| {
            eprintln!("WARNING: Plugin scan failed: {}", e);
            Vec::new()
        });

    if plugins.is_empty() {
        eprintln!("WARNING: No VST3 plugins found. Skipping integration test.");
        return None;
    }

    // Preferred plugin names in priority order (known to have parameters)
    let preferred_names = ["again", "vital", "adelay"];

    // Sort: preferred plugins first, then rest
    let mut sorted_plugins: Vec<&_> = plugins.iter().collect();
    sorted_plugins.sort_by_key(|p| {
        let name_lower = p.name.to_lowercase();
        preferred_names
            .iter()
            .position(|pref| name_lower.contains(pref))
            .unwrap_or(usize::MAX)
    });

    // Try each plugin until we find one that loads and has audio I/O
    for info in &sorted_plugins {
        eprintln!("INFO: Trying plugin '{}' ({}) from {}", info.name, info.uid, info.path.display());

        let module = match VstModule::load(&info.path) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                eprintln!("  SKIP: Failed to load module: {}", e);
                continue;
            }
        };

        let class_id = match scanner::hex_string_to_tuid(&info.uid) {
            Some(id) => id,
            None => {
                eprintln!("  SKIP: Invalid UID format");
                continue;
            }
        };

        let host_app = HostApp::new();
        let handler = ComponentHandler::new();

        let mut instance = match PluginInstance::from_factory(Arc::clone(&module), &class_id, host_app, handler) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("  SKIP: Failed to create instance: {}", e);
                continue;
            }
        };

        // Verify it has audio I/O buses (effect plugin)
        let buses = instance.get_bus_info();
        let has_audio_input = buses.iter().any(|b| {
            b.bus_type == BusType::Audio && b.direction == BusDirection::Input
        });
        let has_audio_output = buses.iter().any(|b| {
            b.bus_type == BusType::Audio && b.direction == BusDirection::Output
        });

        if !has_audio_input || !has_audio_output {
            eprintln!(
                "  SKIP: Plugin '{}' lacks audio I/O buses (input={}, output={})",
                info.name, has_audio_input, has_audio_output
            );
            continue;
        }

        // Setup -> activate -> start processing
        if let Err(e) = instance.setup(44100.0, 512) {
            eprintln!("  SKIP: Plugin setup failed: {}", e);
            continue;
        }
        if let Err(e) = instance.activate() {
            eprintln!("  SKIP: Plugin activate failed: {}", e);
            continue;
        }
        if let Err(e) = instance.start_processing() {
            eprintln!("  SKIP: Plugin start_processing failed: {}", e);
            continue;
        }

        eprintln!("INFO: Using plugin '{}' ({})", info.name, info.uid);
        return Some((instance, info.name.clone()));
    }

    eprintln!("WARNING: No suitable effect plugin found. Skipping integration test.");
    None
}

// ---------------------------------------------------------------------------
// Audio generation helper
// ---------------------------------------------------------------------------

/// Generate a stereo 1-second 440Hz sine wave at 44100 Hz.
/// Returns Vec<Vec<f32>> (2 channels, 44100 samples each).
fn generate_test_audio() -> Vec<Vec<f32>> {
    let sample_rate = 44100;
    let duration_secs = 1.0;
    let total_frames = (sample_rate as f32 * duration_secs) as usize;

    let mut left = Vec::with_capacity(total_frames);
    let mut right = Vec::with_capacity(total_frames);

    for i in 0..total_frames {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * PI * 440.0 * t).sin() * 0.5;
        left.push(sample);
        right.push(sample);
    }

    vec![left, right]
}

// ---------------------------------------------------------------------------
// Audio processing helper
// ---------------------------------------------------------------------------

/// Process audio through a plugin instance.
/// Takes &mut PluginInstance and &[&[f32]] input, returns Vec<Vec<f32>> output.
fn process_with_plugin(plugin: &mut PluginInstance, input: &[&[f32]]) -> Vec<Vec<f32>> {
    let channels = input.len();
    let frames = if channels > 0 { input[0].len() } else { 0 };

    // Allocate output buffers
    let mut output: Vec<Vec<f32>> = (0..channels)
        .map(|_| vec![0.0f32; frames])
        .collect();

    // Build mutable slices for process()
    let mut output_slices: Vec<&mut [f32]> = output
        .iter_mut()
        .map(|ch| ch.as_mut_slice())
        .collect();

    // Process the block
    plugin.process(input, &mut output_slices, frames as i32)
        .expect("plugin process should succeed");

    output
}

// ---------------------------------------------------------------------------
// Audio analysis helpers
// ---------------------------------------------------------------------------

/// Compute RMS (root mean square) of a sample buffer.
#[allow(dead_code)]
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    ((sum_sq / samples.len() as f64).sqrt()) as f32
}

/// Compute maximum absolute difference between two audio buffers (sample-wise).
fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
}
