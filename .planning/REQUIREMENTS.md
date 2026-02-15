# Requirements: AgentAudio

**Defined:** 2026-02-15
**Core Value:** AI can reliably read and write any exposed plugin parameter in real-time while the user makes music in their DAW

## v1 Requirements (Minimal MVP - Offline Processing)

MVP validates AI parameter control before real-time complexity. Scope: offline processing, IParameterChanges implementation, Focus Mode config, MCP tools.

### Plugin Hosting

- [ ] **HOST-01**: System can scan VST3 plugins in standard directories (out-of-process to prevent crashes)
- [ ] **HOST-02**: System can load a specific VST3 plugin by classId
- [ ] **HOST-03**: Plugin follows full VST3 lifecycle (Created → SetupDone → Active → Processing → teardown)
- [ ] **HOST-04**: Plugin teardown sequence prevents crashes (correct COM pointer release order)
- [ ] **HOST-05**: System handles both unified and split Component/Controller plugins

### Audio Processing

- [ ] **AUDIO-01**: System can process audio file through child plugin (offline rendering)
- [ ] **AUDIO-02**: System preserves audio quality (no clipping, artifacts, or unexpected modifications)
- [ ] **AUDIO-03**: System handles multi-channel audio (stereo minimum, up to 8 channels)
- [ ] **AUDIO-04**: Buffer conversion between nih-plug and VST3 formats works correctly
- [ ] **AUDIO-05**: Sample rate matches between wrapper, child plugin, and input file

### Parameter Control

- [ ] **PARAM-01**: System can enumerate all child plugin parameters (id, name, flags)
- [ ] **PARAM-02**: System can read current parameter values (normalized 0.0-1.0)
- [ ] **PARAM-03**: System can write parameter values via IParameterChanges (sample-accurate)
- [ ] **PARAM-04**: System provides parameter display strings (human-readable like "3.5 dB")
- [ ] **PARAM-05**: System filters read-only parameters (kIsReadOnly flag)
- [ ] **PARAM-06**: Parameter writes produce audible changes in output audio

### Focus Mode

- [ ] **FOCUS-01**: System loads Focus Mode configuration from JSON file
- [ ] **FOCUS-02**: Configuration maps plugin classId to set of exposed parameter IDs
- [ ] **FOCUS-03**: System tracks which parameters are marked as "exposed to AI"
- [ ] **FOCUS-04**: Default behavior: all non-read-only parameters are exposed if no config exists

### MCP Integration

- [ ] **MCP-01**: System runs embedded MCP server on background thread
- [ ] **MCP-02**: MCP server accepts stdio transport connections
- [ ] **MCP-03**: MCP tool: `get_plugin_info` returns classId, name, vendor
- [ ] **MCP-04**: MCP tool: `list_params` returns exposed parameters (filtered by Focus Mode)
- [ ] **MCP-05**: MCP tool: `get_param` returns current value and display string
- [ ] **MCP-06**: MCP tool: `set_param` writes normalized value and produces audible change
- [ ] **MCP-07**: MCP tool: `batch_set` writes multiple parameters atomically

### State Management

- [ ] **STATE-01**: System can save child plugin state to .vstpreset file
- [ ] **STATE-02**: System can load child plugin state from .vstpreset file
- [ ] **STATE-03**: State save/load roundtrip preserves parameter values

## v2 Requirements (Real-Time & Advanced Features)

Deferred to validate MVP first.

### Real-Time Processing

- **RT-01**: Real-time audio passthrough in DAW (zero-latency monitoring)
- **RT-02**: Lock-free communication bridge (rtrb queues for MCP ↔ audio thread)
- **RT-03**: Zero allocation in audio thread (pre-allocated buffers)
- **RT-04**: Audio thread latency under 1ms during concurrent MCP requests

### Advanced Focus Mode

- **FOCUS-05**: ComponentHandler intercepts detect user parameter tweaks ("wiggle to expose")
- **FOCUS-06**: GUI toggle to enable/disable "Listen" mode
- **FOCUS-07**: Focus Mode persistence (save config when parameters are exposed/hidden)

### GUI

- **GUI-01**: Wrapper UI (nih-plug-egui) shows parameter list and current values
- **GUI-02**: Focus Mode controls (listen toggle, clear exposed params)
- **GUI-03**: Child plugin GUI embedding (IPlugView in wrapper window)

### MIDI Support

- **MIDI-01**: MIDI event routing from DAW to child plugin (unlocks instrument plugins)
- **MIDI-02**: MCP tools can trigger notes (validate synth parameter changes)

## Out of Scope

Explicitly excluded to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Plugin chains (multi-effect routing) | Turns wrapper into DAW; massive complexity. Use sequential MCP calls instead. |
| Real-time MIDI generation by AI | AI latency (100-500ms) incompatible with musical timing. Pre-render MIDI offline. |
| Custom DSP or built-in effects | Competes with dedicated plugins; maintenance burden. |
| VST2 or AU format support | VST3 only for v1; other formats require separate hosting code. |
| Standalone mode (.exe) | Plugin artifact (.vst3) only; DAW integration is the point. |
| Multi-plugin simultaneous control | One child plugin at a time; reduces state complexity. |

## Traceability

Populated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| | | Pending |

**Coverage:**
- v1 requirements: 0 total (to be counted after final review)
- Mapped to phases: 0
- Unmapped: 0

---
*Requirements defined: 2026-02-15*
*Last updated: 2026-02-15 after research-based minimal MVP scoping*
