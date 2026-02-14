# Technology Stack

**Project:** Headless VST3 Host as MCP Server
**Researched:** 2026-02-14

## Recommended Stack

### VST3 Bindings

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `vst3` (coupler-rs) | 0.3.0 | VST3 COM bindings | Generated from C++ headers, MIT/Apache-2.0 dual license, actively maintained by coupler-rs. Bindings now ship pre-generated (no build-time C++ dependency). More ergonomic COM abstractions than raw vst3-sys. |

**Why not vst3-sys (RustAudio)?** GPLv3 licensed, no published crate versions, plugin-development focused. The coupler-rs `vst3` crate has cleaner COM object manipulation and permissive licensing.

**Why not cutoff-vst?** Not published to crates.io. Appears to be a proprietary/closed-source library for the "Cutoff" ecosystem. Cannot be used as a dependency. LOW confidence -- could not verify availability.

**Why not jesnor/vst3-rs?** Wraps vst3-sys (inheriting GPL), less active maintenance, thin safe wrapper without hosting-specific abstractions.

**Important context:** Steinberg released VST3 SDK 3.8.0 under MIT license (October 2025). This removes all previous licensing friction for VST3 host development. The coupler-rs crate tracks this SDK.

**Known issue:** VST SDK 3.8.0 introduced forward-declared external C structs for Wayland support that broke the coupler-rs binding generator. Check issue #20 on coupler-rs/vst3-rs for status before starting.

### Audio File I/O

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `symphonia` | 0.5.x | Audio file decoding (multi-format) | Pure Rust, supports WAV/FLAC/MP3/OGG/AAC/AIFF. No C dependencies. Best choice for reading diverse input formats. |
| `hound` | 3.5.x | WAV file writing | 7.5M+ downloads, simple API for WAV encoding. Symphonia's writing support is limited; hound is the proven choice for WAV output. |

**Why not libsndfile bindings?** Adds C dependency, cross-compilation complexity. Pure Rust alternatives (symphonia + hound) cover the needed formats without FFI overhead.

**Pattern:** Use symphonia for reading any input format, hound for writing rendered output as WAV. If you need other output formats (FLAC, MP3), consider `opus` or `flac` crates, but WAV is sufficient for a headless renderer.

### MCP Server

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `rmcp` | 0.15.0 | MCP server framework | Official Rust SDK from modelcontextprotocol org. Implements MCP protocol 2025-11-25. Provides `#[tool]` macro for defining tools, supports stdio and SSE transports. Active development (54 releases, 136 contributors). |

**Why not rust-mcp-sdk?** Also implements MCP 2025-11-25 but is third-party. The official SDK has more community support and will track protocol changes first.

**Why not mcpkit?** Macro-based alternative, but less mature than the official SDK.

**Transport choice:** Use stdio transport for Claude Desktop / Claude Code integration. SSE transport for remote/networked access later.

```rust
// Basic MCP server skeleton with rmcp
use rmcp::prelude::*;

#[derive(Debug, Clone)]
struct AudioHost;

#[tool]
impl AudioHost {
    /// Load a VST3 plugin by name
    async fn load_plugin(&self, plugin_name: String) -> Result<String, Error> {
        // Plugin loading logic
        Ok(format!("Loaded {}", plugin_name))
    }

    /// Render audio through the loaded plugin chain
    async fn render(&self, input_file: String, output_file: String) -> Result<String, Error> {
        // Rendering logic
        Ok(format!("Rendered {} -> {}", input_file, output_file))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = AudioHost;
    let transport = (std::io::stdin(), std::io::stdout());
    let server = service.serve(transport).await?;
    server.waiting().await?;
    Ok(())
}
```

### IPC (Supervisor-Worker Architecture)

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `rtrb` | 0.3.x | Lock-free SPSC ring buffer | Wait-free, designed for real-time audio. Single-producer single-consumer. Use for audio data transfer between processes via shared memory. |
| `shared_memory` | 0.12.x | Cross-platform shared memory | OS-agnostic shared memory segments. Pair with `raw_sync` for synchronization primitives. |
| Unix domain sockets (std) | -- | Control channel | Use for commands/responses between supervisor and worker. Lightweight, well-supported in tokio. |

**Architecture pattern:**
- **Control plane:** Unix domain sockets (or `tokio::net::UnixStream`) for JSON-RPC style commands (load plugin, set parameter, start render)
- **Data plane:** Shared memory + ring buffer for audio sample transfer (zero-copy)
- **Signaling:** eventfd (Linux) or pipe (cross-platform) for waking blocked readers

