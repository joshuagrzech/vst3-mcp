# Architecture Research

**Domain:** VST3 wrapper plugin with embedded MCP server (real-time audio + async AI control)
**Researched:** 2026-02-15
**Confidence:** MEDIUM (nih-plug official docs HIGH; wrapper-hosting-inside-plugin pattern LOW -- novel integration)

## System Overview

```
DAW Process
├──────────────────────────────────────────────────────────────────────┐
│  AgentAudio VST3 Plugin (nih-plug)                                  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    AUDIO THREAD (real-time)                   │   │
│  │  ┌─────────────┐    ┌──────────────────┐    ┌────────────┐  │   │
│  │  │ nih-plug    │───>│ Child Plugin     │───>│ nih-plug   │  │   │
│  │  │ Input Buf   │    │ (vst3 hosted)    │    │ Output Buf │  │   │
│  │  └─────────────┘    │ IAudioProcessor  │    └────────────┘  │   │
│  │                     └──────────────────┘                     │   │
│  │       ▲ lock-free read          │ lock-free write            │   │
│  │       │ (param changes)         │ (param notifications)      │   │
│  └───────┼─────────────────────────┼────────────────────────────┘   │
│          │                         │                                │
│  ┌───────┴─────────────────────────┴────────────────────────────┐   │
│  │              SHARED STATE (lock-free)                         │   │
│  │  ┌───────────────┐  ┌───────────────┐  ┌─────────────────┐  │   │
│  │  │ rtrb SPSC     │  │ AtomicF32     │  │ rtrb SPSC       │  │   │
│  │  │ MCP→Audio     │  │ Param Cache   │  │ Audio→MCP       │  │   │
│  │  │ (commands)    │  │ (current vals)│  │ (notifications) │  │   │
│  │  └───────────────┘  └───────────────┘  └─────────────────┘  │   │
│  └──────────────────────────────────────────────────────────────┘   │
│          │                         │                                │
│  ┌───────┴─────────────────────────┴────────────────────────────┐   │
│  │                 MCP THREAD (Tokio runtime)                    │   │
│  │  ┌───────────────┐  ┌──────────────────────────────────┐    │   │
│  │  │ rmcp Server   │  │ Tool Handlers                    │    │   │
│  │  │ (stdio/SSE)   │  │ get_params / set_param / listen  │    │   │
│  │  └───────────────┘  └──────────────────────────────────┘    │   │
│  └──────────────────────────────────────────────────────────────┘   │
│          │                                                          │
│  ┌───────┴──────────────────────────────────────────────────────┐   │
│  │                 GUI THREAD (nih-plug-egui)                    │   │
│  │  ┌──────────────────┐  ┌────────────────────────────────┐   │   │
│  │  │ Wrapper Controls │  │ Child Plugin Editor            │   │   │
│  │  │ (Focus Mode,     │  │ (IPlugView, platform window)   │   │   │
│  │  │  status, params) │  │                                │   │   │
│  │  └──────────────────┘  └────────────────────────────────┘   │   │
│  └──────────────────────────────────────────────────────────────┘   │
├──────────────────────────────────────────────────────────────────────┘
│
│  External: Claude / AI Agent connects via stdio or SSE transport
```

## Component Responsibilities

| Component | Responsibility | Typical Implementation |
|-----------|----------------|------------------------|
| **nih-plug Plugin struct** | Implements `Plugin` trait; owns audio thread lifecycle; delegates to child plugin host | `struct AgentAudio: Plugin` with `process()` forwarding buffers to child |
| **Child Plugin Host** | Loads, initializes, processes child VST3 plugin via COM interfaces | Wraps existing `PluginInstance` from current codebase; adapts for real-time mode |
| **MCP Server** | Handles AI tool calls for parameter read/write, plugin inspection | `rmcp` server running on dedicated Tokio runtime thread |
| **Lock-Free Bridge** | Transfers parameter changes and notifications between threads without blocking | `rtrb` SPSC ring buffers + `AtomicF32` arrays for current values |
| **Focus Mode Manager** | Tracks which parameters are "exposed" to AI based on user interaction | Bitfield or `AtomicBool` array, set by GUI/IComponentHandler, read by MCP |
| **GUI (Wrapper)** | Renders Focus Mode controls, status indicators, parameter overview | `nih-plug-egui` editor with `Arc<Params>` access |
| **GUI (Child Editor)** | Embeds child plugin's native `IPlugView` in a platform window | Platform-specific window parenting (X11 on Linux) |

