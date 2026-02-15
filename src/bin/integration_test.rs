//! Integration test binary for verifying VST3 hosting against real plugins.
//!
//! Exercises all Phase 1 success criteria:
//!   A. Plugin Discovery (HOST-01)
//!   B. Plugin Loading (HOST-02)
//!   C. Full Lifecycle (HOST-03)
//!   D. Teardown Stress Test (HOST-04)
//!   E. Unified vs Split Detection (HOST-05)
//!
//! Usage:
//!   cargo run --bin agent-audio-integration-test
//!   cargo run --bin agent-audio-integration-test -- --path /path/to/vst3
//!   cargo run --bin agent-audio-integration-test -- --class-id AABBCCDD... --path /path/to/bundle.vst3
//!   cargo run --bin agent-audio-integration-test -- --cycles 20

use std::path::PathBuf;
use std::sync::Arc;

use tracing::debug;

use vst3_mcp_host::hosting::host_app::{ComponentHandler, HostApp};
use vst3_mcp_host::hosting::module::VstModule;
use vst3_mcp_host::hosting::plugin::PluginInstance;
use vst3_mcp_host::hosting::scanner::{hex_string_to_tuid, scan_plugins_safe};
use vst3_mcp_host::hosting::types::{PluginInfo, PluginState};

/// CLI arguments parsed manually (no external arg-parsing crate needed).
struct Args {
    /// Custom scan path (or bundle path when used with --class-id).
    path: Option<String>,
    /// Specific class ID to test (hex string, 32 chars).
    class_id: Option<String>,
    /// Number of load/unload cycles for teardown stress test.
    cycles: usize,
    /// Show help text.
    help: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        path: None,
        class_id: None,
        cycles: 10,
        help: false,
    };

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--help" | "-h" => {
                args.help = true;
            }
            "--path" => {
                i += 1;
                if i < raw.len() {
                    args.path = Some(raw[i].clone());
                }
            }
            "--class-id" => {
                i += 1;
                if i < raw.len() {
                    args.class_id = Some(raw[i].clone());
                }
            }
            "--cycles" => {
                i += 1;
                if i < raw.len() {
                    args.cycles = raw[i].parse().unwrap_or(10);
                }
            }
            other => {
                eprintln!("Unknown argument: {}", other);
                args.help = true;
            }
        }
        i += 1;
    }
    args
}

fn print_help() {
    println!("agent-audio-integration-test -- VST3 hosting integration test");
    println!();
    println!("USAGE:");
    println!("  agent-audio-integration-test [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  --path <dir>         Scan a specific directory for plugins");
    println!("  --class-id <hex>     Test a specific plugin by class ID (32 hex chars)");
    println!("  --cycles <N>         Load/unload cycles for teardown test (default: 10)");
    println!("  --help, -h           Show this help message");
    println!();
    println!("EXAMPLES:");
    println!("  # Scan default paths, test all discovered plugins");
    println!("  agent-audio-integration-test");
    println!();
    println!("  # Scan a specific directory");
    println!("  agent-audio-integration-test --path ~/.vst3");
    println!();
    println!("  # Test a specific plugin with 20 teardown cycles");
    println!("  agent-audio-integration-test --class-id AABBCCDD11223344AABBCCDD11223344 --path /path/to/Plugin.vst3 --cycles 20");
}

// ---- Result tracking ----

struct TestResult {
    phase: &'static str,
    plugin_name: String,
    passed: bool,
    detail: String,
}

impl TestResult {
    fn pass(phase: &'static str, plugin_name: &str, detail: &str) -> Self {
        Self {
            phase,
            plugin_name: plugin_name.to_string(),
            passed: true,
            detail: detail.to_string(),
        }
    }

    fn fail(phase: &'static str, plugin_name: &str, detail: &str) -> Self {
        Self {
            phase,
            plugin_name: plugin_name.to_string(),
            passed: false,
            detail: detail.to_string(),
        }
    }
}

// ---- Phase A: Plugin Discovery ----

fn find_scanner_binary() -> Option<PathBuf> {
    // Look for the scanner binary next to our own executable
    if let Ok(exe) = std::env::current_exe() {
        let scanner = exe.parent()?.join("agent-audio-scanner");
        if scanner.exists() {
            return Some(scanner);
        }
    }
    None
}