**Why not shmem-ipc?** Linux-only (uses memfd sealing + eventfd). If Linux-only is acceptable, it is the best option -- purpose-built for untrusted IPC with audio/video streaming use case. For cross-platform, use shared_memory + rtrb.

**Why not ipc-channel (Mozilla/Servo)?** Good for message passing but serializes data. Audio buffers need zero-copy shared memory, not serialized messages.

**Why not pipes alone?** Pipes copy data through the kernel. For audio buffers (potentially megabytes per render), shared memory avoids this overhead entirely.

### Async Runtime & Core

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `tokio` | 1.x | Async runtime | Required by rmcp. Industry standard for Rust async. |
| `serde` + `serde_json` | 1.x / 1.x | Serialization | Required by rmcp for MCP protocol. Also useful for plugin state serialization. |
| `schemars` | 0.8.x | JSON Schema generation | Required by rmcp for tool parameter schemas. |
| `tracing` | 0.1.x | Structured logging | Standard for async Rust applications. Essential for debugging multi-process architecture. |
| `anyhow` | 1.x | Error handling (application) | Ergonomic error handling for the host application layer. |
| `thiserror` | 2.x | Error handling (library) | Derive macro for typed errors in library code. |

### Process Management

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `tokio::process` | (part of tokio) | Child process spawning/management | Async process management for supervisor spawning workers. |
| `nix` | 0.29.x | POSIX APIs | Signal handling, process groups, shared memory primitives on Unix. |

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| VST3 bindings | `vst3` (coupler-rs) | `vst3-sys` (RustAudio) | GPL license, no versioned releases, plugin-focused |
| Audio decoding | `symphonia` | `rodio` | rodio is playback-focused, not file I/O |
| WAV writing | `hound` | `wavers` | hound has 100x more downloads, battle-tested |
| MCP SDK | `rmcp` | `rust-mcp-sdk` | Official SDK, faster protocol tracking |
| Ring buffer | `rtrb` | `ringbuf` | rtrb purpose-built for audio real-time |
| Shared memory | `shared_memory` | `shmem-ipc` | shmem-ipc is Linux-only |
| IPC control | Unix sockets | gRPC (tonic) | Over-engineered for local IPC |

## Installation

```bash
# Core dependencies
cargo add vst3
cargo add rmcp --features server,transport-io,macros
cargo add tokio --features full
cargo add serde --features derive
cargo add serde_json
cargo add schemars

# Audio I/O
cargo add symphonia --features all
cargo add hound

# IPC
cargo add rtrb
cargo add shared_memory

# Process management & utilities
cargo add nix --features signal,process
cargo add tracing
cargo add tracing-subscriber
cargo add anyhow
cargo add thiserror
```

## Confidence Assessment

| Component | Confidence | Notes |
|-----------|------------|-------|
| `vst3` (coupler-rs) | MEDIUM | Confirmed version 0.3.0 exists, MIT licensed, but SDK 3.8.0 compat issue needs checking |
| `rmcp` | HIGH | Official SDK, version 0.15.0 confirmed, active development |
| `symphonia` + `hound` | HIGH | Well-established crates with millions of downloads |
| `rtrb` + `shared_memory` | MEDIUM | Both exist and are used in audio, but IPC architecture is custom work |
| cutoff-vst availability | LOW | Cannot verify -- not on crates.io, may be proprietary |
| VST3 hosting maturity in Rust | MEDIUM | KVR forum reports segfaults with instruments (not effects). Ecosystem is early. |

## Sources

- [RustAudio/vst3-sys](https://github.com/RustAudio/vst3-sys) -- raw COM bindings, GPLv3
- [coupler-rs/vst3-rs](https://github.com/coupler-rs/vst3-rs) -- generated bindings, MIT/Apache-2.0
- [Steinberg VST 3.8.0 MIT announcement](https://www.steinberg.net/press/2025/vst-3-8/)
- [rmcp official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [Symphonia](https://github.com/pdeljanov/Symphonia)
- [hound](https://github.com/ruuda/hound)
- [rtrb](https://github.com/mgeier/rtrb)
- [shmem-ipc](https://github.com/diwic/shmem-ipc) -- Linux-only alternative
- [shared_memory crate](https://docs.rs/shared_memory)
- [KVR Forum: CLI VST3 host in Rust](https://www.kvraudio.com/forum/viewtopic.php?t=622780) -- community experience report
- [Renaud Denis: Robust VST3 Host for Rust](https://renauddenis.com/case-studies/rust-vst) -- cutoff-vst case study
- [Build MCP Servers in Rust guide](https://mcpcat.io/guides/building-mcp-server-rust/)
