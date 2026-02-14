# Project Research Summary

**Project:** Headless VST3 Host as MCP Server
**Domain:** Audio Processing / AI-Controlled Plugin Hosting
**Researched:** 2026-02-14
**Confidence:** MEDIUM-HIGH

## Executive Summary

This project is a headless VST3 audio plugin host that exposes its functionality through the Model Context Protocol (MCP), enabling AI agents to process audio files through professional audio plugins. Expert implementations (DAWs, headless hosts) reveal that VST3 hosting requires careful COM lifecycle management, multi-process isolation for crash protection, and deep understanding of the VST3 specification's threading and state management requirements. The AI-driven use case introduces a unique challenge: VST3 plugins can expose hundreds of parameters, but AI context windows require intelligent parameter subsetting ("focus mode") to remain effective.

The recommended approach combines Rust's safety guarantees with a multi-layered architecture: a supervisor process handling MCP protocol (using the official rmcp SDK) and worker processes hosting VST3 plugins (using coupler-rs vst3 bindings). Critical early decisions include using pure-Rust audio I/O (symphonia for decode, hound for encode), implementing VST3 Unit-based parameter grouping for focus mode, and establishing a two-phase rollout: single-process MVP for validation, followed by multi-process architecture for production robustness.

Key risks center on VST3 ecosystem immaturity in Rust (community reports segfaults with instrument plugins), COM reference counting errors leading to memory leaks or crashes, and the subtle requirement that some plugins need a platform message loop even in headless mode. Mitigation strategies include multi-process isolation (crashes affect only worker, not supervisor), rigorous COM abstraction layers to contain unsafe code, and starting with well-tested open-source plugins (Surge XT) before expanding to commercial plugins.

## Key Findings

### Recommended Stack

The technology stack prioritizes pure-Rust implementations where possible, falling back to well-maintained FFI bindings only for VST3 COM interfaces. The ecosystem has matured significantly with Steinberg's October 2025 release of VST3 SDK 3.8.0 under MIT license, removing previous licensing friction. However, this same SDK update introduced breaking changes in the coupler-rs binding generator (issue #20) that need verification before starting implementation.

**Core technologies:**
- **vst3 (coupler-rs)**: VST3 COM bindings generated from SDK headers, MIT/Apache-2.0 licensed, version 0.3.0 ships pre-generated bindings eliminating build-time C++ dependencies
- **rmcp**: Official Rust MCP SDK from modelcontextprotocol org, version 0.15.0 implements MCP protocol 2025-11-25 with #[tool] macros for clean integration
- **symphonia + hound**: Pure-Rust audio I/O (symphonia for multi-format decoding, hound for WAV encoding) eliminates C dependencies and cross-compilation complexity
- **rtrb + shared_memory**: Lock-free ring buffers paired with cross-platform shared memory for zero-copy audio transfer between supervisor and worker processes
- **tokio**: Async runtime required by rmcp, industry standard for Rust async, must use spawn_blocking for CPU-bound audio work to avoid starving async tasks

**Critical dependency note:** The coupler-rs vst3 crate tracks SDK 3.8.0 but has a known issue with forward-declared Wayland C structs breaking binding generation. Check issue status before starting Phase 1.

### Expected Features

VST3 hosting has well-defined table stakes from the official Steinberg specification, but the AI-driven use case introduces unique differentiators around parameter intelligence and focus mode.

**Must have (table stakes):**
- Plugin scanning, discovery, loading, and lifecycle management (initialize -> setActive -> setProcessing sequence strictly enforced)
- Audio bus configuration negotiation and offline block-based processing pipeline
- Full parameter enumeration, value read/write, and sample-accurate change delivery via process() call
- Plugin state save/restore (binary blobs) with proper controller/processor synchronization
- Host callback interfaces (IHostApplication, IComponentHandler) for bidirectional plugin communication
- Audio file I/O (WAV minimum, multi-format via symphonia as bonus)