fn phase_a_discovery(scan_path: Option<&str>) -> (Vec<PluginInfo>, Vec<TestResult>) {
    let mut results = Vec::new();

    println!();
    println!("========================================");
    println!("  Phase A: Plugin Discovery (HOST-01)");
    println!("========================================");
    println!();

    let scanner_binary = find_scanner_binary();
    if let Some(ref path) = scanner_binary {
        println!("  Using out-of-process scanner: {}", path.display());
    } else {
        println!("  WARNING: Scanner binary not found, using in-process scanning");
        println!("  Build it first: cargo build --bin agent-audio-scanner");
    }
    println!();

    let plugins = match scan_plugins_safe(scan_path, scanner_binary.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            println!("  FAIL  Scanner returned error: {}", e);
            results.push(TestResult::fail("A", "scanner", &format!("scan error: {}", e)));
            return (Vec::new(), results);
        }
    };

    if plugins.is_empty() {
        println!("  FAIL  No plugins discovered");
        results.push(TestResult::fail("A", "scanner", "no plugins found in scan paths"));
        return (plugins, results);
    }

    println!("  Discovered {} plugin(s):", plugins.len());
    println!();
    for (i, p) in plugins.iter().enumerate() {
        println!(
            "    [{}] {} by {} ({})",
            i + 1, p.name, p.vendor, p.category
        );
        println!("        classId: {}", p.uid);
        println!("        path:    {}", p.path.display());
        println!("        version: {}", p.version);
    }
    println!();
    println!("  PASS  Discovered {} plugin(s)", plugins.len());

    results.push(TestResult::pass(
        "A",
        "scanner",
        &format!("discovered {} plugin(s)", plugins.len()),
    ));

    (plugins, results)
}

// ---- Phase B: Plugin Loading ----

fn phase_b_loading(plugin: &PluginInfo) -> (Option<(PluginInstance, Arc<VstModule>)>, TestResult) {
    println!();
    println!("  --- Phase B: Loading \"{}\" ---", plugin.name);

    let tuid = match hex_string_to_tuid(&plugin.uid) {
        Some(t) => t,
        None => {
            println!("    FAIL  Invalid classId hex: {}", plugin.uid);
            return (
                None,
                TestResult::fail("B", &plugin.name, &format!("invalid classId: {}", plugin.uid)),
            );
        }
    };

    let module = match VstModule::load(&plugin.path) {
        Ok(m) => Arc::new(m),
        Err(e) => {
            println!("    FAIL  Module load error: {}", e);
            return (
                None,
                TestResult::fail("B", &plugin.name, &format!("module load: {}", e)),
            );
        }
    };

    let host_app = HostApp::new();
    let handler = ComponentHandler::new();

    match PluginInstance::from_factory(module.clone(), &tuid, host_app, handler) {
        Ok(instance) => {
            println!("    PASS  Loaded \"{}\" by {}", plugin.name, plugin.vendor);
            (
                Some((instance, module)),
                TestResult::pass("B", &plugin.name, "loaded successfully"),
            )
        }
        Err(e) => {
            println!("    FAIL  from_factory error: {}", e);
            (
                None,
                TestResult::fail("B", &plugin.name, &format!("from_factory: {}", e)),
            )
        }
    }
}

// ---- Phase C: Full Lifecycle ----