## Recommended Project Structure

```
src/
├── lib.rs                  # Plugin entry point, nih_export_vst3!() macro
├── plugin.rs               # AgentAudio Plugin trait implementation
├── params.rs               # nih-plug Params struct (wrapper-level params)
├── hosting/                # Child plugin hosting (adapted from current code)
│   ├── mod.rs
│   ├── plugin.rs           # PluginInstance (lifecycle state machine)
│   ├── host_app.rs         # IHostApplication, IComponentHandler
│   ├── module.rs           # VST3 module loading (libloading)
│   ├── scanner.rs          # Plugin discovery
│   ├── types.rs            # Shared types
│   └── realtime.rs         # Real-time process() adapter (kRealtime mode)
├── bridge/                 # Lock-free communication layer
│   ├── mod.rs
│   ├── command.rs          # MCP→Audio command types (SetParam, LoadPreset, etc.)
│   ├── notification.rs     # Audio→MCP notification types (ParamChanged, etc.)
│   └── param_cache.rs      # Atomic parameter value cache
├── mcp/                    # MCP server integration
│   ├── mod.rs
│   ├── server.rs           # Tool definitions (adapted from current server.rs)
│   ├── runtime.rs          # Tokio runtime lifecycle (spawn/shutdown)
│   └── transport.rs        # stdio or SSE transport setup
├── focus/                  # Focus Mode ("wiggle to expose")
│   ├── mod.rs
│   └── manager.rs          # Parameter exposure tracking
├── editor/                 # GUI layer
│   ├── mod.rs
│   ├── wrapper_ui.rs       # egui wrapper controls
│   └── child_view.rs       # IPlugView embedding (platform-specific)
├── audio/                  # Audio utilities (from current code)
│   ├── mod.rs
│   └── buffers.rs          # Interleave/deinterleave helpers
└── preset/                 # Preset management (from current code)
    ├── mod.rs
    ├── state.rs
    └── vstpreset.rs
```

### Structure Rationale

- **hosting/:** Isolated from nih-plug concerns. Can be tested independently. Adapted from current working codebase.
- **bridge/:** Single module owns all cross-thread communication. Makes lock-free invariants auditable in one place.
- **mcp/:** Encapsulates Tokio runtime and tool definitions. Only communicates with audio thread through bridge/.
- **focus/:** Separate because it spans GUI, audio, and MCP contexts. Clean interface prevents threading confusion.
- **editor/:** Two distinct concerns (wrapper egui UI vs child IPlugView) kept together since both are GUI-thread.

## Architectural Patterns

### Pattern 1: nih-plug Plugin Trait as Outer Shell

**What:** The `Plugin` trait implementation is the outer shell that the DAW interacts with. Its `process()` method receives audio from the DAW, forwards to the child plugin, and returns the child's output. The child plugin is an internal detail invisible to the DAW.

**When to use:** Always. This is the fundamental architecture.

**Trade-offs:** nih-plug handles VST3/CLAP wrapping of the outer plugin, but the inner plugin hosting is entirely manual COM work via `vst3` crate. Two levels of VST3 abstraction.

