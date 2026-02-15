---
phase: 01-single-plugin-mvp
plan: 02
subsystem: audio-processing, mcp-server
tags: [symphonia, hound, rmcp, mcp, audio-decode, audio-encode, block-processing, stdio, vst3]

# Dependency graph
requires:
  - "01-01: VST3 hosting core (PluginInstance, scanner, preset I/O)"
provides:
  - "Multi-format audio file decoding to interleaved f32 via symphonia"
  - "WAV file encoding with 32-bit float precision via hound"
  - "Interleaved <-> planar buffer conversion utilities"
  - "Block-based offline rendering through VST3 plugin with tail handling"
  - "MCP server over stdio with 5 tools: scan_plugins, load_plugin, process_audio, save_preset, load_preset"
  - "Complete Phase 1 MVP: end-to-end audio processing through VST3 plugins via MCP"
affects: [phase-2-isolation, testing, integration]

# Tech tracking
tech-stack:
  added: []
  patterns: [rmcp tool_router + tool_handler macros, Mutex-protected single plugin instance, blocking thread audio processing, stderr-only logging]

key-files:
  created:
    - src/audio/mod.rs
    - src/audio/decode.rs
    - src/audio/encode.rs
    - src/audio/buffers.rs
    - src/audio/process.rs
    - src/server.rs
    - src/main.rs
  modified:
    - src/lib.rs
    - src/hosting/plugin.rs

key-decisions:
  - "Synchronous tool methods (not async) with Mutex locking -- rmcp handles the async wrapper via IntoCallToolResult trait"
  - "Result<String, String> return type for tools -- rmcp auto-converts Ok to success content, Err to error content"
  - "Scan cache stored in Arc<Mutex<Vec<PluginInfo>>> for load_plugin UID lookup without re-scanning"
  - "VstModule stored alongside PluginInstance to keep the shared library alive"
  - "re_setup() method on PluginInstance for sample rate changes during process_audio"

patterns-established:
  - "MCP tool pattern: #[tool_router] impl with #[tool] methods + #[tool_handler] impl ServerHandler"
  - "Audio pipeline: decode (interleaved f32) -> deinterleave -> block process -> interleave -> encode WAV"
  - "Tail handling: query getTailSamples(), feed silence blocks, kInfiniteTail capped at 30 seconds"
  - "All audio/plugin work runs synchronously in tool methods (rmcp handles thread management)"

# Metrics
duration: 8min
completed: 2026-02-15
---

# Phase 1 Plan 2: Audio Pipeline and MCP Server Summary

**End-to-end audio processing via MCP: decode multi-format input, block-process through VST3 plugin with tail handling, encode WAV output, exposed as 5 stdio MCP tools**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-15T00:55:29Z
- **Completed:** 2026-02-15T01:03:27Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- Complete audio processing pipeline: decode any format -> process through VST3 plugin -> encode WAV output
- MCP server responding to JSON-RPC over stdio with 5 registered tools
- Buffer conversion utilities with 7 unit tests (round-trip, known signals, edge cases)
- Block-based offline rendering with tail handling for effects (reverb/delay fade-out)
- Phase 1 MVP functionally complete: `cargo run` starts a working MCP server

## Task Commits

Each task was committed atomically:

1. **Task 1: Audio decode, encode, buffer conversion, and block processing pipeline** - `d9c1dce` (feat)
2. **Task 2: MCP server with tool definitions and main entry point** - `dad6a43` (feat)

## Files Created/Modified
- `src/audio/mod.rs` - Audio module re-exports
- `src/audio/decode.rs` - Multi-format audio decoding to interleaved f32 via symphonia
- `src/audio/encode.rs` - WAV output encoding with 32-bit float precision via hound
- `src/audio/buffers.rs` - Interleaved <-> planar conversion with unit tests
- `src/audio/process.rs` - Block-based offline rendering with tail handling
- `src/server.rs` - MCP AudioHost with 5 tool definitions using rmcp macros
- `src/main.rs` - Entry point: tracing init to stderr, MCP server over stdio
- `src/lib.rs` - Added `pub mod audio` module declaration
- `src/hosting/plugin.rs` - Added `get_tail_samples()` and `re_setup()` methods

## Decisions Made
- Used synchronous tool methods instead of async. rmcp's `IntoCallToolResult` trait auto-wraps the return type. Since all plugin operations hold a Mutex lock, async wouldn't help (can't yield while holding the lock).
- Return `Result<String, String>` from tools. rmcp converts `Ok(String)` to `CallToolResult::success` with text content and `Err(String)` to `CallToolResult::error`. Simpler than manually constructing `CallToolResult`.
- Store `VstModule` in the `AudioHost` alongside `PluginInstance`. The module (shared library) must outlive the plugin instance since COM pointers reference code in the loaded library.
- Added `re_setup()` to `PluginInstance` for transparent sample rate matching. When `process_audio` detects the input file's sample rate differs from the plugin's setup rate, it automatically re-initializes the plugin.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] HostApp::new() and ComponentHandler::new() already return ComWrapper<T>**
- **Found during:** Task 2
- **Issue:** Plan suggested `ComWrapper::new(HostApp::new("vst3-mcp-host"))` but HostApp::new() already wraps in ComWrapper and takes no arguments
- **Fix:** Call `HostApp::new()` and `ComponentHandler::new()` directly without re-wrapping
- **Files modified:** src/server.rs
- **Committed in:** dad6a43

**2. [Rule 1 - Bug] Protocol version mismatch in MCP initialization**
- **Found during:** Task 2 verification
- **Issue:** rmcp 0.15.0 uses protocol version "2025-03-26", not "2025-11-25" as suggested in the plan
- **Fix:** Verified with correct protocol version; no code change needed (rmcp handles version negotiation)
- **Impact:** None -- rmcp auto-negotiates protocol version

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Minimal. Both were API mismatches caught during compilation/testing.

## Issues Encountered
- rmcp tool methods work best as synchronous `fn` returning `Result<String, String>` rather than `async fn` returning `Result<CallToolResult, McpError>`. The `IntoCallToolResult` trait handles conversion automatically. This simplified the server code significantly.
- The plan suggested `spawn_blocking` for audio processing, but since the Mutex lock is held for the entire operation, using synchronous methods is equivalent and simpler. rmcp's internal threading handles the async boundary.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 1 MVP is functionally complete
- `cargo run` starts an MCP server that responds to all 5 tool calls
- Real plugin testing requires .vst3 bundles installed on the system
- Ready for Phase 2: process isolation (supervisor-worker split for crash safety)

## Self-Check: PASSED

All 9 created/modified files verified present. Both task commits (d9c1dce, dad6a43) verified in git log.

---
*Phase: 01-single-plugin-mvp*
*Completed: 2026-02-15*