fn phase_c_lifecycle(instance: &mut PluginInstance, name: &str) -> TestResult {
    println!();
    println!("  --- Phase C: Full Lifecycle \"{}\" ---", name);

    // Check Created state
    if instance.state() != PluginState::Created {
        let msg = format!("expected Created, got {:?}", instance.state());
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    println!("    [1/8] State: Created");

    // Setup
    if let Err(e) = instance.setup(44100.0, 512) {
        let msg = format!("setup failed: {}", e);
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    if instance.state() != PluginState::SetupDone {
        let msg = format!("expected SetupDone after setup, got {:?}", instance.state());
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    println!("    [2/8] State: SetupDone (44100 Hz, 512 samples)");

    // Activate
    if let Err(e) = instance.activate() {
        let msg = format!("activate failed: {}", e);
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    if instance.state() != PluginState::Active {
        let msg = format!("expected Active after activate, got {:?}", instance.state());
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    println!("    [3/8] State: Active");

    // Start processing
    if let Err(e) = instance.start_processing() {
        let msg = format!("start_processing failed: {}", e);
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    if instance.state() != PluginState::Processing {
        let msg = format!(
            "expected Processing after start_processing, got {:?}",
            instance.state()
        );
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    println!("    [4/8] State: Processing");

    // Process a block of silence (2 channels, 512 samples)
    let input_l = vec![0.0f32; 512];
    let input_r = vec![0.0f32; 512];
    let inputs: &[&[f32]] = &[&input_l, &input_r];
    let mut output_l = vec![0.0f32; 512];
    let mut output_r = vec![0.0f32; 512];
    let mut outputs: Vec<&mut [f32]> = vec![&mut output_l, &mut output_r];

    if let Err(e) = instance.process(inputs, &mut outputs, 512) {
        let msg = format!("process failed: {}", e);
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    println!("    [5/8] Processed 512 samples of silence (2ch)");

    // Stop processing
    if let Err(e) = instance.stop_processing() {
        let msg = format!("stop_processing failed: {}", e);
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    if instance.state() != PluginState::Active {
        let msg = format!(
            "expected Active after stop_processing, got {:?}",
            instance.state()
        );
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    println!("    [6/8] State: Active (stopped processing)");

    // Deactivate
    if let Err(e) = instance.deactivate() {
        let msg = format!("deactivate failed: {}", e);
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    if instance.state() != PluginState::SetupDone {
        let msg = format!(
            "expected SetupDone after deactivate, got {:?}",
            instance.state()
        );
        println!("    FAIL  {}", msg);
        return TestResult::fail("C", name, &msg);
    }
    println!("    [7/8] State: SetupDone (deactivated)");

    // Print bus info and parameter count
    let buses = instance.get_bus_info();
    let param_count = instance.get_parameter_count();
    println!("    [8/8] Bus info: {} bus(es), {} parameter(s)", buses.len(), param_count);
    for bus in &buses {
        println!(
            "          {:?} {:?} \"{}\" ({} ch, default_active={})",
            bus.direction, bus.bus_type, bus.name, bus.channel_count, bus.is_default_active
        );
    }

    println!("    PASS  Full lifecycle complete");
    TestResult::pass("C", name, "all state transitions succeeded")
}

// ---- Phase D: Teardown Stress Test ----

fn phase_d_teardown(plugin: &PluginInfo, cycles: usize) -> TestResult {
    println!();
    println!(
        "  --- Phase D: Teardown Stress Test \"{}\" ({} cycles) ---",
        plugin.name, cycles
    );

    let tuid = match hex_string_to_tuid(&plugin.uid) {
        Some(t) => t,
        None => {
            let msg = format!("invalid classId: {}", plugin.uid);
            println!("    FAIL  {}", msg);
            return TestResult::fail("D", &plugin.name, &msg);
        }
    };

    for cycle in 1..=cycles {
        debug!("teardown cycle {}/{}", cycle, cycles);

        // Load module
        let module = match VstModule::load(&plugin.path) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                let msg = format!("cycle {}: module load failed: {}", cycle, e);
                println!("    FAIL  {}", msg);
                return TestResult::fail("D", &plugin.name, &msg);
            }
        };

        let host_app = HostApp::new();
        let handler = ComponentHandler::new();

        // Create instance
        let mut instance =
            match PluginInstance::from_factory(module.clone(), &tuid, host_app, handler) {
                Ok(inst) => inst,
                Err(e) => {
                    let msg = format!("cycle {}: from_factory failed: {}", cycle, e);
                    println!("    FAIL  {}", msg);
                    return TestResult::fail("D", &plugin.name, &msg);
                }
            };

        // Full lifecycle: setup -> activate -> start_processing -> process -> stop -> deactivate
        if let Err(e) = instance.setup(44100.0, 512) {
            let msg = format!("cycle {}: setup failed: {}", cycle, e);
            println!("    FAIL  {}", msg);
            return TestResult::fail("D", &plugin.name, &msg);
        }

        if let Err(e) = instance.activate() {
            let msg = format!("cycle {}: activate failed: {}", cycle, e);
            println!("    FAIL  {}", msg);
            return TestResult::fail("D", &plugin.name, &msg);
        }

        if let Err(e) = instance.start_processing() {
            let msg = format!("cycle {}: start_processing failed: {}", cycle, e);
            println!("    FAIL  {}", msg);
            return TestResult::fail("D", &plugin.name, &msg);
        }

        // Process a small block
        let input_l = vec![0.0f32; 512];
        let input_r = vec![0.0f32; 512];
        let inputs: &[&[f32]] = &[&input_l, &input_r];
        let mut output_l = vec![0.0f32; 512];
        let mut output_r = vec![0.0f32; 512];
        let mut outputs: Vec<&mut [f32]> = vec![&mut output_l, &mut output_r];

        if let Err(e) = instance.process(inputs, &mut outputs, 512) {
            let msg = format!("cycle {}: process failed: {}", cycle, e);
            println!("    FAIL  {}", msg);
            return TestResult::fail("D", &plugin.name, &msg);
        }

        if let Err(e) = instance.stop_processing() {
            let msg = format!("cycle {}: stop_processing failed: {}", cycle, e);
            println!("    FAIL  {}", msg);
            return TestResult::fail("D", &plugin.name, &msg);
        }

        if let Err(e) = instance.deactivate() {
            let msg = format!("cycle {}: deactivate failed: {}", cycle, e);
            println!("    FAIL  {}", msg);
            return TestResult::fail("D", &plugin.name, &msg);
        }

        // Drop instance, then module -- Arc handles the ref counting
        drop(instance);
        drop(module);

        if cycle % 5 == 0 || cycle == cycles {
            println!("    Cycle {}/{} complete", cycle, cycles);
        }
    }

    println!("    PASS  {} load/unload cycles completed cleanly", cycles);
    TestResult::pass(
        "D",
        &plugin.name,
        &format!("{} cycles completed", cycles),
    )
}

// ---- Phase E: Unified vs Split Detection ----

fn phase_e_unified_split(instance: &PluginInstance, name: &str) -> (bool, TestResult) {
    println!();
    println!("  --- Phase E: Unified vs Split Detection \"{}\" ---", name);

    let is_separate = instance.is_controller_separate();
    let arch = if is_separate { "SPLIT" } else { "UNIFIED" };
    println!("    {} architecture: {}", name, arch);
    println!("    PASS  Architecture detected");

    (
        is_separate,
        TestResult::pass("E", name, &format!("{} architecture", arch)),
    )
}

// ---- Main ----

fn main() {
    // Initialize tracing with env-filter
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = parse_args();

    if args.help {
        print_help();
        std::process::exit(0);
    }

    println!("================================================");
    println!("  VST3 Hosting Integration Test");
    println!("  Phase 1 Success Criteria Verification");
    println!("================================================");

    let mut all_results: Vec<TestResult> = Vec::new();
    let mut unified_count: usize = 0;
    let mut split_count: usize = 0;

    // If a specific class-id was given, we need a path to the bundle
    if let Some(ref class_id_hex) = args.class_id {
        let path_str = args.path.as_deref().unwrap_or_else(|| {
            eprintln!("ERROR: --class-id requires --path pointing to the .vst3 bundle");
            std::process::exit(1);
        });

        let tuid = match hex_string_to_tuid(class_id_hex) {
            Some(t) => t,
            None => {
                eprintln!("ERROR: Invalid class ID hex string (expected 32 hex chars): {}", class_id_hex);
                std::process::exit(1);
            }
        };

        // Build a synthetic PluginInfo for reporting
        let plugin = PluginInfo {
            name: format!("classId:{}", &class_id_hex[..8]),
            vendor: String::new(),
            uid: class_id_hex.clone(),
            category: String::new(),
            version: String::new(),
            path: PathBuf::from(path_str),
        };

        println!();
        println!("  Testing specific classId: {}", class_id_hex);
        println!("  Bundle: {}", path_str);

        // Phase A: skipped (we already know the class ID)
        all_results.push(TestResult::pass("A", "manual", "classId specified via CLI"));

        // Phase B: Load
        let module = match VstModule::load(&plugin.path) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                println!("  FAIL  Module load: {}", e);
                all_results.push(TestResult::fail("B", &plugin.name, &format!("{}", e)));
                print_summary(&all_results, unified_count, split_count);
                std::process::exit(1);
            }
        };

        let host_app = HostApp::new();
        let handler = ComponentHandler::new();

        let mut instance =
            match PluginInstance::from_factory(module.clone(), &tuid, host_app, handler) {
                Ok(inst) => inst,
                Err(e) => {
                    println!("  FAIL  from_factory: {}", e);
                    all_results.push(TestResult::fail("B", &plugin.name, &format!("{}", e)));
                    print_summary(&all_results, unified_count, split_count);
                    std::process::exit(1);
                }
            };

        println!("  PASS  Plugin loaded");
        all_results.push(TestResult::pass("B", &plugin.name, "loaded"));

        // Phase C: Lifecycle
        all_results.push(phase_c_lifecycle(&mut instance, &plugin.name));

        // Phase E: Unified/Split (before drop)
        let (is_separate, e_result) = phase_e_unified_split(&instance, &plugin.name);
        if is_separate {
            split_count += 1;
        } else {
            unified_count += 1;
        }
        all_results.push(e_result);

        // Drop the instance before Phase D
        drop(instance);
        drop(module);

        // Phase D: Teardown
        all_results.push(phase_d_teardown(&plugin, args.cycles));

        print_summary(&all_results, unified_count, split_count);
        let exit_code = if all_results.iter().all(|r| r.passed) { 0 } else { 1 };
        std::process::exit(exit_code);
    }

    // Normal flow: scan and test all discovered plugins

    // Phase A: Discovery
    let (plugins, a_results) = phase_a_discovery(args.path.as_deref());
    all_results.extend(a_results);

    if plugins.is_empty() {
        print_summary(&all_results, unified_count, split_count);
        std::process::exit(1);
    }

    // Phases B, C, E for each plugin
    println!();
    println!("========================================");
    println!("  Phase B/C/E: Per-Plugin Tests");
    println!("========================================");

    // Collect info for teardown tests (we need the PluginInfo after dropping instances)
    let mut plugins_to_teardown: Vec<PluginInfo> = Vec::new();

    for plugin in &plugins {
        // Phase B: Loading
        let (loaded, b_result) = phase_b_loading(plugin);
        all_results.push(b_result);

        if let Some((mut instance, _module)) = loaded {
            // Phase C: Lifecycle
            all_results.push(phase_c_lifecycle(&mut instance, &plugin.name));

            // Phase E: Unified/Split
            let (is_separate, e_result) = phase_e_unified_split(&instance, &plugin.name);
            if is_separate {
                split_count += 1;
            } else {
                unified_count += 1;
            }
            all_results.push(e_result);

            // Remember for Phase D
            plugins_to_teardown.push(plugin.clone());

            // Drop instance + module before Phase D
            drop(instance);
        }
    }

    // Phase D: Teardown stress test
    if !plugins_to_teardown.is_empty() {
        println!();
        println!("========================================");
        println!("  Phase D: Teardown Stress Test (HOST-04)");
        println!("========================================");

        for plugin in &plugins_to_teardown {
            all_results.push(phase_d_teardown(plugin, args.cycles));
        }
    }

    print_summary(&all_results, unified_count, split_count);
    let exit_code = if all_results.iter().all(|r| r.passed) { 0 } else { 1 };
    std::process::exit(exit_code);
}

fn print_summary(results: &[TestResult], unified_count: usize, split_count: usize) {
    println!();
    println!("================================================");
    println!("  RESULTS SUMMARY");
    println!("================================================");
    println!();

    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.iter().filter(|r| !r.passed).count();
    let total = results.len();

    for r in results {
        let status = if r.passed { "PASS" } else { "FAIL" };
        println!(
            "  [{}] Phase {}: {} -- {}",
            status, r.phase, r.plugin_name, r.detail
        );
    }

    println!();
    println!("  Architecture Summary: {} unified, {} split", unified_count, split_count);
    println!();
    println!("  Total: {}/{} passed, {} failed", passed, total, failed);

    if failed == 0 {
        println!();
        println!("  ALL TESTS PASSED");
    } else {
        println!();
        println!("  SOME TESTS FAILED");
    }

    println!();
}
