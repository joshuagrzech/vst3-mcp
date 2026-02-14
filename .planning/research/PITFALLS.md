# Domain Pitfalls

**Domain:** Headless VST3 Host in Rust
**Researched:** 2026-02-14

## Critical Pitfalls

Mistakes that cause rewrites or major issues.

### Pitfall 1: VST3 COM Lifecycle Mismanagement

**What goes wrong:** VST3 plugins are COM objects with reference counting (AddRef/Release). Missing a Release leaks the plugin. Double-Release causes use-after-free. Incorrect initialization order causes segfaults.
**Why it happens:** Rust's ownership model does not map cleanly to COM reference counting. The vst3 crate provides raw COM pointers, not RAII wrappers.
**Consequences:** Memory leaks, crashes, undefined behavior. Particularly insidious because it may work with some plugins and crash with others.
**Prevention:** Build RAII wrappers (VstPtr<T>) at Layer 2 that call AddRef on clone and Release on drop. Never expose raw COM pointers above Layer 2. Test with multiple different plugins -- each vendor implements COM differently.
**Detection:** Valgrind/AddressSanitizer on worker processes. Monitor plugin memory usage over time. Track refcount mismatches in debug builds.

### Pitfall 2: Plugin Thread Affinity Violations

**What goes wrong:** Calling VST3 methods from the wrong thread. The VST3 spec defines separate threading contexts: "UI thread" for IEditController, "audio thread" for IAudioProcessor, and "main thread" for initialization.
**Why it happens:** In a headless host without a GUI, it is tempting to call everything from one thread. Some plugins internally assert thread affinity or use thread-local storage.
**Consequences:** Crashes, deadlocks, or silent corruption in specific plugins. Works fine in testing with simple plugins, fails in production with complex ones.
**Prevention:** Even in headless mode, respect the threading model. Initialize on the main thread, process audio on a dedicated audio thread, never mix. Use Rust's type system to enforce this (make processors !Send or use marker types).
**Detection:** TSAN (ThreadSanitizer). Plugin-specific test suite across multiple vendors.

### Pitfall 3: Plugin Segfaults Killing the Host

**What goes wrong:** A buggy VST3 plugin dereferences null, accesses freed memory, or panics. In a single-process architecture, this kills the entire host and the MCP connection.
**Why it happens:** VST3 plugins are arbitrary native code. You cannot trust them.
**Consequences:** AI agent loses connection mid-conversation. Unrecoverable state loss.
**Prevention:** Multi-process architecture from Phase 2 onward. Each plugin (or chain) runs in a worker process. Supervisor detects worker crash and reports error to AI agent gracefully.
**Detection:** Worker process exit codes, signal handlers (SIGSEGV, SIGABRT).

### Pitfall 4: Blocking the Tokio Runtime with Audio Work

**What goes wrong:** Running CPU-intensive audio processing (symphonia decode, plugin process calls) on tokio async worker threads.
**Why it happens:** Natural to await everything in an async MCP handler. Audio processing is CPU-bound, not I/O-bound.
**Consequences:** All other MCP requests stall while audio processes. Connection timeouts. Terrible latency.
**Prevention:** Always use `tokio::task::spawn_blocking` for audio work in single-process mode. In multi-process mode, the worker handles audio on its own thread -- the supervisor just awaits IPC responses.
**Detection:** tokio-console for runtime diagnostics. Monitor MCP response latencies.

## Moderate Pitfalls

### Pitfall 5: Incorrect Audio Buffer Layout

**What goes wrong:** VST3 uses non-interleaved (planar) audio buffers -- one float array per channel. Many Rust audio crates use interleaved layout. Mixing them up produces garbage audio.
**Prevention:** Build explicit interleave/deinterleave functions. Unit test with known audio signals (sine wave in, sine wave out).

### Pitfall 6: Sample Rate / Block Size Mismatch

**What goes wrong:** Initializing the plugin at 44100 Hz but feeding it audio decoded at 48000 Hz. Or processing blocks larger than what the plugin declared as its maximum.
**Prevention:** Query the input file's sample rate from symphonia. Initialize the plugin with the matching sample rate. Respect the plugin's getLatencySamples() and maxBlockSize. Resample if needed (consider the `rubato` crate).

