# Feature Landscape

**Domain:** Headless VST3 Host / AI-Controlled Audio Processing via MCP
**Researched:** 2026-02-14
**Overall confidence:** HIGH (primary sources: Steinberg VST3 Developer Portal, SDK interface docs, real-world host implementations)

---

## Table Stakes

Features ANY VST3 host must provide. Missing = non-functional or non-compliant.

### Plugin Lifecycle Management

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Plugin scanning and discovery | Must find .vst3 bundles in OS-standard locations | Medium | Scan system paths, load module, query IPluginFactory for class info. Cache results -- rescanning is expensive because plugins load sample libraries or check copy protection during init. Modern plugins include `moduleinfo.json` in the bundle for fast metadata extraction without loading the binary. |
| Plugin loading and instantiation | Core host duty: create IComponent + IEditController from factory | High | COM-style interface initialization. Must call `initialize()` with IHostApplication context immediately after creation. Component and controller may be combined (single component via queryInterface) or separate -- host must detect and handle both. |
| Component-controller connection | Processor-controller communication required by spec | Low | Establish bidirectional IConnectionPoint links between processor and controller. Required for state sync and message passing between the two halves. |
| Plugin deactivation and teardown | Clean resource release, strict ordering mandated by spec | Low | `setProcessing(false)` -> `setActive(false)` -> `terminate()` -> release. Violating this order causes crashes in many plugins. |

### Audio Processing Pipeline

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Audio bus configuration | Plugins declare input/output bus layouts; host must negotiate | Medium | Call `getBusArrangement` / `setBusArrangements` to negotiate channel configs (mono, stereo, surround). Must provide buffer pointers for ALL buses including inactive ones (data pointer can be null, but the bus entry must exist). |
| ProcessSetup configuration | Tell plugin about sample rate, block size, processing mode | Low | Must call before `setActive(true)`. Set `processMode = kOffline` for our use case. |
| Offline processing mode flag | Signal non-realtime operation to plugin | Low | Call `setIoMode(kOfflineProcessing)` on IComponent. Plugins may use higher-quality algorithms (oversampling, longer convolutions) when they know processing is offline. |
| Block-based audio processing | Feed audio buffers through plugin via process() | High | Call `process()` with ProcessData containing audio buffers and parameter changes. Must handle variable block sizes, proper buffer allocation, and 32-bit float sample format. |
| Silence flag handling | Performance optimization required by well-behaved hosts | Low | Set `kSilent` flag on input buffers containing silence. Check output silence flags to skip downstream processing. Plugins optimize when they know input is silent. |
| Tail handling | Reverbs/delays produce output after input ends | Medium | Query `getTailSamples()` after processing. Continue calling `process()` with silent input until tail completes. `kInfiniteTail` means plugin is a generator -- keep processing indefinitely (relevant for instrument plugins). |
| Latency reporting | Plugins introduce processing delay that must be compensated | Medium | Query `getLatencySamples()`. For offline processing, pre-roll input or offset output alignment. Listen for `restartComponent(kLatencyChanged)` callbacks -- plugin latency can change during operation. |
| Audio file input/output | Must accept and produce audio files | Low | Read/write WAV, FLAC, etc. Use symphonia (decode) and hound (encode) in Rust. Convert between file sample format and 32-bit float processing format. |

