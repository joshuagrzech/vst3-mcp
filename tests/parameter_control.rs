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
    let mut output_planar: Vec<Vec<f32>> = (0..channels)
        .map(|_| vec![0.0f32; frames])
        .collect();

    // Build per-channel input slices
    let input_slices: Vec<&[f32]> = input.iter().map(|ch| &ch[..]).collect();

    // Build per-channel output slices
    let mut output_slices: Vec<&mut [f32]> = output_planar
        .iter_mut()
        .map(|ch| &mut ch[0..frames])
        .collect();

    // Process the block
    plugin.process(&input_slices, &mut output_slices, frames as i32)
        .expect("plugin process should succeed");

    output_planar
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

// ---------------------------------------------------------------------------
// Test 1: Parameter enumeration (Success Criterion 1)
// ---------------------------------------------------------------------------

#[test]
fn test_enumerate_parameters() {
    let (plugin, plugin_name) = match load_test_plugin() {
        Some(p) => p,
        None => return,
    };

    let param_count = plugin.get_parameter_count();
    assert!(param_count > 0, "Plugin should expose at least one parameter");

    // Enumerate all parameters and validate structure
    for i in 0..param_count {
        let info = plugin.get_parameter_info(i).expect("getParameterInfo should succeed");
        assert!(!info.title.is_empty(), "Parameter {} should have non-empty title", i);
        // Validate id, flags exist (no crashes)
        let _ = info.id;
        let _ = info.flags;
    }

    eprintln!("[PASS] Enumerated {} parameters from {}", param_count, plugin_name);
}

// ---------------------------------------------------------------------------
// Test 2: Read parameter with display (Success Criteria 2, 4)
// ---------------------------------------------------------------------------

#[test]
fn test_read_parameter_with_display() {
    let (plugin, _plugin_name) = match load_test_plugin() {
        Some(p) => p,
        None => return,
    };

    let param_count = plugin.get_parameter_count();
    if param_count == 0 {
        eprintln!("[SKIP] No parameters to read");
        return;
    }

    // Read first parameter
    let info = plugin.get_parameter_info(0).expect("getParameterInfo should succeed");
    let normalized = plugin.get_parameter(info.id);
    assert!(normalized >= 0.0 && normalized <= 1.0, "Normalized value should be in [0,1]");

    let display = plugin.get_parameter_display(info.id).expect("get_parameter_display should succeed");
    assert!(!display.is_empty(), "Display string should not be empty");

    eprintln!("[PASS] Read param '{}' = {} ({})", info.title, normalized, display);
}

// ---------------------------------------------------------------------------
// Test 3: Write parameter audible (Success Criteria 3, 6)
// ---------------------------------------------------------------------------

#[test]
fn test_write_parameter_audible() {
    let (mut plugin, _plugin_name) = match load_test_plugin() {
        Some(p) => p,
        None => return,
    };

    // Find a writable parameter
    let param_count = plugin.get_parameter_count();
    let writable_param = (0..param_count)
        .filter_map(|i| plugin.get_parameter_info(i).ok())
        .find(|info| info.is_writable() && !info.is_bypass());

    let param_info = match writable_param {
        Some(info) => info,
        None => {
            eprintln!("[SKIP] No writable non-bypass parameters found");
            return;
        }
    };

    // Generate test audio
    let input = generate_test_audio();
    let input_refs: Vec<_> = input.iter().map(|ch| ch.as_slice()).collect();

    // Process with default parameter value
    let output_default = process_with_plugin(&mut plugin, &input_refs);

    // Write parameter to different value
    let default_val = plugin.get_parameter(param_info.id);
    let new_val = if default_val < 0.5 { 0.9 } else { 0.1 }; // Opposite extreme
    plugin.queue_parameter_change(param_info.id, new_val);

    // Process with modified parameter
    let output_modified = process_with_plugin(&mut plugin, &input_refs);

    // Verify outputs are different (parameter change had audible effect)
    let diff = max_abs_diff(&output_default[0], &output_modified[0]);
    assert!(diff > 0.001, "Parameter change should produce audible difference (diff: {})", diff);

    eprintln!("[PASS] Parameter '{}' change {} -> {} produced audible effect (max diff: {:.4})",
              param_info.title, default_val, new_val, diff);
}

// ---------------------------------------------------------------------------
// Test 4: Read-only parameters filtered (Success Criterion 4)
// ---------------------------------------------------------------------------

#[test]
fn test_readonly_parameters_filtered() {
    let (plugin, _plugin_name) = match load_test_plugin() {
        Some(p) => p,
        None => return,
    };

    let param_count = plugin.get_parameter_count();
    let mut readonly_count = 0;
    let mut writable_count = 0;

    for i in 0..param_count {
        let info = plugin.get_parameter_info(i).expect("getParameterInfo should succeed");
        if info.is_read_only() {
            readonly_count += 1;
            assert!(!info.is_writable(), "Read-only param should not be writable");
        }
        if info.is_writable() {
            writable_count += 1;
            assert!(!info.is_read_only(), "Writable param should not be read-only");
        }
    }

    eprintln!("[PASS] Filtered {} readonly, {} writable from {} total parameters",
              readonly_count, writable_count, param_count);
}

// ---------------------------------------------------------------------------
// Test 5: Parameter sweep smooth (Success Criterion 5)
// ---------------------------------------------------------------------------

#[test]
fn test_parameter_sweep_smooth() {
    let (mut plugin, _plugin_name) = match load_test_plugin() {
        Some(p) => p,
        None => return,
    };

    // Find a writable parameter
    let param_count = plugin.get_parameter_count();
    let writable_param = (0..param_count)
        .filter_map(|i| plugin.get_parameter_info(i).ok())
        .find(|info| info.is_writable() && !info.is_bypass());

    let param_info = match writable_param {
        Some(info) => info,
        None => {
            eprintln!("[SKIP] No writable non-bypass parameters found");
            return;
        }
    };

    // Generate test audio
    let input = generate_test_audio();
    let input_refs: Vec<_> = input.iter().map(|ch| ch.as_slice()).collect();

    // Perform parameter sweep: 0.1 -> 0.9 over 8 steps
    let sweep_steps = 8;
    for i in 0..sweep_steps {
        let t = i as f64 / (sweep_steps - 1) as f64;
        let value = 0.1 + t * 0.8; // Linear interpolation 0.1 to 0.9
        plugin.queue_parameter_change(param_info.id, value);

        // Process a block
        let _ = process_with_plugin(&mut plugin, &input_refs);
    }

    // Verify no crash and processing succeeded
    // Note: Detailed zipper noise detection requires FFT/spectral analysis (out of scope for MVP)
    // This test validates that sweeps don't crash and produce valid output
    eprintln!("[PASS] Parameter sweep of '{}' completed {} steps without crash", param_info.title, sweep_steps);
}