### Pitfall 7: Forgetting to Activate Processing

**What goes wrong:** Calling IAudioProcessor::process() without first calling setProcessing(true) and setActive(true). Some plugins produce silence, others crash.
**Prevention:** Encode the activation state machine in Rust types:

```rust
enum PluginState {
    Created,               // After IComponent::initialize
    SetupDone,             // After setupProcessing + activateBus
    Active,                // After setActive(true)
    Processing,            // After setProcessing(true) -- ready to process
}
```

Only allow `process()` calls when in `Processing` state.

### Pitfall 8: Platform-Specific Plugin Paths

**What goes wrong:** Hardcoding plugin scan paths for one OS. VST3 plugins live in different locations per platform.
**Prevention:**
- Linux: `~/.vst3/`, `/usr/lib/vst3/`, `/usr/local/lib/vst3/`
- macOS: `~/Library/Audio/Plug-Ins/VST3/`, `/Library/Audio/Plug-Ins/VST3/`
- Windows: `C:\Program Files\Common Files\VST3\`

Use a platform detection module, not cfg-gated string literals scattered through code.

### Pitfall 9: MCP Tool Timeout on Long Renders

**What goes wrong:** Rendering a 10-minute audio file through a complex plugin chain takes minutes. The MCP client times out waiting for the tool response.
**Prevention:** Use MCP progress notifications (if supported by rmcp) or implement a two-phase pattern: start_render returns a job ID, check_render polls for completion. The rmcp crate implements task lifecycle from SEP-1686 for long-running operations.

## Minor Pitfalls

### Pitfall 10: Plugin State Serialization Incompatibilities

**What goes wrong:** Saving plugin state with getState() and restoring with setState() across different plugin versions or platforms produces unexpected behavior.
**Prevention:** Store plugin version metadata alongside state data. Warn users about cross-version state restoration.

### Pitfall 11: Denormalized Float Values in Audio Buffers

**What goes wrong:** Very small float values (denormals) near zero cause massive CPU spikes in some plugins' internal processing.
**Prevention:** Flush denormals to zero before passing buffers to plugins. Use platform-specific FPU control (DAZ/FTZ flags).

```rust
// Set DAZ+FTZ on x86
#[cfg(target_arch = "x86_64")]
fn set_flush_denormals() {
    unsafe {
        std::arch::x86_64::_mm_setcsr(
            std::arch::x86_64::_mm_getcsr() | 0x8040
        );
    }
}
```

### Pitfall 12: Shared Memory Cleanup on Crash

**What goes wrong:** Worker process crashes without cleaning up shared memory segments. Segments accumulate over time.
**Prevention:** Supervisor owns shared memory lifecycle. Use RAII or explicit cleanup when worker exit is detected. On Linux, memfd-based shared memory (anonymous) is auto-cleaned on process exit.

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| VST3 bindings integration | COM lifecycle, SDK 3.8.0 compat issues | Start with a single well-known plugin (e.g., Surge XT, open source). Validate COM patterns early. |
| Audio pipeline | Buffer layout, sample rate mismatch | Unit tests with known signals. Test deinterleave round-trip. |
| MCP server | Blocking async runtime with audio | spawn_blocking from day one. Never process audio on tokio threads. |
| Multi-process IPC | Shared memory cleanup, protocol versioning | Supervisor owns all shared memory. Version the wire protocol. |
| Plugin scanning | Platform paths, slow scans | Cache scan results. Background scanning. |
| Parameter control | Thread affinity for parameter changes | Route parameter changes through the correct thread context. |
| Batch rendering | Memory pressure from many open files | Stream input/output, do not load entire files into memory. |

## Sources

- [KVR Forum: segfaults with VST3 instruments in Rust host](https://www.kvraudio.com/forum/viewtopic.php?t=622780)
- [JUCE Forum: headless VST3 host rendering silence](https://forum.juce.com/t/headless-vst3-host-some-plugins-render-silence/58169)
- [VST3 SDK threading model documentation](https://steinbergmedia.github.io/vst3_dev_portal/)
- [Rust FFI safety patterns](https://rust-unofficial.github.io/patterns/patterns/ffi/intro.html)
- [cutoff-vst architecture](https://renauddenis.com/case-studies/rust-vst) -- safe abstraction approach
