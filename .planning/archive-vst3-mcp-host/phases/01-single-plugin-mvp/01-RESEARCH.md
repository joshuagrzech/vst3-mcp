# Phase 1: Single-Plugin MVP - Research

**Researched:** 2026-02-15
**Domain:** VST3 hosting in Rust, offline audio processing, MCP server integration
**Confidence:** MEDIUM (coupler-rs/vst3 hosting patterns are uncharted; rmcp and audio I/O are well-documented)

## Summary

Phase 1 validates the core hypothesis: can Rust host VST3 plugins and expose audio processing through MCP tools? This is a single-process architecture (no supervisor-worker split yet) that combines the MCP server, plugin hosting, and audio pipeline in one binary. The primary risk -- SDK 3.8.0 compatibility with coupler-rs/vst3 -- is resolved (issue #20 closed November 2025), but the crate provides low-level COM bindings, not a hosting framework. We must build the hosting layer ourselves: plugin scanning, COM lifecycle management (RAII wrappers around ComPtr), the VST3 initialization state machine, block-based offline audio processing, and .vstpreset file I/O.

The MCP integration side is well-supported by rmcp (official Rust SDK, v0.15.0, actively maintained). Audio file I/O is handled by symphonia (decode multi-format) and hound (WAV encode). The main engineering challenge is the VST3 hosting layer -- there is no existing Rust crate that provides safe, high-level hosting APIs. We must wrap the raw COM bindings from the `vst3` crate into safe abstractions ourselves.

**Primary recommendation:** Start with VST3 hosting validation first (can we load a plugin and call process()?), then build outward to audio I/O and MCP integration. Use a known open-source plugin (e.g., Surge XT) for initial testing. Build the hosting layer in concentric safety rings: raw COM (vst3 crate) -> RAII wrappers -> safe public API.

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `vst3` (coupler-rs) | 0.3.0 | VST3 COM bindings | Only maintained, permissively-licensed (MIT/Apache-2.0) VST3 binding crate for Rust. Pre-generated bindings (no build-time C++ dependency). SDK 3.8.0 compatible after issue #20 fix (Nov 2025). Provides ComPtr/ComRef smart pointers for COM lifecycle. |
| `rmcp` | 0.15.0 | MCP server framework | Official Rust SDK from modelcontextprotocol org. Implements MCP protocol 2025-11-25. `#[tool_router]` + `#[tool]` macros for defining tools. Stdio transport via `stdio()` function. 54 releases, 3k stars, actively maintained. |
| `symphonia` | 0.5.x | Audio file decoding | Pure Rust, supports WAV/FLAC/MP3/OGG/AAC/AIFF. No C dependencies. Decode any input format to `AudioBufferRef`, convert to f32 via `SampleBuffer`. |
| `hound` | 3.5.x | WAV file writing | 7.5M+ downloads, simple API. Write f32 samples directly with `SampleFormat::Float` + `bits_per_sample: 32`. Reliable for WAV output. |
| `tokio` | 1.x | Async runtime | Required by rmcp. Use `spawn_blocking` for CPU-bound audio work. |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `serde` + `serde_json` | 1.x / 1.x | Serialization | Required by rmcp. Also for plugin catalog JSON, preset metadata. |
| `schemars` | 1.0.x | JSON Schema generation | Required by rmcp for tool parameter schemas. Use `#[derive(JsonSchema)]` on tool input structs. |
| `tracing` + `tracing-subscriber` | 0.1.x / 0.3.x | Structured logging | Essential for debugging VST3 COM interactions and audio pipeline. |
| `anyhow` | 1.x | Error handling (app) | Ergonomic error chains for the application binary. |
| `thiserror` | 2.x | Error handling (lib) | Typed errors for the hosting library layer. |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `vst3` (coupler-rs) | `vst3-sys` (RustAudio) | GPLv3 license, no versioned releases, plugin-dev focused |
| `vst3` (coupler-rs) | `jesnor/vst3-rs` | Wraps vst3-sys (inherits GPL), thin wrapper, less maintained |
| `symphonia` | `rodio` | rodio is playback-focused, not file I/O |
| `hound` | `wavers` | hound has 100x more downloads, battle-tested |
| `rmcp` | `rust-mcp-sdk` | Third-party, official SDK tracks protocol changes first |
| `rmcp` | `mcpkit` | Less mature, macro-based but smaller community |

**Installation:**
```bash
cargo add vst3
cargo add rmcp --features server,transport-io,macros
cargo add tokio --features full
cargo add serde --features derive
cargo add serde_json
cargo add schemars
cargo add symphonia --features all
cargo add hound
cargo add tracing
cargo add tracing-subscriber --features env-filter
cargo add anyhow
cargo add thiserror
```

## Architecture Patterns

### Recommended Project Structure (Phase 1: Single Binary)

```
src/
  main.rs              # Entry point: start MCP server over stdio
  server.rs            # MCP tool definitions (#[tool_router] impl)
  hosting/
    mod.rs             # Re-exports
    scanner.rs         # Plugin discovery: scan OS paths, read moduleinfo.json, build catalog
    module.rs          # VST3 module loading (dlopen the .so/.dylib/.dll)
    plugin.rs          # PluginInstance: safe wrapper around IComponent + IAudioProcessor + IEditController
    host_app.rs        # IHostApplication + IComponentHandler implementations (COM callbacks)
    types.rs           # PluginInfo, BusInfo, ParamInfo, PluginState enum
  audio/
    mod.rs             # Re-exports
    decode.rs          # symphonia-based input file decoding to f32 buffers
    encode.rs          # hound-based WAV output encoding
    process.rs         # Block-based offline processing loop (deinterleave -> process -> interleave)
    buffers.rs         # Pre-allocated audio buffer management
  preset/
    mod.rs             # Re-exports
    vstpreset.rs       # .vstpreset binary format read/write
    state.rs           # Plugin state save/restore via getState/setState
```

**Why this structure:** Separates hosting concerns (COM/VST3) from audio concerns (I/O, processing) and integration concerns (MCP). The hosting module is the complexity center -- it must contain all unsafe COM code. Audio and preset modules use only safe Rust.

### Pattern 1: VST3 Plugin Initialization State Machine

**What:** Encode the mandatory VST3 lifecycle as Rust types to prevent invalid call sequences.
**When to use:** Always -- calling VST3 methods in wrong order causes crashes.
**Confidence:** HIGH (from Steinberg VST3 Developer Portal)

```rust
// Enforce correct call ordering at the type level
pub struct Created {
    component: ComPtr<IComponent>,
    controller: ComPtr<IEditController>,
}

pub struct SetupDone {
    component: ComPtr<IComponent>,
    controller: ComPtr<IEditController>,
    processor: ComPtr<IAudioProcessor>,
}

pub struct Active {
    inner: SetupDone,
}

pub struct Processing {
    inner: Active,
}

impl Created {
    /// Call setupProcessing + activateBus -> transition to SetupDone
    pub fn setup(self, sample_rate: f64, max_block_size: i32) -> Result<SetupDone> { ... }
}

impl SetupDone {
    /// Call setActive(true) -> transition to Active
    pub fn activate(self) -> Result<Active> { ... }
}

impl Active {
    /// Call setProcessing(true) -> transition to Processing
    pub fn start_processing(self) -> Result<Processing> { ... }
}

impl Processing {
    /// Actually call process() with audio buffers
    pub fn process(&mut self, input: &[&[f32]], output: &mut [&mut [f32]], block_size: usize) -> Result<()> { ... }

    /// Reverse teardown: setProcessing(false) -> setActive(false)
    pub fn stop(self) -> Result<SetupDone> { ... }
}
```

**Source:** [VST3 Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html) -- "setProcessing can be called without intervening process() calls" and the required sequence: `setupProcessing -> setActive(true) -> setProcessing(true) -> process()`

### Pattern 2: Block-Based Offline Processing

**What:** Process audio in fixed-size blocks, matching the plugin's declared max block size.
**When to use:** For all offline audio rendering.
**Confidence:** HIGH (standard VST3 pattern)

```rust
pub fn render_offline(
    plugin: &mut Processing,     // Already in Processing state
    input_channels: &[Vec<f32>], // Deinterleaved: one Vec per channel
    output_channels: &mut [Vec<f32>],
    max_block_size: usize,
) -> Result<()> {
    let total_frames = input_channels[0].len();
    let mut offset = 0;

    while offset < total_frames {
        let block_frames = (total_frames - offset).min(max_block_size);

        // Build per-channel slices for this block
        let input_slices: Vec<&[f32]> = input_channels.iter()
            .map(|ch| &ch[offset..offset + block_frames])
            .collect();
        let mut output_slices: Vec<&mut [f32]> = output_channels.iter_mut()
            .map(|ch| &mut ch[offset..offset + block_frames])
            .collect();

        plugin.process(&input_slices, &mut output_slices, block_frames)?;
        offset += block_frames;
    }

    Ok(())
}
```

### Pattern 3: MCP Tool Definitions with rmcp

**What:** Define MCP tools using rmcp macros that map to hosting operations.
**When to use:** For all MCP-exposed functionality.
**Confidence:** HIGH (verified from rmcp README and Shuttle tutorial)

```rust
use rmcp::prelude::*;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScanPluginsRequest {
    #[schemars(description = "Optional directory path to scan instead of default locations")]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LoadPluginRequest {
    #[schemars(description = "Plugin UID from scan results")]
    pub uid: String,
    #[schemars(description = "Sample rate for processing (default: from input file)")]
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProcessAudioRequest {
    #[schemars(description = "Path to input audio file")]
    pub input_file: String,
    #[schemars(description = "Path for output WAV file")]
    pub output_file: String,
}

#[tool_router]
impl AudioHost {
    #[tool(description = "Scan for installed VST3 plugins and return a list with UIDs")]
    async fn scan_plugins(
        &self,
        Parameters(req): Parameters<ScanPluginsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let plugins = tokio::task::spawn_blocking(move || {
            // Plugin scanning is I/O + CPU bound
            scanner::scan_plugins(req.path.as_deref())
        }).await.map_err(|e| McpError::internal(e.to_string()))?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&plugins).unwrap()
        )]))
    }

    #[tool(description = "Load a VST3 plugin by its UID from scan results")]
    async fn load_plugin(
        &self,
        Parameters(req): Parameters<LoadPluginRequest>,
    ) -> Result<CallToolResult, McpError> { ... }

    #[tool(description = "Process an audio file through the loaded plugin")]
    async fn process_audio(
        &self,
        Parameters(req): Parameters<ProcessAudioRequest>,
    ) -> Result<CallToolResult, McpError> { ... }

    #[tool(description = "Save the current plugin state as a .vstpreset file")]
    async fn save_preset(
        &self,
        Parameters(req): Parameters<SavePresetRequest>,
    ) -> Result<CallToolResult, McpError> { ... }

    #[tool(description = "Load a .vstpreset file into the current plugin")]
    async fn load_preset(
        &self,
        Parameters(req): Parameters<LoadPresetRequest>,
    ) -> Result<CallToolResult, McpError> { ... }
}

#[tool_handler]
impl ServerHandler for AudioHost {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("VST3 audio processing host. Scan for plugins, load one, process audio files through it, and manage presets.".into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::init();
    let host = AudioHost::new();
    let service = host.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

### Pattern 4: COM RAII Wrappers (Safety Layer)

**What:** Wrap raw COM pointers from the vst3 crate in safe Rust types.
**When to use:** All VST3 COM interactions must go through wrappers.
**Confidence:** MEDIUM (pattern is sound; exact vst3 crate API needs validation during implementation)

```rust
use vst3::com::ComPtr;
use vst3::Steinberg::Vst::{IComponent, IAudioProcessor, IEditController};
use vst3::Steinberg::{IPluginFactory, FUnknown};

/// Safe wrapper around a loaded VST3 plugin instance
pub struct PluginInstance {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: ComPtr<IEditController>,
    state: PluginState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PluginState {
    Created,
    SetupDone,
    Active,
    Processing,
}

impl PluginInstance {
    /// Load a plugin from a factory by class ID
    pub fn from_factory(
        factory: &ComPtr<IPluginFactory>,
        class_id: &[u8; 16],
        host_context: &ComPtr<FUnknown>,
    ) -> Result<Self> {
        // 1. Create component via factory.createInstance()
        // 2. Initialize component with host context
        // 3. Query IAudioProcessor from component
        // 4. Create or query IEditController
        // 5. Connect component <-> controller via IConnectionPoint
        todo!("Implementation depends on exact vst3 crate API")
    }
}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        // Ensure proper teardown order:
        // setProcessing(false) -> setActive(false) -> terminate()
        // ComPtr handles Release automatically
    }
}
```

### Anti-Patterns to Avoid

- **Processing audio on the tokio runtime:** Audio processing is CPU-bound. Always use `tokio::task::spawn_blocking`. Blocking tokio worker threads starves all MCP request handling.
- **Sharing plugin instances across threads:** VST3 COM objects have thread affinity. Do NOT wrap in `Arc<Mutex<>>`. Keep each plugin instance on a single thread.
- **Calling COM methods outside wrappers:** All unsafe COM calls must live in the hosting module. Business logic (MCP handlers, audio pipeline) only touches safe APIs.
- **Allocating in the processing loop:** Pre-allocate all audio buffers before the block processing loop. No Vec::new(), String creation, or other heap allocation inside the hot loop.
- **Interleaved/deinterleaved confusion:** VST3 uses planar (deinterleaved) buffers -- one float slice per channel. symphonia can output interleaved via SampleBuffer. hound writes interleaved. Build explicit conversion functions and unit test them with known signals.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Audio file decoding | Custom WAV/FLAC/MP3 parser | `symphonia` with `--features all` | Format quirks, metadata handling, codec edge cases |
| WAV file encoding | Custom WAV writer | `hound` with `SampleFormat::Float` | WAV spec has subtleties (RF64, extensible format) |
| MCP protocol handling | Custom JSON-RPC over stdio | `rmcp` with `server,transport-io,macros` features | Protocol compliance, schema generation, transport handling |
| JSON schema generation | Manual schema construction | `schemars` derive macro | rmcp requires JsonSchema-compatible types |
| COM reference counting | Manual AddRef/Release calls | `ComPtr`/`ComRef` from vst3 crate | One missed Release = leak, one extra = use-after-free |
| .vstpreset binary format | Fully custom binary parser | Build minimal reader/writer following Steinberg spec | Format is simple enough (48B header + chunks) but must match spec exactly for DAW interop |

**Key insight:** The VST3 hosting layer IS the hand-built part. Everything around it (audio I/O, MCP protocol, serialization) should use existing crates. Focus engineering effort on the hosting layer where no crate exists.

## Common Pitfalls

### Pitfall 1: VST3 COM Lifecycle Mismanagement
**What goes wrong:** Missing Release leaks plugins. Double-Release causes use-after-free. Wrong initialization order causes segfaults.
**Why it happens:** Rust ownership doesn't map to COM reference counting. The vst3 crate provides ComPtr but the hosting sequence (factory -> component -> processor -> controller connections) must be done manually and correctly.
**How to avoid:** Never expose raw COM pointers outside the hosting module. Use ComPtr exclusively. Implement Drop on PluginInstance to enforce teardown order. Test with multiple plugins -- each vendor implements COM differently.
**Warning signs:** Valgrind errors, sporadic crashes on plugin unload, memory growth over time.

### Pitfall 2: Forgetting the Activation State Machine
**What goes wrong:** Calling `process()` without `setProcessing(true)` and `setActive(true)`. Some plugins produce silence, others crash.
**Why it happens:** Easy to forget when prototyping. The required sequence is: `initialize() -> setupProcessing() -> activateBus() -> setActive(true) -> setProcessing(true) -> process()`.
**How to avoid:** Encode states as Rust types (see Pattern 1). Only the `Processing` state type has a `process()` method. Compile-time enforcement.
**Warning signs:** Silent audio output, plugin crashes on first process() call.

### Pitfall 3: Parameter Changes Must Go Through process()
**What goes wrong:** Setting parameters via `IEditController::setParamNormalized()` but the processor never sees the change. Audio output is unchanged.
**Why it happens:** VST3 spec requires parameter changes to reach the processor ONLY through `IParameterChanges` in the `process()` call's ProcessData. The controller setter is for UI display, not processor state.
**How to avoid:** Build a parameter change queue. When MCP tool sets a parameter, enqueue it. On next `process()` call, deliver all queued changes via `IParamValueQueue` with sample offset 0.
**Warning signs:** Parameter changes appear to do nothing. Plugin ignores set_parameter calls.

### Pitfall 4: Audio Buffer Layout Mismatch
**What goes wrong:** Passing interleaved audio to VST3 (which expects planar/deinterleaved) or vice versa. Audio becomes garbage.
**Why it happens:** symphonia's `SampleBuffer::copy_interleaved_ref()` outputs interleaved. VST3 wants one `f32*` per channel. hound writes interleaved.
**How to avoid:** Build explicit `deinterleave(interleaved, channels) -> Vec<Vec<f32>>` and `interleave(planar) -> Vec<f32>` functions. Unit test with known signals (sine wave round-trip).
**Warning signs:** Garbage audio output, channel swapping, distortion that shouldn't be there.

### Pitfall 5: Sample Rate Mismatch Between Input and Plugin
**What goes wrong:** Decoding a 48kHz file but initializing the plugin at 44100Hz (or some default). Pitch shifts, timing errors.
**Why it happens:** Forgetting to query the input file's sample rate before plugin setup, or hardcoding a sample rate.
**How to avoid:** Decode the input file header FIRST to get its sample rate. Pass that rate to `setupProcessing()`. If the plugin needs a different rate, resample (consider `rubato` crate), but for Phase 1, just match the input rate.
**Warning signs:** Output sounds pitched up or down. Duration changes.

### Pitfall 6: Controller-Processor State Desync After Preset Load
**What goes wrong:** Loading a preset via `IComponent::setState()` but forgetting to call `IEditController::setComponentState()` with the same data. Controller reports stale parameter values.
**Why it happens:** The spec requires two separate calls to sync both halves. Easy to miss.
**How to avoid:** After `component.setState(stream)`, always call `controller.setComponentState(stream)` with the SAME data. Then rescan parameter values.
**Warning signs:** Parameters read back stale values after preset load. Plugin behaves as if preset loaded but MCP reports old values.

### Pitfall 7: Blocking the Async Runtime with Audio Processing
**What goes wrong:** Running symphonia decode, plugin process(), or hound encode inside an `async fn` MCP tool handler without spawn_blocking. All other MCP requests hang.
**Why it happens:** Natural to just `await` everything in async context. Audio processing is CPU-bound, not I/O-bound.
**How to avoid:** Wrap ALL audio/hosting work in `tokio::task::spawn_blocking`. The MCP handler should only do lightweight coordination and then delegate to a blocking thread.
**Warning signs:** MCP client timeouts. Second tool call hangs while first is processing.

## Code Examples

### Decoding Audio Input with symphonia

```rust
// Source: symphonia docs + GETTING_STARTED.md
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct DecodedAudio {
    pub samples: Vec<f32>,       // Interleaved f32 samples
    pub channels: usize,
    pub sample_rate: u32,
    pub total_frames: usize,
}