### Parameter System

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Parameter enumeration | Must discover all plugin parameters and their metadata | Low | `getParameterCount()` + `getParameterInfo(index)` on IEditController. Returns ParameterInfo: ID, title, shortTitle, units string, stepCount, defaultNormalizedValue, flags, unitId. |
| Parameter value read/write | Core interaction for controlling plugins | Medium | All values normalized to `[0.0, 1.0]` as 64-bit doubles. Use `setParamNormalized()` / `getParamNormalized()`. Discrete params: `stepCount` of N means N+1 possible states. Normalization: `discrete_value / stepCount`. |
| Parameter change delivery via process() | The ONLY way to change plugin processor state | High | Parameters reach the processor ONLY through `IParameterChanges` in the `process()` call. Host must create `IParamValueQueue` objects per changed parameter, with sample-accurate offsets within the audio block. This is non-negotiable -- calling setParamNormalized on the controller is not enough. |
| Parameter display conversion | Show human-readable values to user/AI | Low | `getParamStringByValue(id, value)` returns display text (e.g., "-6.0 dB", "1/4 note"). `getParamValueByString(id, string)` does reverse lookup. Essential for AI to speak in musical terms, not raw 0-1 floats. |
| Parameter flags interpretation | Know what each parameter can do | Low | `kCanAutomate` = host can automate it. `kIsBypass` = the plugin's bypass control. `kIsReadOnly` = informational only, don't modify. `kIsHidden` = plugin wants it hidden from users. `kIsList` = present as dropdown. |

### State and Preset Management

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| State save/restore (processor) | Persist and recall plugin configuration | Medium | `IComponent::getState(stream)` serializes processor state to binary blob. `setState(stream)` restores. This is the canonical, complete plugin state including internal data not exposed as parameters. |
| State sync to controller | Keep controller in sync after state load | Medium | After loading processor state, MUST call `IEditController::setComponentState(stream)` with the SAME processor state stream. Then rescan parameter values. Failure to do this causes controller/processor desync -- a common host bug. |
| Preset save/load (.vstpreset) | Standard preset interchange format for DAW interop | Medium | Binary format: 48-byte header ("VST3" 4B, version 4B, classID 32B ASCII, chunkListOffset 8B), data chunks ("Comp" for processor, "Cont" for controller), chunk list ("List" 4B, count 4B, entries with chunkID/offset/size). SDK provides `PresetFile` helper class. |
| Preset location awareness | Find factory and user presets on disk | Low | OS-defined standard paths. Host scans these directories to enumerate available presets per plugin. |

### Host Callback Interfaces

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| IHostApplication | Plugins query host identity during init | Low | Provide host name and version. Passed as context during `initialize()`. Must also support `createInstance()` for host-provided objects. |
| IComponentHandler | Plugin-to-host parameter communication channel | Medium | Must implement `beginEdit(id)`, `performEdit(id, value)`, `endEdit(id)`, `restartComponent(flags)`. Plugins call these when their internal state changes, e.g., from program change or inter-parameter dependencies. |
| Parameter flush (no audio mode) | Plugins need param updates when audio engine is idle | Medium | When not actively processing but plugin is activated, host must periodically call `process()` with null audio buffers to deliver pending parameter changes. Without this, parameter changes made via controller are never seen by processor. |

### Plugin Metadata

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Plugin category reading | Know if plugin is effect vs instrument, and what subcategory | Low | Read `subCategories` from class info. Pipe-separated: "Fx\|EQ", "Fx\|Reverb", "Instrument\|Sampler". Full list includes ~40 categories: kFxDelay, kFxDynamics, kFxEQ, kFxFilter, kFxModulation, kFxPitchShift, kFxReverb, kFxSpatial, kFxVocals, kInstrumentDrum, kInstrumentPiano, kInstrumentSampler, kInstrumentSynth, etc. |
| Plugin info extraction | Name, vendor, version, SDK version | Low | Available from IPluginFactory class info. Can be read without full instantiation (for scanning/caching). |
| MCP tool interface | AI agents interact via MCP protocol | Medium | Expose all plugin operations as MCP tools with typed schemas. This is the primary interface for AI interaction. |

---

## Differentiators

Features that set this product apart. Not required for VST3 compliance, but critical for the AI-driven use case.

