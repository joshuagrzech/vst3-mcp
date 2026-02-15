# Phase 1: Plugin Hosting - Research

**Researched:** 2026-02-15
**Domain:** VST3 plugin hosting in Rust -- scanning, loading, lifecycle, teardown, unified/split component handling
**Confidence:** HIGH (existing codebase already implements most of this; Steinberg spec well-documented; remaining gaps are hardening concerns)

## Summary

Phase 1 is about proving that a child VST3 plugin can be loaded, fully lifecycle-managed, and torn down cleanly. The good news: the existing codebase already implements the vast majority of this. The `hosting/` module contains working plugin scanning (`scanner.rs`), module loading via `libloading` (`module.rs`), full lifecycle state machine (`plugin.rs`), COM host interfaces (`host_app.rs`), and both unified and split component/controller handling. What remains is primarily hardening work: out-of-process scanning for crash isolation, explicit COM teardown ordering to prevent segfaults, and verification with multiple real-world plugins from different vendors.

The existing implementation uses the `vst3` crate (0.3.0, coupler-rs) for COM bindings, which provides `ComPtr`/`ComWrapper` smart pointers and pre-generated bindings without requiring libclang or the C++ SDK at build time. This is the correct choice for hosting. The scanner already supports both the fast path (reading `moduleinfo.json` without loading the binary) and the slow path (loading the module and querying `IPluginFactory`/`IPluginFactory2`). The lifecycle state machine correctly enforces `Created -> SetupDone -> Active -> Processing` transitions with proper state checks.

The primary gaps are: (1) scanning currently happens in-process, meaning a buggy plugin can crash the entire host during scan; (2) the `Drop` implementation for `PluginInstance` should explicitly release connection point COM pointers before calling `terminate()` rather than relying on implicit field drop order; and (3) testing with multiple plugin brands is needed to verify both unified and split component/controller patterns work reliably across vendors.

