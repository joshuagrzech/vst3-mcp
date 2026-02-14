# Architecture Research: VST3 Multi-Process Host with Crash Isolation

**Domain:** Headless VST3 Plugin Hosting (Multi-Process, Offline Rendering)
**Researched:** 2026-02-14
**Overall Confidence:** HIGH (patterns well-established across multiple commercial implementations)

---

## 1. Process Isolation Patterns in Commercial DAWs

### 1.1 The Spectrum of Isolation

Commercial DAWs and plugin hosts implement crash isolation along a spectrum from no isolation to full per-instance process isolation. The key implementations surveyed:

| Product | Model | Isolation Level | Recovery |
|---------|-------|-----------------|----------|
| **Bitwig Studio** | Configurable sandbox modes | None / Grouped / Per-manufacturer / Per-plugin / Per-instance | Crash contained to sandbox; audio engine continues |
| **REAPER** | Optional bridging | Per-plugin or grouped | Bridged plugins can crash independently |
| **Ardour** | In-process only | None | No crash isolation (deliberate choice) |
| **AudioGridder** | Client-server over network | Per-chain or per-plugin | Auto-reconnect, state restoration |
| **Fourier transform.engine** | Per-channel sandbox | Per-channel | Auto-reboot crashed channel in ~15 seconds |
| **yabridge** | Wine bridge processes | Per-plugin or grouped | Host process crash doesn't affect DAW |

**Confidence: HIGH** -- sourced from official documentation and architecture docs for each product.

### 1.2 Bitwig's Five-Mode Model

Bitwig offers the most granular configurable isolation, worth studying as a reference:

1. **Within Bitwig** -- plugins load in-process with audio engine. Minimal overhead, maximum risk.
2. **Together** -- all plugins in one separate process. Audio engine survives plugin crashes but all plugins go down together.
3. **By Manufacturer** -- separate process per manufacturer. Allows inter-plugin communication within a manufacturer's suite.
4. **By Plugin** -- all instances of the same plugin share a process. Reduces memory vs. full isolation.
5. **Individually** -- every plugin instance gets its own process. Maximum isolation, maximum memory cost.

For our use case (single-plugin-at-a-time offline processing), **option 5 is effectively what we get for free** -- each worker process hosts exactly one plugin instance.

### 1.3 Ardour's Counter-Argument (and Why It Doesn't Apply)

