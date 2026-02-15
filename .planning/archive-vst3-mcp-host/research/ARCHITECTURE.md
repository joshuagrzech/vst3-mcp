# Architecture Patterns

**Domain:** Headless VST3 Host with MCP Server Interface
**Researched:** 2026-02-14

## Recommended Architecture

```
                    MCP Client (Claude, AI Agent)
                           |
                      stdio / SSE
                           |
                    +------v-------+
                    |  MCP Server  |  (Supervisor Process)
                    |   (rmcp)     |
                    +------+-------+
                           |
              +------------+------------+
              |                         |
       Unix Socket              Unix Socket
              |                         |
     +--------v--------+      +--------v--------+
     |  Worker Process  |      |  Worker Process  |
     |  (Plugin Host)   |      |  (Plugin Host)   |
     |                  |      |                  |
     | +-- VST3 Plugin  |      | +-- VST3 Plugin  |
     | +-- VST3 Plugin  |      | +-- VST3 Plugin  |
     +--------+---------+      +--------+---------+
              |                         |
         Shared Memory             Shared Memory
         (audio buffers)           (audio buffers)
```

### Component Boundaries

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| **MCP Server (Supervisor)** | Protocol handling, session management, worker lifecycle, plugin registry | AI clients (MCP), Workers (Unix sockets) |
| **Plugin Registry** | Scan, cache, and query available VST3 plugins | MCP Server (in-process) |
| **Worker Process** | Load plugins, process audio, isolate crashes | Supervisor (Unix socket + shared memory) |
| **Audio Pipeline** | Decode input, feed through plugin chain, encode output | Worker (in-process) |
| **Plugin Instance** | Wrapper around VST3 COM interfaces for a single plugin | Audio Pipeline (in-process) |

### Data Flow

**Offline Render Request:**

```
1. AI Agent -> MCP Server: tool call "render" with {input: "drums.wav", plugin: "FabFilter Pro-Q 3", params: {...}}
2. MCP Server -> Plugin Registry: resolve plugin path
3. MCP Server -> Worker: spawn or reuse worker process
4. MCP Server -> Worker (Unix socket): LoadPlugin command + SetParameters command
5. MCP Server -> Shared Memory: write decoded audio samples from input file
6. MCP Server -> Worker (Unix socket): Render command {input_offset, num_samples, output_offset}
7. Worker: reads audio from shared memory, processes through VST3 plugin, writes output to shared memory
8. Worker -> Supervisor (Unix socket): RenderComplete {output_offset, num_samples}
9. MCP Server: reads rendered audio from shared memory, encodes to WAV
10. MCP Server -> AI Agent: tool result {output_file: "drums_processed.wav", peak_db: -3.2}
```

**Simplified single-process flow (MVP):**

```
1. AI Agent -> MCP Server: tool call
2. MCP Server: decode input audio (symphonia)
3. MCP Server: load VST3 plugin (vst3 crate)
4. MCP Server: process audio blocks through plugin
5. MCP Server: encode output (hound)
6. MCP Server -> AI Agent: result
```

## Patterns to Follow

### Pattern 1: Layered VST3 Abstraction

Build three layers to contain unsafe code:

```
Layer 3: Safe Host API (public)     -- PluginInstance, PluginChain, Renderer
Layer 2: COM Wrapper (internal)     -- SafeComponent, SafeProcessor, ref counting
Layer 1: Raw Bindings (vst3 crate)  -- IComponent, IAudioProcessor, raw pointers
```

```rust
// Layer 2: COM wrapper that handles unsafe
pub(crate) struct SafeComponent {
    component: VstPtr<dyn IComponent>,
}

impl SafeComponent {
    pub fn get_bus_count(&self, media_type: MediaType, dir: BusDirection) -> i32 {
        // SAFETY: IComponent::getBusCount is a simple query with no side effects
        unsafe { self.component.getBusCount(media_type as i32, dir as i32) }
    }
}

// Layer 3: Safe public API
pub struct PluginInstance {
    component: SafeComponent,
    processor: SafeProcessor,
    controller: SafeController,
    // ...
}

impl PluginInstance {
    pub fn audio_bus_count(&self) -> usize {
        self.component.get_bus_count(MediaType::Audio, BusDirection::Input) as usize
    }
}
```

### Pattern 2: Worker Process Protocol

Define a clear command/response protocol for supervisor-worker communication.

```rust
// Shared protocol types (in a shared crate)
#[derive(Serialize, Deserialize)]
enum WorkerCommand {
    LoadPlugin { path: PathBuf, class_id: [u8; 16] },
    SetParameter { param_id: u32, value: f64 },
    SetState { data: Vec<u8> },
    Process { block_size: usize, num_blocks: usize },
    GetState,
    Shutdown,
}

#[derive(Serialize, Deserialize)]
enum WorkerResponse {
    PluginLoaded { bus_info: BusInfo, parameters: Vec<ParamInfo> },
    ParameterSet { param_id: u32, normalized: f64 },
    ProcessComplete { samples_written: usize },
    State { data: Vec<u8> },
    Error { code: i32, message: String },
}
```