**Primary recommendation:** Focus on hardening the existing code rather than rewriting it. Add out-of-process scanning, strengthen the teardown sequence, and write integration tests against at least two different plugin brands (one unified, one split component/controller).

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `vst3` (coupler-rs) | 0.3.0 | VST3 COM bindings for hosting child plugins | Pre-generated Rust bindings to VST3 COM interfaces. MIT/Apache-2.0 licensed. Provides `ComPtr`, `ComRef`, `ComWrapper`, and the `Class` trait for implementing host-side COM objects. SDK 3.8.0 compatible (issue #20 resolved Nov 2025). Already used in current codebase. |
| `libloading` | 0.9.0 | Dynamic library loading (.so/.dylib/.dll) | Cross-platform `dlopen`/`LoadLibrary` wrapper. Loads .vst3 bundle's shared library at runtime. Already used in current codebase. |
| `serde` + `serde_json` | 1.0.x / 1.0.x | JSON parsing for moduleinfo.json and scanner output | Already used for parsing `moduleinfo.json` during fast-path scanning. |
| `tracing` | 0.1.x | Structured logging | Essential for debugging COM lifecycle and plugin interactions. Already used throughout codebase. |
| `thiserror` | 2.0.x | Typed error handling | Used for `HostError` enum in hosting layer. Already in codebase. |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `tracing-subscriber` | 0.3.x | Log output formatting | Enable `env-filter` for runtime log level control. Already in codebase. |
| `anyhow` | 1.0.x | Error handling (application layer) | For the scanner binary and integration tests. Already in codebase. |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `vst3` (coupler-rs) 0.3.0 | `vst3-sys` (RustAudio) | GPLv3, no versioned crates.io releases, plugin-dev focused (not hosting), conflicts with nih-plug's internal fork |
| `vst3` (coupler-rs) 0.3.0 | `rack` 0.4.8 | Higher-level hosting abstractions, but VST3 support on Linux listed as "untested" with "no GUI yet". Not mature enough. |
| `libloading` 0.9.0 | Manual `dlopen` FFI | No cross-platform abstraction, error handling burden |
| In-process scanning | Out-of-process scanning | In-process is simpler but one crashy plugin kills the host. Every major DAW uses out-of-process scanning. |

**Dependencies (Cargo.toml for Phase 1 only):**
```toml
[dependencies]
vst3 = "0.3.0"
libloading = "0.9.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "2.0"
anyhow = "1.0"
```

## Architecture Patterns

### Recommended Project Structure (Phase 1 scope)

```
src/
  hosting/
    mod.rs             # Re-exports
    scanner.rs         # Plugin discovery (in-process + out-of-process)
    module.rs          # VstModule: dlopen, InitDll/ExitDll, GetPluginFactory
    plugin.rs          # PluginInstance: lifecycle state machine + COM RAII
    host_app.rs        # IHostApplication + IComponentHandler COM implementations
    types.rs           # PluginInfo, BusInfo, ParamInfo, PluginState, HostError
```

This structure already exists in the codebase. No restructuring needed for Phase 1.

### Pattern 1: VST3 Lifecycle State Machine (Runtime Enum)

**What:** The `PluginInstance` struct uses a `PluginState` enum to enforce the mandatory VST3 lifecycle sequence at runtime. Each transition method checks the current state and returns an error if called out of order.

**When to use:** Always. Calling VST3 methods in the wrong order causes undefined behavior or crashes.

**Current implementation status:** Already implemented in `src/hosting/plugin.rs`. The state transitions are:
- `from_factory()` -> `Created`
- `setup(sample_rate, max_block_size)` -> `SetupDone`
- `activate()` -> `Active`
- `start_processing()` -> `Processing`
- `stop_processing()` -> `Active`
- `deactivate()` -> `SetupDone`

**The required VST3 call sequence (from Steinberg spec):**

```
Host                              Plugin
 |                                  |
 |-- createInstance(classId) ------>|  (IPluginFactory)
 |-- initialize(hostContext) ----->|  (IComponent)
 |-- [query IAudioProcessor] ----->|  (cast from IComponent)
 |-- [get/create IEditController]->|  (cast or separate factory create)
 |-- [controller.initialize()] --->|  (if separate controller)
 |-- [connect IConnectionPoints]-->|  (bidirectional)
 |-- [setComponentHandler()] ----->|  (on controller)
 |-- setupProcessing(setup) ------>|  (IAudioProcessor)
 |-- activateBus(...) ------------>|  (IComponent, for each default-active bus)
 |-- setActive(true) ------------->|  (IComponent)
 |-- setProcessing(true) --------->|  (IAudioProcessor)
 |-- process(data) --------------->|  (IAudioProcessor, repeated)
 |-- setProcessing(false) -------->|  (teardown begins)
 |-- setActive(false) ------------>|
 |-- [disconnect IConnectionPoints]|
 |-- [controller.terminate()] ---->|  (if separate controller)
 |-- component.terminate() ------->|
 |-- [release all COM pointers] -->|
 |-- [unload module] ------------->|
```

**Source:** [Steinberg VST3 API Documentation - Creation and Initialization](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/API+Documentation/Index.html)

### Pattern 2: Unified vs Split Component/Controller Handling

**What:** VST3 plugins come in two flavors:
1. **Unified (Single Component):** IComponent and IEditController are implemented on the same COM object. Casting `IComponent` to `IEditController` succeeds.
2. **Split (Separate Components):** IComponent and IEditController are separate COM classes. The host must call `IComponent::getControllerClassId()` to get the controller's class ID, then create the controller from the factory as a separate instance, then call `controller.initialize()`.

**When to use:** Always. The host must handle both patterns.

**Current implementation status:** Already implemented correctly in `PluginInstance::from_factory()`:
1. First tries `component.cast::<IEditController>()` (unified path)
2. If that fails, calls `component.getControllerClassId()` and creates controller from factory (split path)
3. If neither works, proceeds with `controller: None` (some plugins have no controller)

**Critical detail for split controllers:** When the controller is a separate object, it MUST be initialized with `controller.initialize(hostContext)` and terminated with `controller.terminate()` independently. The current `Drop` implementation correctly checks whether the controller is the same COM object as the component (via pointer comparison) before calling `terminate()`.

**Steinberg guidance:** "A plug-in that supports this separation has to set the `Vst::kDistributable` flag in the class info of the processor component. Additionally, a host does not need to instantiate the controller part of a plug-in for processing it."

**Source:** [Steinberg Forums - VST3 Single Component Question](https://forums.steinberg.net/t/vst-3-single-component-process-and-controller-question/201967), [Surge issue #164](https://github.com/surge-synthesizer/surge/issues/164)

### Pattern 3: Out-of-Process Plugin Scanning

**What:** Scan plugins in a child process so that if a buggy plugin crashes during loading, only the scanner process dies, not the host.

**When to use:** Always for scanning. This is what every major DAW does (Bitwig, REAPER, Cubase, Ableton). Steinberg's own guidance states: "You need to init plug-ins on the main thread" and recommends "use a helper app you call which does the first scan."

**Implementation approach for Rust:**

```
1. Build a small scanner binary (e.g., `agent-audio-scanner`)
2. The main host spawns the scanner as a child process per .vst3 bundle
3. The scanner loads the module, queries the factory, writes PluginInfo JSON to stdout
4. The host reads stdout, parses JSON
5. If the child process crashes (non-zero exit, timeout), mark that plugin as broken and continue
6. Use a heartbeat/timeout mechanism (e.g., 10 seconds per plugin) to detect hangs
```

**Current status:** The existing scanner runs in-process. It needs to be wrapped with an out-of-process coordinator.

**Two scanning tiers:**
- **Fast path (no binary load):** Read `moduleinfo.json` from `Contents/Resources/`. This is safe in-process since it is just JSON file I/O. Already implemented.
- **Slow path (binary load required):** Load the `.vst3` module via `dlopen`, call `GetPluginFactory`, query class info. This is the crash risk. Must be done out-of-process.

**Source:** [JUCE VST3 Plugin Scanning Crash Protection](https://forum.juce.com/t/vst3-plugin-scanning-crash-protection/58485), [Steinberg Forums - Plugin Crash While Scanning](https://forums.steinberg.net/t/vst3-host-plugin-crash-while-scanning-bundleentry-bundleexit/776824)

### Pattern 4: COM RAII via ComPtr

**What:** The `vst3` crate's `ComPtr<T>` manages reference counting automatically. `ComPtr::from_raw()` takes ownership of an existing reference (does NOT AddRef). When `ComPtr` is dropped, it calls `Release`. `ComWrapper::new()` creates a host-side COM object with automatic reference counting.

**When to use:** For all COM pointer management. Never manually call `AddRef`/`Release`.

**Current implementation status:** Already used correctly throughout `plugin.rs`, `module.rs`, and `host_app.rs`.

**Key gotcha:** `ComPtr::from_raw()` takes ownership. If you also manually release, you get double-free. If you forget `from_raw()` and let the raw pointer leak, you get a memory leak.

### Anti-Patterns to Avoid

- **Dropping the module before all COM pointers:** The `VstModule` owns the `Library` (dlopen handle). If the library is unloaded while COM pointers still reference code in it, use-after-free occurs. The current design stores `VstModule` separately from `PluginInstance`. The caller MUST ensure `VstModule` outlives all `PluginInstance` objects created from its factory.

- **Relying on implicit struct field drop order for COM teardown:** Rust drops struct fields in declaration order, but the VST3 teardown requires explicit sequencing (disconnect -> terminate -> release). The `Drop` impl should use `Option::take()` to explicitly control release order.

- **Scanning on non-main thread:** Some plugins (Waves, SSL) crash when loaded on a non-main thread. Even for out-of-process scanning, the scanner process should load plugins on its main thread.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| COM reference counting | Manual AddRef/Release calls | `ComPtr`/`ComRef` from vst3 crate | One missed Release = leak, one extra Release = use-after-free |
| Dynamic library loading | Manual dlopen/dlsym FFI | `libloading` crate | Cross-platform, error handling, lifetime safety |
| moduleinfo.json parsing | Custom binary format parser | `serde_json` with `serde_json::Value` | JSON is the standard fast-scan format; already implemented |
| UTF-16 string conversion | Manual byte manipulation | `String::from_utf16_lossy()` | Already used in `string128_to_string()` helper |
| Plugin lifecycle enforcement | Manual state tracking with booleans | Enum state machine (already in `PluginState`) | Compile-time clarity, runtime safety, exhaustive matching |
| Out-of-process scanning IPC | Custom socket protocol | `std::process::Command` + stdout JSON | Simple, reliable, no external dependencies |

**Key insight:** The VST3 hosting layer IS the hand-built part. There is no Rust crate that provides safe, high-level VST3 hosting APIs. The `vst3` crate gives raw COM bindings; we must build the hosting layer (lifecycle, scanning, teardown) ourselves. This is already done -- the focus is hardening, not building from scratch.

## Common Pitfalls

### Pitfall 1: COM Pointer Drop Order Causing Segfaults on Teardown

**What goes wrong:** Dropping COM pointers after calling `terminate()` on the component, or dropping the module (unloading the shared library) while COM pointers still reference code inside it. Results in use-after-free or segfault.

**Why it happens:** Rust's struct `Drop` executes in field declaration order, but VST3 requires a specific teardown sequence: `setProcessing(false)` -> `setActive(false)` -> disconnect `IConnectionPoint` -> `controller.terminate()` (if split) -> `component.terminate()` -> drop all COM pointers -> unload module. If connection point `ComPtr`s are dropped after `terminate()`, the Release call may touch freed memory.

**How to avoid:**
- In the `Drop` impl, explicitly take and drop connection point pointers BEFORE calling `terminate()`. Use `Option::take()` to consume them.
- Ensure the caller keeps `VstModule` alive strictly longer than all `PluginInstance` objects. Consider using `Arc<VstModule>` stored inside `PluginInstance` to enforce this.
- Test with AddressSanitizer (`ASAN`) enabled to catch use-after-free.

**Warning signs:** Segfaults during plugin unload, especially with certain plugin brands. Crashes that only appear on repeated load/unload cycles.

**Current status:** The existing `Drop` impl is mostly correct but could be strengthened. Connection point `ComPtr`s are stored as `Option<ComPtr<IConnectionPoint>>` but are disconnected via reference rather than taken. The controller pointer is properly handled (checks if it is the same object as the component before calling terminate).

**Source:** [Steinberg API Documentation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/API+Documentation/Index.html), [JUCE Memory Leak When Hosting VST3](https://forum.juce.com/t/memory-leak-when-hosting-vst3-plugin-instances/35924)

### Pitfall 2: In-Process Scanning Crashes

**What goes wrong:** Loading a buggy plugin during scanning crashes the entire host process. This is especially common with plugins that require UI initialization (Waves, SSL, some iLok-protected plugins) or have bugs in their `BundleEntry`/`InitDll` functions.

**Why it happens:** `dlopen` runs constructor functions in the shared library. `GetPluginFactory` and `IPluginFactory::getClassInfo` execute plugin code. Any of these can crash.

**How to avoid:**
- Scan plugins in a separate child process.
- Use the fast path (moduleinfo.json) whenever possible -- this is pure JSON I/O with zero crash risk.
- For the slow path (binary load), spawn a child process per plugin bundle with a timeout.
- If the child process crashes, mark the plugin as broken and continue scanning.

**Warning signs:** Scanner process dies with no error message. Specific plugins consistently crash the scanner.

**Source:** [JUCE VST3 Plugin Scanning Crash Protection](https://forum.juce.com/t/vst3-plugin-scanning-crash-protection/58485), [Steinberg Forums - Plugin Crash While Scanning](https://forums.steinberg.net/t/vst3-host-plugin-crash-while-scanning-bundleentry-bundleexit/776824)

### Pitfall 3: Missing Bus Activation Before Processing

**What goes wrong:** Calling `process()` without first activating the plugin's audio buses via `activateBus()`. Some plugins produce silence; others crash.

**Why it happens:** The `activateBus()` call is easy to forget because it is separate from `setupProcessing()` and `setActive()`.

**How to avoid:** Activate all default-active buses (those with `kDefaultActive` flag) during the setup phase, after `setupProcessing()` but before `setActive()`.

**Current status:** Already implemented in `PluginInstance::activate_default_buses()`, called from `setup()`. The implementation iterates all bus types (Audio, Event) and directions (Input, Output), checks the `kDefaultActive` flag, and activates matching buses.

### Pitfall 4: Incorrect Controller State Sync

**What goes wrong:** After loading a preset via `IComponent::setState()`, forgetting to also call `IEditController::setComponentState()` with the same data. The controller reports stale parameter values.

**Why it happens:** VST3 requires three calls for state restoration: (1) `component.setState(stream)` -- processor gets its state, (2) `controller.setComponentState(stream)` -- controller syncs to processor state, (3) `controller.setState(stream)` -- controller gets its own state (UI settings, etc.). Missing step 2 means the controller's parameter cache is out of date.

**How to avoid:** Always call `setComponentState()` on the controller after setting processor state. Always seek the stream back to position 0 between calls.

**Current status:** Already implemented in `src/preset/state.rs` (the preset save/load code).

**Source:** [Steinberg Forums - Audio Processor Call Sequence](https://forums.steinberg.net/t/audio-processor-call-sequence/892085), [VST3 API Documentation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/API+Documentation/Index.html)

### Pitfall 5: Module Lifetime Not Enforced

**What goes wrong:** The `VstModule` (which holds the `Library` handle from `dlopen`) is dropped while `PluginInstance` objects created from its factory still exist. This unloads the shared library's code from memory, and subsequent COM method calls or `Release` calls on the plugin's COM pointers segfault.

**Why it happens:** `VstModule` and `PluginInstance` are separate objects with no enforced lifetime relationship. The caller is responsible for keeping the module alive, but this is easy to violate.

**How to avoid:** Store an `Arc<VstModule>` inside each `PluginInstance` so the module cannot be dropped while any instance exists. Alternatively, document the lifetime requirement clearly and test with repeated load/unload cycles under ASAN.

**Current status:** The current code stores `VstModule` and `PluginInstance` separately. The `server.rs` MCP handler stores them in `Arc<Mutex<Option<...>>>` which does not enforce ordering. This is a latent bug that should be fixed by having `PluginInstance` hold a reference to the module.

**Warning signs:** Segfaults after unloading a plugin. Crashes that appear only when loading a second plugin after unloading the first.

## Code Examples

### Out-of-Process Scanner Architecture

```rust
// Scanner binary (separate executable: src/bin/scanner.rs)
// Receives bundle path as argument, outputs PluginInfo JSON to stdout

use std::process;

fn main() {
    let bundle_path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: scanner <bundle_path>");
        process::exit(1);
    });

    // This is the crash-risky part -- isolated in child process
    match scan_bundle_binary(Path::new(&bundle_path)) {
        Ok(plugins) => {
            let json = serde_json::to_string(&plugins).unwrap();
            println!("{}", json);
            process::exit(0);
        }
        Err(e) => {
            eprintln!("scan error: {}", e);
            process::exit(1);
        }
    }
}
```

```rust
// Host-side coordinator (in scanner.rs)

use std::process::Command;
use std::time::Duration;

pub fn scan_bundle_out_of_process(
    scanner_path: &Path,
    bundle_path: &Path,
    timeout: Duration,
) -> Result<Vec<PluginInfo>, HostError> {
    let child = Command::new(scanner_path)
        .arg(bundle_path.as_os_str())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| HostError::ScanError(format!("failed to spawn scanner: {}", e)))?;

    let output = child
        .wait_with_output()
        .map_err(|e| HostError::ScanError(format!("scanner failed: {}", e)))?;

    if !output.status.success() {
        return Err(HostError::ScanError(format!(
            "scanner crashed or failed for {}: exit={:?}, stderr={}",
            bundle_path.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        )));
    }

    let plugins: Vec<PluginInfo> = serde_json::from_slice(&output.stdout)
        .map_err(|e| HostError::ScanError(format!("invalid scanner output: {}", e)))?;

    Ok(plugins)
}
```

### Hardened Drop Implementation

```rust
impl Drop for PluginInstance {
    fn drop(&mut self) {
        // 1. Stop processing if active
        if self.state == PluginState::Processing {
            unsafe { let _ = self.processor.setProcessing(0); }
            self.state = PluginState::Active;
        }

        // 2. Deactivate if active
        if self.state == PluginState::Active {
            unsafe { let _ = self.component.setActive(0); }
            self.state = PluginState::SetupDone;
        }

        // 3. Disconnect connection points (BEFORE terminate)
        if let (Some(ccp), Some(kcp)) = (
            self._comp_connection.take(),  // take() consumes the Option
            self._ctrl_connection.take(),
        ) {
            unsafe {
                let _ = ccp.disconnect(kcp.as_ptr());
                let _ = kcp.disconnect(ccp.as_ptr());
            }
            // ccp and kcp are dropped here, releasing COM references
        }

        // 4. Terminate controller (if separate from component)
        if let Some(ctrl) = self.controller.take() {
            let comp_as_ctrl: Option<ComPtr<IEditController>> = self.component.cast();
            let is_same = comp_as_ctrl.as_ref().is_some_and(|c| {
                std::ptr::eq(c.as_ptr(), ctrl.as_ptr())
            });
            if !is_same {
                unsafe { let _ = ctrl.terminate(); }
            }
            // ctrl is dropped here, releasing COM reference
        }

        // 5. Terminate component
        unsafe { let _ = self.component.terminate(); }
        // component and processor ComPtrs dropped when struct is dropped
    }
}
```

### Enforcing Module Lifetime

```rust
use std::sync::Arc;

pub struct PluginInstance {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    state: PluginState,
    class_id: TUID,
    _host_app: ComWrapper<HostApp>,
    _handler: ComWrapper<ComponentHandler>,
    _comp_connection: Option<ComPtr<IConnectionPoint>>,
    _ctrl_connection: Option<ComPtr<IConnectionPoint>>,
    param_changes: VecDeque<ParameterChange>,
    // Keep module alive as long as this instance exists
    _module: Arc<VstModule>,
}
```

### Verified Lifecycle Test Pattern

```rust
#[test]
fn test_load_unload_cycle() {
    // Repeat to catch teardown ordering bugs
    for i in 0..10 {
        let module = VstModule::load(Path::new("/path/to/test.vst3")).unwrap();
        let host_app = HostApp::new();
        let handler = ComponentHandler::new();

        let mut plugin = PluginInstance::from_factory(
            module.factory(),
            &known_class_id,
            host_app,
            handler,
        ).unwrap();

        assert_eq!(plugin.state(), PluginState::Created);

        plugin.setup(44100.0, 4096).unwrap();
        assert_eq!(plugin.state(), PluginState::SetupDone);

        plugin.activate().unwrap();
        assert_eq!(plugin.state(), PluginState::Active);

        plugin.start_processing().unwrap();
        assert_eq!(plugin.state(), PluginState::Processing);

        // Process a small block of silence
        let input = vec![vec![0.0f32; 512]; 2];
        let mut output = vec![vec![0.0f32; 512]; 2];
        let input_refs: Vec<&[f32]> = input.iter().map(|c| c.as_slice()).collect();
        let mut output_refs: Vec<&mut [f32]> = output.iter_mut().map(|c| c.as_mut_slice()).collect();
        plugin.process(&input_refs, &mut output_refs, 512).unwrap();

        // Explicit teardown (or just drop)
        drop(plugin);
        drop(module);  // module must be dropped AFTER plugin

        eprintln!("cycle {} complete", i);
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| VST3 SDK GPLv3 license | VST3 SDK MIT license | Oct 2025 (SDK 3.8.0) | Removes licensing friction for open-source hosts |
| `vst3-sys` (RustAudio) as only Rust option | `vst3` crate (coupler-rs) 0.3.0 with pre-generated bindings | 2024-2025 | No C++ build dependency, permissive license, cleaner API |
| `vst3` crate incompatible with SDK 3.8.0 | Issue #20 fixed (forward-declared extern types) | Nov 2025 | SDK 3.8.0 now works with coupler-rs bindings |
| Manual moduleinfo parsing | Standardized `moduleinfo.json` in .vst3 bundles | VST3 3.7.4+ | Fast scanning without loading binaries |
| In-process scanning only | Out-of-process scanning standard in all major DAWs | Established practice | Crash isolation for buggy plugins |

**Deprecated/outdated:**
- `vst3-sys` (RustAudio): GPLv3, no versioned crates.io releases, plugin-dev focused. Do not use for hosting alongside nih-plug.
- `vst-rs` (RustAudio): VST2 only, format deprecated by Steinberg.
- `rack` 0.4.8: VST3 on Linux "untested", no GUI. Not ready for production.

## Open Questions

1. **Module lifetime enforcement strategy**
   - What we know: `VstModule` must outlive all `PluginInstance` objects. Current code does not enforce this structurally.
   - What's unclear: Whether `Arc<VstModule>` inside `PluginInstance` introduces any issues with `ExitDll` being called at the wrong time (when the last Arc is dropped inside a Drop impl).
   - Recommendation: Use `Arc<VstModule>` and test that `ExitDll` is called after all COM pointers are released. Verify with ASAN.

2. **Scanner binary packaging**
   - What we know: Out-of-process scanning needs a separate binary. JUCE includes a scanner executable with its plugin.
   - What's unclear: For the current offline MVP (not yet a plugin), the scanner can be a separate cargo binary target. When this becomes a DAW plugin in later phases, the scanner binary needs to be shipped alongside the .vst3 bundle.
   - Recommendation: Add a `[[bin]]` target to Cargo.toml for the scanner. For Phase 1, this is sufficient.

3. **Which plugins to test with**
   - What we know: Need at least one unified component/controller plugin and one split plugin. Need plugins from different vendors to exercise different COM behaviors.
   - What's unclear: Which freely available Linux VST3 plugins provide good coverage of both patterns.
   - Recommendation: Use Surge XT (open source, split component/controller, well-tested) and a simple JUCE-built plugin (typically unified). Check what is installed on the test system.

4. **Connection point disconnect before terminate -- is it strictly required?**
   - What we know: The current code disconnects, which is correct. The Steinberg spec says to disconnect before terminate.
   - What's unclear: Whether all plugins handle missing disconnect gracefully, or whether some crash if disconnect is not called.
   - Recommendation: Always disconnect. The current implementation is correct -- just needs to use `Option::take()` instead of borrowing to ensure proper drop order.

## Sources

### Primary (HIGH confidence)
- [Steinberg VST3 API Documentation - Creation and Initialization](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/API+Documentation/Index.html) -- lifecycle sequence, component/controller relationship
- [Steinberg VST3 Hosting FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Hosting.html) -- unified/split handling, restartComponent flags
- [Steinberg VST3 Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html) -- process call requirements
- [Steinberg Plugin Locations](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Plugin+Locations.html) -- OS-specific scan paths
- [Steinberg Plugin Format](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Plugin+Format.html) -- .vst3 bundle structure
- [Steinberg ModuleInfo JSON](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+Module+Architecture/ModuleInfo-JSON.html) -- fast scan format
- [coupler-rs/vst3-rs GitHub](https://github.com/coupler-rs/vst3-rs) -- Rust VST3 bindings, ComPtr/ComWrapper API
- [vst3 crate API docs](https://coupler.rs/vst3-rs/vst3/) -- ComPtr, ComRef, Class trait

### Secondary (MEDIUM confidence)
- [JUCE VST3 Plugin Scanning Crash Protection](https://forum.juce.com/t/vst3-plugin-scanning-crash-protection/58485) -- out-of-process scanning architecture, heartbeat mechanism
- [Steinberg Forums - Plugin Crash While Scanning](https://forums.steinberg.net/t/vst3-host-plugin-crash-while-scanning-bundleentry-bundleexit/776824) -- "init plugins on main thread", use helper app for scanning
- [Steinberg Forums - Audio Processor Call Sequence](https://forums.steinberg.net/t/audio-processor-call-sequence/892085) -- setState/setComponentState ordering
- [Steinberg Forums - Single Component Question](https://forums.steinberg.net/t/vst-3-single-component-process-and-controller-question/201967) -- unified vs split patterns
- [JUCE Memory Leak When Hosting VST3](https://forum.juce.com/t/memory-leak-when-hosting-vst3-plugin-instances/35924) -- COM pointer leak patterns
- [Surge Synthesizer Issue #164 - Split Processor vs Controller](https://github.com/surge-synthesizer/surge/issues/164) -- real-world split component discussion
- Existing codebase: `src/hosting/*.rs` -- working implementation, validated against real plugins

### Tertiary (LOW confidence)
- [KVR Forum - CLI VST3 Host in Rust](https://www.kvraudio.com/forum/viewtopic.php?t=622780) -- community experience (could not fetch, 403)
- [Renaud Denis - Robust VST3 Host for Rust](https://renauddenis.com/case-studies/rust-vst) -- cutoff-vst case study (proprietary)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all crates verified, already in use in codebase, versions confirmed
- Architecture: HIGH -- existing implementation is correct and tested against real plugins
- Teardown/lifecycle: HIGH -- Steinberg spec is clear, current implementation follows it, hardening improvements are well-understood
- Out-of-process scanning: MEDIUM -- pattern is well-established (JUCE, every major DAW), but Rust-specific implementation is straightforward (`std::process::Command`)
- Pitfalls: HIGH -- documented across Steinberg docs, JUCE forums, and validated against existing codebase

**Research date:** 2026-02-15
**Valid until:** 2026-03-15 (30 days -- stack is stable, vst3 crate releases are infrequent)