### Parameter Intelligence (Core Product Value)

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Unit-based parameter grouping (IUnitInfo) | Organize parameters by functional section for focused AI context | Medium | Plugins optionally implement IUnitInfo. Units form a tree: root(id=0) -> children. Each unit has ID, name, parentID. Parameters belong to units via `unitId` field in ParameterInfo. This is THE primary mechanism for "focus mode" -- present only parameters from a relevant unit (e.g., show only EQ params when user says "brighter"). |
| Semantic parameter classification | Understand what parameters DO based on name/context | High | Parse parameter names against keyword dictionaries. "freq/frequency/hz" -> tone. "threshold/ratio/attack/release" -> dynamics. "reverb/delay/room/decay" -> spatial. "gain/volume/level/drive" -> level. Enables mapping user intent to parameter subsets. |
| Smart parameter subsetting (Focus Mode) | Expose only relevant parameters to AI per interaction | High | Given user intent, select 5-15 relevant parameters from potentially hundreds. Combine: (1) unit hierarchy if available, (2) flag filtering (exclude kIsReadOnly, kIsHidden), (3) semantic name matching, (4) importance ranking. This IS the product's core differentiator. |
| Parameter value snapshotting and diffing | Track AI changes for undo, A/B, audit trails | Low | Capture parameter ID->value maps before and after AI modifications. Enable "undo last change", side-by-side comparison, and human-readable diff reports ("EQ Band 3 Gain: +2.0 dB -> +5.5 dB"). |
| IComponentHandler2 grouped edits | Coordinate multi-parameter AI changes atomically | Low | Since VST 3.1. Enables grouped begin/end edit for multiple parameters at once. When AI adjusts 5 parameters simultaneously (e.g., "add warmth"), group them so plugin handles the change atomically. |

### Preset Management (Advanced)

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Parameter-level preset storage | Human/AI-readable preset representation alongside binary state | Medium | Store presets as `{paramID: normalizedValue, ...}` maps WITH display values. Enables: partial preset application ("apply only the EQ settings"), AI-readable descriptions, and preset diffs. SUPPLEMENT to binary state, not replacement -- some plugin state is not in parameters. |
| Preset A/B comparison | Compare AI-modified state vs original | Low | Store two full state snapshots, provide toggle mechanism and parameter-level diff. Essential UX for iterative AI tweaking. |
| Preset indexing and search | Find presets by name, tags, characteristics | Medium | Scan preset directories, parse .vstpreset metadata, build searchable index by plugin classID, name, source (factory/user/ai-generated), tags. |
| Program list support (IUnitInfo) | Access built-in plugin program lists | Medium | Some plugins expose program lists via IUnitInfo. Host can enumerate and select programs. Enables "start from the warm pad preset" type AI commands. |

### Processing Intelligence

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Plugin chain management | Route audio through multiple plugins in series | Medium | Essential for real workflows: EQ -> Compressor -> Reverb. Manage buffer passing between plugins, accumulate latency compensation, maintain per-plugin state. |
| Batch rendering | Process multiple files with same chain/settings | Low | Iterate over input files, reuse loaded plugin instances and state. Avoid per-file plugin instantiation overhead. |
| Audio analysis feedback | Quantify the effect of AI changes | Medium | Compute loudness (LUFS), peak levels, spectral centroid, RMS before and after processing. Return as structured data. Enables AI to verify: "I tried to make it brighter -- did the spectral centroid actually shift up?" |
| Sample rate flexibility | Process at elevated quality when offline | Low | Since we're offline, can process at 88.2k or 96k for quality, then downsample. Plugin must accept the rate in ProcessSetup -- negotiate gracefully. |
| Progress reporting | Track completion of long offline renders | Low | Count blocks processed vs total, estimate time remaining. Important for large files and plugin chains. |

