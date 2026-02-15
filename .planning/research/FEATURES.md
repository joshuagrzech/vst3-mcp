# Feature Research: VST3 Wrapper Plugin with AI Parameter Control

**Domain:** VST3 plugin hosting / AI-controlled audio processing
**Researched:** 2026-02-15
**Confidence:** HIGH (VST3 spec well-documented, existing codebase provides grounding)

## Feature Landscape

### Table Stakes (Users Expect These)

Features that a VST3 host/wrapper must have or it is not a functional product.

#### Plugin Lifecycle

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Load child VST3 from .vst3 bundle | Cannot function without it | MEDIUM | Already implemented. Module loading via `libloading`, factory scanning, COM instantiation. Must handle both bundled and flat layouts. |
| Plugin scanning (discover installed plugins) | User needs to find what is available | LOW | Already implemented via `scanner.rs`. Scans OS-standard paths. |
| Full lifecycle state machine (Created -> SetupDone -> Active -> Processing) | VST3 spec mandates this sequence; skipping causes crashes | HIGH | Already implemented. Drop guard ensures correct teardown order. This is the hardest table-stakes feature to get right. |
| Component + Controller separation | Many plugins use separate IComponent and IEditController; host must handle both patterns (unified and split) | MEDIUM | Already implemented. Tries cast first, then factory creation with separate controller class ID. |
| IConnectionPoint wiring (component <-> controller) | Required for plugins that use message-based communication between processor and controller | LOW | Already implemented. Connects bidirectionally if both sides support IConnectionPoint. |
| IComponentHandler callbacks (beginEdit/performEdit/endEdit/restartComponent) | Plugins call these during operation; host must accept them or plugin misbehaves | MEDIUM | Partially implemented -- currently logs but does not act on restartComponent flags. Must handle kLatencyChanged, kParamValuesChanged, kParamTitlesChanged, kIoChanged. |

#### Audio Processing

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Audio passthrough (route audio through child plugin) | Core purpose of a wrapper | MEDIUM | Already implemented for offline processing. Handles planar buffers, block-based processing, silence flags. |
| Sample rate negotiation | Plugin may not support arbitrary rates; host must query and adapt | LOW | Already implemented. Re-setup on sample rate mismatch with input file. |
| Block size management | Plugins have maximum block size constraints | LOW | Already implemented with 4096 default. |
| Tail handling (reverb/delay fade-out) | Effects produce output after input ends; cutting it off clips the effect | MEDIUM | Already implemented. Queries getTailSamples(), handles kInfiniteTail with configurable max. |
| Multi-channel support (stereo at minimum) | Mono-only wrapper is unusable for most plugins | MEDIUM | Already implemented. Bus info enumeration, default bus activation. |
| Offline rendering | Process audio files through plugin without real-time constraint | LOW | Already implemented as the primary processing mode (kOffline). |

#### Parameter Management

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Enumerate all parameters | Must know what the plugin exposes before controlling it | LOW | Already implemented via getParameterCount/getParameterInfo. Captures id, title, units, default, step count, flags. |
| Read parameter values (normalized) | Must query current state of any parameter | LOW | Already implemented via getParamNormalized. |
| Write parameter values | Core requirement for any automation or AI control | MEDIUM | Partially implemented. Queue exists but IParameterChanges delivery to process() is TODO. For non-real-time, setParamNormalized on controller works but does not sync to processor correctly without IParameterChanges. |
| Parameter value display (normalized -> display string) | Users need readable values ("3.5 dB") not raw floats ("0.72") | LOW | Not yet implemented. IEditController::getParamStringByValue provides this. Straightforward to add. |
| Parameter flags interpretation (kCanAutomate, kIsReadOnly, kIsBypass, kIsHidden) | Must respect read-only params, identify bypass, hide internal params | LOW | Flags captured but not yet interpreted. Critical for Focus Mode -- AI should not try to write read-only params. |

#### State Management

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Save/load plugin state (.vstpreset format) | Users need to persist and recall settings | HIGH | Already implemented. Handles component state + controller state, class ID validation, setComponentState sync. |
| Preset compatibility with DAWs | .vstpreset files should work across hosts | MEDIUM | Already implemented using Steinberg binary format with correct chunk structure. |