### Pattern 3: Block-Based Audio Processing

VST3 plugins process audio in blocks. Match the host's buffer management to the plugin's preferred block size.

```rust
pub fn render_offline(
    plugin: &mut PluginInstance,
    input: &[f32],        // interleaved samples
    output: &mut [f32],
    sample_rate: f64,
    max_block_size: usize,
) -> Result<()> {
    let channels = plugin.audio_bus_count();
    let frames = input.len() / channels;

    // Activate plugin for offline processing
    plugin.set_processing(true)?;

    let mut offset = 0;
    while offset < frames {
        let block_frames = (frames - offset).min(max_block_size);

        // Deinterleave into per-channel buffers
        let input_buffers = deinterleave(&input[offset * channels..], channels, block_frames);
        let mut output_buffers = vec![vec![0.0f32; block_frames]; channels];

        // Process block
        plugin.process(&input_buffers, &mut output_buffers, block_frames)?;

        // Interleave back
        interleave(&output_buffers, &mut output[offset * channels..], block_frames);

        offset += block_frames;
    }

    plugin.set_processing(false)?;
    Ok(())
}
```

### Pattern 4: MCP Resource Exposure

Expose plugin metadata as MCP resources so AI agents can discover capabilities before acting.

```rust
// Expose available plugins as resources
#[resource]
impl AudioHost {
    /// List all available VST3 plugins
    async fn plugins(&self) -> Vec<PluginInfo> {
        self.registry.list_plugins()
    }

    /// Get parameters for a loaded plugin
    async fn parameters(&self, instance_id: String) -> Vec<ParamInfo> {
        self.instances.get(&instance_id)
            .map(|p| p.parameter_info())
            .unwrap_or_default()
    }
}
```

## Anti-Patterns to Avoid

### Anti-Pattern 1: Processing Audio on the Async Runtime

**What:** Running audio processing on tokio worker threads.
**Why bad:** Audio processing is CPU-bound and will starve async tasks. Tokio threads should not block.
**Instead:** Use `tokio::task::spawn_blocking` for in-process audio work, or delegate to worker processes (the recommended architecture).

### Anti-Pattern 2: Sharing VST3 Plugin Instances Across Threads

**What:** Wrapping a plugin instance in `Arc<Mutex<PluginInstance>>` and sharing across threads.
**Why bad:** VST3 COM objects have thread affinity requirements. Some plugins expect all calls from the same thread. Mutex contention defeats real-time guarantees.
**Instead:** Each worker process owns its plugins. Communication happens via message passing.

### Anti-Pattern 3: Direct COM Pointer Manipulation in Business Logic

**What:** Calling unsafe COM methods directly in the MCP tool handlers.
**Why bad:** Unsafe code scattered throughout the codebase. One incorrect call = UB.
**Instead:** Layer 2 wraps all COM calls. Business logic only touches Layer 3 safe API.

### Anti-Pattern 4: Allocating in the Audio Processing Loop

**What:** Creating Vec, String, or other heap allocations while processing audio blocks.
**Why bad:** Allocator may block, causing audio glitches in real-time scenarios. Bad habit even for offline processing.
**Instead:** Pre-allocate all buffers before the processing loop.

## Scalability Considerations

| Concern | Single Plugin | 10 Plugins | 100+ Plugins |
|---------|---------------|------------|--------------|
| Memory | ~50-200MB per plugin | Worker pool with limits | Lazy loading, LRU eviction |
| Processing | Single worker | Multiple workers, parallel chains | Worker pool with queue |
| Plugin scanning | Sequential, fast | Same | Cache aggressively, scan in background |
| Crash isolation | Single worker restart | Affected worker only | Same, with circuit breaker |

## Crate Organization

```
vst3-mcp-host/
  crates/
    protocol/          # Shared types: WorkerCommand, WorkerResponse, ParamInfo
    vst3-host/         # VST3 hosting logic (Layer 2+3), plugin scanning
    worker/            # Worker process binary
    supervisor/        # Supervisor + MCP server binary (main entry point)
  src/
    main.rs            # Delegates to supervisor crate
```

**Why a workspace?** Supervisor and worker are separate binaries. Protocol types must be shared between them. The vst3-host crate is a library used by the worker.

## Sources

- [VST3 SDK Architecture](https://steinbergmedia.github.io/vst3_dev_portal/)
- [coupler-rs/vst3-rs](https://github.com/coupler-rs/vst3-rs) -- COM abstraction patterns
- [rtrb for audio ring buffers](https://github.com/mgeier/rtrb)
- [Rust FFI patterns](https://rust-unofficial.github.io/patterns/patterns/ffi/intro.html)
- [rmcp tool/resource macros](https://github.com/modelcontextprotocol/rust-sdk)