### Robustness

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Plugin scan crash protection | Survive buggy plugin initialization | High | Run scanning in a subprocess. If a plugin crashes during scan, the subprocess dies but the main host survives. Mark that plugin as problematic, continue scanning others. JUCE and most DAWs do this. |
| Hidden message loop for headless operation | Prevent the ~5% of plugins that render silence without a GUI message loop | Medium | CRITICAL: Some plugins require a platform message loop even when no GUI is displayed. Without it, they produce silence or fail to initialize. Solution: start a platform event loop (e.g., via JUCE GUI app template or platform-specific init) but never create windows. This is a known gotcha documented in JUCE forums. |
| Graceful timeout handling | Detect and recover from hung plugins | Medium | Set processing timeouts. If `process()` doesn't return within threshold, terminate plugin process and report error. Prevents indefinite hangs on misbehaving plugins. |
| State round-trip verification | Catch save/load bugs early | Low | After setState, rescan parameters and compare to expected values. REAPER has documented issues with VST3 parameter persistence -- don't trust that setState "just works". |

---

## Anti-Features

Features to explicitly NOT build.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| Plugin GUI rendering | Headless by design. GUI adds massive platform complexity (windowing, event loops, rendering, plugin view embedding). No user is looking at plugin UIs. | Expose all control via MCP tools and parameter API. Still need hidden message loop (see robustness above). |
| DAW-style timeline/arrangement | Scope creep. Not needed for AI-controlled processing of audio files. | Simple input-file -> plugin chain -> output-file pipeline. |
| VST2 support | Deprecated. Steinberg discontinued licensing. Ecosystem has moved to VST3. | VST3 only. |
| CLAP plugin support | Different API entirely. Doubles hosting complexity. | VST3 first. CLAP could be a future milestone if demand exists. |
| Real-time audio device I/O | Adds CPAL dependency, real-time thread constraints, ASIO/CoreAudio/ALSA driver management. Separate domain. | Start with offline file-based rendering. Add real-time as a future milestone. |
| Plugin marketplace/download | Out of scope. Users install their own plugins. | Scan system paths only. |
| ARA (Audio Random Access) | Specialized protocol for tight DAW integration (Melodyne, etc.). Irrelevant for headless batch processing. | Standard VST3 processing only. |
| MIDI device enumeration | No live MIDI input. AI generates parameter changes, not MIDI performance. | Support programmatic MIDI event injection for instrument plugins, but no device management. |
| Visual waveform/spectrum display | No GUI. | Return analysis data as structured output (JSON) for potential frontend consumption. |

---

## Feature Dependencies

```
Plugin Scanning ──────> Plugin Loading ──────> Audio Processing Pipeline
       │                      │                        │
       │                      ├──> Parameter Enumeration ──> Parameter Change Delivery
       │                      │           │                         │
       │                      │           └──> Unit/Group Discovery ──> Focus Mode (AI subset)
       │                      │                                              │
       │                      │                                     AI Context Window Mgmt
       │                      │
       │                      ├──> State Save/Restore ──> Preset Management
       │                      │         │                       │
       │                      │         ├──> A/B Comparison     │
       │                      │         └──> Diff/Delta Tracking│
       │                      │                                 │
       │                      └──> Bus Configuration ───────────┘
       │
       └──> Metadata Caching (avoid re-scanning)

Audio Processing ──> Plugin Chain Management ──> Batch Rendering
       │
       └──> Audio Analysis (pre/post comparison)

Parameter Enumeration ──> Semantic Classification ──> Importance Ranking
                                    │
                                    └──> Intent Mapping ("brighter" -> high-freq EQ params)
```

---

## Parameter Exposure Strategies for AI Context Windows

This is the core differentiator. Plugins can have 50-500+ parameters. An AI context window cannot reason about all of them.

### Strategy 1: Unit-Based Focusing (Primary -- Best Quality)

Use VST3 Unit hierarchy to present structural parameter subsets.

```
Example: Channel strip plugin with IUnitInfo:
  Root Unit (id=0)
  +-- EQ Unit (id=1) -- 20 parameters
  |   +-- Band 1 (id=4): freq, gain, Q, type, enable
  |   +-- Band 2 (id=5): freq, gain, Q, type, enable
  |   +-- Band 3 (id=6): freq, gain, Q, type, enable
  |   +-- Band 4 (id=7): freq, gain, Q, type, enable
  +-- Compressor Unit (id=2) -- 8 parameters
  |   threshold, ratio, attack, release, knee, makeup, sidechain, enable
  +-- Output Unit (id=3) -- 4 parameters
      level, pan, phase, mute

User says "make the vocal brighter":
  -> Focus to EQ Unit -> 20 parameters
  -> Further focus to high-freq bands -> 10 parameters
```