**Example:**
```rust
struct AgentAudio {
    params: Arc<AgentAudioParams>,
    // Owned by audio thread exclusively after editor()/task_executor() called
    child: Option<PluginInstance>,
    // Lock-free bridge endpoints (audio-side)
    command_rx: rtrb::Consumer<Command>,
    notification_tx: rtrb::Producer<Notification>,
    param_cache: Arc<ParamCache>,  // AtomicF32 array
}

impl Plugin for AgentAudio {
    type BackgroundTask = AgentTask;

    fn process(
        &mut self,
        buffer: &mut Buffer<'_>,
        _aux: &mut AuxiliaryBuffers<'_>,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // 1. Drain command queue (lock-free)
        while let Ok(cmd) = self.command_rx.pop() {
            match cmd {
                Command::SetParam { id, value } => {
                    self.child.as_mut().unwrap()
                        .queue_parameter_change(id, value);
                }
                // ...
            }
        }

        // 2. Forward audio to child plugin
        // Convert nih-plug Buffer to child's planar format
        // Call child.process()
        // Copy output back to nih-plug Buffer

        ProcessStatus::Normal
    }
}
```

### Pattern 2: Lock-Free SPSC Ring Buffers for Cross-Thread Communication

**What:** Use `rtrb` (Real-Time Ring Buffer) for wait-free single-producer single-consumer communication between the MCP thread and audio thread. Commands flow MCP-to-Audio; notifications flow Audio-to-MCP. Never use mutexes on the audio thread.

**When to use:** Every cross-thread data transfer involving the audio thread.

**Trade-offs:** SPSC means one producer, one consumer. If GUI also needs to send commands, either route through MCP thread or add a second ring buffer pair for GUI-to-Audio. Capacity must be pre-allocated; overflow means dropped messages (acceptable for parameter changes, not for state loads).

**Example:**
```rust
// At plugin creation time
let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Command>::new(256);
let (notif_tx, notif_rx) = rtrb::RingBuffer::<Notification>::new(256);

// MCP thread (producer):
cmd_tx.push(Command::SetParam { id: 42, value: 0.75 }).ok();

// Audio thread (consumer, in process()):
while let Ok(cmd) = cmd_rx.pop() {
    // Handle command, no allocation, no blocking
}
```

### Pattern 3: Tokio Runtime on Dedicated Background Thread

**What:** Spawn a Tokio `Runtime` on a dedicated thread during `Plugin::initialize()` or `Plugin::editor()`. The runtime runs the MCP server. It communicates with the audio thread exclusively through the lock-free bridge. Shutdown the runtime in `Plugin::deactivate()` or on Drop.

**When to use:** For the MCP server integration. Tokio must never touch the audio thread.

**Trade-offs:** Spawning a full Tokio runtime inside a plugin is unconventional. Must ensure clean shutdown (no dangling tasks). The runtime's thread pool is separate from the audio thread. `Runtime::new()` allocates, so do it during `initialize()` (not `process()`).

**Critical constraint:** Never call `block_on()` from the audio thread. Never pass the Tokio `Handle` to audio thread code.

**Example:**
```rust
fn initialize(
    &mut self,
    _audio_io_layout: &AudioIOLayout,
    _buffer_config: &BufferConfig,
    _context: &mut impl InitContext<Self>,
) -> bool {
    // Spawn Tokio runtime on background thread
    let cmd_tx = self.bridge.cmd_tx.clone(); // for MCP to send commands
    let notif_rx = self.bridge.notif_rx.clone(); // for MCP to read notifications

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");

        rt.block_on(async move {
            let server = McpServer::new(cmd_tx, notif_rx);
            server.serve(rmcp::transport::io::stdio()).await.unwrap();
        });
    });

    true
}
```

### Pattern 4: Atomic Parameter Cache for GUI Reads

**What:** Maintain an array of `AtomicF32` values representing current parameter states. The audio thread writes updated values after processing. The GUI and MCP threads read atomically without locking.

**When to use:** When GUI or MCP needs to display/report current parameter values without blocking the audio thread.

**Trade-offs:** Eventual consistency -- GUI may show slightly stale values (1-2 audio buffers behind). Acceptable for display purposes. Not suitable for sample-accurate automation.

### Pattern 5: Child Plugin Lifecycle Tied to Wrapper Lifecycle

**What:** The child plugin is loaded during `initialize()`, set up with the same sample rate and buffer size the DAW provides, and torn down during `deactivate()` or Drop. The child's lifecycle mirrors the wrapper's lifecycle.

**When to use:** Always. The child plugin must be initialized before `process()` and torn down cleanly.