pub fn decode_audio_file(path: &std::path::Path) -> Result<DecodedAudio> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;

    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow::anyhow!("no audio track"))?;
    let track_id = track.id;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())?;

    let mut all_samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };

        if packet.track_id() != track_id { continue; }

        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        let duration = decoded.capacity() as u64;

        let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
        sample_buf.copy_interleaved_ref(decoded);
        all_samples.extend_from_slice(sample_buf.samples());
    }

    let total_frames = all_samples.len() / channels;

    Ok(DecodedAudio {
        samples: all_samples,
        channels,
        sample_rate,
        total_frames,
    })
}
```

### Encoding WAV Output with hound

```rust
// Source: hound docs
use hound::{WavSpec, WavWriter, SampleFormat};

pub fn write_wav(
    path: &std::path::Path,
    samples: &[f32],        // Interleaved
    channels: u16,
    sample_rate: u32,
) -> Result<()> {
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };

    let mut writer = WavWriter::create(path, spec)?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(())
}
```

### Plugin Scanning (Conceptual)

```rust
// Source: Steinberg Plugin Locations docs + moduleinfo.json spec
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub vendor: String,
    pub uid: String,          // Class ID as hex string
    pub category: String,     // e.g., "Fx|EQ", "Fx|Reverb"
    pub version: String,
    pub path: PathBuf,        // Path to .vst3 bundle
}

