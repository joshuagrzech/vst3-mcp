# Roadmap: AgentAudio

## Overview

AgentAudio validates that AI can control VST3 plugin parameters through an offline MVP before tackling real-time DAW integration. The roadmap moves from proving plugin hosting works, through audio processing and parameter control, to a complete MCP-driven AI interface with Focus Mode filtering and state persistence. Each phase delivers one verifiable capability that builds on the previous, with the hardest integration risks (COM lifecycle, IParameterChanges, Tokio-in-plugin) tackled early.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Plugin Hosting** - Load and lifecycle-manage a child VST3 plugin
- [x] **Phase 2: Audio Processing** - Process audio files through the hosted child plugin
- [x] **Phase 3: Parameter Control** - Read and write child plugin parameters via IParameterChanges
- [ ] **Phase 4: MCP Server & Tools** - Embedded MCP server with AI-accessible parameter tools
- [ ] **Phase 5: Focus Mode** - Parameter filtering so AI only sees user-selected parameters
- [ ] **Phase 6: State Management** - Save and load child plugin presets

## Phase Details

### Phase 1: Plugin Hosting
**Goal**: A child VST3 plugin can be loaded, initialized through its full lifecycle, and torn down without crashes
**Depends on**: Nothing (first phase)
**Requirements**: HOST-01, HOST-02, HOST-03, HOST-04, HOST-05
**Success Criteria** (what must be TRUE):
  1. Running the scanner against a standard VST3 directory produces a list of discovered plugins with classId and name
  2. Specifying a classId loads the corresponding plugin and reports its name and vendor
  3. A loaded plugin transitions through Created, SetupDone, Active, and Processing states without errors
  4. Unloading a plugin (teardown) completes without segfaults or resource leaks, verified by repeated load/unload cycles
  5. Both unified and split Component/Controller plugins load and initialize correctly (tested with at least two different plugin brands)
**Plans**: 2 plans

Plans:
- [x] 01-01-PLAN.md -- Out-of-process scanner and hardened teardown ordering
- [x] 01-02-PLAN.md -- Integration tests with real plugins (lifecycle verification)

### Phase 2: Audio Processing
**Goal**: An audio file can be processed through the hosted child plugin with correct output
**Depends on**: Phase 1
**Requirements**: AUDIO-01, AUDIO-02, AUDIO-03, AUDIO-04, AUDIO-05
**Success Criteria** (what must be TRUE):
  1. User can process a WAV file through a hosted effect plugin and get a valid output WAV file
  2. Processing a file through a bypassed or transparent plugin produces bit-identical (or near-identical) output, confirming no audio corruption
  3. Stereo files process correctly with left/right channels preserved (no channel swap or mono collapse)
  4. The output file has the same sample rate as the input file, and the child plugin receives the correct sample rate during setup
  5. Buffer conversion between nih-plug and VST3 formats produces no audible artifacts (no clicks, pops, or silence gaps at buffer boundaries)
**Plans**: 2 plans

Plans:
- [x] 02-01-PLAN.md -- Fix process() bugs (bus count, ProcessContext, pre-allocation) and harden pipeline (denormals, sample rate errors)
- [x] 02-02-PLAN.md -- Integration tests verifying all 5 success criteria with real plugins

### Phase 3: Parameter Control
**Goal**: All child plugin parameters can be enumerated, read, and written with sample-accurate automation via IParameterChanges
**Depends on**: Phase 2
**Requirements**: PARAM-01, PARAM-02, PARAM-03, PARAM-04, PARAM-05, PARAM-06
**Success Criteria** (what must be TRUE):
  1. Enumerating parameters on a loaded plugin returns a complete list with id, name, and flags matching the plugin's advertised parameters
  2. Reading a parameter returns its current normalized value (0.0-1.0) and a human-readable display string (e.g., "3.5 dB", "100 Hz")
  3. Writing a parameter value via IParameterChanges and then re-processing audio produces an audibly different output compared to the default
  4. Read-only parameters (kIsReadOnly flag) are identified and excluded from write operations
  5. Parameter sweeps (gradual value changes across a buffer) produce smooth output without zipper noise or discontinuities
**Plans**: 2 plans

Plans:
- [x] 03-01-PLAN.md — IParameterChanges delivery, display strings, and flag filtering
- [x] 03-02-PLAN.md — Integration tests for all 5 success criteria

### Phase 4: MCP Server & Tools
**Goal**: An AI agent (Claude) can connect via MCP and inspect/control the hosted plugin's parameters
**Depends on**: Phase 3
**Requirements**: MCP-01, MCP-02, MCP-03, MCP-04, MCP-05, MCP-06, MCP-07
**Success Criteria** (what must be TRUE):
  1. The MCP server starts on a background thread when the system initializes and accepts connections via stdio transport
  2. Calling `get_plugin_info` returns the loaded plugin's classId, name, and vendor
  3. Calling `list_params` returns all non-read-only parameters with their ids, names, and current values
  4. Calling `get_param` with a parameter id returns its current normalized value and display string
  5. Calling `set_param` with a parameter id and value produces an audible change in the next processed audio output
  6. Calling `batch_set` with multiple parameter id/value pairs applies all changes and the resulting audio reflects all parameter modifications
**Plans**: 1 plan

Plans:
- [ ] 04-01-PLAN.md — Six MCP parameter tools (get_plugin_info, list_params, get_param, set_param, batch_set) plus integration tests

### Phase 5: Focus Mode
**Goal**: Only user-selected parameters are exposed to AI, reducing noise from hundreds of irrelevant controls
**Depends on**: Phase 4
**Requirements**: FOCUS-01, FOCUS-02, FOCUS-03, FOCUS-04
**Success Criteria** (what must be TRUE):
  1. With a Focus Mode JSON config specifying 3 exposed parameters, `list_params` returns exactly those 3 parameters
  2. A config file maps plugin classId to a set of exposed parameter IDs, so different plugins have independent Focus Mode settings
  3. Without any config file, `list_params` returns all non-read-only parameters (sensible default)
  4. Modifying the Focus Mode config and reloading changes which parameters appear in `list_params` without restarting the system
**Plans**: TBD

Plans:
- [ ] 05-01: TBD

### Phase 6: State Management
**Goal**: Child plugin presets can be saved to disk and restored, preserving exact parameter state
**Depends on**: Phase 3
**Requirements**: STATE-01, STATE-02, STATE-03
**Success Criteria** (what must be TRUE):
  1. User can save the current child plugin state to a .vstpreset file on disk
  2. User can load a .vstpreset file and the child plugin's parameters update to match the saved state
  3. A save/load roundtrip preserves parameter values: saving, reloading, and comparing all parameter values shows they match the pre-save values
**Plans**: TBD

Plans:
- [ ] 06-01: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5 -> 6

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Plugin Hosting | 2/2 | ✓ Complete | 2026-02-15 |
| 2. Audio Processing | 2/2 | ✓ Complete | 2026-02-15 |
| 3. Parameter Control | 2/2 | ✓ Complete | 2026-02-15 |
| 4. MCP Server & Tools | 0/1 | Not started | - |
| 5. Focus Mode | 0/TBD | Not started | - |
| 6. State Management | 0/TBD | Not started | - |