**Key adaptation from current code:** Current `PluginInstance` uses `kOffline` process mode. For real-time hosting, switch to `kRealtime`. The `ProcessSetup` must match what the DAW provides to the wrapper.

```rust
// In initialize():
let mut setup = ProcessSetup {
    processMode: kRealtime as i32,     // NOT kOffline
    symbolicSampleSize: kSample32 as i32,
    maxSamplesPerBlock: buffer_config.max_buffer_size as i32,
    sampleRate: buffer_config.sample_rate as f64,
};
```

## Data Flow

### Audio Processing Flow (per buffer callback)

```
DAW calls AgentAudio::process(buffer, aux, context)
    │
    ├── 1. Drain command queue (rtrb Consumer::pop, lock-free)
    │       └── Apply parameter changes to child plugin queue
    │
    ├── 2. Convert nih-plug Buffer to child's planar format
    │       └── nih-plug provides interleaved channel slices
    │       └── Child expects per-channel pointers (AudioBusBuffers)
    │
    ├── 3. Call child.process(inputs, outputs, num_samples)
    │       └── Child writes processed audio to output buffers
    │
    ├── 4. Copy child output back to nih-plug Buffer
    │       └── May need format conversion (planar → nih-plug layout)
    │
    ├── 5. Update atomic param cache (if child reports changes)
    │       └── Write new values to AtomicF32 array
    │
    └── 6. Push notifications to MCP (rtrb Producer::push, lock-free)
            └── ParamChanged events for Focus Mode parameters
```

### MCP Tool Call Flow

```
AI Agent sends JSON-RPC tool call (e.g., set_param)
    │
    ├── 1. rmcp deserializes request on Tokio task
    │
    ├── 2. Tool handler validates (param exists, is exposed, value in range)
    │
    ├── 3. Push Command::SetParam to rtrb ring buffer
    │       └── Wait-free, returns immediately
    │       └── If buffer full, return error to AI (backpressure)
    │
    └── 4. Return success to AI
            └── Actual application happens next audio callback
            └── Latency: 1 buffer period (typically 1-10ms)
```

### Focus Mode ("Wiggle to Expose") Flow

```
User enables "Listen" in wrapper GUI
    │
    ├── 1. GUI sets listen_mode = true (atomic flag)
    │
    ├── 2. User tweaks knob on child plugin editor
    │
    ├── 3. Child calls IComponentHandler::performEdit(param_id, value)
    │       └── Our ComponentHandler implementation receives this
    │       └── If listen_mode is active, mark param_id as "exposed"
    │
    ├── 4. Update exposed_params bitfield (atomic)
    │
    └── 5. User disables "Listen"
            └── Exposed params remain exposed until manually cleared
            └── MCP tools only operate on exposed params
```

### MIDI Event Flow

```
DAW sends MIDI to AgentAudio
    │
    ├── 1. nih-plug provides NoteEvent iterator via ProcessContext
    │
    ├── 2. Convert nih-plug NoteEvents to VST3 Event structs
    │       └── Map NoteOn/NoteOff/CC to Steinberg event types
    │
    ├── 3. Populate child's ProcessData.inputEvents
    │       └── Build IEventList with converted events
    │
    └── 4. Child processes MIDI alongside audio
```

## Integration Points

### nih-plug <-> Child Plugin Host

| Boundary | Direction | Data | Mechanism |
|----------|-----------|------|-----------|
| Audio buffers | nih-plug -> Child -> nih-plug | f32 sample data | Buffer format conversion (nih-plug Buffer <-> VST3 AudioBusBuffers) |
| Sample rate / block size | nih-plug -> Child | BufferConfig values | Passed during initialize(), child's ProcessSetup mirrors wrapper's |
| MIDI events | nih-plug -> Child | NoteEvent -> VST3 Event | Manual conversion in process() |
| Parameter changes | Child -> nih-plug | IComponentHandler callbacks | Our ComponentHandler captures, pushes to notification queue |

### nih-plug <-> MCP Server

