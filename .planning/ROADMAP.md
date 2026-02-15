# Roadmap: VST3 MCP Host (Headless)

## Overview

This roadmap delivers a headless Rust VST3 host that AI agents control through MCP. The progression validates the riskiest hypothesis first (can Rust host VST3 plugins at all), then adds crash isolation for production robustness, then builds the AI-specific parameter intelligence that makes the system actually useful, and finally proves the whole thing works end-to-end in a real conversation.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3, 4): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [ ] **Phase 1: Single-Plugin MVP** - Validate VST3 hosting in Rust and deliver working audio processing via MCP
- [ ] **Phase 2: Crash Isolation** - Supervisor-worker process split with crash recovery and plugin blocklist
- [ ] **Phase 3: AI Parameter Intelligence** - Dynamic schema generation and focus mode for context-efficient AI interaction
- [ ] **Phase 4: System Validation** - End-to-end demo conversation proving conversational audio processing works

## Phase Details

### Phase 1: Single-Plugin MVP
**Goal**: A single-process host that can scan, load, and process audio through a VST3 plugin, controllable via MCP tools -- proving the core VST3 hosting hypothesis
**Depends on**: Nothing (first phase)
**Requirements**: HOST-01, DISC-01, PROC-01, PROC-02, PRES-01, INTEG-01
**Risk**: HOST-01 is a hypothesis -- coupler-rs/vst3 SDK 3.8.0 compatibility is unverified. Check issue #20 status before starting. If blocked, fallback to SDK 3.7.x or patched bindings.
**Success Criteria** (what must be TRUE):
  1. Running `cargo run` starts an MCP server that responds to tool calls over stdio
  2. An MCP client can call a scan tool and receive a list of installed VST3 plugins with UIDs
  3. An MCP client can load a plugin, send an audio file, and receive a processed output file
  4. Output audio preserves the input sample rate and bit depth (no quality degradation from the host)
  5. An MCP client can save and load .vstpreset files for a loaded plugin
**Plans**: 2 plans

Plans:
- [ ] 01-01-PLAN.md -- VST3 hosting core: project setup, module loading, plugin scanner, lifecycle state machine, COM RAII wrappers, preset I/O
- [ ] 01-02-PLAN.md -- Audio pipeline and MCP integration: symphonia/hound I/O, block processing, buffer conversion, rmcp server, tool definitions

### Phase 2: Crash Isolation
**Goal**: Plugin crashes are contained in worker processes, keeping the MCP server alive and automatically avoiding known-bad plugins
**Depends on**: Phase 1
**Requirements**: ARCH-01, ARCH-02, DISC-02
**Risk**: Multi-process VST3 hosting is less documented than single-process. IPC patterns (shared memory + ring buffers for audio, Unix sockets for control) need careful validation. Research confidence: MEDIUM.
**Success Criteria** (what must be TRUE):
  1. Supervisor and worker run as separate OS processes, visible in process listing
  2. Killing the worker process (simulating a plugin crash) does not terminate the MCP server -- the supervisor reports an error and remains responsive
  3. A plugin that crashed during a previous session is automatically skipped during scanning (blocklist active)
  4. Audio processing still works end-to-end through the supervisor-worker boundary (no regression from Phase 1)
**Plans**: TBD

Plans:
- [ ] 02-01: Supervisor-worker architecture (process split, IPC protocol, shared memory audio transfer)
- [ ] 02-02: Crash recovery and blocklist (crash detection, graceful error reporting, persistent blocklist)

### Phase 3: AI Parameter Intelligence
**Goal**: Claude can discover and manipulate plugin parameters through dynamically generated MCP tool schemas, with focus mode reducing parameter noise from 100+ to 5-15 relevant params
**Depends on**: Phase 2
**Requirements**: PARAM-01, PARAM-02
**Risk**: Both requirements are hypotheses. PARAM-01 (dynamic schema generation) has no known precedent -- runtime plugin parameter scanning mapped to MCP JSON schemas is novel. PARAM-02 (focus mode) depends on IUnitInfo adoption which is unknown; fallback to semantic name analysis if rarely implemented.
**Success Criteria** (what must be TRUE):
  1. An MCP client calling list_parameters for a loaded plugin receives a JSON schema describing all available parameters with names, ranges, and current values
  2. An MCP client can set parameter values by name and the changes affect the next audio render
  3. Calling list_parameters with a .vstpreset mask returns only parameters that differ from the preset baseline (focus mode active)
  4. Focus mode reduces a typical plugin's exposed parameters from 50+ to under 20
**Plans**: TBD

Plans:
- [ ] 03-01: Dynamic schema generation and focus mode (parameter enumeration, MCP schema mapping, preset diffing, IUnitInfo grouping)

### Phase 4: System Validation
**Goal**: The complete system works as intended -- an AI agent can have a natural conversation about audio processing and achieve a desired result without manual intervention
**Depends on**: Phase 3
**Requirements**: INTEG-02
**Risk**: This is the full system validation hypothesis. Failure here means the interaction model needs rethinking, not just a bug fix. Success depends on all previous phases working together seamlessly.
**Success Criteria** (what must be TRUE):
  1. Starting from "make this vocal brighter," Claude can scan plugins, select an appropriate EQ, load it, apply parameter changes, render audio, and deliver the result -- all through MCP tool calls
  2. The conversation completes in under 10 tool-call round trips
  3. No manual intervention is required (no editing config files, restarting processes, or fixing errors by hand)
**Plans**: TBD

Plans:
- [ ] 04-01: End-to-end demo and integration hardening (demo script, error handling polish, conversation flow validation)

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Single-Plugin MVP | 0/2 | Not started | - |
| 2. Crash Isolation | 0/2 | Not started | - |
| 3. AI Parameter Intelligence | 0/1 | Not started | - |
| 4. System Validation | 0/1 | Not started | - |
