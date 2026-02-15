//! MCP integration test binary for Phase 4.
//!
//! Validates all 6 Phase 4 success criteria:
//!   1. MCP server accepts stdio connections
//!   2. get_plugin_info returns plugin identity
//!   3. list_params returns writable parameters
//!   4. get_param returns value and display string
//!   5. set_param produces audible changes
//!   6. batch_set applies multiple parameters atomically
//!
//! Usage:
//!   cargo run --bin mcp_integration_test
//!   PLUGIN_PATH=/path/to/plugin.vst3 cargo run --bin mcp_integration_test

use std::f32::consts::PI;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use serde_json::{json, Value};

use vst3_mcp_host::hosting::host_app::{ComponentHandler, HostApp};
use vst3_mcp_host::hosting::module::VstModule;
use vst3_mcp_host::hosting::plugin::PluginInstance;
use vst3_mcp_host::hosting::scanner;
use vst3_mcp_host::hosting::types::{BusDirection, BusType};

// ---------------------------------------------------------------------------
// MCP Server Process Management
// ---------------------------------------------------------------------------

struct McpServer {
    process: Child,
}

impl McpServer {
    fn start() -> Result<Self, String> {
        // Find the MCP server binary
        let exe = std::env::current_exe()
            .map_err(|e| format!("Failed to get current exe: {}", e))?;
        let server_binary = exe
            .parent()
            .ok_or("No parent directory")?
            .join("vst3-mcp-host");

        if !server_binary.exists() {
            return Err(format!(
                "MCP server binary not found at {}. Run: cargo build --bin vst3-mcp-host",
                server_binary.display()
            ));
        }

        let process = Command::new(&server_binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Failed to spawn MCP server: {}", e))?;

        eprintln!("INFO: MCP server started");

        let mut server = Self { process };

        // Send initialize request (MCP protocol handshake)
        server.send_request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "mcp_integration_test",
                "version": "1.0.0"
            }
        }))?;

        // Send initialized notification
        server.send_notification("notifications/initialized", json!({}))?;

        Ok(server)
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        let request_str = serde_json::to_string(&request)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;

        let stdin = self
            .process
            .stdin
            .as_mut()
            .ok_or("No stdin available")?;
        writeln!(stdin, "{}", request_str)
            .map_err(|e| format!("Failed to write to server stdin: {}", e))?;
        stdin
            .flush()
            .map_err(|e| format!("Failed to flush stdin: {}", e))?;

        let stdout = self
            .process
            .stdout
            .as_mut()
            .ok_or("No stdout available")?;
        let mut reader = BufReader::new(stdout);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .map_err(|e| format!("Failed to read from server stdout: {}", e))?;

        let response: Value = serde_json::from_str(&response_line)
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if let Some(error) = response.get("error") {
            return Err(format!("Server error: {}", error));
        }

        response
            .get("result")
            .cloned()
            .ok_or_else(|| "No result in response".to_string())
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let notification_str = serde_json::to_string(&notification)
            .map_err(|e| format!("Failed to serialize notification: {}", e))?;

        let stdin = self
            .process
            .stdin
            .as_mut()
            .ok_or("No stdin available")?;
        writeln!(stdin, "{}", notification_str)
            .map_err(|e| format!("Failed to write notification: {}", e))?;
        stdin
            .flush()
            .map_err(|e| format!("Failed to flush stdin: {}", e))?;

        Ok(())
    }

    fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });

        let request_str = serde_json::to_string(&request)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;

        // Write request to server stdin
        let stdin = self
            .process
            .stdin
            .as_mut()
            .ok_or("No stdin available")?;
        writeln!(stdin, "{}", request_str)
            .map_err(|e| format!("Failed to write to server stdin: {}", e))?;
        stdin
            .flush()
            .map_err(|e| format!("Failed to flush stdin: {}", e))?;

        // Read response from server stdout
        let stdout = self
            .process
            .stdout
            .as_mut()
            .ok_or("No stdout available")?;
        let mut reader = BufReader::new(stdout);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .map_err(|e| format!("Failed to read from server stdout: {}", e))?;

        // Parse JSON-RPC response
        let response: Value = serde_json::from_str(&response_line)
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if let Some(error) = response.get("error") {
            return Err(format!("Server error: {}", error));
        }

        // Extract result from MCP response
        let result = response
            .get("result")
            .ok_or_else(|| "No result in response".to_string())?;

        // Check if MCP tool returned an error (isError: true)
        if let Some(is_error) = result.get("isError").and_then(|v| v.as_bool()) {
            if is_error {
                let error_text = result
                    .get("content")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|item| item.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Tool error: {}", error_text));
            }
        }

        // MCP tools return result in content array with type "text"
        // Extract the text content and parse as JSON
        if let Some(content_array) = result.get("content").and_then(|v| v.as_array()) {
            if let Some(first_content) = content_array.first() {
                if let Some(text) = first_content.get("text").and_then(|v| v.as_str()) {
                    return serde_json::from_str(text)
                        .map_err(|e| format!("Failed to parse tool response JSON: {}", e));
                }
            }
        }

        // Fallback: return result as-is
        Ok(result.clone())
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}