### Differentiators (Competitive Advantage)

Features that make AI control practical and powerful. Not required for a basic wrapper, but are the entire reason AgentAudio exists.

#### Focus Mode (Parameter Filtering for AI)

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Parameter exposure selection (Focus Mode) | Plugins expose 50-200+ parameters. AI needs a curated subset to be useful, not overwhelmed. User marks which params matter. | MEDIUM | Core differentiator. Needs a persistent mapping: plugin class ID -> set of exposed param IDs. No VST3 API for this -- it is a wrapper-layer concept. |
| Parameter grouping by unit hierarchy (IUnitInfo) | Present params in logical groups ("EQ Band 1", "Compressor") rather than flat list. Makes Focus Mode selection intuitive. | MEDIUM | IUnitInfo provides hierarchical unit structure. Each parameter has a unitId linking it to a group. Not all plugins implement IUnitInfo, so fallback to flat list is needed. |
| Focus Mode persistence | Remembered per plugin class ID across sessions. User does not re-mark params every time. | LOW | Store as JSON: `{ classId: [paramId, paramId, ...] }`. Save alongside or separate from .vstpreset. |
| Semantic parameter naming for AI | Rename "Param 42" to "reverb_decay_time" so AI tools read naturally in MCP | MEDIUM | User-defined aliases stored in Focus Mode config. MCP tools use alias if set, fall back to plugin-provided title. |

#### MCP Server (AI Control Interface)

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| MCP tool: list exposed parameters | AI discovers what it can control. Returns only Focus Mode params with names, ranges, current values. | LOW | Already partially exists (scan, load, process tools). Needs filtering by Focus Mode set. |
| MCP tool: get parameter value | AI reads current parameter state with display string ("3.5 dB") not just normalized float | LOW | Wrap getParamNormalized + getParamStringByValue. Filter by Focus Mode. |
| MCP tool: set parameter value | AI writes parameter values. Accept both normalized (0.0-1.0) and display values ("3.5 dB") | MEDIUM | Need IParameterChanges for proper delivery to processor. Also need normalizedParamToPlain/plainParamToNormalized for unit conversion. beginEdit/performEdit/endEdit sequence required. |
| MCP tool: batch parameter set | Set multiple params atomically in one call. Critical for coordinated changes (e.g., EQ band freq+gain+Q together). | MEDIUM | Deliver all changes in same process() call via IParameterChanges with multiple IParamValueQueue entries. |
| MCP tool: save/load preset | AI can snapshot and recall states. Already exists but needs Focus Mode awareness. | LOW | Already implemented. Add metadata about which params were Focus-exposed. |
| MCP tool: get plugin info | AI learns what plugin is loaded, its category, vendor, bus configuration | LOW | Already partially exists in load_plugin response. Formalize as dedicated tool. |
| MCP resource: parameter schema | Expose parameter metadata as MCP resource so AI can reason about available controls without tool calls | LOW | Static data after plugin load. Publish as resource with parameter names, ranges, units, groups. |

#### Real-Time Processing (beyond offline)

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Real-time audio streaming | Process live audio (microphone, system audio) not just files. Enables live performance AI control. | HIGH | Requires audio I/O backend (CPAL or JACK on Linux). Must run process() on audio thread with real-time constraints. Significantly changes architecture from current offline model. |
| MIDI event routing | Pass MIDI to instrument plugins. AI can trigger notes, send CC, program changes. | MEDIUM | Requires IEventList implementation. Build Event structs with kNoteOnEvent/kNoteOffEvent, set sampleOffset for timing. Current process() passes null inputEvents. |
| Process context (transport info) | Provide tempo, time signature, bar position to plugin. Many effects/instruments use transport info. | MEDIUM | ProcessContext struct with tempo, timeSigNumerator/Denominator, projectTimeSamples. Currently passes null processContext. |