**Should have (competitive differentiators):**
- **Unit-based parameter grouping (IUnitInfo)**: VST3's built-in mechanism for organizing parameters by functional section (EQ unit, compressor unit, etc.) — THE primary enabler for focus mode
- **Semantic parameter classification**: Parse parameter names against keyword dictionaries to understand function (frequency/gain/threshold/reverb) enabling intent mapping ("make it brighter" -> high-freq EQ params)
- **Smart parameter subsetting**: Present only 5-15 relevant parameters to AI per interaction, combining unit hierarchy + flag filtering + semantic matching + importance ranking
- **Plugin chain management**: Route audio through multiple plugins in series (EQ -> Compressor -> Reverb), essential for real workflows
- **Preset A/B comparison and parameter-level diffs**: Track AI changes for undo, side-by-side comparison, human-readable audit trails

**Defer (v2+):**
- Plugin GUI rendering (headless by design, massive platform complexity)
- Real-time audio device I/O (offline processing first, real-time as separate milestone)
- VST2/CLAP/AU support (VST3-only initially)
- ARA protocol (specialized DAW integration, irrelevant for headless batch processing)
- MIDI device enumeration (AI generates parameter changes, not MIDI performance)

**Critical insight from FEATURES.md:** Approximately 5% of VST3 plugins require a platform message loop even when no GUI is displayed, otherwise producing silence. Solution: initialize platform event loop without creating windows (documented JUCE pattern). This is a non-negotiable table stake despite being headless.

### Architecture Approach

A supervisor-worker architecture with clear separation between MCP protocol handling and crash-prone VST3 plugin hosting. The supervisor process runs the rmcp MCP server, manages plugin registry and scanning, and owns worker process lifecycle. Each worker process hosts a plugin or plugin chain, isolating crashes to avoid killing the MCP connection. Communication uses two channels: Unix domain sockets for control plane (commands, responses, parameter changes) and shared memory with ring buffers for data plane (zero-copy audio transfer).

**Major components:**
1. **MCP Server (Supervisor)** — Protocol handling, session management, worker lifecycle orchestration, plugin metadata registry with cached scan results
2. **Worker Process** — Loads VST3 plugins, executes audio processing, isolates crashes from supervisor, communicates via Unix sockets + shared memory
3. **Plugin Registry** — Scans OS-standard VST3 paths, caches metadata (using moduleinfo.json for fast extraction), provides plugin discovery API
4. **VST3 Host Abstraction** — Three-layer architecture: raw COM bindings (Layer 1), safe COM wrappers with RAII (Layer 2), public plugin API (Layer 3)
5. **Audio Pipeline** — Symphonia decode -> deinterleave to planar buffers -> block-based processing -> interleave -> hound encode

**Architectural patterns:**
- Layered VST3 abstraction to contain unsafe COM code in Layer 2 only
- State machine enforcement for plugin activation sequence (Created -> SetupDone -> Active -> Processing)
- Block-based processing with pre-allocated buffers (no allocation in audio loop)
- MCP resource exposure for plugin metadata discovery before tool calls

**Simplification path:** Single-process MVP combines supervisor + worker in one binary using spawn_blocking for audio work. Multi-process architecture added in Phase 2 for production robustness.

### Critical Pitfalls

Research identified 12 pitfalls across three severity tiers. Top 5 critical/moderate issues:

1. **COM Lifecycle Mismanagement** — VST3 plugins use reference-counted COM objects. Missing Release leaks memory, double-Release causes use-after-free. Prevention: build RAII VstPtr wrappers at Layer 2 that call AddRef on clone and Release on drop, never expose raw pointers above Layer 2, test with multiple plugin vendors
2. **Plugin Thread Affinity Violations** — VST3 spec defines separate threading contexts (UI thread for controller, audio thread for processor, main thread for init). Calling from wrong thread causes crashes/deadlocks in specific plugins. Prevention: respect threading model even in headless mode, use Rust type system to enforce (marker types, !Send traits)
3. **Plugin Segfaults Killing the Host** — Buggy plugins can crash. In single-process architecture, this kills MCP connection. Prevention: multi-process isolation from Phase 2 onward, supervisor detects worker crash and reports error gracefully to AI agent
4. **Blocking Tokio Runtime with Audio** — Processing audio on tokio async threads starves async tasks, causes timeouts. Prevention: always use spawn_blocking for CPU-bound work (symphonia decode, plugin processing), or delegate to worker processes
5. **Incorrect Buffer Layout** — VST3 uses non-interleaved (planar) buffers, many Rust crates use interleaved. Mixing produces garbage audio. Prevention: explicit interleave/deinterleave functions, unit tests with known signals (sine wave round-trip)