Ardour deliberately runs plugins in-process, arguing that out-of-process hosting is impractical for real-time low-latency work. Their analysis (from [ardour.org/plugins-in-process.html](https://ardour.org/plugins-in-process.html)):

- Context switch cost: ~3 microseconds fixed + 10-300 microseconds variable (average ~30 microseconds)
- At 48kHz/64-sample buffers (1.3ms processing window), 384 plugins need 768 context switches = 7.7-23ms overhead -- far exceeding the buffer window
- Viable only at buffer sizes of 700-2000 samples (14-40ms latency)

**Why this doesn't apply to us:** We are doing offline rendering with no real-time constraint. We can use buffer sizes of 4096-65536 samples. Context switch overhead becomes negligible relative to processing time. The entire Ardour argument is predicated on real-time constraints we explicitly don't have.

**Confidence: HIGH** -- Ardour's numbers are published and well-analyzed.

---

## 2. IPC Mechanisms for Audio Transfer

### 2.1 Mechanism Comparison

| Mechanism | Latency | Throughput | Complexity | Used By |
|-----------|---------|------------|------------|---------|
| **POSIX shared memory + ring buffer** | ~1-3 microseconds | Highest (zero-copy) | Medium | yabridge (audio path), iceoryx |
| **Unix domain sockets** | ~10-30 microseconds | High | Low | AudioGridder (local mode), yabridge (control) |
| **Named pipes** | ~10-50 microseconds | Medium-High | Low | Common in Windows bridges |
| **TCP sockets** | ~50-200 microseconds | Medium | Low | AudioGridder (remote mode) |
| **stdin/stdout pipes** | ~20-100 microseconds | Medium | Lowest | Simple but limited |

### 2.2 Recommended: Shared Memory for Audio, Pipes/Sockets for Control

**The yabridge architecture is the closest analog to our needs** and provides a proven pattern:

```
SUPERVISOR (MCP Server)                    WORKER (Plugin Host)
+---------------------------+              +---------------------------+
|                           |              |                           |
|  MCP Protocol Handler     |              |  VST3 Plugin Instance     |
|         |                 |              |         |                 |
|  Worker Manager           |              |  Audio Processor          |
|         |                 |              |         |                 |
|  [Control Socket] --------+----- IPC ----+- [Control Socket]        |
|  [Shared Memory]  --------+----- mmap ---+- [Shared Memory]         |
|                           |              |                           |
+---------------------------+              +---------------------------+
```

**Audio path (shared memory):**
- Supervisor allocates shared memory region via `shm_open()` + `mmap()` with `MAP_SHARED`
- Region contains: input buffer, output buffer, process metadata (sample count, sample rate)
- Synchronization via atomic flags or eventfd (no mutexes on audio path)
- Zero-copy: supervisor writes input samples directly, worker reads directly, writes output directly
- Buffer sized for the full processing block (e.g., 4096-65536 samples * channels * sizeof(f32))

**Control path (Unix domain socket or pipe):**
- Commands: load plugin, set parameter, get state, process block, shutdown
- Serialization: simple binary protocol or MessagePack (not JSON -- too slow for parameter changes)
- Can handle variable-length messages (plugin state blobs, parameter lists)

**Synchronization flow for a process() call:**
```
Supervisor                          Worker
    |                                  |
    |-- Write audio to shared mem ---->|
    |-- Send "process" command ------->|
    |                                  |-- Read input from shared mem
    |                                  |-- Call plugin.process()
    |                                  |-- Write output to shared mem
    |<-- Send "done" response ---------|
    |-- Read audio from shared mem     |
    |                                  |
```

**Why not just pipes for audio?**
At 48kHz stereo with 4096-sample buffers: 4096 * 2 * 4 bytes = 32KB per block. Pipe throughput is fine for this, but shared memory eliminates the kernel copy. For offline rendering where we might use 65536-sample buffers (512KB), shared memory's zero-copy advantage grows. More importantly, shared memory lets us reuse buffer allocations across process() calls -- no per-call allocation.

**Confidence: HIGH** -- yabridge's hybrid approach is production-proven on Linux, and the POSIX shared memory API is stable and well-documented.

### 2.3 iceoryx2 as an Alternative

Eclipse iceoryx2 provides a Rust-native zero-copy IPC framework with lock-free shared memory. It handles all the complexity of shared memory lifecycle, but adds a heavy dependency. For our single-producer-single-consumer case, raw `shm_open`/`mmap` with a simple protocol is likely simpler and more maintainable than pulling in iceoryx2.

**Recommendation:** Start with raw POSIX shared memory. Consider iceoryx2 only if the shared memory lifecycle management becomes burdensome.

---

## 3. Worker Lifecycle Management

### 3.1 Supervisor-Worker Process Model

Drawing from Erlang/OTP supervision principles (the gold standard for "let it crash" architectures) and AudioGridder's implementation:

```
                    +-----------------+
                    |   SUPERVISOR    |
                    |  (MCP Server)   |
                    +--------+--------+
                             |
              +--------------+--------------+
              |              |              |
        +-----+-----+  +----+----+   +-----+-----+
        |  Scanner   |  | Worker  |   | Worker    |
        |  Process   |  | (idle)  |   | (active)  |
        +------------+  +---------+   +-----------+
```

### 3.2 Worker States

```
SPAWNING --> READY --> LOADING --> ACTIVE --> PROCESSING --> ACTIVE --> ...
    |          |         |          |            |
    +----------+---------+----------+------------+--> CRASHED --> DEAD
                                    |
                                    +--> UNLOADING --> READY
```

- **SPAWNING:** Process fork/exec, shared memory setup, control socket connection
- **READY:** Worker process alive, awaiting plugin load command
- **LOADING:** Worker loading VST3 plugin (can crash here -- bad plugins)
- **ACTIVE:** Plugin loaded, parameters accessible, ready to process
- **PROCESSING:** Currently executing process() on audio data
- **CRASHED:** Worker process died (detected via SIGCHLD or socket EOF)
- **DEAD:** Cleaned up, removed from worker pool

### 3.3 Crash Detection Strategies

| Method | Latency | Reliability | Complexity |
|--------|---------|-------------|------------|
| **SIGCHLD handler** | Immediate | HIGH | Medium (signal handling in Rust) |
| **Socket/pipe EOF** | Immediate | HIGH | Low (natural in async I/O) |
| **Heartbeat timeout** | Configurable | HIGH | Medium |
| **waitpid() polling** | Polling interval | HIGH | Low |

**Recommended approach:** Use socket/pipe EOF as primary detection (when the worker dies, the socket closes, and the supervisor's read returns EOF). Use `waitpid(WNOHANG)` to reap zombie processes. No heartbeat needed for offline processing -- we only care about crashes during active operations.

### 3.4 Crash Recovery Flow

```
1. Supervisor detects worker death (socket EOF)
2. Supervisor calls waitpid() to reap process, collect exit status
3. Supervisor cleans up shared memory region (shm_unlink)
4. Supervisor logs crash with plugin ID, adds to crash counter
5. If crash count for plugin exceeds threshold --> add to blocklist
6. Supervisor returns error to MCP client: "Plugin crashed during processing"
7. On next request for same plugin: spawn new worker, reload plugin
```

**Key principle from AudioGridder:** "Slave process crashes trigger automatic restart without interrupting client connections." The supervisor stays alive and responsive regardless of worker failures.

### 3.5 Resource Cleanup

When a worker crashes, the supervisor must clean up:

- **Shared memory segments:** `shm_unlink()` the named region. On Linux, shared memory persists in `/dev/shm/` until explicitly unlinked. A crashed worker won't unlink its segments.
- **File descriptors:** Automatically closed when the process dies.
- **Temporary files:** Worker may have written partial output. Supervisor should use atomic file writes (write to temp, rename on success) so partial outputs are never visible.
- **Plugin state:** Lost on crash. For our offline use case, this is acceptable -- the MCP client simply retries with the same parameters.

**Confidence: HIGH** -- supervisor patterns are battle-tested in Erlang/OTP, Kubernetes, and AudioGridder.

---

## 4. Plugin Scanning and Discovery

### 4.1 VST3 Plugin Discovery

VST3 plugins live in standard locations:
- Linux: `~/.vst3/`, `/usr/lib/vst3/`, `/usr/local/lib/vst3/`
- macOS: `~/Library/Audio/Plug-Ins/VST3/`, `/Library/Audio/Plug-Ins/VST3/`
- Windows: `C:\Program Files\Common Files\VST3\`

Each `.vst3` bundle is a directory containing the shared library and resources. Discovery is filesystem enumeration -- straightforward.

### 4.2 Scanning Architecture: Always in a Separate Process

**This is non-negotiable.** Plugin scanning requires loading the plugin's shared library and calling initialization functions (`InitModule`/`ExitModule`, factory enumeration). Badly written plugins crash during this phase. Multiple sources confirm this is a common failure mode:

- JUCE forums discuss VST3 scanning crash protection as an active concern
- Steinberg forums report crashes during `BundleEntry`/`BundleExit` calls
- Plugins with login UIs or heavy initialization are known to crash during scan

**Pattern: Fork-per-scan with timeout**

```
For each .vst3 bundle:
    1. Fork scanner process
    2. Scanner loads plugin, enumerates components, extracts metadata:
       - Plugin name, vendor, version
       - Category (effect, instrument, analyzer)
       - Number and types of audio buses
       - Parameter list with IDs, names, ranges, defaults
       - Supported sample rates and channel configs
    3. Scanner serializes metadata to stdout (or shared memory)
    4. Supervisor reads metadata, adds to catalog
    5. If scanner crashes or times out (e.g., 10 seconds):
       - Kill scanner process
       - Add plugin to blocklist
       - Log failure, continue to next plugin
```

### 4.3 Scanning Optimization Strategies

| Strategy | Benefit | Tradeoff |
|----------|---------|----------|
| **Scan cache with mtime check** | Skip known-good plugins | Must invalidate on plugin update |
| **Parallel scanning** | Faster initial scan | More processes, more memory |
| **Sequential scanning** | Simpler, predictable | Slower for large collections |
| **On-demand scanning** | Zero startup cost | First use of a plugin is slow |

**Recommendation for our use case:**

1. **Cache-first with mtime validation.** Store scan results in a JSON/MessagePack catalog file. On startup, compare file mtimes. Only rescan changed/new plugins.
2. **Sequential scanning in separate processes.** Parallel scanning is an optimization for large plugin collections (100+). Start sequential, add parallelism later if needed.
3. **Blocklist persistence.** Store crashed-during-scan plugins in a blocklist file. Don't attempt to rescan them unless the user explicitly requests it or the plugin file changes.

### 4.4 Scan Result Schema

```rust
struct PluginInfo {
    uid: String,           // VST3 component ID (GUID)
    name: String,
    vendor: String,
    version: String,
    category: String,      // "Fx", "Instrument", etc.
    path: PathBuf,         // Path to .vst3 bundle
    file_mtime: u64,       // For cache invalidation
    audio_inputs: u32,     // Number of input channels
    audio_outputs: u32,    // Number of output channels
    parameters: Vec<ParameterInfo>,
    scan_status: ScanStatus, // Ok, Crashed, Timeout, Blocklisted
}

struct ParameterInfo {
    id: u32,               // VST3 parameter ID
    name: String,
    default_value: f64,    // Normalized 0.0-1.0
    units: String,
    step_count: i32,       // 0 = continuous, >0 = discrete
    flags: u32,            // Read-only, hidden, etc.
}
```

**Confidence: HIGH** -- scanning in separate processes is the established pattern across JUCE, Bitwig, and REAPER.

---

## 5. Offline Rendering Pipeline

### 5.1 VST3 Offline Processing Mode

The VST3 SDK explicitly supports offline processing via:

- **`IComponent::setIoMode(kOfflineProcessing)`** -- tells the plugin it's in offline mode (allows different algorithms, e.g., higher quality reverb tails)
- **`ProcessSetup.processMode = kOffline`** -- set during `setupProcessing()`
- **Variable buffer sizes** -- the spec allows any buffer size from 1 to `maxSamplesPerBlock` per process() call
- **No real-time constraint** -- the host can take as long as needed between process() calls

### 5.2 Recommended Pipeline

```
INPUT FILE                    WORKER PROCESS                 OUTPUT FILE
+----------+                  +-------------------+          +----------+
| WAV/FLAC |                  | VST3 Plugin       |          | WAV/FLAC |
|          |                  |                   |          |          |
| Read     +---> [Shared  +-->| process()         +---> [Shared  +--->| Write    |
| Blocks   |     Memory]  |   | process()         |     Memory]  |    | Blocks   |
|          |              |   | process()         |              |    |          |
+----------+              |   | ...               |              |    +----------+
                          |   +-------------------+              |
                          |                                      |
                    SUPERVISOR reads                       SUPERVISOR writes
                    input file,                           output file from
                    writes to shm                         shm after each block
```

**Processing flow:**

```
1. Supervisor reads input file header (sample rate, channels, bit depth, total frames)
2. Supervisor spawns worker, sends "load plugin" command
3. Worker loads plugin, calls:
   - setIoMode(kOfflineProcessing)
   - setupProcessing(sampleRate, maxBlockSize, kSample32/64, kOffline)
   - setActive(true)
   - setProcessing(true)
4. Supervisor opens output file for writing
5. For each block of input audio:
   a. Supervisor reads block from input file into shared memory input buffer
   b. Supervisor sends "process" command with sample count
   c. Worker reads input from shared memory
   d. Worker calls plugin.process(inputBuffers, outputBuffers, numSamples)
   e. Worker writes output to shared memory output buffer
   f. Worker sends "done" response
   g. Supervisor reads output from shared memory, writes to output file
6. After all blocks processed:
   - Supervisor sends "stop" command
   - Worker calls setProcessing(false), setActive(false)
   - Cleanup
```

### 5.3 Buffer Size Considerations for Offline Rendering

Since we have no real-time constraint:

- **Use large buffers (4096-8192 samples).** Reduces IPC overhead per sample. Larger buffers amortize context switch and synchronization costs.
- **Don't go too large (>65536).** Some plugins assume reasonable buffer sizes. The VST3 spec says the host can pass up to `maxSamplesPerBlock`, but plugins may allocate internal buffers based on this value.
- **Match plugin's preferred size if available.** Some plugins perform best at specific block sizes.
- **Handle tail.** After input is exhausted, continue calling process() with silent input until the plugin's tail (reverb decay, delay feedback) is complete. The plugin reports remaining tail via `getTailSamples()`.

### 5.4 Audio File I/O

For the supervisor side (reading/writing audio files), two viable Rust options:

- **symphonia** -- Pure Rust, supports WAV, FLAC, MP3, OGG, etc. No C dependencies. Active development.
- **hound** -- Simple WAV-only reader/writer. Very lightweight. Good if we only need WAV support initially.

**Recommendation:** Use symphonia for reading (broad format support) and hound or symphonia for writing (WAV output is sufficient for v1).

**Confidence: HIGH** -- VST3 offline processing is well-documented in the Steinberg SDK portal.

---

## 6. Recommended Architecture for This Project

### 6.1 System Architecture

```
+------------------------------------------------------------------+
|                        MCP CLIENT (Claude)                       |
|                         (stdio transport)                        |
+--------------------------------+---------------------------------+
                                 |
                                 | JSON-RPC over stdin/stdout
                                 |
+--------------------------------+---------------------------------+
|                        SUPERVISOR PROCESS                        |
|                                                                  |
|  +------------------+  +------------------+  +----------------+  |
|  | MCP Server       |  | Plugin Catalog   |  | Worker Manager |  |
|  | (tool handlers)  |  | (cached scan DB) |  | (spawn/reap)   |  |
|  +------------------+  +------------------+  +----------------+  |
|  +------------------+  +------------------+  +----------------+  |
|  | Audio File I/O   |  | Blocklist        |  | Preset Manager |  |
|  | (symphonia)      |  | (crash tracking) |  | (.vstpreset)   |  |
|  +------------------+  +------------------+  +----------------+  |
|                                                                  |
+-----+--------------------+--------------------+------ -----------+
      |                    |                    |
      | Unix Socket        | Unix Socket        | Unix Socket
      | + Shared Memory    | + Shared Memory    | + Shared Memory
      |                    |                    |
+-----+------+      +-----+------+      +------+-----+
| SCANNER    |      | WORKER     |      | WORKER     |
| PROCESS    |      | PROCESS    |      | PROCESS    |
|            |      |            |      |            |
| Loads .vst3|      | Loads .vst3|      | Loads .vst3|
| Enumerates |      | Processes  |      | Processes  |
| Metadata   |      | Audio      |      | Audio      |
+------------+      +------------+      +------------+
```

### 6.2 Component Responsibilities

| Component | Responsibility | Never Does |
|-----------|---------------|------------|
| **MCP Server** | Receives tool calls, returns results via JSON-RPC/stdio | Load plugins, process audio |
| **Plugin Catalog** | Stores/queries scan results, manages cache invalidation | Load plugin libraries |
| **Worker Manager** | Spawns workers, monitors health, detects crashes, manages shared memory lifecycle | Load plugins itself |
| **Audio File I/O** | Reads input files, writes output files | Run in worker process |
| **Blocklist** | Tracks plugins that crashed during scan or processing | Remove entries automatically |
| **Scanner Process** | Loads plugin library, enumerates metadata, exits | Process audio, persist state |
| **Worker Process** | Loads plugin, processes audio blocks, manages plugin state | Read/write audio files |

### 6.3 Separation of Concerns: Why Supervisor Never Touches Audio

The supervisor process handles MCP protocol, file I/O, and orchestration. It NEVER loads VST3 shared libraries because:

1. **VST3 plugins are C++ shared libraries loaded via `dlopen()`/`LoadLibrary()`.** A faulty plugin can corrupt the process's memory space, overwrite the heap, trigger segfaults, or deadlock.
2. **If the supervisor crashes, the MCP connection dies.** Claude loses its tool interface and cannot retry.
3. **The supervisor manages state that must survive crashes:** plugin catalog, blocklist, in-progress file handles, MCP session state.

### 6.4 Binary Architecture

Two separate binaries:

1. **`vst3-mcp-server`** (supervisor) -- the main binary, started by the MCP client
2. **`vst3-worker`** (worker/scanner) -- spawned by the supervisor as child processes

The worker binary is a single executable with subcommands:
- `vst3-worker scan <plugin-path>` -- scan a single plugin, output metadata to stdout
- `vst3-worker host` -- start as a plugin host worker, listen on socket for commands

This keeps deployment simple (two binaries) while maintaining clear process boundaries.

---

## 7. Key Design Decisions and Trade-offs

### 7.1 One Worker Per Plugin Instance (Not Pooled)

For v1, spawn a fresh worker for each processing request and shut it down after. This is simpler than maintaining a pool, and for offline rendering the spawn overhead (~50-100ms) is negligible relative to processing time (seconds to minutes).

**Pool workers later** if the interactive experimentation workflow demands faster turnaround for parameter tweaking (load plugin once, process multiple times with different parameters).

### 7.2 Shared Memory Sizing

Pre-allocate shared memory for the maximum buffer configuration:
- `max_block_size * max_channels * sizeof(f32) * 2` (input + output)
- At 8192 samples, 2 channels, f32: 8192 * 2 * 4 * 2 = 128KB
- At 65536 samples, 2 channels, f32: 65536 * 2 * 4 * 2 = 1MB

This is trivial memory overhead. Allocate once at worker spawn, reuse for all process() calls.

### 7.3 Serialization for Control Messages

Use a simple binary protocol rather than JSON or MessagePack:

```
[u32: message_type][u32: payload_length][payload_bytes...]
```

Message types: LoadPlugin, SetParameter, GetParameters, Process, GetState, SetState, Shutdown, Error, Ok.

This avoids serialization library dependencies on the worker side and is trivially fast.

### 7.4 When to Use Larger Buffers

| Buffer Size | Samples | Duration at 48kHz | Use Case |
|-------------|---------|-------------------|----------|
| Small | 512 | 10.7ms | Testing, parameter tweaking |
| Medium | 4096 | 85.3ms | Standard offline rendering |
| Large | 16384 | 341ms | Batch processing |
| Very Large | 65536 | 1.37s | Maximum throughput |

**Default to 4096.** It balances throughput and responsiveness. The worker isn't blocked for too long per call, and IPC overhead is minimal.

---

## 8. Risks and Open Questions

### 8.1 vst3-sys Hosting Maturity

The `vst3-sys` crate provides raw COM bindings but is primarily used for **writing** plugins, not hosting them. Building a host requires implementing several COM interfaces:

- `IHostApplication` -- host identification
- `IComponentHandler` -- parameter change notifications from plugin
- `IPlugFrame` -- (not needed for headless, but plugins may query it)

The hosting side of vst3-sys is less well-trodden. The Rust audio community has more experience with nih-plug (plugin authoring) than with hosting. This is the **highest technical risk** in the project.

**Mitigation:** Study the VST3 SDK's reference host implementation (C++), JUCE's hosting code, and any Rust hosting examples. Consider wrapping a thin C++ hosting layer if vst3-sys proves too painful.

### 8.2 Thread Safety in Worker

VST3 plugins expect specific threading contracts:
- `process()` called from a dedicated audio thread
- `setParameter()` and state operations from the "UI thread" (in our case, the main thread)
- Some plugins use thread-local storage that breaks if called from unexpected threads

The worker must maintain a consistent threading model even though it has no GUI. Designate one thread as "main/UI" and another as "audio processing."

### 8.3 Plugin State Serialization

VST3 plugins serialize state via `IComponent::getState()` and `IEditController::getState()`. These produce opaque binary blobs. For preset management, the supervisor must:
1. Send "get state" command to worker
2. Worker calls `getState()`, returns blob over socket
3. Supervisor writes blob to `.vstpreset` file

This works, but the blob format is plugin-specific and opaque. No way to inspect or merge states.

---

## Sources

### Official Documentation
- [Steinberg VST3 Developer Portal -- Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html) -- Offline processing modes, buffer size rules, lifecycle
- [POSIX shm_overview(7)](https://man7.org/linux/man-pages/man7/shm_overview.7.html) -- Shared memory API reference

### Architecture References
- [Ardour: Why plugins run in-process](https://ardour.org/plugins-in-process.html) -- Detailed analysis of context switch costs (numbers cited in Section 1.3)
- [yabridge architecture.md](https://github.com/robbert-vdh/yabridge/blob/master/docs/architecture.md) -- Hybrid shared memory + socket IPC, group hosting, crash isolation
- [AudioGridder DeepWiki](https://deepwiki.com/apohl79/audiogridder) -- Multi-process sandbox modes, lock-free audio streaming, fault recovery
- [AudioGridder GitHub](https://github.com/apohl79/audiogridder) -- Source reference for sandbox implementation

### Product Documentation
- [Bitwig Plugin Hosting & Crash Protection](https://www.bitwig.com/learnings/plug-in-hosting-crash-protection-in-bitwig-studio-20/) -- Five sandbox modes, crash recovery behavior
- [Bitwig Userguide: VST Handling](https://www.bitwig.com/userguide/latest/vst_plug-in_handling_and_options/) -- Bit-bridging, plugin format support
- [Fourier Audio transform.engine](https://fourieraudio.com/transform) -- Per-channel sandboxing, 15-second crash recovery

### Rust Ecosystem
- [RustAudio/vst3-sys](https://github.com/RustAudio/vst3-sys) -- Raw VST3 COM bindings for Rust
- [Eclipse iceoryx2](https://github.com/eclipse-iceoryx/iceoryx2) -- Zero-copy IPC framework with Rust core (considered, not recommended for v1)

### Community Discussion
- [JUCE Forum: VST3 scanning crash protection](https://forum.juce.com/t/vst3-plugin-scanning-crash-protection/58485)
- [Steinberg Forum: Plugin crash during scanning](https://forums.steinberg.net/t/vst3-host-plugin-crash-while-scanning-bundleentry-bundleexit/776824)
- [KVR Forum: Sandboxing in DAWs](https://www.kvraudio.com/forum/viewtopic.php?t=553714)
- [Erlang Supervisor Behaviour](https://www.erlang.org/doc/system/sup_princ.html) -- Supervisor pattern reference

### Confidence Assessment

| Area | Confidence | Rationale |
|------|------------|-----------|
| Process isolation patterns | HIGH | Multiple commercial implementations documented |
| IPC mechanisms | HIGH | yabridge architecture doc + POSIX shared memory well-established |
| Worker lifecycle | HIGH | Erlang supervision + AudioGridder patterns well-documented |
| Plugin scanning | HIGH | Multiple DAWs implement this, failure modes well-known |
| Offline rendering pipeline | HIGH | VST3 SDK explicitly documents offline mode |
| vst3-sys hosting maturity | MEDIUM | Less community experience with hosting vs. plugin authoring |
| Specific buffer size recommendations | MEDIUM | Based on general audio engineering practice, not benchmarked |