**Caveat:** Not all plugins implement IUnitInfo. Need fallback strategies.

### Strategy 2: Flag-Based Filtering (Always Available)

```
Exclude: kIsReadOnly, kIsHidden, kIsBypass (handle bypass separately)
Prioritize: kCanAutomate (the "important" parameters plugins expose)
```

### Strategy 3: Semantic Name Analysis (Fallback When No Units)

```
Tone keywords:     "freq", "high", "treble", "bright", "presence", "air", "tone", "tilt"
Dynamics keywords: "threshold", "ratio", "attack", "release", "compress", "gate", "limit"
Spatial keywords:  "reverb", "delay", "room", "size", "decay", "wet", "dry", "width", "pan"
Level keywords:    "gain", "volume", "level", "output", "input", "mix", "drive"
```

### Strategy 4: Importance Ranking

```
1. Wide-range continuous params are more impactful than toggles
2. Params far from default are already "in use" and more contextually relevant
3. Plugin category informs ranking (EQ plugin -> frequency/gain params rank highest)
4. User intent narrows further ("brighter" -> high-frequency params)
```

### Recommended Combined Approach

```
1. Try IUnitInfo for structural grouping (best, not always available)
2. Apply flag filtering to remove noise (always available)
3. Use semantic name analysis for intent mapping (always available)
4. Apply importance ranking to order results (always available)
5. Present top-N parameters to AI (target: 5-15 per interaction round)
```

---

## Preset Management Implementation Patterns

### Pattern 1: Direct State Serialization (MVP)

Works with every VST3 plugin. Most reliable.

```
Save:  IComponent::getState(stream) -> processor blob
       IEditController::getState(stream) -> controller blob (optional)
       Store both with metadata (plugin classID, timestamp, label)

Load:  IComponent::setState(stream) <- processor blob
       IEditController::setComponentState(stream) <- SAME processor blob (syncs controller)
       IEditController::setState(stream) <- controller blob (if saved)
       Rescan all parameter values via getParamNormalized()
```

### Pattern 2: .vstpreset Format (Interoperability)

Enables preset sharing with Cubase, other DAWs.

```
Binary structure:
  Header (48B): "VST3"(4B) | version(4B) | classID(32B) | chunkListOffset(8B)
  Data:         [processor state chunk "Comp"] [controller state chunk "Cont"]
  Chunk list:   "List"(4B) | count(4B) | entries[]{id(4B), offset(8B), size(8B)}

SDK PresetFile class handles read/write.
```

### Pattern 3: Parameter-Level Storage (AI Interaction Supplement)

Store as structured `{paramID: {normalized, display, name}}` maps alongside binary state.

Enables: human-readable diffs, partial application, AI-readable descriptions.
**Not a replacement for binary state** -- some plugin internals are not in parameters.

---

## Real-World Host Lessons

### Bitwig Studio
- Five sandbox levels: "within engine" to "individual per instance"
- Crash recovery replaces crashed plugin UI with reload notification
- **Lesson:** Plugin isolation is worth the complexity for production use

### REAPER
- Documented VST3 parameter persistence bugs on save/reload
- **Lesson:** Always verify state round-trips. Don't trust setState blindly.

### Cubase/Nuendo
- Direct Offline Processing (DOP): process audio through plugins non-realtime using `setIoMode(kOfflineProcessing)`
- **Lesson:** Offline VST3 processing is a well-supported, first-class path in the spec

### Plugalyzer (Headless CLI Host)
- Reads audio/MIDI, processes through plugin, writes output. No GUI.
- **Lesson:** Validates headless approach, but is a simple tool -- we're building something more sophisticated