#### Child Plugin GUI

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Display child plugin GUI (IPlugView) | Users need visual feedback and manual control alongside AI control | HIGH | Platform-specific: Linux requires X11 XEmbed or Wayland subsurface. Must call createView("editor"), attached(parentWindow), onSize(). Need IPlugFrame for resize callbacks. Many cross-platform pitfalls. |
| Wrapper UI (param list + Focus Mode toggles) | UI for marking which params are exposed to AI | HIGH | Needs a custom UI framework. Could be terminal-based initially (simpler) or full GUI (egui/iced). |
| Side-by-side: wrapper UI + child GUI | See plugin GUI and Focus Mode controls simultaneously | HIGH | Window management complexity. Either embed child in wrapper window or coordinate two windows. |

### Anti-Features (Commonly Requested, Often Problematic)

Features that seem appealing but create disproportionate complexity or architectural problems.

| Feature | Why Requested | Why Problematic | Alternative |
|---------|---------------|-----------------|-------------|
| Plugin chaining (load multiple plugins in series) | "Process through EQ then compressor then reverb" | Massively increases complexity: routing matrix, per-plugin state, latency compensation between plugins, ordering UI. This is building a DAW. | Load one plugin at a time. Chain by processing audio files sequentially (output of plugin A becomes input to plugin B). The MCP server can orchestrate this. |
| Real-time MIDI generation by AI | "AI plays melodies in real-time" | Timing precision requires sub-millisecond scheduling. AI inference latency (100ms+) makes real-time MIDI generation musically useless. | Offline MIDI: AI generates MIDI file, host renders it through instrument plugin. Timing is perfect because it is not real-time. |
| Parameter automation curves | "Record parameter changes over time like a DAW" | Requires timeline, transport, automation lane UI, sample-accurate playback. This is a DAW feature, not a wrapper feature. | Snapshot-based: AI sets params, processes audio, done. For time-varying params, AI generates multiple snapshots and wrapper processes segments. |
| Multi-format support (VST2, AU, LV2, CLAP) | "Support all plugin formats" | Each format has completely different APIs, lifecycle, parameter models. VST2 is officially deprecated. AU is macOS-only. LV2 and CLAP have different architectures. | VST3 only. It is the industry standard with the broadest cross-platform support. If CLAP gains significant traction, add it as a second format later. |
| Sandboxed plugin loading (out-of-process) | "Isolate plugin crashes from host" | Requires IPC for all COM calls (process, parameter get/set, GUI events). Audio data must cross process boundary every block. Extreme complexity for marginal benefit in an offline tool. | In-process loading with graceful error handling. Catch panics around plugin calls. For scanning, use separate process (already common pattern). |
| Custom DSP (built-in EQ, compressor, etc.) | "Add basic processing without loading a plugin" | Scope creep. Every built-in effect is a maintenance burden and competes with dedicated plugins that do it better. | The wrapper wraps plugins. If user wants EQ, they load an EQ plugin. |
| DAW integration (ReWire, ARA, inter-app audio) | "Use inside my DAW" | These are heavyweight integration protocols. ReWire is dead. ARA is for specific use cases (audio editing). Inter-app audio is iOS only. | Standalone tool that processes files. DAW users export audio, process with AgentAudio, import back. Or wrap AgentAudio as a VST3 plugin itself (future, very complex). |

## Feature Dependencies

```
[Plugin Scanning]
    |
    v
[Plugin Loading (lifecycle state machine)]
    |
    +------> [Parameter Enumeration]
    |             |
    |             +------> [Parameter Read/Write]
    |             |             |
    |             |             +------> [Focus Mode (param filtering)]
    |             |             |             |
    |             |             |             +------> [MCP Parameter Tools]
    |             |             |                         |
    |             |             |                         +------> [MCP Resource: param schema]
    |             |             |
    |             |             +------> [IParameterChanges delivery]
    |             |
    |             +------> [Parameter Display Strings]
    |             |
    |             +------> [Unit Hierarchy (IUnitInfo)]
    |
    +------> [Audio Processing (offline)]
    |             |
    |             +------> [Process Context (transport)]
    |             |
    |             +------> [MIDI Event Routing]
    |             |
    |             +------> [Real-Time Audio Streaming]
    |
    +------> [State Save/Load (.vstpreset)]
    |             |
    |             +------> [Focus Mode Persistence]
    |
    +------> [Child Plugin GUI (IPlugView)]
                  |
                  +------> [Wrapper UI (Focus Mode toggles)]
                                |
                                +------> [Side-by-side UI]
```