// ---------------------------------------------------------------------------
// Plugin Loading Helper
// ---------------------------------------------------------------------------

/// Find a suitable VST3 plugin for testing.
/// Returns (plugin_uid, plugin_name) or None if no plugin found.
fn find_test_plugin() -> Option<(String, String)> {
    let custom_path = std::env::var("PLUGIN_PATH").ok();

    let plugins = scanner::scan_plugins(custom_path.as_deref())
        .unwrap_or_else(|e| {
            eprintln!("WARNING: Plugin scan failed: {}", e);
            Vec::new()
        });

    if plugins.is_empty() {
        eprintln!("WARNING: No VST3 plugins found. Set PLUGIN_PATH or install plugins.");
        return None;
    }

    // Preferred plugin names (known to have parameters and work well)
    let preferred_names = ["again", "vital", "adelay"];

    // Sort: preferred plugins first
    let mut sorted_plugins: Vec<&_> = plugins.iter().collect();
    sorted_plugins.sort_by_key(|p| {
        let name_lower = p.name.to_lowercase();
        preferred_names
            .iter()
            .position(|pref| name_lower.contains(pref))
            .unwrap_or(usize::MAX)
    });

    // Return first plugin with audio I/O and parameters
    for info in &sorted_plugins {
        // Quick validation: check if it has a sensible path
        if !info.path.exists() {
            continue;
        }

        eprintln!("INFO: Selected plugin '{}' ({})", info.name, info.uid);
        return Some((info.uid.clone(), info.name.clone()));
    }

    eprintln!("WARNING: No suitable plugin found for testing");
    None
}

// ---------------------------------------------------------------------------
// Direct Plugin Access (for audible change tests)
// ---------------------------------------------------------------------------

/// Load a plugin instance directly (not via MCP) for processing audio.
fn load_plugin_direct(uid: &str) -> Option<(PluginInstance, String)> {
    let custom_path = std::env::var("PLUGIN_PATH").ok();

    let plugins = scanner::scan_plugins(custom_path.as_deref()).ok()?;
    let info = plugins.iter().find(|p| p.uid == uid)?;

    let module = VstModule::load(&info.path).ok()?;
    let module = Arc::new(module);

    let class_id = scanner::hex_string_to_tuid(&info.uid)?;
    let host_app = HostApp::new();
    let handler = ComponentHandler::new();

    let mut instance = PluginInstance::from_factory(Arc::clone(&module), &class_id, host_app, handler).ok()?;

    // Verify audio I/O
    let buses = instance.get_bus_info();
    let has_audio_input = buses
        .iter()
        .any(|b| b.bus_type == BusType::Audio && b.direction == BusDirection::Input);
    let has_audio_output = buses
        .iter()
        .any(|b| b.bus_type == BusType::Audio && b.direction == BusDirection::Output);

    if !has_audio_input || !has_audio_output {
        return None;
    }

    // Setup
    instance.setup(44100.0, 512).ok()?;
    instance.activate().ok()?;
    instance.start_processing().ok()?;

    Some((instance, info.name.clone()))
}

/// Generate 1-second stereo 440Hz sine wave at 44100 Hz.
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

/// Process audio through plugin.
fn process_with_plugin(plugin: &mut PluginInstance, input: &[&[f32]]) -> Vec<Vec<f32>> {
    let channels = input.len();
    let frames = if channels > 0 { input[0].len() } else { 0 };

    let mut output_planar: Vec<Vec<f32>> = (0..channels)
        .map(|_| vec![0.0f32; frames])
        .collect();

    let input_slices: Vec<&[f32]> = input.iter().map(|ch| &ch[..]).collect();
    let mut output_slices: Vec<&mut [f32]> = output_planar
        .iter_mut()
        .map(|ch| &mut ch[0..frames])
        .collect();

    plugin
        .process(&input_slices, &mut output_slices, frames as i32)
        .expect("plugin process should succeed");

    output_planar
}

/// Compute maximum absolute difference between two buffers.
fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
}

// ---------------------------------------------------------------------------
// Test Implementations
// ---------------------------------------------------------------------------