### Sushi (Elk Audio OS)
- Track-based headless DAW on embedded Linux. Plugin chains, MIDI, full hosting.
- **Lesson:** Production headless VST3 hosting is proven at scale

### JUCE AudioPluginHost
- `moduleinfo.json` for fast scanning without loading binaries
- Parameter updates cached from audio thread, dispatched at 60Hz on message thread
- `juce_audio_processors_headless` module exists for GUI-less hosting
- **Lesson:** JUCE provides battle-tested infrastructure. Study its approach even if not using JUCE directly.

---

## MVP Recommendation

**Phase 1 -- Single Plugin Processing (Table Stakes):**
1. Plugin scanning and discovery with metadata caching
2. Plugin loading, instantiation, lifecycle management
3. Audio bus configuration (stereo in/out minimum)
4. Offline block-based processing pipeline
5. Full parameter enumeration with metadata
6. Parameter read/write with proper process() delivery
7. State save/restore (binary blob)
8. Hidden message loop (prevents ~5% of plugins rendering silence)
9. IComponentHandler implementation
10. MCP tool interface for all above

**Phase 2 -- Chain, State, and Focus:**
1. Plugin chain management (multiple plugins in series)
2. .vstpreset read/write for interoperability
3. Unit-based parameter grouping (IUnitInfo)
4. Semantic parameter classification and Focus Mode
5. Scan crash protection (subprocess scanning)
6. Tail and latency handling
7. Preset A/B comparison

**Phase 3 -- Intelligence and Polish:**
1. Audio analysis feedback (pre/post loudness, spectrum)
2. Parameter importance ranking
3. Batch rendering
4. Preset indexing and search
5. State round-trip verification
6. Plugin compatibility tracking (which plugins work well headless)

**Defer Indefinitely:**
- Plugin GUI rendering
- Realtime audio device I/O
- VST2/CLAP/AU support
- ARA support
- MIDI device management
- DAW timeline/arrangement

---

## Sources

### Official Steinberg Documentation (HIGH confidence)
- [VST 3 Developer Portal](https://steinbergmedia.github.io/vst3_dev_portal/)
- [VST 3 API Documentation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/API+Documentation/Index.html)
- [Parameters and Automation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Parameters+Automation/Index.html)
- [Presets and Program Lists](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Presets+Program+Lists/Index.html)
- [Preset Format Specification](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Preset+Format.html)
- [VST 3 Units](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+3+Units/Index.html)
- [Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html)
- [Plugin Type SubCategories](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/group__plugType.html)
- [IComponentHandler Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IComponentHandler.html)
- [IEditController Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IEditController.html)

### Real-World Host Implementations (MEDIUM confidence)
- [Bitwig Crash Protection](https://www.bitwig.com/learnings/plug-in-hosting-crash-protection-in-bitwig-studio-20/)
- [Bitwig Plugin Handling Options](https://www.bitwig.com/userguide/latest/vst_plug-in_handling_and_options/)
- [Sushi - Elk Audio Headless DAW](https://github.com/elk-audio/sushi)
- [Plugalyzer - Command-line VST3 Host](https://github.com/CrushedPixel/Plugalyzer)
- [Robust VST3 Host in Rust (cutoff-vst)](https://renauddenis.com/case-studies/rust-vst)

### Community Discussion (MEDIUM confidence)
- [JUCE Headless VST3 Host Silence Issue](https://forum.juce.com/t/headless-vst3-host-some-plugins-render-silence/58169)
- [KVR: What does scanning VST plugins mean](https://www.kvraudio.com/forum/viewtopic.php?t=530945)
- [JUCE VST3 Parameter Updates Discussion](https://forum.juce.com/t/vst3-parameter-updates-automation-vs-host-refresh/67373)
- [Steinberg Forums: Plugin Crash During Scanning](https://forums.steinberg.net/t/vst3-host-plugin-crash-while-scanning-bundleentry-bundleexit/776824)
