# Pitfalls Research

**Domain:** Real-time VST3 plugin hosting with async MCP server (Rust)
**Researched:** 2026-02-15
**Confidence:** HIGH (core audio/threading rules are well-established; project-specific risks verified against codebase)

## Critical Pitfalls

### Pitfall 1: Memory Allocation on the Audio Thread

**What goes wrong:**
Calling `malloc`, `free`, `Vec::push` (which may reallocate), `String::format`, `Box::new`, or any allocating operation inside the real-time audio callback causes unpredictable latency spikes. The allocator may need to acquire a global lock, request memory from the OS (a syscall), or trigger garbage collection in the child plugin. A single allocation in a 2ms buffer window can cause an audible glitch.

**Why it happens:**
Rust makes allocation invisible. `Vec::collect()`, `.to_string()`, `format!()`, `Box::new()`, closures that capture by value, and even error handling with `anyhow::Context` all allocate. The current `process()` method in `plugin.rs` creates `Vec<*mut f32>` channel pointer arrays on every call (lines 371-374, 388-391), which allocates on the heap each invocation.

**How to avoid:**
- Pre-allocate all buffers during `setup()` and reuse them across `process()` calls. Store channel pointer arrays as fields on `PluginInstance`.
- Enable nih-plug's `assert_process_allocs` feature during debug builds to catch allocations at runtime.
- Use `#[global_allocator]` with a tracing allocator in test builds to detect allocations in the audio path.
- Never use `format!()`, `String`, `Vec::push`, `Box::new`, or `anyhow` error types in the process callback.
- Return error codes (integers) rather than `Result<_, String>` from the inner process loop.

**Warning signs:**
- Intermittent audio glitches that appear under load but not in isolation.
- Glitches that worsen when system memory is fragmented or under pressure.
- `assert_process_allocs` panics in debug builds.

**Phase to address:**
Phase 1 (audio pipeline). The current offline-only code allocates freely in `render_offline()` and `process()`. This must be fixed before any real-time path is added. For offline-only processing, allocations are tolerable but should still be eliminated from the inner `process()` call to establish correct patterns early.

---

### Pitfall 2: Mutex on the Audio Thread (Priority Inversion)

**What goes wrong:**
Using `std::sync::Mutex` (or any OS mutex) in the audio callback causes priority inversion. The audio thread runs at elevated priority. If a lower-priority thread (GUI, MCP server) holds the mutex, the audio thread blocks waiting for a thread that the OS scheduler may deprioritize. The audio thread stalls for milliseconds or longer, causing buffer underruns and audible dropouts.

**Why it happens:**
The current `AudioHost` in `server.rs` wraps `PluginInstance` in `Arc<Mutex<Option<PluginInstance>>>` (line 73). Any MCP tool call locks this mutex. If real-time processing is later added on a separate thread, the MCP server and audio thread will contend on the same mutex. Even `try_lock()` is not safe because `unlock()` (in the `MutexGuard` destructor) may trigger a syscall to wake a waiting thread.

**How to avoid:**
- Never use `std::sync::Mutex`, `RwLock`, or `Condvar` on the audio thread.
- Use lock-free communication: SPSC ring buffers (`ringbuf` crate or `crossbeam::channel` bounded) for sending commands/parameter changes to the audio thread.
- Use `std::sync::atomic` for single-value state flags and parameter values.
- For shared data structures, use the "triple buffer" or "SeqLock" pattern: the audio thread always reads the latest snapshot without blocking.
- The MCP server should send parameter changes into a lock-free queue; the audio thread drains the queue at the start of each `process()` call.

**Warning signs:**
- Audio dropouts that correlate with MCP server activity or GUI interactions.
- High "audio thread wakeup latency" in profiler traces.
- Deadlocks when the MCP server and audio thread both try to lock the plugin.

**Phase to address:**
Phase 2 (real-time audio). The current Phase 1 offline architecture can use mutexes safely because there is no real-time thread. But the architecture must be redesigned before adding real-time processing. Plan the lock-free communication layer before implementing the real-time path.