### Dependency Notes

- **Focus Mode requires Parameter Enumeration + Read/Write:** Cannot filter params you cannot enumerate or control.
- **MCP Parameter Tools require Focus Mode:** Without filtering, AI gets 200 params and produces garbage. Focus Mode is not optional for practical AI control.
- **IParameterChanges delivery required for correct parameter writing:** setParamNormalized on controller works for display sync but does NOT reliably reach the processor. The spec requires changes via ProcessData.inputParameterChanges.
- **Real-Time Audio requires fundamental architecture change:** Current offline model is synchronous. Real-time needs audio thread, callback-based processing, lock-free communication.
- **Child Plugin GUI conflicts with headless operation:** GUI requires display server (X11/Wayland). Server deployments and CI environments have no display. Must remain functional without GUI.
- **MIDI Event Routing enhances Audio Processing:** Instrument plugins (synths, samplers) are non-functional without MIDI input. Adding MIDI unlocks an entire plugin category.

## MVP Definition

### Launch With (v1)

Minimum viable product -- what is needed to validate that AI can usefully control a VST3 plugin.

- [x] Plugin scanning and loading -- already done
- [x] Offline audio processing -- already done
- [x] Parameter enumeration and read -- already done
- [x] State save/load (.vstpreset) -- already done
- [ ] **Parameter write via IParameterChanges** -- critical gap. Current implementation queues but does not deliver. Without this, AI cannot actually change plugin behavior.
- [ ] **Parameter display strings** -- AI needs "3.5 dB" not "0.72". Small effort, high impact.
- [ ] **Parameter flag interpretation** -- filter out kIsReadOnly, kIsHidden. Prevent AI from trying to write read-only params.
- [ ] **Focus Mode (basic)** -- JSON config mapping classId to list of exposed paramIds. MCP tools filter by this set.
- [ ] **MCP tools for parameter control** -- list_params, get_param, set_param, filtered by Focus Mode.

### Add After Validation (v1.x)

Features to add once core parameter control is proven with real plugins.

- [ ] **Batch parameter set** -- set multiple params in one call for coordinated changes
- [ ] **Unit hierarchy (IUnitInfo)** -- organize params into groups for better Focus Mode UX
- [ ] **Semantic parameter aliases** -- user-defined names for MCP readability
- [ ] **MIDI event routing** -- unlock instrument plugins (synths, samplers)
- [ ] **Process context** -- provide tempo/transport info for time-aware plugins
- [ ] **restartComponent handling** -- respond to kLatencyChanged, kParamValuesChanged, kIoChanged properly
- [ ] **Focus Mode persistence** -- save/load Focus Mode configs per plugin

### Future Consideration (v2+)

Features to defer until product-market fit is established.

- [ ] **Child plugin GUI (IPlugView)** -- requires platform windowing, significant complexity. Terminal-based param control is sufficient for AI use case.
- [ ] **Real-time audio streaming** -- fundamentally different architecture. Validate offline first.
- [ ] **Wrapper UI** -- graphical Focus Mode editor. Defer until Focus Mode concept is validated via config files.
- [ ] **MCP resource: parameter schema** -- nice optimization but tools work fine for MVP

## Feature Prioritization Matrix

