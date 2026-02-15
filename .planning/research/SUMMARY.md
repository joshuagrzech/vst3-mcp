# Project Research Summary

**Project:** AgentAudio (VST3 Wrapper Plugin with Embedded MCP Server)
**Domain:** Real-time audio plugin hosting with AI parameter control
**Researched:** 2026-02-15
**Confidence:** MEDIUM-HIGH

## Executive Summary

AgentAudio is a VST3 plugin built with nih-plug that wraps and hosts a child VST3 plugin while exposing AI-controlled parameter manipulation through an embedded MCP server. This is a plugin-hosting-inside-plugin architecture with three asynchronous contexts: a real-time audio thread (nih-plug process callback), a Tokio runtime (MCP server on background thread), and a GUI thread (nih-plug-egui + child IPlugView). The core technical challenge is bridging the real-time audio thread with async AI control without violating real-time safety constraints.

The recommended approach is to build in risk-ordered phases: first prove that nih-plug can host a child VST3 plugin with correct real-time audio passthrough, then add lock-free communication between the audio thread and MCP server, and finally implement the "Focus Mode" parameter filtering UI. The current codebase provides a working offline headless VST3 host with MCP server that can be adapted by replacing the mutex-based architecture with lock-free ring buffers (rtrb) and moving from kOffline to kRealtime process mode.

The primary risks are: (1) memory allocation or mutex use on the audio thread causing glitches, (2) VST3 threading model violations where IEditController is called from wrong threads, (3) COM pointer lifetime bugs causing crashes on plugin unload, and (4) Tokio runtime interactions blocking the audio thread. All are avoidable through disciplined architecture: audio thread exclusively owns the child plugin, communicates only through rtrb SPSC ring buffers, pre-allocates all buffers, and never touches Tokio APIs.

## Key Findings

### Recommended Stack