**Phase-specific warning:** Plugin scanning is slow (plugins load sample libraries, check copy protection during init) and crash-prone. Run scanning in subprocess from day one, cache aggressively using moduleinfo.json for metadata extraction without binary loading.

## Implications for Roadmap

Based on research, suggested four-phase structure balancing early validation, production robustness, and AI-specific intelligence features.

### Phase 1: Single-Plugin MVP (Foundation + Validation)
**Rationale:** Validates core VST3 hosting without multi-process complexity. Establishes safe COM abstractions and audio pipeline patterns that persist through all phases. Delivers working end-to-end flow for AI testing.

**Delivers:** Single plugin processing (load plugin -> set parameters -> process audio file -> write output) exposed via MCP tools. Supervisor + worker combined in one binary.

**Addresses features:**
- Plugin scanning with moduleinfo.json caching
- Plugin loading and lifecycle management (full state machine)
- Offline block-based processing
- Parameter enumeration and basic value manipulation
- State save/restore (binary blobs)
- Hidden message loop initialization (prevents silence bug)
- Audio I/O (symphonia decode, hound WAV encode)

**Avoids pitfalls:**
- COM lifecycle errors via Layer 2 RAII wrappers
- Tokio blocking via spawn_blocking for all audio work
- Buffer layout bugs via explicit interleave/deinterleave with tests
- Thread affinity violations via state machine enforcement

**Technology focus:** vst3 (coupler-rs) bindings, rmcp MCP server, symphonia/hound audio I/O, tokio spawn_blocking

**Research confidence:** HIGH — single plugin hosting has clear patterns from Steinberg docs and existing implementations (Plugalyzer, JUCE examples)

### Phase 2: Multi-Process Architecture + Plugin Chains
**Rationale:** Production robustness requires crash isolation. Plugin chains unlock real workflows (EQ -> Compressor -> Reverb). Both features naturally share the supervisor-worker split architecture.

**Delivers:** Supervisor-worker separation via Unix sockets + shared memory. Worker crash recovery. Multi-plugin chains with latency compensation and buffer routing.

**Addresses features:**
- Plugin crash protection (worker dies, supervisor survives)
- Plugin chain management (series routing)
- Tail handling for reverbs/delays
- Latency reporting and compensation across chain
- Graceful timeout detection for hung plugins

**Implements architecture:**
- Worker process binary (separate from supervisor)
- Unix domain sockets for control channel
- Shared memory + rtrb ring buffers for audio data
- Worker protocol (WorkerCommand/WorkerResponse types)

**Avoids pitfalls:**
- Plugin segfaults killing host (isolation contains crashes)
- Shared memory cleanup on crash (supervisor owns lifecycle)
- MCP timeout on long renders (async worker communication)

**Technology focus:** rtrb, shared_memory, tokio::process, nix (POSIX signals), workspace crate organization (protocol crate shared between binaries)

**Research confidence:** MEDIUM — multi-process VST3 hosting less documented than single-process, but IPC patterns are well-established

### Phase 3: AI-Specific Intelligence (Focus Mode + Presets)
**Rationale:** Parameter intelligence is the core product differentiator. Defer until hosting foundation is solid to avoid building on unstable base.

**Delivers:** Unit-based parameter grouping, semantic classification, smart parameter subsetting (focus mode), preset A/B comparison, parameter-level diffs.

**Addresses features:**
- IUnitInfo parsing for structural parameter grouping
- Semantic name analysis (freq/threshold/reverb keyword dictionaries)
- Parameter importance ranking
- Focus mode: present 5-15 relevant params to AI per interaction
- .vstpreset read/write for DAW interoperability
- Preset A/B toggle and parameter-level diff reports

