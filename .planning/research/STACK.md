# Technology Stack

**Project:** AgentAudio (VST3 Wrapper Plugin with Embedded MCP Server)
**Researched:** 2026-02-15

## Recommended Stack

### Core Plugin Framework

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| nih-plug | git (master) | VST3 wrapper plugin framework | Only serious Rust VST3/CLAP framework with active maintenance (last commit 2025-02-23). Handles plugin lifecycle, parameter system, buffer management, real-time safety patterns. Not published to crates.io -- git dependency only. | HIGH |
| nih-plug-egui | git (master, same repo) | Wrapper plugin GUI | Integrates egui 0.31 with nih-plug's parameter system. Provides immediate-mode UI for Focus Mode controls. Ships as workspace member of nih-plug. | HIGH |

**nih-plug Cargo.toml pattern:**
```toml
[dependencies]
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git", features = ["assert_process_allocs"] }
nih_plug_egui = { git = "https://github.com/robbert-vdh/nih-plug.git" }
```

**Critical note:** nih-plug is for **building** the wrapper plugin itself. It handles the VST3 export, audio callbacks, parameter declarations, and GUI windowing. It does NOT handle loading/hosting child VST3 plugins -- that requires separate hosting code (see below).

### Child Plugin Hosting (VST3 Loading)

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| vst3 | 0.3.0 | VST3 API bindings for hosting child plugins | Pure Rust bindings to VST3 COM interfaces. As of 0.3.0, bindings are pre-generated (no libclang/SDK needed at build time). Provides IComponent, IAudioProcessor, IEditController interfaces needed to load, initialize, and control child plugins. Already used in current codebase. | MEDIUM |
| libloading | 0.9.0 | Dynamic library loading | Loads .vst3 bundle's shared library (.so on Linux) at runtime. Cross-platform dlopen/LoadLibrary wrapper. Already used in current codebase. | HIGH |