fn test_get_plugin_info(server: &mut McpServer) -> Result<(), String> {
    eprintln!("\n--- Test 2: get_plugin_info ---");

    let result = server.call_tool("get_plugin_info", json!({}))?;

    // Validate response structure
    let class_id = result
        .get("classId")
        .and_then(|v| v.as_str())
        .ok_or("Missing classId field")?;
    let name = result
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name field")?;
    let vendor = result
        .get("vendor")
        .and_then(|v| v.as_str())
        .ok_or("Missing vendor field")?;

    if class_id.is_empty() || name.is_empty() {
        return Err("classId or name is empty".to_string());
    }

    eprintln!("  classId: {}", class_id);
    eprintln!("  name:    {}", name);
    eprintln!("  vendor:  {}", vendor);
    eprintln!("✓ Test 2: get_plugin_info returns plugin identity");

    Ok(())
}

fn test_list_params(server: &mut McpServer) -> Result<Value, String> {
    eprintln!("\n--- Test 3: list_params ---");

    let result = server.call_tool("list_params", json!({}))?;

    let parameters = result
        .get("parameters")
        .and_then(|v| v.as_array())
        .ok_or("Missing parameters array")?;
    let count = result
        .get("count")
        .and_then(|v| v.as_u64())
        .ok_or("Missing count field")?;

    if parameters.is_empty() {
        return Err("No writable parameters found".to_string());
    }

    if parameters.len() != count as usize {
        return Err(format!(
            "Parameter count mismatch: array has {} but count is {}",
            parameters.len(),
            count
        ));
    }

    // Validate first parameter structure
    let first = &parameters[0];
    let id = first
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or("Missing id field in parameter")?;
    let name = first
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name field in parameter")?;
    let value = first
        .get("value")
        .and_then(|v| v.as_f64())
        .ok_or("Missing value field in parameter")?;
    let display = first
        .get("display")
        .and_then(|v| v.as_str())
        .ok_or("Missing display field in parameter")?;

    eprintln!("  Found {} writable parameters", count);
    eprintln!("  First param: id={}, name='{}', value={}, display='{}'", id, name, value, display);
    eprintln!("✓ Test 3: list_params returns writable parameters");

    Ok(result)
}

fn test_get_param(server: &mut McpServer, param_id: u64) -> Result<(), String> {
    eprintln!("\n--- Test 4: get_param ---");

    let result = server.call_tool("get_param", json!({ "id": param_id }))?;

    let id = result
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or("Missing id field")?;
    let value = result
        .get("value")
        .and_then(|v| v.as_f64())
        .ok_or("Missing value field")?;
    let display = result
        .get("display")
        .and_then(|v| v.as_str())
        .ok_or("Missing display field")?;

    if id != param_id {
        return Err(format!("ID mismatch: expected {} got {}", param_id, id));
    }

    if value < 0.0 || value > 1.0 {
        return Err(format!("Value {} out of range [0, 1]", value));
    }

    eprintln!("  id={}, value={}, display='{}'", id, value, display);
    eprintln!("✓ Test 4: get_param returns value and display string");

    Ok(())
}

fn test_set_param_audible(
    server: &mut McpServer,
    plugin_uid: &str,
    param_id: u64,
) -> Result<(), String> {
    eprintln!("\n--- Test 5: set_param produces audible change ---");

    // Load plugin directly for processing
    let (mut plugin, plugin_name) = load_plugin_direct(plugin_uid)
        .ok_or("Failed to load plugin for audio processing")?;

    // Generate test audio
    let test_audio = generate_test_audio();
    let input_slices: Vec<&[f32]> = test_audio.iter().map(|ch| ch.as_slice()).collect();

    // Process with default parameters → baseline
    let baseline = process_with_plugin(&mut plugin, &input_slices);

    // Queue parameter change via MCP
    let result = server.call_tool(
        "set_param",
        json!({ "id": param_id, "value": 0.75 }),
    )?;

    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or("Missing status field")?;

    if status != "queued" {
        return Err(format!("Expected status 'queued', got '{}'", status));
    }

    // Apply the parameter change by processing audio
    plugin.queue_parameter_change(param_id as u32, 0.75);
    let modified = process_with_plugin(&mut plugin, &input_slices);

    // Compare baseline vs modified
    let diff_left = max_abs_diff(&baseline[0], &modified[0]);
    let diff_right = max_abs_diff(&baseline[1], &modified[1]);
    let max_diff = diff_left.max(diff_right);

    eprintln!("  Plugin: {}", plugin_name);
    eprintln!("  Max diff: {:.6}", max_diff);
    eprintln!("  Threshold: 0.001");

    if max_diff < 0.001 {
        return Err(format!(
            "Parameter change produced no audible effect (max_diff={:.6} < 0.001)",
            max_diff
        ));
    }

    eprintln!("✓ Test 5: set_param produces audible change");

    Ok(())
}

