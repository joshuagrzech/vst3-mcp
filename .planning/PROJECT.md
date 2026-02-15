# AgentAudio (VST3 Wrapper & MCP Server)

## What This Is

A VST3 wrapper plugin built in Rust that hosts other VST3 plugins inside itself and runs an embedded MCP server, enabling AI agents (Claude) to inspect and control the wrapped plugin's parameters in real-time during music production. Think "Cursor for music production" - infrastructure that makes any DAW plugin AI-controllable.

## Core Value

AI can reliably read and write any exposed plugin parameter in real-time while the user makes music in their DAW.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] Plugin-in-plugin architecture (nih-plug wrapper loads child VST3 via vst3-sys)
- [ ] Real-time audio passthrough (DAW → child plugin → DAW, no latency/locking)
- [ ] Real-time MIDI passthrough (DAW → child plugin, note events and CC)
- [ ] Embedded MCP server (Tokio runtime runs inside the plugin)
- [ ] Lock-free communication (MCP thread ↔ audio thread via atomic ring buffers)
- [ ] Parameter control API (MCP tools to read/write exposed params)
- [ ] Focus Mode UI ("Listen" toggle to mark params as exposed)
- [ ] Child plugin GUI rendering (IPlugView displayed in wrapper window)
- [ ] Wrapper GUI (egui interface for Focus Mode controls)

### Out of Scope

- Offline rendering or file-based processing — Real-time DAW integration only
- Standalone mode (.exe host) — Plugin artifact (.vst3) only
- Plugin chains or multi-effect routing — Single child plugin in v1
- MIDI generation or sequencing — Parameter control only, no MIDI creation
- VST2 or AU formats — VST3 only

## Context

**Vision:** Infrastructure for AI-assisted music production. Like Cursor transformed coding by making editors AI-controllable, AgentAudio makes DAW plugins AI-controllable. The specific musical tasks don't matter - the infrastructure for reliable parameter control does.

**Architecture novelty:** Each component exists in isolation (nih-plug works, MCP servers work, VST hosting works). The challenge is integration - running a Tokio MCP server inside a real-time audio plugin without blocking the audio thread.

**Threading model:**
- Audio thread (real-time, no locks): Copies buffers to/from child plugin
- GUI thread: Renders wrapper UI (egui) and child plugin's editor (IPlugView)
- MCP thread: Runs Tokio runtime, communicates with audio thread via lock-free queues

**Focus Mode ("The Wiggle"):** VST3 plugins have 100+ parameters. Exposing all to Claude wastes context. Focus Mode solves this: user enables "Listen" toggle, tweaks knobs they care about, wrapper intercepts IEditController events and flags only those parameters as "exposed" to AI.

**Learning context:** Discovering nih-plug during this project. Will learn the framework while building.

**Testing environment:** Bitwig Studio on Linux (native VST3 support).

**Distribution:** Open source project. Design for usability by other producers, not just personal use.

## Constraints

- **Framework: nih-plug** — Best Rust VST3 framework, handles the plugin boilerplate
- **Hosting: vst3-sys** — Raw VST3 SDK bindings to load child plugin
- **GUI: nih-plug-egui** — For wrapper interface (Focus Mode controls)
- **Async: tokio** — For MCP server runtime
- **Artifact: .vst3 plugin** — Not a standalone .exe, must load in DAW
- **Real-time safety: Lock-free audio thread** — No mutexes, allocations, or blocking in audio callback
- **Platform: Linux first** — Testing on Bitwig Linux, cross-platform later

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Plugin-in-plugin architecture | AI must integrate into existing DAW workflows, not replace them. Wrapper plugin is transparent to user. | — Pending |
| Embedded MCP server (not external process) | External process requires IPC overhead and breaks plugin sandboxing model. Embedded server runs in plugin's process space. | — Pending |
| nih-plug framework | Best Rust VST3 framework with active development, good abstractions, and real-time safety patterns. | — Pending |
| Real-time operation (not offline) | Music production is inherently real-time. AI adjustments must be audible immediately during playback/recording. | — Pending |
| Focus Mode for parameter selection | 100+ params overwhelm Claude's context. User-driven selection (wiggle to expose) makes AI interaction practical. | — Pending |
| Lock-free ring buffers for MCP ↔ audio | Audio thread cannot block. Atomic queues let MCP thread send parameter changes without locking. | — Pending |

---
*Last updated: 2026-02-15 after project pivot from headless host to DAW plugin*