pub fn default_scan_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".vst3"));
        }
        paths.push(PathBuf::from("/usr/lib/vst3"));
        paths.push(PathBuf::from("/usr/local/lib/vst3"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
        }
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(local).join("Programs/Common/VST3"));
        }
        paths.push(PathBuf::from("C:/Program Files/Common Files/VST3"));
    }

    paths
}

pub fn scan_plugins(custom_path: Option<&str>) -> Vec<PluginInfo> {
    let paths = match custom_path {
        Some(p) => vec![PathBuf::from(p)],
        None => default_scan_paths(),
    };

    let mut plugins = Vec::new();
    for scan_path in paths {
        // Walk directory recursively looking for .vst3 bundles
        // For each bundle:
        //   1. Try reading Contents/Resources/moduleinfo.json (fast path, no binary load)
        //   2. If no moduleinfo.json, load the module and query IPluginFactory
        //   3. Extract class info: CID, name, vendor, category, version
        //   4. Build PluginInfo and append
    }
    plugins
}
```

### .vstpreset Binary Format

```rust
// Source: Steinberg Preset Format specification
// Header: 48 bytes total
//   "VST3"          - 4 bytes magic
//   version         - 4 bytes i32 (little-endian, currently 1)
//   class_id        - 32 bytes ASCII (hex-encoded class ID)
//   chunk_list_off  - 8 bytes i64 (offset to chunk list from file start)
//
// Data area: variable length
//   Processor state chunk ("Comp"): raw bytes from IComponent::getState()
//   Controller state chunk ("Cont"): raw bytes from IEditController::getState()
//
// Chunk list (at chunk_list_off):
//   "List"          - 4 bytes magic
//   count           - 4 bytes i32 (number of chunks)
//   entries[]:
//     chunk_id      - 4 bytes (e.g., "Comp" or "Cont")
//     offset        - 8 bytes i64 (from file start)
//     size          - 8 bytes i64