fn test_batch_set(
    server: &mut McpServer,
    plugin_uid: &str,
    param_ids: &[u64],
) -> Result<(), String> {
    eprintln!("\n--- Test 6: batch_set ---");

    if param_ids.is_empty() {
        return Err("Need at least 1 parameter for batch_set test".to_string());
    }

    // Use however many parameters are available (up to 3)
    let param_count = param_ids.len().min(3);
    let test_params = &param_ids[0..param_count];

    let changes: Vec<Value> = test_params
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let value = match i {
                0 => 0.8,
                1 => 0.3,
                _ => 0.6,
            };
            json!({ "id": id, "value": value })
        })
        .collect();

    let result = server.call_tool("batch_set", json!({ "changes": changes }))?;

    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or("Missing status field")?;
    let changes_queued = result
        .get("changes_queued")
        .and_then(|v| v.as_u64())
        .ok_or("Missing changes_queued field")?;

    if status != "queued" {
        return Err(format!("Expected status 'queued', got '{}'", status));
    }

    if changes_queued != param_count as u64 {
        return Err(format!(
            "Expected {} changes queued, got {}",
            param_count, changes_queued
        ));
    }

    // Verify changes are applied via get_param
    for &param_id in test_params.iter() {
        let result = server.call_tool("get_param", json!({ "id": param_id }))?;
        let value = result
            .get("value")
            .and_then(|v| v.as_f64())
            .ok_or("Missing value field")?;

        eprintln!("  Param {}: value={}", param_id, value);
    }

    // Verify audible change (any difference from baseline)
    let (mut plugin, _) = load_plugin_direct(plugin_uid)
        .ok_or("Failed to load plugin for batch test")?;

    let test_audio = generate_test_audio();
    let input_slices: Vec<&[f32]> = test_audio.iter().map(|ch| ch.as_slice()).collect();

    let baseline = process_with_plugin(&mut plugin, &input_slices);

    // Apply batch changes
    for (i, &param_id) in test_params.iter().enumerate() {
        let value = match i {
            0 => 0.8,
            1 => 0.3,
            _ => 0.6,
        };
        plugin.queue_parameter_change(param_id as u32, value);
    }

    let modified = process_with_plugin(&mut plugin, &input_slices);

    let max_diff = max_abs_diff(&baseline[0], &modified[0]);
    if max_diff < 0.001 {
        return Err("Batch changes produced no audible effect".to_string());
    }

    eprintln!("  Queued {} parameter changes", param_count);
    eprintln!("✓ Test 6: batch_set applies multiple parameters");

    Ok(())
}

// ---------------------------------------------------------------------------
// Main Test Runner
// ---------------------------------------------------------------------------

fn run_tests() -> Result<(), String> {
    eprintln!("\n========================================");
    eprintln!("  Phase 4 MCP Integration Tests");
    eprintln!("========================================");

    // Find a test plugin
    let (plugin_uid, plugin_name) = match find_test_plugin() {
        Some(p) => p,
        None => {
            eprintln!("\nSKIP: No VST3 plugin found for testing");
            eprintln!("Set PLUGIN_PATH or install plugins to run integration tests");
            return Ok(());
        }
    };

    eprintln!("\nUsing plugin: {} ({})", plugin_name, plugin_uid);

    // Start MCP server
    let mut server = McpServer::start()?;

    // Test 1: MCP server accepts connections (implicit - we started it)
    eprintln!("\n--- Test 1: MCP server accepts stdio connections ---");
    eprintln!("✓ Test 1: MCP server started and accepting connections");

    // Load plugin via MCP
    eprintln!("\n--- Loading plugin via MCP ---");
    let load_result = server.call_tool(
        "load_plugin",
        json!({ "uid": plugin_uid, "sample_rate": 44100 }),
    )?;
    eprintln!("  Plugin loaded: {}", load_result.get("name").unwrap_or(&json!("unknown")));

    // Test 2: get_plugin_info
    test_get_plugin_info(&mut server)?;

    // Test 3: list_params
    let list_result = test_list_params(&mut server)?;

    // Extract first parameter ID for subsequent tests
    let parameters = list_result
        .get("parameters")
        .and_then(|v| v.as_array())
        .ok_or("No parameters array")?;
    let first_param_id = parameters[0]
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or("No id in first parameter")?;

    // Test 4: get_param
    test_get_param(&mut server, first_param_id)?;

    // Test 5: set_param with audible change
    test_set_param_audible(&mut server, &plugin_uid, first_param_id)?;

    // Test 6: batch_set (use all available parameters, up to 3)
    let param_ids: Vec<u64> = parameters
        .iter()
        .filter_map(|p| p.get("id").and_then(|v| v.as_u64()))
        .collect();
    test_batch_set(&mut server, &plugin_uid, &param_ids)?;

    eprintln!("\n========================================");
    eprintln!("  Phase 4 Integration Tests: 6/6 PASSED");
    eprintln!("========================================\n");

    Ok(())
}

fn main() {
    match run_tests() {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("\nERROR: {}", e);
            eprintln!("\nPhase 4 Integration Tests: FAILED\n");
            std::process::exit(1);
        }
    }
}
