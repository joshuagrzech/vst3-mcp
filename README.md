# VST3 MCP Host + AgentAudio Wrapper
[![Rust](https://github.com/joshuagrzech/vst3-mcp/actions/workflows/rust.yml/badge.svg)](https://github.com/joshuagrzech/vst3-mcp/actions/workflows/rust.yml)
This repository now contains two related runtimes for AI-driven VST3 control:

1. **`vst3-mcp-host`**: a headless offline host (stdio MCP transport).
2. **`agentaudio-wrapper-vst3`**: a DAW-loadable VST3 wrapper with embedded MCP server for realtime parameter updates.

If your goal is **LLM control in realtime while a plugin is running in a DAW**, use the wrapper plugin flow.

## Current Realtime Path

The wrapper plugin in `crates/agentaudio-wrapper-vst3` provides:

- Child plugin hosting inside a DAW-loadable VST3.
- Per-instance embedded MCP server (`http://127.0.0.1:<ephemeral>/mcp`).
- Realtime parameter event enqueue tools:
  - `set_param_realtime`
  - `batch_set_realtime`
- Audio-thread application of queued events.
- Wrapper UI for:
  - scanning/selecting/loading child plugins,
  - opening child plugin editor,
  - viewing instance ID + MCP endpoint.

## Quick Start

### Build wrapper crate

```bash
cargo build --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
```

## Optional: Global MCP Router (recommended for coding agents)

The wrapper plugin exposes a per-instance MCP endpoint on an **ephemeral** port (`http://127.0.0.1:<dynamic>/mcp`). To avoid manually reconfiguring clients, this repo includes a **stable router daemon** plus an **installer** that adds/removes a single MCP server entry for multiple coding agents.

### Start the router daemon

```bash
cargo run -p agentaudio-mcp-router --bin agentaudio-mcp-routerd
```

Defaults:
- bind: `127.0.0.1:38765` (override with `AGENTAUDIO_MCP_ROUTERD_BIND`)
- MCP endpoint: `http://127.0.0.1:38765/mcp`

### Install into Claude Code / Gemini CLI / Cursor

```bash
cargo run --bin agentaudio-mcp -- install
```

To remove:

```bash
cargo run --bin agentaudio-mcp -- uninstall
```

Cursor is configured to use the stdio shim (`agentaudio-mcp-stdio`) which forwards tool calls to the router daemon over HTTP.

### Run tests

```bash
cargo test --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
```

## Build a precompiled installer release bundle

To create a distributable installer package that already includes all required artifacts:

```bash
./scripts/master-release-installer.sh
```

The generated bundle (under `dist/`) includes `agentaudio-installer` plus `precompiled-target/release/*`, so installer runs can skip local builds and only perform placement/service/config setup.

### In DAW

1. Install/bundle the built wrapper plugin.
2. Insert `AgentAudio Wrapper` on a track.
3. Open wrapper editor and click **Scan Plugins**.
4. Select child plugin and click **Load Child**.
5. Copy MCP endpoint shown in wrapper UI (or use `wrapper_status` tool).
6. Send realtime updates via MCP (`set_param_realtime` / `batch_set_realtime`).

## Docs

- `docs/USAGE.md` — end-to-end usage, wrapper-first.
- `docs/API.md` — MCP tool reference (wrapper + legacy host).
- `docs/EXAMPLES.md` — practical realtime control examples.
- `crates/agentaudio-wrapper-vst3/README.md` — wrapper crate-specific notes.

## Legacy Offline Host

The root crate `vst3-mcp-host` remains available for offline file-based processing and legacy `.vstpreset` workflows, but this is not the recommended path for realtime LLM control in a DAW.