---

### Pitfall 3: VST3 COM Pointer Lifetime and Drop Order

**What goes wrong:**
VST3 plugins are COM objects with reference-counted lifetimes. If the host drops COM pointers in the wrong order, or drops the module (shared library) before releasing all COM pointers, the result is use-after-free or segfault. Similarly, calling `terminate()` on a component before releasing all interface pointers obtained from it leads to dangling pointers.

**Why it happens:**
Rust's `Drop` order is deterministic (reverse declaration order in structs), but COM pointers obtained via `QueryInterface`/`cast()` create additional reference counts that must be released before `terminate()`. The current `PluginInstance::drop()` correctly sequences teardown (stop processing, deactivate, disconnect, terminate), but it relies on implicit `ComPtr::drop()` happening after `terminate()` returns, which may not release the last reference correctly for all plugins.

**How to avoid:**
- The teardown order must be: `setProcessing(false)` -> `setActive(false)` -> disconnect connection points -> release controller COM pointers -> `controller.terminate()` -> release component COM pointers -> `component.terminate()` -> drop module.
- Explicitly drop connection point ComPtrs before calling terminate (use `Option::take()` to consume them).
- Keep the `VstModule` alive strictly longer than all `PluginInstance` objects. The current code stores module in a separate `Arc<Mutex<Option<VstModule>>>` which could be dropped independently -- this is a latent bug if drop order is not enforced.
- Use `ComWrapper` no-op reference counting for host-side objects (HostApp, ComponentHandler) that outlive all plugins, as recommended by the VST3 SDK.

**Warning signs:**
- Segfaults during plugin unload, especially with certain plugin brands.
- Double-free or heap corruption detected by address sanitizer.
- Plugins that work in one DAW but crash in yours (different hosts have different drop ordering).

**Phase to address:**
Phase 1 (plugin hosting). The current `Drop` impl needs hardening. Explicitly take and drop COM pointers in the correct order rather than relying on implicit struct field drop order.

---

### Pitfall 4: Blocking the Audio Thread with Tokio

**What goes wrong:**
Running `tokio::runtime::Runtime::block_on()` or any Tokio future from the audio thread blocks it. Even `tokio::sync::mpsc::Sender::send()` can block if the channel is full. The Tokio runtime's internal task scheduler uses mutexes and condition variables, so any interaction with it from a real-time thread is unsafe.

**Why it happens:**
The project requires both a Tokio runtime (for the MCP server via `rmcp`) and a real-time audio thread. Developers instinctively reach for `tokio::sync` channels to bridge them, not realizing these channels use OS synchronization primitives internally.

**How to avoid:**
- Run the Tokio runtime on a completely separate thread. Never call any Tokio API from the audio thread.
- Use non-Tokio, lock-free channels for audio-thread communication: `ringbuf::HeapRb` (SPSC), `crossbeam::channel::bounded` (MPMC, but uses CAS not locks), or a custom atomic ring buffer.
- The audio thread should only read from and write to lock-free data structures. The MCP/Tokio side writes commands; the audio thread reads them.
- If the MCP server needs audio thread results (e.g., metering data), the audio thread writes to a lock-free queue, and a Tokio task polls it.
- Never call `block_on()` from inside an async context either -- Tokio will panic ("Cannot start a runtime from within a runtime").

**Warning signs:**
- Audio glitches during MCP server requests.
- The audio thread showing up blocked in `futex_wait` or `pthread_cond_wait` in stack traces.
- Tokio panics about nested runtimes.

**Phase to address:**
Phase 2 (MCP + real-time integration). Design the bridge layer between Tokio and the audio thread early. This is the highest-risk architectural decision in the project.

---

### Pitfall 5: VST3 Threading Model Violations

**What goes wrong:**
The VST3 specification mandates strict threading rules: `IAudioProcessor::process()` must only be called from the audio thread. `IEditController` methods (parameter queries, GUI operations) must only be called from the "UI thread" (main thread in most hosts). `restartComponent()` must be called from the UI thread. Violating these rules causes data races in the plugin, leading to crashes, corrupted state, or silent wrong behavior.