**Avoids pitfalls:**
- State round-trip verification (REAPER-documented persistence bugs)
- Parameter change delivery via process() (only valid route to processor)

**Technology focus:** VST3 IUnitInfo interface, preset format parsing, semantic analysis heuristics

**Research confidence:** MEDIUM-HIGH — IUnitInfo is well-documented in VST3 spec, but semantic classification is custom heuristic work

### Phase 4: Analysis + Polish
**Rationale:** Audio analysis provides feedback loop for AI ("did the change work?"). Batch rendering and quality-of-life features complete production readiness.

**Delivers:** Pre/post audio analysis (LUFS, peak, spectral centroid), batch rendering, progress reporting, plugin compatibility tracking.

**Addresses features:**
- Quantify effect of AI changes (loudness, spectrum shifts)
- Process multiple files with same chain/settings
- Elevated sample rate processing (88.2k/96k offline quality)
- Track which plugins work well headless

**Technology focus:** Audio analysis algorithms (potentially ebur128 crate for LUFS), batch orchestration

**Research confidence:** HIGH — audio analysis is well-documented domain, batch processing is straightforward orchestration

### Phase Ordering Rationale

**Dependency chain:**
- Phase 1 establishes VST3 COM abstractions and audio pipeline that Phases 2-4 build on
- Phase 2 requires Phase 1's working plugin hosting to test crash isolation
- Phase 3 requires Phase 1's parameter system and Phase 2's robust hosting
- Phase 4 requires all previous phases' working audio processing

**Risk mitigation:**
- Phase 1 validates VST3 ecosystem compatibility before heavy multi-process investment
- Phase 2 adds production robustness before exposing to users
- Phase 3 defers AI-specific complexity until core hosting proven stable
- Phase 4 polishes already-working system

**Early validation:**
- Phase 1 delivers working end-to-end MCP integration for AI agent testing
- Single-process simplicity accelerates learning and iteration
- Focus mode can be prototyped in Phase 1 with simple heuristics, formalized in Phase 3

### Research Flags