The stack centers on nih-plug (Rust's primary VST3/CLAP framework) for the outer wrapper plugin, the vst3 crate (0.3.0) for hosting the inner child plugin via raw COM interfaces, rmcp for the MCP server (requires Tokio), and rtrb for wait-free communication between threads.

**Core technologies:**
- **nih-plug (git)**: VST3 wrapper framework for the outer plugin — only serious Rust plugin framework with active maintenance, handles parameter system and real-time buffer management
- **vst3 (0.3.0)**: VST3 COM bindings for hosting child plugins — provides IComponent/IAudioProcessor interfaces needed to load and control child plugins
- **rmcp (0.15.0)**: Official Rust MCP SDK — provides tool macro and transport abstractions for AI integration
- **Tokio (1.49.0)**: Async runtime for MCP server — runs on dedicated background thread, completely isolated from audio thread
- **rtrb (0.3.2)**: Wait-free SPSC ring buffer — enables real-time safe communication between MCP thread and audio thread
- **nih-plug-egui (git)**: Immediate-mode GUI framework — integrates egui 0.31 with nih-plug's parameter system for Focus Mode controls
- **libloading (0.9.0)**: Dynamic library loading — loads .vst3 bundles at runtime

**Critical architectural notes:**
- nih-plug uses a forked vst3-sys internally; the vst3 crate (0.3.0) is a separate binding that can coexist
- Never use mutexes on the audio thread; rtrb provides wait-free SPSC for MCP-to-audio commands
- Tokio runtime must run on separate thread from audio processing
- Child plugin process mode must be kRealtime (not kOffline as in current code)

### Expected Features

The feature landscape divides into table stakes (VST3 hosting requirements), differentiators (Focus Mode and MCP integration), and anti-features (plugin chaining, real-time MIDI generation by AI).

**Must have (table stakes):**
- Plugin loading with full VST3 lifecycle state machine (Created -> SetupDone -> Active -> Processing)
- Audio passthrough with correct buffer format conversion and tail handling
- Parameter enumeration, read, and write via IParameterChanges
- State save/load with .vstpreset format
- Component/Controller separation handling (both unified and split patterns)
- IComponentHandler callbacks (beginEdit/performEdit/endEdit/restartComponent)

**Should have (competitive advantage):**
- Focus Mode: parameter filtering UI where user marks which params are AI-exposed (solves "200 params overwhelm AI" problem)
- MCP tools: list_params, get_param, set_param, batch_set filtered by Focus Mode
- Parameter display strings (AI needs "3.5 dB" not "0.72")
- Unit hierarchy (IUnitInfo) for grouping params logically
- Semantic parameter aliases (rename "Param 42" to "reverb_decay_time" for AI readability)
- MIDI event routing (unlocks instrument plugins: synths, samplers)

**Defer (v2+):**
- Child plugin GUI embedding (IPlugView) — high complexity, platform-specific X11 work on Linux
- Real-time audio streaming — requires audio I/O backend (CPAL/JACK), fundamentally changes architecture from offline
- Plugin chaining — building a DAW, massive scope creep
- Custom DSP — compete with dedicated plugins, maintenance burden

**MVP definition:** Offline processing with working parameter write (IParameterChanges implementation), parameter display strings, Focus Mode basic JSON config, and MCP tools. This validates that AI can usefully control a VST3 plugin before investing in real-time or GUI complexity.

### Architecture Approach

The architecture is a three-thread system: (1) audio thread runs nih-plug's process callback and forwards buffers to the child plugin, (2) MCP thread runs a Tokio runtime hosting the rmcp server, (3) GUI thread runs nih-plug-egui for wrapper controls and optionally embeds child IPlugView. Threads communicate exclusively through lock-free data structures: rtrb SPSC ring buffers for command/notification flow, AtomicF32 arrays for parameter value caching.

**Major components:**
1. **nih-plug Plugin struct** — implements Plugin trait, owns audio thread lifecycle, delegates processing to child plugin host
2. **Child Plugin Host** — loads/initializes/processes child VST3 via COM (adapts current PluginInstance from codebase to real-time mode)
3. **MCP Server** — handles AI tool calls for parameter control, runs on dedicated Tokio runtime thread
4. **Lock-Free Bridge** — rtrb ring buffers transfer commands (MCP->audio) and notifications (audio->MCP) without blocking
5. **Focus Mode Manager** — tracks which parameters are exposed to AI via atomic bitfield
6. **GUI (Wrapper)** — nih-plug-egui editor with Focus Mode controls and status display
7. **GUI (Child Editor)** — optional IPlugView embedding with platform-specific window parenting (deferred to v2)

**Critical patterns:**
- Audio thread exclusively owns PluginInstance, never shares via Arc/Mutex
- Tokio runtime spawned in initialize(), shut down in deactivate()/Drop
- Pre-allocate all buffers during setup, zero allocation in process()
- Child plugin tied to wrapper lifecycle (mirror sample rate, buffer size, activation state)
- Parameter changes delivered via IParameterChanges, not direct controller calls

**Highest-risk integration:** IPlugView embedding on Linux requires X11/Wayland window parenting inside egui context. No clear examples found. This is deferred to Phase 4.

### Critical Pitfalls

The top five pitfalls are all related to real-time safety and threading correctness. The audio thread is unforgiving: any allocation, mutex lock, or blocking call causes audible glitches.

1. **Memory allocation on audio thread** — Heap allocation (Vec::push, String::format, Box::new) causes unbounded latency. Current code allocates Vec per process call (lines 371-374, 388-391 in plugin.rs). Fix by pre-allocating buffers during setup and reusing. Enable assert_process_allocs feature to catch violations.

2. **Mutex on audio thread (priority inversion)** — Current AudioHost uses Arc<Mutex<Option<PluginInstance>>> which prevents real-time usage. High-priority audio thread blocks waiting for low-priority MCP thread holding mutex. Redesign with lock-free ring buffers (rtrb) before adding real-time path.

3. **VST3 COM pointer lifetime and drop order** — Dropping module before COM pointers or calling terminate() before releasing interfaces causes use-after-free. Must follow strict teardown order: setProcessing(false) -> setActive(false) -> disconnect -> release controller pointers -> controller.terminate() -> release component pointers -> component.terminate() -> drop module. Current Drop impl needs explicit ordering.

4. **Blocking audio thread with Tokio** — Any Tokio API call (block_on, tokio::sync channels) from audio thread is catastrophic. Tokio runtime must run on completely separate thread. Use rtrb (not tokio::sync::mpsc) for audio-thread communication.

5. **VST3 threading model violations** — VST3 spec mandates IEditController calls from UI thread, IAudioProcessor::process from audio thread. MCP server calling controller methods from Tokio workers violates this. Route all controller calls through dedicated controller thread or command queue.

**Secondary pitfalls:** Plugin crashes taking down host (need out-of-process scanning), incorrect IParameterChanges implementation (no automation without it), denormal floating-point performance traps (flush to zero on audio thread start).

## Implications for Roadmap

Based on research, the roadmap should be risk-ordered: build the hardest/riskiest integrations first to fail fast. The architecture has three high-risk integrations (nih-plug + child hosting, lock-free bridge + MCP, child GUI embedding) that must be proven independently before combining.

### Phase 1: Real-Time Audio Passthrough (Prove nih-plug + Child Hosting)
**Rationale:** This is the highest-risk integration. Proving that nih-plug can host a child VST3 with correct real-time audio passthrough de-risks the entire project. If this fails, the project pivots to a different architecture.
**Delivers:** Minimal nih-plug plugin that loads a child VST3 and passes audio through it in a DAW with zero glitches
**Addresses:** Plugin loading, lifecycle state machine, audio passthrough, sample rate negotiation, tail handling, multi-channel support (table stakes from FEATURES.md)
**Avoids:** Allocation on audio thread (pre-allocate buffers), COM drop order bugs (explicit teardown sequence)
**Validates:**
- Buffer format conversion (nih-plug Buffer <-> VST3 AudioBusBuffers) works correctly
- Lifecycle alignment (initialize/process/deactivate) between wrapper and child
- kRealtime process mode (current code uses kOffline) produces stable output
- No audio glitches under continuous processing in Bitwig/REAPER

### Phase 2: Lock-Free Bridge + MCP Integration (Prove MCP + Real-Time Coexistence)
**Rationale:** The second-highest risk is proving that Tokio runtime and audio thread can coexist without mutual interference. This phase adds the lock-free communication layer and MCP server.
**Delivers:** MCP server running on background thread, AI tools can read/write parameters, audio thread shows zero blocking
**Uses:** rtrb (0.3.2) for SPSC ring buffers, rmcp (0.15.0) for MCP server, Tokio (1.49.0) on dedicated thread
**Implements:** Lock-Free Bridge component (command/notification queues), MCP Server component (tool handlers)
**Addresses:** MCP tool: get_param, set_param, get_plugin_info (differentiators from FEATURES.md)
**Avoids:** Mutex on audio thread (use rtrb), blocking audio thread with Tokio (separate runtime thread), VST3 threading violations (controller thread separation)
**Validates:**
- Tokio runtime spawns/shuts down cleanly within plugin lifecycle
- Parameter changes from MCP reach child plugin and produce audible results
- Audio thread latency stays under 1ms during concurrent MCP requests
- stdio transport works inside DAW process (or pivot to SSE if it doesn't)

### Phase 3: IParameterChanges Implementation + Parameter Display
**Rationale:** The current codebase passes null for inputParameterChanges, which prevents proper parameter automation. This phase implements the missing COM interfaces and adds human-readable parameter values.
**Delivers:** Sample-accurate parameter automation, parameter display strings ("3.5 dB"), parameter flag filtering
**Implements:** IParameterChanges and IParamValueQueue COM objects (host-side), parameter value display via getParamStringByValue
**Addresses:** Parameter write via IParameterChanges (table stakes), parameter display strings (differentiator)
**Avoids:** Incorrect IParameterChanges (zipper noise, ignored params), allocation in process (pre-allocated change queues)
**Validates:**
- Parameter sweeps produce smooth, zipper-free output
- Plugins respond correctly to automated parameters
- Read-only params (kIsReadOnly flag) are filtered out

### Phase 4: Focus Mode + ComponentHandler Integration
**Rationale:** Focus Mode is the core differentiator. This phase adds the parameter exposure tracking and MCP filtering.
**Delivers:** "Wiggle to expose" parameter selection, MCP tools filtered to exposed params only
**Implements:** Focus Mode Manager (atomic bitfield tracking), ComponentHandler intercepts (performEdit captures user tweaks)
**Addresses:** Focus Mode parameter filtering, Focus Mode persistence (differentiators from FEATURES.md)
**Avoids:** Threading violations (ComponentHandler callbacks may come from plugin on any thread, handle with atomics)
**Validates:**
- User can mark/unmark parameters via GUI interaction
- MCP list_params returns only exposed parameters
- Focus Mode config persists across plugin reload

### Phase 5: Wrapper GUI + Polish
**Rationale:** GUI is lower priority than core functionality. Basic wrapper controls (Focus Mode toggles, parameter list) can be terminal-based initially.
**Delivers:** nih-plug-egui wrapper UI, parameter overview, Focus Mode controls
**Implements:** GUI (Wrapper) component with egui widgets
**Addresses:** Wrapper UI (differentiator)
**Validates:**
- GUI displays child plugin parameters with current values
- Focus Mode listen/clear buttons work
- GUI reads atomic param cache without blocking audio thread

### Phase 6: Child Plugin GUI Embedding (DEFERRED)
**Rationale:** This is high-risk, platform-specific, and not required for AI control. Defer until core functionality is proven.
**Delivers:** Child plugin's native GUI embedded alongside wrapper controls
**Implements:** GUI (Child Editor) component with platform-specific window parenting
**Risk:** X11/Wayland window management inside egui context has no clear examples. May be infeasible without significant research.

### Phase Ordering Rationale

- **Risk-ordered, not feature-ordered:** Phase 1 proves the hardest technical problem (plugin-in-plugin with real-time audio). If this fails, we know early.
- **Dependency-driven:** Lock-free bridge (Phase 2) depends on working audio pipeline (Phase 1). Focus Mode (Phase 4) depends on parameter automation (Phase 3).
- **Incremental validation:** Each phase adds one high-risk integration. No phase combines multiple unknowns.
- **Defer GUI complexity:** Child GUI embedding (Phase 6) is deferred because AI control doesn't require visual feedback. Terminal-based parameter display is sufficient for MVP.
- **Pitfall avoidance:** Phase 1 establishes allocation-free patterns before adding MCP (Phase 2). Phase 2 establishes lock-free communication before adding Focus Mode threading (Phase 4).

### Research Flags

Phases likely needing deeper research during planning:
- **Phase 1:** nih-plug Buffer <-> VST3 AudioBusBuffers format conversion — research buffer layouts, channel ordering, sample format (f32 vs f64)
- **Phase 2:** stdio transport inside DAW plugin — verify stdin/stdout are accessible, or research SSE/WebSocket transport
- **Phase 6:** IPlugView X11 embedding in egui — platform-specific window parenting, resize coordination, IPlugFrame callbacks (LOW confidence, no examples found)

Phases with standard patterns (skip research-phase):
- **Phase 3:** IParameterChanges is well-documented in VST3 SDK, straightforward COM implementation
- **Phase 4:** Atomic bitfield and ComponentHandler interception are standard patterns
- **Phase 5:** egui basics are well-documented, nih-plug-egui provides integration

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | nih-plug, rmcp, rtrb, Tokio are all actively maintained with clear docs. vst3 crate (0.3) is proven in current codebase. |
| Features | HIGH | VST3 spec is authoritative. Feature research cross-referenced spec with real-world hosting implementations (cutoff-vst, JUCE wrappers). |
| Architecture | MEDIUM | Core patterns (lock-free queues, Tokio on background thread) are well-established. Plugin-in-plugin is novel but feasible. Child GUI embedding on Linux is LOW confidence (no examples). |
| Pitfalls | HIGH | Real-time audio constraints are well-documented. Pitfalls verified against current codebase (e.g., allocation in process(), mutex on PluginInstance). |

**Overall confidence:** MEDIUM-HIGH

The core real-time audio + MCP integration is feasible with known patterns. The primary uncertainty is child GUI embedding (Phase 6), which is deferred. The architecture is sound if lock-free communication discipline is maintained.

### Gaps to Address

**Child GUI embedding on Linux:** No examples found of embedding IPlugView inside egui context. The standard approach is separate native window positioned adjacent to egui window. This needs prototype validation in Phase 6. Fallback: skip child GUI entirely, provide parameter list in wrapper UI only.

**stdio transport in DAW plugin:** DAWs may redirect or close stdin/stdout. If stdio doesn't work, pivot to SSE (HTTP Server-Sent Events) transport. Research SSE setup during Phase 2 as contingency.

**vst3 crate vs nih-plug's vst3-sys fork:** These are separate type hierarchies. As long as COM objects are never passed between them (they're not — wrapper uses nih-plug types, child uses vst3 crate types), there's no conflict. Validate during Phase 1.

**IComponentHandler thread affinity:** VST3 spec is ambiguous about which thread plugins call restartComponent from. Current assumption is plugins may call from audio thread (spec violation but real-world behavior). Use atomic flags for restartComponent handling. Validate with multiple plugin brands during Phase 3.

## Sources

### Primary (HIGH confidence)
- [nih-plug GitHub repository](https://github.com/robbert-vdh/nih-plug) — plugin framework docs, examples, CHANGELOG confirming egui 0.31
- [VST3 Developer Portal](https://steinbergmedia.github.io/vst3_dev_portal/) — official spec for hosting, parameters, lifecycle
- [IEditController API Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IEditController.html) — parameter interface contracts
- [IParameterChanges API Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IParameterChanges.html) — automation delivery pattern
- [rtrb GitHub](https://github.com/mgeier/rtrb) — wait-free SPSC ring buffer design
- [rmcp GitHub](https://github.com/modelcontextprotocol/rust-sdk) — official Rust MCP SDK
- [Using locks in real-time audio processing, safely](https://timur.audio/using-locks-in-real-time-audio-processing-safely) — authoritative article on RT constraints

### Secondary (MEDIUM confidence)
- [cutoff-vst case study](https://renauddenis.com/case-studies/rust-vst) — real-world VST3 hosting in Rust
- [nih-plug background tasks issue #172](https://github.com/robbert-vdh/nih-plug/issues/172) — background thread patterns in plugins
- [Streamable HTTP MCP in Rust](https://www.shuttle.dev/blog/2025/10/29/stream-http-mcp) — axum + rmcp pattern
- [Tokio: Bridging with sync code](https://tokio.rs/tokio/topics/bridging) — Runtime::new() on std::thread pattern
- [VST3 + Linux + X11 crash (Dplug issue #434)](https://github.com/AuburnSounds/Dplug/issues/434) — real-world GUI embedding bug
- [JUCE forum: VST3 threading issues](https://forum.juce.com/t/vst3-crashing-due-to-ieditcontroller-thread-issues/31168) — threading violations in practice

### Tertiary (LOW confidence)
- [rack crate](https://github.com/sinkingsugar/rack) — VST3 Linux support listed as untested
- [Carla MCP Server](https://mcp.aibase.com/server/1538249238906150913) — competitor, third-party listing
- Community forum discussions on IPlugView X11 embedding (KVR Audio) — anecdotal, low signal

---
*Research completed: 2026-02-15*
*Ready for roadmap: yes*