**Why it happens:**
The MCP server runs on a Tokio worker thread. When the server calls `get_parameter_info()` or `get_parameter()` (which call `IEditController` methods), it does so from whatever thread the Tokio executor assigns. Meanwhile, if real-time processing is happening, `process()` is called from the audio thread. Many plugins use shared mutable state between their processor and controller without synchronization, relying on the host to enforce the threading contract.

**How to avoid:**
- Designate one thread as the "controller thread" and route all `IEditController` calls through it. In a GUI application, this would be the main/UI thread. In a headless MCP server, create a dedicated thread for controller operations.
- Never call `IEditController` methods from the audio thread or from arbitrary Tokio worker threads.
- Use a command queue pattern: MCP server enqueues "get parameter" requests, the controller thread processes them and sends results back.
- Document which COM interfaces are safe to call from which threads.

**Warning signs:**
- Sporadic crashes in plugin code during parameter queries.
- Data races detected by ThreadSanitizer (TSAN).
- Parameters reading stale or corrupted values.
- Plugins that "mostly work" but occasionally produce wrong output.

**Phase to address:**
Phase 2 (real-time + parameter automation). Must be designed into the threading architecture from the start. Retrofitting correct threading is extremely expensive.

---

### Pitfall 6: Plugin Crashes Taking Down the Host

**What goes wrong:**
A buggy child plugin can segfault, abort, or throw an unhandled C++ exception during any COM method call (`process()`, `setState()`, `initialize()`). Since the plugin runs in-process, its crash kills the entire host process. This is especially common during plugin scanning (loading unknown plugins from disk).

**Why it happens:**
VST3 plugins are shared libraries loaded via `dlopen`/`LoadLibrary`. They run in the same address space as the host. There is no isolation. Plugins from different vendors have varying quality levels, and some have bugs triggered only by specific host behaviors.

**How to avoid:**
- **Plugin scanning:** Scan plugins in a separate child process. If the child crashes, the host survives and marks that plugin as broken. This is what every major DAW does (Bitwig, REAPER, Ableton).
- **Runtime protection:** Wrap unsafe COM calls in `std::panic::catch_unwind()` where possible (catches Rust panics, not C++ exceptions or segfaults). For C++ exceptions, use signal handlers (`SIGSEGV`, `SIGABRT`) to detect crashes, though recovery is unreliable.
- **State validation:** Validate all data returned from plugin calls (null checks, bounds checks on sample counts, channel counts).
- Accept that some crashes are unrecoverable. Document which plugins are known to be problematic.

**Warning signs:**
- Crashes during `scan_plugins` with no error message.
- Segfaults in plugin code visible in stack traces.
- Plugins that work in other hosts but crash in yours (host behavior difference triggers a plugin bug).

**Phase to address:**
Phase 1 (scanner hardening) and Phase 3 (runtime resilience). Out-of-process scanning should be added early. Runtime crash isolation is a later hardening concern.

---

### Pitfall 7: Incorrect IParameterChanges Implementation

**What goes wrong:**
The VST3 spec requires parameter changes to be delivered through `IParameterChanges` in the `ProcessData` struct. The current code passes `null` for `inputParameterChanges` (line 421 in `plugin.rs`). Without a proper `IParameterChanges` implementation, the host cannot automate plugin parameters during processing, and some plugins may behave incorrectly or ignore parameter changes entirely.

**Why it happens:**
`IParameterChanges` and `IParamValueQueue` are COM interfaces that the host must implement. They have a non-trivial API: the host provides an `IParameterChanges` object containing one `IParamValueQueue` per changed parameter, each queue containing sample-accurate (offset, value) pairs. Getting this wrong means parameters jump discontinuously, zip, or are ignored.

**How to avoid:**
- Implement `IParameterChanges` and `IParamValueQueue` as COM objects on the host side.
- Pre-allocate the parameter change objects during setup (fixed-size arrays, no allocation during process).
- Deliver parameter changes at sample-accurate offsets within the buffer for smooth automation.
- After each process call, clear the change queues without deallocating (reset length to zero, keep capacity).
- Test with plugins that rely heavily on automation (synths with modulated parameters, compressors with sidechain).