| Boundary | Direction | Data | Mechanism |
|----------|-----------|------|-----------|
| Parameter commands | MCP -> Audio | SetParam, GetState | rtrb SPSC ring buffer (Command enum) |
| Parameter notifications | Audio -> MCP | ParamChanged, StateChanged | rtrb SPSC ring buffer (Notification enum) |
| Current param values | Audio -> MCP/GUI | f32 values | AtomicF32 array (param_cache) |
| Exposed params list | GUI -> MCP | Bitfield | AtomicU64 or Vec<AtomicBool> |

### nih-plug <-> GUI (egui)

| Boundary | Direction | Data | Mechanism |
|----------|-----------|------|-----------|
| Wrapper params | Plugin <-> GUI | Focus Mode toggle, status | nih-plug Params (handled automatically) |
| Param display values | Audio -> GUI | Current child param values | AtomicF32 array (param_cache), read-only |
| Focus Mode state | GUI -> Focus Manager | Listen toggle, clear exposed | Atomic flags |

### Child Plugin GUI (IPlugView)

| Boundary | Direction | Mechanism | Notes |
|----------|-----------|-----------|-------|
| Window handle | Wrapper -> Child | IPlugView::attached(parent_hwnd) | Platform-specific: X11 window ID on Linux |
| Resize | Child -> Wrapper | IPlugView::onSize / IPlugFrame::resizeView | Wrapper must implement IPlugFrame |
| Parameter edits | Child -> Wrapper | IComponentHandler::performEdit | Wrapper's ComponentHandler intercepts |

**CRITICAL NOTE on IPlugView:** Embedding a child plugin's GUI inside an egui window is the riskiest integration point. egui uses OpenGL/wgpu for rendering. The child plugin's IPlugView expects a native platform window (X11 window on Linux, HWND on Windows). These are fundamentally different windowing models. The solution is to create a separate native platform window (not managed by egui) and position it adjacent to or overlapping the egui window. This is how JUCE-based wrappers handle it.

**Confidence: LOW** -- I could not find any examples of embedding IPlugView inside an egui context specifically. This needs prototype validation.

## Anti-Patterns

### Anti-Pattern 1: Mutex on the Audio Thread

**What people do:** Use `Arc<Mutex<PluginInstance>>` to share the child plugin between threads (as the current standalone MCP server does).
**Why it's wrong:** Mutex::lock() can block the audio thread if the MCP thread holds the lock. Even a brief contention causes audio glitches (clicks, dropouts). The DAW's audio callback has a hard deadline (typically 1-10ms).
**Do this instead:** The audio thread exclusively owns the PluginInstance. Other threads communicate via lock-free ring buffers only.

### Anti-Pattern 2: Allocating in process()

**What people do:** Create Vec, String, or Box inside the audio callback.
**Why it's wrong:** Heap allocation can trigger system calls, page faults, or lock the global allocator. All are unbounded-time operations.
**Do this instead:** Pre-allocate all buffers during initialize(). Use fixed-capacity containers. Enable `assert_process_allocs` in debug builds (nih-plug feature).

### Anti-Pattern 3: Tokio block_on() from Audio Thread

**What people do:** Call `handle.block_on(async_fn())` from the audio callback to execute async operations.
**Why it's wrong:** `block_on()` parks the current thread until the future completes. On the audio thread, this is catastrophic.
**Do this instead:** Tokio runtime runs on its own threads. Communication with audio thread is purely through lock-free queues.

### Anti-Pattern 4: Child Plugin COM Calls from Wrong Thread