| Feature | User Value | Implementation Cost | Priority |
|---------|------------|---------------------|----------|
| IParameterChanges delivery | HIGH | MEDIUM | P1 |
| Parameter display strings | HIGH | LOW | P1 |
| Parameter flag filtering | HIGH | LOW | P1 |
| Focus Mode (basic JSON config) | HIGH | LOW | P1 |
| MCP param tools (list/get/set) | HIGH | MEDIUM | P1 |
| Batch parameter set | MEDIUM | MEDIUM | P2 |
| IUnitInfo hierarchy | MEDIUM | MEDIUM | P2 |
| MIDI event routing | HIGH | MEDIUM | P2 |
| Process context (transport) | MEDIUM | LOW | P2 |
| restartComponent handling | MEDIUM | MEDIUM | P2 |
| Semantic param aliases | LOW | LOW | P2 |
| Focus Mode persistence | MEDIUM | LOW | P2 |
| Child plugin GUI | MEDIUM | HIGH | P3 |
| Real-time audio | HIGH | HIGH | P3 |
| Wrapper UI (graphical) | MEDIUM | HIGH | P3 |
| MCP resource schema | LOW | LOW | P3 |

**Priority key:**
- P1: Must have for launch (AI cannot control plugins without these)
- P2: Should have, add when core is proven
- P3: Nice to have, future consideration

## Competitor Feature Analysis

| Feature | Carla MCP Server | Blue Cat Patchwork | DDMF Metaplugin | AgentAudio (Our Approach) |
|---------|-----------------|-------------------|-----------------|--------------------------|
| Plugin format support | VST2/3, LV2, LADSPA, AU, SF2/SFZ | VST, VST3, AU, AAX | VST, VST3, AU | VST3 only (focused) |
| Plugin chaining | Yes (full routing) | Yes (64 slots, serial+parallel) | Yes (8 channels, routing matrix) | No -- single plugin, chain via file processing |
| Parameter control | 45 tools, NLP support | GUI-based, MIDI mapping | GUI-based | MCP tools, Focus Mode filtering |
| AI integration | NLP interface, JACK-based | None | None | Native MCP server, Focus Mode for AI curation |
| Real-time audio | Yes (JACK) | Yes (plugin format) | Yes (plugin format) | Offline first, real-time later |
| GUI display | Carla GUI + plugin GUIs | Host GUI + plugin GUIs | Host GUI + plugin GUIs | Headless first, GUI later |
| State management | Session snapshots | DAW-managed | DAW-managed | .vstpreset files |
| Deployment model | Standalone app (Linux-focused) | DAW plugin | DAW plugin | Standalone MCP server |

**Key insight:** Carla MCP Server is the closest competitor, offering 45 tools with NLP support. However, it wraps Carla (a full-featured plugin host), which means it carries enormous complexity. AgentAudio's advantage is Focus Mode -- the insight that AI does not need all 200 parameters, it needs the 5-10 that matter for the current task. No competitor offers parameter curation for AI consumption.

## Sources

- [VST3 Developer Portal - Parameters and Automation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Parameters+Automation/Index.html) - HIGH confidence
- [VST3 Developer Portal - Hosting FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Hosting.html) - HIGH confidence
- [IEditController Class Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IEditController.html) - HIGH confidence
- [IComponentHandler Class Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IComponentHandler.html) - HIGH confidence
- [IPlugView Class Reference](https://steinbergmedia.github.io/vst3_doc/base/classSteinberg_1_1IPlugView.html) - HIGH confidence
- [IParameterChanges Class Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IParameterChanges.html) - HIGH confidence
- [VST3 Units Documentation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+3+Units/Index.html) - HIGH confidence
- [Presets and Program Lists](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Presets+Program+Lists/Index.html) - HIGH confidence
- [Carla MCP Server](https://mcp.aibase.com/server/1538249238906150913) - MEDIUM confidence (third-party listing)
- [DDMF Metaplugin](https://ddmf.eu/metaplugin-chainer-vst-au-rtas-aax-wrapper/) - MEDIUM confidence
- [Steinberg Forums - IEditController::setParamNormalized hosting](https://forums.steinberg.net/t/vst3-hosting-when-to-use-ieditcontroller-setparamnormalized/787800) - MEDIUM confidence
- [KVR Forum - VST3 host IPlugView GUI display](https://www.kvraudio.com/forum/viewtopic.php?t=334466) - LOW confidence

---
*Feature research for: VST3 wrapper plugin with AI parameter control via MCP*
*Researched: 2026-02-15*