**Warning signs:**
- Plugin parameters do not respond to automation.
- Zipper noise (audible stepping) during parameter sweeps.
- Plugins behaving as if parameters are always at default values.

**Phase to address:**
Phase 2 (parameter automation). This is deferred in Phase 1 (the TODO on line 428-431 acknowledges it), but must be implemented before real-time parameter control.

---

## Technical Debt Patterns

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| `Arc<Mutex<PluginInstance>>` for shared access | Simple, works for offline | Prevents real-time usage; must be redesigned for RT path | Phase 1 offline only |
| Allocating `Vec` in `process()` per call | Simpler code, no state management | Allocation per audio block; prevents RT safety | Phase 1 offline only; fix before Phase 2 |
| Passing `null` for `IParameterChanges` | Avoids complex COM impl | No parameter automation; some plugins may misbehave | Phase 1 only |
| `UnsafeCell` in `VecStream` with "single-threaded" comment | Avoids `RefCell` overhead | Unsound if ever called from multiple threads | Acceptable if stream is truly single-use per operation |
| Logging (`tracing::debug!`) in process path | Useful debugging | Allocates strings; not RT-safe | Never in release RT builds; use `nih_dbg!` pattern |
| Single-process plugin scanning | Simpler architecture | One buggy plugin crashes the entire host | Phase 1 prototype only |
| No `ProcessContext` (null on line 425) | Avoids implementing transport info | Plugins that need tempo/position info will malfunction | Acceptable until transport sync is needed |

## Integration Gotchas

| Integration | Common Mistake | Correct Approach |
|-------------|----------------|------------------|
| VST3 `vst3` crate (Rust bindings) | Assuming `ComPtr::from_raw()` adds a reference | `from_raw` takes ownership of an existing reference. Double-release if you also manually release. Let `ComPtr::drop()` handle it. |
| VST3 `IEditController::setComponentState()` | Forgetting to call it after `IComponent::setState()` | Always sync state to controller after setting processor state. Some plugins store all state in the processor; others split it. |
| Tokio + rmcp | Calling `Runtime::block_on()` from within an async context | Use `tokio::spawn()` for nested async work. For sync-to-async bridging, use `Handle::spawn()` from a non-async thread. |
| `libloading` for VST3 modules | Dropping the `Library` before all symbols are released | Keep the module handle alive as long as any COM pointer from its factory exists. The current `VstModule` must outlive `PluginInstance`. |
| Child plugin GUI (IPlugView) on Linux | Creating X11 windows on the wrong thread | All X11/Wayland window operations must happen on the same thread. Use the main thread or a dedicated GUI thread. Never from Tokio workers. |
| VST3 `restartComponent` callback | Handling it on whatever thread the plugin calls from | Queue the request and handle it on the UI/controller thread. Plugins may call this from the audio thread (spec violation, but real plugins do it). |

## Performance Traps

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| Denormal floating-point numbers in audio buffers | CPU usage spikes to 100% on certain plugins, especially reverbs and filters with near-zero signals | Flush denormals to zero using `_MM_SET_FLUSH_ZERO_MODE(_MM_FLUSH_ZERO_ON)` at audio thread start. nih-plug handles this automatically for plugins, but as a host you must do it yourself. | Any time a plugin processes near-silence through IIR filters |
| Excessive tail processing | Output files are 30+ seconds longer than input; wasted CPU on silence | Check output for silence during tail processing and truncate early. Use the plugin's reported tail length as a maximum, not a target. | Plugins reporting `kInfiniteTail` (current `MAX_TAIL_SECONDS = 30.0` is reasonable but should be configurable) |
| Per-block allocation in process loop | Latency spikes proportional to block count; GC pressure | Pre-allocate all buffers. The current `render_offline()` allocates `Vec<&[f32]>` and `Vec<&mut [f32]>` per block (lines 71-80, 97-105). | Thousands of small blocks (small buffer sizes with long files) |
| String formatting in audio path | Unpredictable latency from allocation + UTF-8 encoding | Use integer error codes, not string errors, in the process inner loop. `anyhow::Context` allocates. | Any real-time path |
| Unnecessary sample rate conversion on re-setup | Audible artifacts from repeated setup/teardown cycles | Only call `re_setup()` when sample rate actually changes (compare current vs. requested). | When processing many files at different sample rates |

