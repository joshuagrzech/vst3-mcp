---
phase: 01-single-plugin-mvp
plan: 01
subsystem: hosting
tags: [vst3, com, libloading, preset, plugin-lifecycle, coupler-rs]

# Dependency graph
requires: []
provides:
  - "VST3 module loading via libloading with factory access"
  - "Plugin scanner with fast path (moduleinfo.json) and slow path (binary query)"
  - "PluginInstance lifecycle state machine (Created->SetupDone->Active->Processing)"
  - "HostApp (IHostApplication) and ComponentHandler (IComponentHandler) COM implementations"
  - "IBStream implementation (VecStream) for plugin state capture"
  - ".vstpreset binary format read/write"
  - "Plugin state save/restore with component+controller sync"
  - "Shared types: PluginInfo, BusInfo, ParamInfo, PluginState, HostError"
affects: [01-02, audio-processing, mcp-server, preset-management]

# Tech tracking
tech-stack:
  added: [vst3 0.3.0, libloading, rmcp, tokio, symphonia, hound, serde, schemars, tracing, anyhow, thiserror]
  patterns: [COM RAII via ComPtr/ComWrapper, UnsafeCell for IBStream interior mutability, runtime state machine enum]

key-files:
  created:
    - Cargo.toml
    - src/lib.rs
    - src/hosting/mod.rs
    - src/hosting/types.rs
    - src/hosting/module.rs
    - src/hosting/scanner.rs
    - src/hosting/plugin.rs
    - src/hosting/host_app.rs
    - src/preset/mod.rs
    - src/preset/vstpreset.rs
    - src/preset/state.rs
  modified: []

key-decisions:
  - "Runtime enum state machine over type-level states for PluginInstance (simpler with COM pointer moves)"
  - "UnsafeCell for VecStream interior mutability (IBStreamTrait takes &self but needs mutation)"
  - "IHostApplication in Vst namespace, not Steinberg root (discovered during implementation)"
  - "Suppress ParameterChange dead_code warning -- kept for future IParameterChanges delivery"

patterns-established:
  - "COM class implementation: struct + Class trait + interface Trait impl + ComWrapper::new()"
  - "Plugin lifecycle: from_factory -> setup -> activate -> start_processing -> process -> stop -> deactivate"
  - "Preset sync: After component.setState(), always call controller.setComponentState() with same data"
  - "TUID handling: 16-byte i8 arrays, hex-encoded as 32-char strings for PluginInfo"

# Metrics
duration: 13min
completed: 2026-02-15
---

# Phase 1 Plan 1: VST3 Hosting Core Summary

**VST3 hosting layer with COM lifecycle state machine, plugin scanning, and .vstpreset binary I/O using coupler-rs/vst3 0.3.0**

## Performance

- **Duration:** 13 min
- **Started:** 2026-02-15T00:39:38Z
- **Completed:** 2026-02-15T00:52:47Z
- **Tasks:** 2
- **Files modified:** 13

## Accomplishments
- Compilable Rust project with all Phase 1 dependencies (vst3, rmcp, tokio, symphonia, hound, etc.)
- Full VST3 plugin hosting layer: module loading, scanning, lifecycle management, preset I/O
- PluginInstance manages Created->SetupDone->Active->Processing lifecycle with RAII cleanup
- .vstpreset binary format compliant with Steinberg spec (48-byte header, chunk list)
- 14 unit tests covering scanner paths, string conversion, class filtering, preset round-trips

## Task Commits

Each task was committed atomically:

1. **Task 1: Project setup, VST3 module loading, and plugin scanner** - `e55d5d1` (feat)
2. **Task 2: Plugin lifecycle, host COM interfaces, and preset I/O** - `3cb0990` (feat)

## Files Created/Modified
- `Cargo.toml` - Project manifest with all Phase 1 dependencies
- `src/lib.rs` - Crate root re-exporting hosting and preset modules
- `src/hosting/mod.rs` - Hosting module re-exports
- `src/hosting/types.rs` - PluginInfo, BusInfo, ParamInfo, PluginState, HostError
- `src/hosting/module.rs` - VstModule: dlopen .vst3 bundles, get IPluginFactory
- `src/hosting/scanner.rs` - Plugin discovery with moduleinfo.json fast path
- `src/hosting/plugin.rs` - PluginInstance lifecycle state machine, VecStream IBStream
- `src/hosting/host_app.rs` - HostApp (IHostApplication) and ComponentHandler (IComponentHandler)
- `src/preset/mod.rs` - Preset module re-exports
- `src/preset/vstpreset.rs` - .vstpreset binary format read/write
- `src/preset/state.rs` - Plugin state save/restore bridging PluginInstance to .vstpreset files

## Decisions Made
- Used runtime enum state machine (PluginState) instead of type-level states. COM pointer management with multiple interfaces makes type-level states unwieldy -- the plan allowed this fallback.
- Used UnsafeCell for VecStream interior mutability because IBStreamTrait methods take `&self` but need to mutate position and data buffer. This is safe for single-threaded VST3 use.
- Discovered IHostApplication lives in `vst3::Steinberg::Vst` not `vst3::Steinberg` -- adapted imports accordingly.
- ProcessModes kOffline is a u32 on non-Windows platforms; explicit cast to i32 needed for ProcessSetup.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Import path corrections for vst3 crate**
- **Found during:** Task 1, Task 2
- **Issue:** kResultOk is in `Steinberg` module not crate root; IHostApplication is in `Steinberg::Vst` not `Steinberg`; IPluginFactoryTrait needs explicit import for method resolution
- **Fix:** Corrected all import paths to match actual vst3 0.3.0 crate structure
- **Files modified:** src/hosting/scanner.rs, src/hosting/host_app.rs, src/hosting/plugin.rs
- **Committed in:** e55d5d1, 3cb0990

**2. [Rule 3 - Blocking] UnsafeCell for VecStream IBStream implementation**
- **Found during:** Task 2
- **Issue:** Rust 2024 edition denies casting `&T` to `&mut T` even via raw pointers. IBStreamTrait requires `&self` but read/write/seek need to mutate state.
- **Fix:** Wrapped VecStream internals in UnsafeCell with VecStreamInner struct
- **Files modified:** src/hosting/plugin.rs
- **Committed in:** 3cb0990

**3. [Rule 3 - Blocking] Edition 2024 pattern matching changes**
- **Found during:** Task 2
- **Issue:** Rust 2024 implicit borrow rules disallow explicit `ref` bindings in implicitly-borrowing patterns
- **Fix:** Removed `ref` keywords from pattern matches in Drop implementation
- **Files modified:** src/hosting/plugin.rs
- **Committed in:** 3cb0990

---

**Total deviations:** 3 auto-fixed (all Rule 3 - blocking issues)
**Impact on plan:** All fixes were necessary for compilation with Rust 2024 edition and actual vst3 crate API. No scope creep.

## Issues Encountered
- vst3 crate API exploration required reading the raw bindings.rs (15k lines) since no host-side documentation exists. The plan correctly flagged this as LOW confidence.
- Rust 2024 edition has stricter rules around unsafe function bodies and pattern matching that required adaptation from the plan's code patterns.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Hosting layer complete: VstModule, PluginInstance, scanner, preset I/O all compile and pass tests
- Ready for Plan 01-02: audio processing pipeline (decode, process blocks, encode)
- MCP server integration can build on top of the hosting and preset APIs
- Real plugin testing requires .vst3 bundles on the system (e.g., Surge XT)

---
*Phase: 01-single-plugin-mvp*
*Completed: 2026-02-15*