pub struct VstPresetHeader {
    pub version: i32,
    pub class_id: [u8; 32],      // ASCII hex
    pub chunk_list_offset: i64,
}

pub struct ChunkEntry {
    pub id: [u8; 4],             // "Comp" or "Cont"
    pub offset: i64,
    pub size: i64,
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| VST3 SDK GPLv3 license | VST3 SDK MIT license | Oct 2025 (SDK 3.8.0) | Removes licensing friction for host development |
| vst3-sys (RustAudio) as only Rust option | coupler-rs/vst3 with pre-generated bindings | 2024-2025 | No C++ build dependency, permissive license |
| Custom MCP protocol handling | rmcp official SDK | 2025 | Standard protocol compliance, tool macros |
| vst3 crate incompatible with SDK 3.8.0 | Issue #20 fixed (forward-declared extern types) | Nov 2025 | SDK 3.8.0 now works with coupler-rs bindings |

**Deprecated/outdated:**
- `vst3-sys` (RustAudio): GPLv3, no versioned crates.io releases, plugin-dev focused
- `vst-rs` (RustAudio): VST2 only, VST2 format deprecated by Steinberg
- Manual MCP JSON-RPC: Use rmcp SDK instead

## Open Questions

1. **Exact vst3 crate hosting API surface**
   - What we know: ComPtr/ComRef exist, IComponent/IAudioProcessor/IEditController bindings exist, SDK 3.8.0 compatible
   - What's unclear: Exact API for loading a .vst3 module (dlopen equivalent), creating instances from IPluginFactory, the precise Rust signatures for process() with ProcessData
   - Recommendation: First task should be a spike: `cargo add vst3`, write a minimal program that loads one plugin and calls process(). Validate the API before building full abstractions. LOW confidence on exact API shapes.

2. **IParameterChanges implementation for parameter delivery**
   - What we know: Parameters must be delivered through process() call, not via controller setters. Need to implement IParameterChanges and IParamValueQueue COM interfaces.
   - What's unclear: Does the vst3 crate provide helper types for this, or must we implement the COM interfaces ourselves from scratch?
   - Recommendation: Check vst3 crate docs for any host-side helpers. If none exist, we must implement these COM interfaces using the crate's `Class` trait and `ComWrapper`.

3. **IHostApplication and IComponentHandler COM implementation**
   - What we know: Plugins require these host-side interfaces. IHostApplication is passed during initialize(). IComponentHandler handles parameter edit callbacks.
   - What's unclear: Exact pattern for implementing COM interfaces in Rust with the coupler-rs vst3 crate's `Class` trait
   - Recommendation: Study the vst3 crate's `Class` trait docs and any examples. This is required infrastructure -- cannot skip it.

4. **Hidden message loop for headless operation**
   - What we know: ~5% of plugins render silence without a platform message loop, even headless. JUCE forums document this issue.
   - What's unclear: Whether this applies to Linux specifically, and what the minimal message loop looks like without JUCE
   - Recommendation: Defer to Phase 2 or later. Phase 1 targets well-behaved plugins. Document which plugins fail silently and investigate message loop if needed.

5. **Plugin tail handling for effects with reverb/delay**
   - What we know: Plugins report tail length via `getTailSamples()`. Host should continue calling `process()` with silent input after the audio file ends.
   - What's unclear: How to handle `kInfiniteTail` (generator plugins) in offline mode
   - Recommendation: Implement basic tail handling (query tail length, feed silence for that many samples after input ends). For kInfiniteTail, use a configurable timeout (e.g., 30 seconds of silence).

6. **rmcp tool parameter extraction pattern**
   - What we know: Tools use `Parameters<T>` wrapper with `#[derive(Deserialize, JsonSchema)]` structs. The `#[tool]` macro handles routing.
   - What's unclear: Exact version to pin (README shows 0.8.0 but releases show 0.15.0), whether schemars v1.0 or v0.8 is needed
   - Recommendation: Use git dependency initially to get latest, then pin to crates.io version once confirmed working. Check schemars version compatibility with rmcp.

## Sources

### Primary (HIGH confidence)
- [Steinberg VST3 Developer Portal](https://steinbergmedia.github.io/vst3_dev_portal/) -- plugin lifecycle, processing FAQ, preset format, plugin locations
- [Steinberg VST3 Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html) -- initialization sequence, offline mode, block processing
- [Steinberg Preset Format](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Preset+Format.html) -- .vstpreset binary specification
- [Steinberg Plugin Locations](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Plugin+Locations.html) -- OS-specific scan paths
- [Steinberg Plugin Format Structure](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Plugin+Format.html) -- bundle directory layout
- [Steinberg ModuleInfo JSON](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+Module+Architecture/ModuleInfo-JSON.html) -- fast metadata extraction
- [rmcp official Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) -- MCP server framework, tool macros, stdio transport
- [rmcp README](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/README.md) -- API examples, ServerHandler pattern
- [coupler-rs/vst3-rs](https://github.com/coupler-rs/vst3-rs) -- Rust VST3 bindings, ComPtr/ComRef
- [coupler-rs/vst3-rs issue #20](https://github.com/coupler-rs/vst3-rs/issues/20) -- SDK 3.8.0 compatibility (RESOLVED, Nov 2025)
- [symphonia](https://github.com/pdeljanov/Symphonia) -- audio decoding
- [hound](https://github.com/ruuda/hound) -- WAV encoding

### Secondary (MEDIUM confidence)
- [Shuttle: Build a stdio MCP Server in Rust](https://www.shuttle.dev/blog/2025/07/18/how-to-build-a-stdio-mcp-server-in-rust) -- rmcp tutorial with full code example
- [MCPcat: Building MCP Server Rust](https://mcpcat.io/guides/building-mcp-server-rust/) -- rmcp patterns
- [Steinberg IComponentHandler Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IComponentHandler.html) -- required host callbacks
- [Steinberg IHostApplication Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IHostApplication.html) -- host identity interface
- [JUCE Headless VST3 silence issue](https://forum.juce.com/t/headless-vst3-host-some-plugins-render-silence/58169) -- message loop gotcha

### Tertiary (LOW confidence)
- [KVR Forum: CLI VST3 host in Rust](https://www.kvraudio.com/forum/viewtopic.php?t=622780) -- community experience (could not fetch, 403)
- [Renaud Denis: Robust VST3 Host for Rust](https://renauddenis.com/case-studies/rust-vst) -- cutoff-vst case study (proprietary, not available as crate)
- vst3 crate exact hosting API patterns -- LOW confidence until validated by implementation spike

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all crates verified on crates.io/GitHub, versions confirmed, features documented
- Architecture (project structure, patterns): MEDIUM -- patterns are sound but exact vst3 crate hosting API needs validation
- VST3 hosting layer: MEDIUM-LOW -- no existing Rust hosting framework exists; must build from raw COM bindings; exact API ergonomics unknown until spike
- MCP integration: HIGH -- rmcp well-documented, multiple tutorials available, macros simplify tool definition
- Audio I/O: HIGH -- symphonia and hound are mature, well-documented crates
- Preset format: MEDIUM -- binary spec documented by Steinberg, but implementing in Rust from scratch needs care
- Pitfalls: HIGH -- well-documented across Steinberg docs, JUCE forums, and KVR community

**Research date:** 2026-02-15
**Valid until:** 2026-03-15 (30 days -- stack is stable, but monitor rmcp for breaking changes given active release cadence)