## Security Mistakes

| Mistake | Risk | Prevention |
|---------|------|------------|
| Loading arbitrary `.vst3` bundles from user-specified paths without validation | Malicious shared libraries execute arbitrary code on load (`dlopen` runs constructor functions) | Only load from known VST3 directories. Validate bundle structure before loading. Consider sandboxing the scanner process. |
| Trusting plugin-reported buffer sizes and channel counts | Buffer overflow if a plugin lies about its bus configuration | Validate all values returned from `getBusInfo()`. Clamp channel counts to reasonable maximums. |
| Passing user-controlled file paths directly to plugin `setState` | Path traversal if plugin writes state to arbitrary locations | Validate and sandbox file paths. Use the `VecStream` approach (in-memory streams) rather than file-backed streams. |
| MCP server exposing plugin control without authentication | Unauthorized audio processing, plugin state modification | Add authentication to the MCP server if exposed beyond localhost. Current stdio transport is safe; network transport would need auth. |

## UX Pitfalls

| Pitfall | User Impact | Better Approach |
|---------|-------------|-----------------|
| No feedback during long audio processing | User thinks the tool is hung | Report progress (percentage or frames processed) via MCP progress notifications |
| Silent failure when plugin does not support sample rate | Output is empty or corrupted with no error message | Check `setupProcessing` return value and report unsupported sample rate clearly |
| Plugin UID is opaque hex string | Users cannot identify which plugin to load | Include human-readable name alongside UID in scan results (already done, but ensure UID is always paired with name in error messages) |
| No preset preview or parameter listing | User cannot understand what the plugin does before processing | Expose a `list_parameters` tool that shows all parameters with names, ranges, and current values |
| Tail processing with no timeout | Some plugins report infinite tail; processing hangs | Cap tail processing at a configurable maximum (current 30s is good). Inform the user when tail was truncated. |

## "Looks Done But Isn't" Checklist

- [ ] **Plugin loading:** Often missing `setComponentState()` call to sync controller -- verify both component and controller receive state on preset load
- [ ] **Audio processing:** Often missing denormal flushing on the host side -- verify `MXCSR` register is set before calling `process()`
- [ ] **Plugin teardown:** Often missing connection point disconnect -- verify `IConnectionPoint::disconnect()` is called before `terminate()`
- [ ] **Parameter automation:** Often missing sample-accurate offsets -- verify parameter changes include buffer-position offsets, not just final values
- [ ] **Bus activation:** Often missing auxiliary bus handling -- verify all default-active buses are activated, and optional buses are handled
- [ ] **State save/restore:** Often missing controller state -- verify both `IComponent::getState()` and `IEditController::getState()` are saved separately
- [ ] **Error handling:** Often missing COM result code checking -- verify every unsafe COM call checks its return value
- [ ] **Module lifetime:** Often missing explicit ordering -- verify `VstModule` is dropped strictly after all `PluginInstance` objects

## Recovery Strategies

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| Audio thread allocation | LOW | Pre-allocate buffers; store as struct fields. Mechanical refactor. |
| Mutex on audio thread | HIGH | Requires architectural redesign: lock-free queues, triple buffers, command pattern. Plan before building. |
| COM drop order bugs | MEDIUM | Add explicit `Option::take()` and manual drop calls in `Drop` impl. Test with ASAN. |
| Tokio blocking audio | MEDIUM | Introduce lock-free bridge layer. Requires new data structures but not a full rewrite. |
| Threading model violation | HIGH | Requires dedicated controller thread and command routing. Architectural change. |
| Plugin crash taking down host | HIGH | Out-of-process scanning requires IPC layer. Runtime isolation is even harder (process sandboxing). |
| Missing IParameterChanges | MEDIUM | Implement COM interfaces. Tedious but well-defined. Pre-allocate queues. |