**What people do:** Call `IEditController::setParamNormalized()` from the MCP/background thread.
**Why it's wrong:** VST3 SDK expects certain calls from specific threads. Controller methods should be called from the UI thread. Audio processing methods from the audio thread. Cross-thread COM calls cause undefined behavior in many plugins.
**Do this instead:** Route parameter changes through the audio thread (via ring buffer -> child's ProcessData.inputParameterChanges) for audio-thread changes. Route UI-thread calls through nih-plug's `schedule_gui()` mechanism.

### Anti-Pattern 5: Sharing PluginInstance Across Threads

**What people do:** Wrap PluginInstance in Arc to share between audio, GUI, and MCP threads.
**Why it's wrong:** PluginInstance contains COM pointers that are not thread-safe. VST3 COM objects have thread affinity. The audio processor must be called from the audio thread; the controller from the UI thread.
**Do this instead:** Audio thread exclusively owns the PluginInstance. GUI accesses the controller through a separate reference obtained during initialization. MCP thread never directly touches COM objects.

## Scaling Considerations

These are not traditional "users" scaling concerns. Instead, consider plugin complexity scaling.

| Concern | Simple Plugin (10 params) | Complex Plugin (200+ params) | Edge Case (instruments with note expression) |
|---------|--------------------------|-------------------------------|----------------------------------------------|
| Param cache size | 10 * 4 bytes = 40B trivial | 200 * 4 bytes = 800B trivial | Same, plus note expression arrays |
| Ring buffer capacity | 64 entries sufficient | 256 entries to handle rapid automation | May need 512+ for dense MIDI+param |
| Focus Mode UI | Simple list | Needs search/filter/categories | Category grouping essential |
| MCP tool response size | Small JSON | Large JSON for get_all_params | Paginate or filter by exposed only |
| Audio buffer conversion | Negligible overhead | Same | Same |

## Suggested Build Order (Risk-Ordered)

Build the riskiest integrations first to fail fast.

### Phase 1: Prove nih-plug + Child Hosting Works (HIGHEST RISK)

Build a minimal nih-plug plugin that loads a child VST3 and passes audio through it in real-time. No MCP, no GUI beyond basic egui scaffold.

**Validates:**
- nih-plug Plugin trait implementation compiles and loads in Bitwig
- Child plugin loading via vst3 crate COM works inside nih-plug's lifecycle
- Real-time audio passthrough with correct buffer format conversion
- `initialize()` / `process()` / `deactivate()` lifecycle alignment between wrapper and child
- No audio glitches under normal operation

**Risk factors:**
- Buffer format mismatch (nih-plug Buffer vs VST3 AudioBusBuffers)
- Lifecycle ordering (when exactly to initialize/teardown child)
- Current code uses `kOffline` mode; switching to `kRealtime` may surface bugs in child plugins
- COM pointer lifetime management in nih-plug's ownership model

### Phase 2: Lock-Free Bridge + MCP Server Integration (HIGH RISK)

Add the `rtrb` ring buffers and spawn a Tokio runtime for the MCP server. Implement basic `get_params` and `set_param` tools.

**Validates:**
- Tokio runtime can be spawned and shut down cleanly inside a plugin lifecycle
- Lock-free parameter changes actually reach the child plugin and produce audible results
- MCP server can communicate with external AI agents while plugin runs in DAW
- No audio thread blocking under MCP load

**Risk factors:**
- Tokio runtime lifecycle tied to plugin lifecycle (what happens on DAW restart, plugin bypass, etc.)
- stdio transport may conflict with DAW's process IO (may need SSE/WebSocket transport instead)
- Ring buffer overflow handling under rapid AI parameter changes

### Phase 3: Focus Mode + ComponentHandler Integration (MEDIUM RISK)

Implement the "wiggle to expose" pattern. ComponentHandler intercepts child plugin parameter edits and marks them as exposed.

**Validates:**
- IComponentHandler::performEdit correctly intercepts child plugin knob changes
- Atomic bitfield approach works for parameter exposure tracking
- MCP tools correctly filter to exposed-only parameters

### Phase 4: Child Plugin GUI Embedding (HIGH RISK, can be deferred)

Embed the child plugin's IPlugView in a window alongside the wrapper's egui UI.

**Validates:**
- IPlugView can be attached to a native window created alongside egui
- Window positioning and resizing works on Linux/X11
- Child plugin's GUI and wrapper's GUI coexist without conflicts

**Risk factors:**
- Platform-specific windowing code (X11, Wayland)
- egui's window management vs native window parenting
- Some plugins may not support being embedded in arbitrary parent windows
- IPlugFrame implementation for resize coordination

### Phase 5: Polish, Presets, MIDI (LOWER RISK)

MIDI passthrough, preset save/load through MCP, parameter value display, error handling.

## Key Technical Decisions

### vst3 crate (0.3) vs vst3-sys

The project currently uses the `vst3` crate (version 0.3.0), which is the newer binding generator from coupler-rs. This is distinct from `vst3-sys` (from RustAudio). The `vst3` crate generates COM bindings directly from VST3 headers and provides `ComPtr`/`ComWrapper` smart pointers.

**Decision: Continue with `vst3` crate 0.3** because:
- Already working in the current codebase
- Provides COM class definition via `Class` trait
- `ComWrapper` handles reference counting
- No need to switch; the hosting code is already proven

**Caveat:** nih-plug internally uses its own VST3 wrapper that is separate from the `vst3` crate. The wrapper plugin (outer) uses nih-plug's VST3 wrapper. The hosted child plugin (inner) uses the `vst3` crate directly. These are independent -- they do not share COM infrastructure.

### rtrb vs crossbeam-queue

**Decision: Use `rtrb`** because:
- Wait-free (not just lock-free) SPSC ring buffer
- Designed specifically for real-time audio use cases
- No contention, no CAS loops, no allocation after creation
- Used widely in the Rust audio ecosystem
- `crossbeam::ArrayQueue` is MPMC which has higher overhead for the SPSC case we need

### Tokio vs async-std vs manual threads

**Decision: Use Tokio** because:
- `rmcp` (the MCP SDK) already requires Tokio
- Well-tested, large ecosystem for async networking
- `spawn_blocking()` available if MCP handlers need sync operations
- Use `runtime::Builder::new_multi_thread()` with 2 worker threads to minimize resource usage

### Child Plugin Process Mode

**Decision: Use `kRealtime`** (not `kOffline` as in current code) because:
- The wrapper runs inside a DAW with real-time audio constraints
- Some plugins behave differently in offline vs realtime mode (quality settings, latency)
- The DAW expects real-time behavior from the wrapper plugin

### MCP Transport: stdio vs SSE

**Decision: Start with stdio, plan for SSE** because:
- stdio is simplest to implement (already working in current code)
- But stdio inside a DAW plugin is problematic: the DAW owns the process's stdin/stdout
- Will likely need to switch to SSE (HTTP Server-Sent Events) or WebSocket for production
- SSE allows multiple AI agents to connect to the same plugin instance

**Confidence: LOW** on stdio working inside a DAW plugin. This needs immediate validation in Phase 2.

## Sources

- [nih-plug Plugin trait documentation](https://nih-plug.robbertvanderhelm.nl/nih_plug/plugin/trait.Plugin.html) -- HIGH confidence
- [nih-plug GitHub repository](https://github.com/robbert-vdh/nih-plug) -- HIGH confidence
- [nih-plug egui editor](https://nih-plug.robbertvanderhelm.nl/nih_plug_egui/index.html) -- HIGH confidence
- [rtrb SPSC ring buffer](https://github.com/mgeier/rtrb) -- HIGH confidence
- [VST3 Hosting FAQ (Steinberg)](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Hosting.html) -- HIGH confidence
- [IPlugView Class Reference](https://steinbergmedia.github.io/vst3_doc/base/classSteinberg_1_1IPlugView.html) -- HIGH confidence
- [cutoff-vst case study](https://renauddenis.com/case-studies/rust-vst) -- MEDIUM confidence
- [nih-plug background tasks issue #172](https://github.com/robbert-vdh/nih-plug/issues/172) -- MEDIUM confidence
- [AU-VST3-Wrapper (JUCE-based wrapper reference)](https://github.com/ivicamil/AU-VST3-Wrapper) -- MEDIUM confidence
- [JUCE plugin wrapper template](https://github.com/getdunne/juce-plugin-wrapper) -- MEDIUM confidence

---
*Architecture research for: AgentAudio VST3 Wrapper Plugin with Embedded MCP Server*
*Researched: 2026-02-15*