**Needs deeper research during planning:**
- **Phase 1:** VST3 SDK 3.8.0 compatibility with coupler-rs (issue #20 status check), platform-specific message loop initialization patterns for headless operation
- **Phase 2:** Shared memory + ring buffer synchronization patterns (shmem-ipc vs shared_memory tradeoffs), worker crash detection and graceful recovery flows
- **Phase 3:** IUnitInfo adoption rate across plugin ecosystem (fallback strategies if rarely implemented), semantic classification accuracy validation

**Standard patterns (skip research-phase):**
- **Phase 4:** Audio analysis algorithms well-documented, batch processing is straightforward orchestration

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | MEDIUM-HIGH | rmcp and audio I/O crates verified and stable. vst3 (coupler-rs) exists at v0.3.0 but has SDK 3.8.0 compat issue needing verification. VST3 hosting in Rust is early ecosystem — community reports segfaults with instruments. |
| Features | HIGH | VST3 specification is comprehensive and official. Table stakes features well-documented. AI-specific features (focus mode) are novel but built on standard VST3 primitives (IUnitInfo). |
| Architecture | MEDIUM-HIGH | Supervisor-worker pattern is proven (Bitwig, Sushi examples). Layered COM abstraction matches successful Rust FFI patterns. IPC primitives well-established, but VST3-specific threading details require careful attention. |
| Pitfalls | HIGH | Multiple independent sources (KVR forums, JUCE community, Steinberg docs) confirm the same critical issues. COM lifecycle, thread affinity, and headless message loop are consistently documented pain points. |

**Overall confidence:** MEDIUM-HIGH

### Gaps to Address

**VST3 ecosystem maturity in Rust:**
- Gap: Limited production usage reports for Rust VST3 hosts. KVR forum mentions segfaults with instrument plugins, but unclear scope.
- Mitigation: Start with well-behaved open-source plugins (Surge XT, Dexed) for development. Build plugin compatibility tracker in Phase 4. Multi-process isolation limits blast radius.

**coupler-rs SDK 3.8.0 compatibility:**
- Gap: Issue #20 documents binding generation failure with new SDK's Wayland structs. Unknown resolution status.
- Mitigation: Check issue status before Phase 1 start. If unresolved, consider pinning to SDK 3.7.x or patching bindings locally. This is a blocking issue for Phase 1.

**IUnitInfo adoption rate:**
- Gap: Research unclear on what percentage of plugins implement IUnitInfo for parameter grouping.
- Mitigation: Phase 3 includes fallback strategies (semantic name analysis, flag filtering). Test with diverse plugin set (Waves, FabFilter, Native Instruments, etc.) during Phase 1 to gather adoption data.

**Hidden message loop patterns:**
- Gap: JUCE forum confirms ~5% of plugins need message loop even headless, but platform-specific implementation details unclear.
- Mitigation: Research platform event loop initialization during Phase 1 planning. JUCE source code provides reference implementation. May require platform-specific code (Cocoa runloop on macOS, Win32 message pump on Windows, X11/Wayland on Linux).

**Long-running MCP operations:**
- Gap: Unclear if rmcp supports progress notifications or task lifecycle patterns for renders exceeding typical MCP timeout.
- Mitigation: Check rmcp SEP-1686 task lifecycle implementation during Phase 1. If unsupported, implement two-phase pattern (start_render returns job ID, check_render polls).

## Sources

### Primary (HIGH confidence)
- [VST 3 Developer Portal](https://steinbergmedia.github.io/vst3_dev_portal/) — Official specification, architecture, threading model, parameter system, state management
- [VST 3 API Documentation](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/) — Interface reference for IComponent, IEditController, IAudioProcessor, IUnitInfo
- [Steinberg VST 3.8.0 MIT announcement](https://www.steinberg.net/press/2025/vst-3-8/) — Licensing change confirmation
- [coupler-rs/vst3-rs GitHub](https://github.com/coupler-rs/vst3-rs) — Confirmed v0.3.0, MIT/Apache-2.0, binding architecture
- [modelcontextprotocol/rust-sdk (rmcp)](https://github.com/modelcontextprotocol/rust-sdk) — Official MCP SDK v0.15.0, protocol 2025-11-25

### Secondary (MEDIUM confidence)
- [Bitwig Plugin Crash Protection](https://www.bitwig.com/learnings/plug-in-hosting-crash-protection-in-bitwig-studio-20/) — Multi-process architecture patterns
- [Sushi - Elk Audio Headless DAW](https://github.com/elk-audio/sushi) — Production headless VST3 hosting reference
- [Plugalyzer CLI Host](https://github.com/CrushedPixel/Plugalyzer) — Headless VST3 processing validation
- [Renaud Denis: Robust VST3 Host in Rust](https://renauddenis.com/case-studies/rust-vst) — cutoff-vst architecture (COM abstraction layers)
- [Symphonia](https://github.com/pdeljanov/Symphonia) — Pure-Rust audio decoding, 2.3M+ downloads
- [hound](https://github.com/ruuda/hound) — WAV encoding, 7.5M+ downloads
- [rtrb](https://github.com/mgeier/rtrb) — Real-time lock-free ring buffers for audio

### Tertiary (LOW-MEDIUM confidence)
- [KVR Forum: CLI VST3 host in Rust](https://www.kvraudio.com/forum/viewtopic.php?t=622780) — Community experience, segfault reports with instruments
- [JUCE Forum: Headless VST3 silence issue](https://forum.juce.com/t/headless-vst3-host-some-plugins-render-silence/58169) — Message loop requirement validation
- [JUCE Forum: Parameter updates](https://forum.juce.com/t/vst3-parameter-updates-automation-vs-host-refresh/67373) — Parameter delivery patterns
- [Steinberg Forums: Scan crashes](https://forums.steinberg.net/t/vst3-host-plugin-crash-while-scanning-bundleentry-bundleexit/776824) — Scan crash protection necessity

---
*Research completed: 2026-02-14*
*Ready for roadmap: yes*