## Pitfall-to-Phase Mapping

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| Audio thread allocation | Phase 1 (establish patterns) | Enable `assert_process_allocs` in debug; no panics during offline render |
| Mutex on audio thread | Phase 2 (real-time architecture) | Audio thread has zero mutex acquisitions (verify with TSAN or `strace`) |
| COM drop order | Phase 1 (plugin hosting) | Run with AddressSanitizer; zero use-after-free reports on load/unload cycles |
| Tokio blocking audio | Phase 2 (MCP + RT bridge) | Audio callback latency stays under 1ms with concurrent MCP requests |
| Threading model violations | Phase 2 (threading architecture) | ThreadSanitizer clean run with concurrent parameter + process operations |
| Plugin crashes | Phase 1 (scanner) / Phase 3 (runtime) | Scanner survives loading a known-crashy plugin; main process stays alive |
| IParameterChanges | Phase 2 (parameter automation) | Parameter sweeps produce smooth, zipper-free automation in test plugins |
| Denormal flushing | Phase 1 (audio pipeline) | CPU usage stays flat when processing near-silence through IIR filter plugins |
| IPlugView/GUI on Linux | Phase 3 (GUI integration) | Plugin GUI opens and closes without X11 errors on i3wm, GNOME, KDE |
| Graceful Tokio shutdown | Phase 2 (MCP lifecycle) | No panics or hangs when host unloads while MCP server has active connections |

## Sources

- [Using locks in real-time audio processing, safely -- timur.audio](https://timur.audio/using-locks-in-real-time-audio-processing-safely) -- HIGH confidence (authoritative article on RT audio locking, widely referenced)
- [VST 3 Developer Portal: Hosting FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Hosting.html) -- HIGH confidence (official Steinberg documentation)
- [VST 3 API: IEditController](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IEditController.html) -- HIGH confidence (official API reference)
- [VST 3 API: IComponentHandler](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IComponentHandler.html) -- HIGH confidence (official API reference)
- [nih-plug GitHub: assert_process_allocs feature](https://github.com/robbert-vdh/nih-plug) -- HIGH confidence (official nih-plug documentation)
- [Basedrop: garbage collector for real-time audio in Rust](https://micahrj.github.io/posts/basedrop/) -- MEDIUM confidence (community article, technically sound)
- [Tokio: Graceful Shutdown](https://tokio.rs/tokio/topics/shutdown) -- HIGH confidence (official Tokio documentation)
- [Tokio: Bridging with sync code](https://tokio.rs/tokio/topics/bridging) -- HIGH confidence (official Tokio documentation)
- [VST3 + Linux + X11 crash (Dplug issue #434)](https://github.com/AuburnSounds/Dplug/issues/434) -- MEDIUM confidence (real-world bug report)
- [VST3 crashing due to IEditController thread issues (JUCE forum)](https://forum.juce.com/t/vst3-crashing-due-to-ieditcontroller-thread-issues/31168) -- MEDIUM confidence (real-world bug report)
- [The plugin API is unsound due to multi-threading (vst-rs issue #49)](https://github.com/RustAudio/vst-rs/issues/49) -- MEDIUM confidence (documents threading unsoundness in Rust VST bindings)
- [Four common mistakes in audio development -- A Tasty Pixel](https://atastypixel.com/four-common-mistakes-in-audio-development/) -- MEDIUM confidence (practitioner article)
- [Rust's Hidden Dangers: Unsafe, Embedded, and FFI Risks](https://www.trust-in-soft.com/resources/blogs/rusts-hidden-dangers-unsafe-embedded-and-ffi-risks) -- MEDIUM confidence (general FFI safety article)

---
*Pitfalls research for: AgentAudio -- Real-time VST3 plugin hosting with async MCP server*
*Researched: 2026-02-15*
