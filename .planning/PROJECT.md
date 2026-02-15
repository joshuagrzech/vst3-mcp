# VST3 MCP Host (Headless)

## What This Is

A headless, Rust-based VST3 host that runs as an MCP (Model Context Protocol) Server, enabling AI agents like Claude to discover, load, and process audio through professional VST3 plugins using structured tool calls. The system supports interactive experimentation workflows where Claude can iteratively apply plugins, adjust parameters, and evaluate results conversationally (e.g., "make this vocal brighter").

## Core Value

Safe, conversational control of professional audio plugins for AI agents, with crash isolation that keeps the system stable even when plugins fail.

## Requirements

### Validated

- [ ] Worker process safely loads VST3 SDK (vst3-sys) and plugins — **HYPOTHESIS:** coupler-rs/vst3 maturity unknown, SDK 3.8.0 compat needs verification
- [ ] Dynamic schema generation (scan plugin parameters at runtime → MCP tool JSON schema) — **HYPOTHESIS:** Novel approach, no precedent found
- [ ] Focus mode ("wiggle") - list_parameters accepts .vstpreset mask, exposes only params that differ from default — **HYPOTHESIS:** Novel AI-specific feature, IUnitInfo adoption rate unknown
- [ ] Demo conversation works end-to-end ("brighten this vocal" → success without manual intervention) — **HYPOTHESIS:** Full system validation target

### Active

- [ ] Multi-process supervisor-worker architecture (supervisor handles MCP, worker loads plugins)
- [ ] Plugin scanner that discovers VST3s and generates catalog with UIDs
- [ ] Crash isolation (plugin crashes only kill worker, not supervisor)
- [ ] Blocklist system (automatically skip plugins that previously crashed)
- [ ] Offline audio processing pipeline (file in → VST process → file out)
- [ ] Preset management (save/load .vstpreset files)
- [ ] Transparent audio quality (preserve sample rate, bit depth, plugin native characteristics)
- [ ] MCP integration over stdio (Claude can call tools, get results)

### Out of Scope

- Plugin chains (multi-effect routing) — v1 processes single plugin at a time
- Plugin GUIs or editors — headless only, no UI windows
- Real-time processing or live audio streams — all rendering is offline
- MIDI events or parameter automation curves — audio processing only

## Context

**VST3 ecosystem:** Professional audio plugins (VST3 format) are powerful but notoriously unstable — crashes are common and expected. This drives the mandatory multi-process isolation architecture.

**AI context windows:** VST3 plugins expose 100+ parameters. Exposing all params to Claude wastes context and makes interaction unwieldy. Focus mode solves this by masking parameters that match a preset baseline, showing only the 5-10 params that actually differ.

**Interactive experimentation workflow:** The primary use case is conversational iteration: Claude processes audio → evaluates result → adjusts params → re-renders → converges on desired sound. This requires fast turnaround and context-efficient parameter exposure.

**MCP as standard:** Model Context Protocol is the standard for Claude tool integration. Using MCP makes this host compatible with any MCP client, not just Claude.

## Constraints

- **Architecture: Multi-process supervisor-worker model** — Non-negotiable for crash isolation. Supervisor never loads plugins directly.
- **Tech stack: Rust** — Required for memory safety (VST3 C++ FFI is unsafe), performance (audio processing), and ecosystem (vst3-sys exists).
- **Audio I/O: symphonia or libsndfile** — For file reading/writing with format support.
- **Transport: Stdio for MCP, IPC for audio** — Stdio follows MCP standard. Shared memory or pipes for supervisor-worker audio transfer.
- **Quality: Transparent audio processing** — Must preserve plugin's native quality. No sample rate conversion, bit depth changes, or artifacts introduced by host.
- **Deployment: Headless and offline** — No GUI. No real-time streaming. All processing is batch/offline.

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Supervisor-worker process model | VST3 plugins crash frequently. If supervisor loads plugins directly, crash kills entire MCP server. Workers isolate crashes. | — Pending |
| Focus mode (preset masking) | VST3 plugins have 100+ params. Claude's context window can't afford full exposure on every call. Masking against default/preset shows only relevant params. | — Pending |
| Offline rendering only | Real-time audio streaming adds latency/buffer complexity. Interactive experimentation workflow doesn't need real-time — results can be 2-5 seconds delayed. | — Pending |
| Rust language | Memory safety critical for FFI with C++ VST3 SDK. Performance matters for audio. vst3-sys provides existing bindings. | — Pending |
| Single plugin v1 scope | Plugin chains require routing/graph architecture. Ship single-plugin processing first to validate core workflow. | — Pending |

---
*Last updated: 2026-02-14 after initialization*