**Why NOT vst3-sys:** nih-plug internally uses a fork of vst3-sys (from robbert-vdh's branch). Using the upstream vst3-sys alongside nih-plug would create COM type conflicts. The `vst3` crate (0.3.0, by coupler-rs) is a separate, cleaner binding generator that can coexist. However, there is a risk of type incompatibility between nih-plug's internal vst3-sys types and the `vst3` crate's types when passing COM objects across the boundary.

**Why NOT rack (0.4.8):** The `rack` crate provides high-level plugin hosting but its VST3 support on Linux is listed as "untested" with "no GUI yet." It is not mature enough for production use on Linux, and its abstractions may conflict with nih-plug's own VST3 wrapper layer. The architecture needs direct COM-level control for the plugin-in-plugin pattern.

**Hosting architecture concern (MEDIUM confidence):** The wrapper plugin (built with nih-plug) needs to load a child VST3 plugin using raw COM interfaces. This means:
1. Use `libloading` to load the .vst3 bundle's .so file
2. Call the VST3 module entry point (`GetPluginFactory`)
3. Create IComponent and IAudioProcessor instances via COM
4. Forward audio buffers from nih-plug's `process()` to child's `process()`
5. Create IEditController for parameter access
6. Optionally create IPlugView for child GUI embedding

This is essentially writing a minimal VST3 host inside a VST3 plugin. The `vst3` crate provides the raw interfaces but no hosting abstractions -- all lifecycle management is manual.

### Async Runtime & MCP Server

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| tokio | 1.49.0 | Async runtime for MCP server | Required by rmcp. Runs on a background thread, NOT on the audio thread. Create runtime with `tokio::runtime::Runtime::new()` on a dedicated `std::thread`, then use `rt.block_on()` to drive the MCP server. | HIGH |
| rmcp | 0.15.0 | MCP protocol SDK (server) | Official Rust MCP SDK from modelcontextprotocol org. Provides `#[tool]` macro for defining MCP tools, transport abstractions. Already used in current codebase. | HIGH |

**Transport decision:** Use `transport-streamable-http-server` feature with axum for HTTP-based MCP. This allows Claude Desktop or any MCP client to connect over HTTP to the plugin's embedded server. The stdio transport is unsuitable because the plugin runs inside a DAW process (no stdin/stdout access).

```toml
[dependencies]
rmcp = { version = "0.15.0", features = ["server", "transport-streamable-http-server", "macros"] }
tokio = { version = "1.49.0", features = ["full"] }
axum = "0.8"
```

**Tokio-in-plugin feasibility (HIGH confidence):** Spawning a Tokio runtime on a background thread inside a VST3 plugin is viable. The pattern:
1. In nih-plug's `initialize()`, spawn `std::thread::spawn` with a new Tokio runtime
2. The Tokio runtime runs the MCP HTTP server (axum + rmcp)
3. Communication with the audio thread happens via lock-free queues (NOT via tokio channels)
4. On plugin `deactivate()` / drop, signal the runtime to shut down gracefully

**The audio thread NEVER touches tokio.** Tokio lives entirely on the MCP background thread. The audio thread only reads/writes lock-free ring buffers.

### Lock-Free Communication

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| rtrb | 0.3.2 | SPSC ring buffer for MCP-to-audio commands | Wait-free (not just lock-free), designed specifically for real-time audio. Single-producer single-consumer. MCP thread produces parameter change commands, audio thread consumes them. Originated from a rejected crossbeam PR -- the implementation was considered excellent. | HIGH |
| atomic_float | (via nih-plug) | Atomic f32/f64 for parameter state | nih-plug already depends on this. Use for sharing current parameter values between threads without locks. | HIGH |

**Why rtrb over crossbeam-channel:** crossbeam-channel uses exponential backoff spinning which is not real-time safe. rtrb is wait-free SPSC -- the producer and consumer never block each other, and neither side ever spins. This is the correct choice for audio thread communication.

**Why NOT crossbeam (0.8.4):** crossbeam's channels are MPMC and use internal synchronization that can cause priority inversion on real-time threads. For the SPSC pattern (MCP thread sends commands, audio thread reads commands), rtrb is simpler and provably wait-free.

**Communication pattern:**
```
MCP Thread (tokio) --[rtrb Producer]--> Audio Thread (nih-plug process())
Audio Thread --[rtrb Producer]--> MCP Thread (parameter state updates)
```
Two separate rtrb channels: one for commands (MCP->audio), one for state (audio->MCP).

### Serialization & Error Handling

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| serde | 1.0.228 | Serialization framework | Required by rmcp for MCP protocol messages. Also useful for parameter state serialization. | HIGH |
| serde_json | 1.0.149 | JSON serialization | MCP protocol uses JSON-RPC. Required by rmcp. | HIGH |
| schemars | 1.2.1 | JSON Schema generation | Required by rmcp's `#[tool]` macro to generate parameter schemas for MCP tools. | HIGH |
| anyhow | 1.0.101 | Error handling (application code) | Simple error handling for non-library code. Good for the MCP server layer. | HIGH |
| thiserror | 2.0.18 | Error handling (library code) | Derive macro for custom error types. Good for the hosting layer where structured errors matter. | HIGH |

### Logging & Diagnostics

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| tracing | 0.1.44 | Structured logging | Industry standard for Rust async tracing. Works well with tokio. Critical for debugging the multi-threaded plugin. | HIGH |
| tracing-subscriber | 0.3.22 | Log output formatting | Use `env-filter` feature for runtime log level control. Write to file since plugin has no console. | HIGH |
| nih_log | (via nih-plug) | Plugin-side logging | nih-plug provides its own logging macro that writes to stderr (captured by some DAWs). Use for audio-thread diagnostics. | MEDIUM |

### GUI (Child Plugin Display)

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| raw-window-handle | 0.6.x | Window handle abstraction | Required to pass parent window handle to child plugin's IPlugView for GUI embedding. nih-plug-egui provides the parent window; child plugin needs the raw handle to attach its editor. | MEDIUM |
| x11 / xcb bindings | TBD | Linux window embedding | On Linux (X11), embedding a child plugin's GUI requires creating an X11 child window and passing the XID to the child's IPlugView::attached(). May need `x11` or `xcb` crate for window management. | LOW |

**Child GUI embedding is the hardest part.** IPlugView::attached() expects a platform-specific parent window handle. On Linux with X11, this means creating a sub-window in the wrapper's window and passing it to the child. nih-plug-egui uses its own window, and you need to extract the raw handle from it. This needs phase-specific research.

## Crates to Remove from Current Cargo.toml

The current project has crates from the pre-pivot headless host that are no longer needed:

| Crate | Why Remove |
|-------|-----------|
| hound | WAV file I/O -- no longer doing offline processing |
| symphonia | Audio file decoding -- no longer doing offline processing |

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Plugin framework | nih-plug (git) | JUCE (C++) | Rust-native is the goal; JUCE requires C++ FFI and different toolchain |
| Plugin framework | nih-plug (git) | beamer (0.2.3) | Much newer/less mature than nih-plug, smaller community, less documentation |
| VST3 bindings | vst3 (0.3.0) | vst3-sys (git) | nih-plug uses its own vst3-sys fork internally; using upstream vst3-sys risks COM type conflicts |
| VST3 hosting | Manual (vst3 + libloading) | rack (0.4.8) | VST3 on Linux is untested, no GUI support, abstractions may conflict with nih-plug |
| Ring buffer | rtrb (0.3.2) | crossbeam-channel (0.5.x) | crossbeam uses spinning/backoff, not wait-free, unsuitable for real-time audio thread |
| Ring buffer | rtrb (0.3.2) | ringbuf (various) | rtrb has strongest real-time audio pedigree, originated from crossbeam review |
| MCP SDK | rmcp (0.15.0) | rust-mcp-sdk | rmcp is the official SDK from modelcontextprotocol org |
| MCP transport | Streamable HTTP | stdio | Plugin runs inside DAW process -- no stdin/stdout available |
| GUI framework | nih-plug-egui | nih-plug-vizia, nih-plug-iced | egui has largest Rust community, simplest API, already declared as project constraint |

## Installation

```toml
[package]
name = "agent-audio"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
# Plugin framework (git dependency, not on crates.io)
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git", features = ["assert_process_allocs"] }
nih_plug_egui = { git = "https://github.com/robbert-vdh/nih-plug.git" }

# Child plugin hosting
vst3 = "0.3.0"
libloading = "0.9.0"

# MCP server
rmcp = { version = "0.15.0", features = ["server", "transport-streamable-http-server", "macros"] }
tokio = { version = "1.49.0", features = ["full"] }
axum = "0.8"

# Lock-free communication
rtrb = "0.3.2"

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
schemars = "1.2"

# Error handling
anyhow = "1.0"
thiserror = "2.0"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

**Build note:** nih-plug plugins compile as `cdylib`. Use `cargo nih-plug bundle <name> --release` to create the .vst3 bundle directory structure. Install `cargo-nih-plug` via:
```bash
cargo install --git https://github.com/robbert-vdh/nih-plug.git cargo_nih_plug
```

## Key Compatibility Concerns

### 1. nih-plug's vst3-sys vs vst3 crate (MEDIUM risk)
nih-plug internally uses a forked vst3-sys for its VST3 wrapper. The hosting code uses the `vst3` crate (0.3.0) for child plugin COM interfaces. These are separate type hierarchies -- a `vst3::Steinberg::Vst::IComponent` is NOT the same Rust type as `vst3_sys::IComponent`. This is fine as long as you never need to pass COM objects between the two layers. The wrapper plugin (nih-plug) and the hosted child plugin (vst3 crate) operate through separate COM interfaces anyway.

### 2. GPL licensing (MEDIUM concern)
Both nih-plug's vst3-sys fork and the upstream vst3-sys are GPLv3. The `vst3` crate (0.3.0) generates its own bindings but is also derived from the VST3 SDK headers. The project is planned as open source, so GPL is acceptable, but worth noting for distribution.

### 3. Tokio runtime lifecycle (LOW risk)
The Tokio runtime must be created after the plugin is loaded and shut down before the plugin is unloaded. Use nih-plug's `initialize()` to start and rely on `Drop` to shut down. The runtime runs on its own OS thread, completely decoupled from the audio thread.

### 4. egui version alignment (LOW risk)
nih-plug-egui currently bundles egui 0.31. If you add direct egui dependencies, they MUST match this version exactly or you get type conflicts. Always use the egui re-exported from nih-plug-egui, never add `egui` as a separate dependency.

## Sources

- [nih-plug GitHub repository](https://github.com/robbert-vdh/nih-plug) -- HIGH confidence
- [nih-plug CHANGELOG](https://github.com/robbert-vdh/nih-plug/blob/master/CHANGELOG.md) -- egui 0.31 update confirmed 2025-02-23
- [nih-plug Cargo.toml](https://github.com/robbert-vdh/nih-plug/blob/master/Cargo.toml) -- workspace structure, vst3-sys fork dependency
- [nih-plug background tasks issue #172](https://github.com/robbert-vdh/nih-plug/issues/172) -- background thread patterns
- [vst3 crate on crates.io](https://crates.io/crates/vst3) -- version 0.3.0 confirmed via `cargo search`
- [vst3-sys GitHub (RustAudio)](https://github.com/RustAudio/vst3-sys) -- GPLv3 license, raw COM bindings
- [rtrb GitHub](https://github.com/mgeier/rtrb) -- version 0.3.2 confirmed via `cargo search`, wait-free SPSC
- [rtrb Rust Audio announcement](https://rust-audio.discourse.group/t/announcement-real-time-ring-buffer-rtrb/346) -- design rationale
- [rmcp GitHub (official Rust MCP SDK)](https://github.com/modelcontextprotocol/rust-sdk) -- transport features documented
- [rmcp on crates.io](https://crates.io/crates/rmcp) -- version 0.15.0 confirmed via `cargo search`
- [Streamable HTTP MCP in Rust (Shuttle blog)](https://www.shuttle.dev/blog/2025/10/29/stream-http-mcp) -- axum + rmcp pattern
- [rack crate GitHub](https://github.com/sinkingsugar/rack) -- VST3 Linux untested, no GUI
- [Renaud Denis: Robust VST3 Host for Rust](https://renauddenis.com/case-studies/rust-vst) -- cutoff-vst hosting architecture
- [Tokio bridging with sync code](https://tokio.rs/tokio/topics/bridging) -- std::thread + Runtime::new() pattern
- [libloading crate](https://docs.rs/libloading/latest/libloading/) -- version 0.9.0 confirmed via `cargo search`
