---
phase: 02-audio-processing
plan: 01
subsystem: audio
tags: [vst3, audio-processing, denormals, mxcsr, process-data, offline-rendering]

# Dependency graph
requires:
  - phase: 01-single-plugin-mvp
    provides: "PluginInstance with process(), setup(), re_setup() lifecycle"
provides:
  - "Correct ProcessData with actual bus counts from getBusCount()"
  - "Valid ProcessContext with sampleRate and advancing projectTimeSamples"
  - "Pre-allocated channel pointer arrays (no per-call allocation)"
  - "Denormal flushing (FTZ+DAZ) around offline render loop"
  - "Hard error on sample rate mismatch in process_audio"
affects: [02-02-verification, audio-processing, real-time-safety]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Inline asm for MXCSR register manipulation (x86_64)"
    - "Pre-allocated buffer reuse pattern for process() calls"
    - "Closure-based error handling for MXCSR save/restore"

key-files:
  created: []
  modified:
    - "src/hosting/plugin.rs"
    - "src/audio/process.rs"
    - "src/server.rs"

key-decisions:
  - "Inline asm instead of deprecated _mm_getcsr/_mm_setcsr for MXCSR manipulation"
  - "unsafe impl Send for PluginInstance -- raw pointers in pre-allocated buffers are single-threaded behind Mutex"
  - "Auxiliary buses get silence (input) or scratch (output) buffers pre-allocated during setup()"

patterns-established:
  - "Pre-allocate all process() buffers during setup(), write pointers per-call without allocation"
  - "Query getBusCount() and getBusInfo() at setup time, store bus layout as struct fields"
  - "Always provide ProcessContext -- never pass null to plugin process()"

# Metrics
duration: 8min
completed: 2026-02-15
---

# Phase 2 Plan 01: Audio Processing Pipeline Hardening Summary

**Fixed three process() bugs (bus count, ProcessContext, per-call allocation), added FTZ+DAZ denormal flushing, and made sample rate mismatch a hard error**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-15T08:59:49Z
- **Completed:** 2026-02-15T09:07:31Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- process() now uses actual bus counts from getBusCount() instead of hardcoded 0/1, with pre-allocated silence/scratch buffers for auxiliary buses
- process() provides a valid ProcessContext with sampleRate and advancing projectTimeSamples (was null pointer)
- Channel pointer arrays pre-allocated during setup() -- no Vec::collect() allocation per process() call
- Denormal flushing (FTZ+DAZ) wraps the entire offline render loop with save/restore via inline asm
- Sample rate mismatch in server.rs process_audio now returns error to MCP caller instead of silently continuing

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix process() bugs -- bus count, ProcessContext, pre-allocated buffers** - `f4e2d41` (feat)
2. **Task 2: Add denormal flushing and hard sample rate error** - `4cede32` (feat)

## Files Created/Modified
- `src/hosting/plugin.rs` - Fixed process() with correct bus count, ProcessContext, and pre-allocated buffers; added bus layout fields to PluginInstance
- `src/audio/process.rs` - Added FTZ+DAZ denormal flushing via inline asm around offline render loop
- `src/server.rs` - Changed re_setup() error handling from swallowed warning to hard error

## Decisions Made
- Used inline asm (`stmxcsr`/`ldmxcsr`) instead of deprecated `_mm_getcsr`/`_mm_setcsr` intrinsics -- avoids deprecation warnings and aligns with Rust's direction for MXCSR manipulation
- Added `unsafe impl Send for PluginInstance` since pre-allocated `*mut f32` pointers are only accessed single-threaded behind the AudioHost's Mutex
- Auxiliary bus buffers (silence for input, scratch for output) are pre-allocated during setup() with correct channel counts from getBusInfo()

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed deprecated _mm_getcsr/_mm_setcsr causing warnings**
- **Found during:** Task 2 (denormal flushing)
- **Issue:** `_mm_getcsr` and `_mm_setcsr` are deprecated in Rust std::arch with warnings recommending inline assembly
- **Fix:** Replaced with inline asm using `stmxcsr`/`ldmxcsr` instructions directly
- **Files modified:** src/audio/process.rs
- **Verification:** cargo build produces no warnings
- **Committed in:** 4cede32 (Task 2 commit)

**2. [Rule 3 - Blocking] Added unsafe impl Send for PluginInstance**
- **Found during:** Task 1 (adding Vec<*mut f32> fields)
- **Issue:** Adding `Vec<*mut f32>` fields made PluginInstance !Send, breaking AudioHost's Mutex<Option<PluginInstance>> which requires Send
- **Fix:** Added `unsafe impl Send` with safety documentation -- pointers are single-threaded behind Mutex
- **Files modified:** src/hosting/plugin.rs
- **Verification:** cargo build succeeds, AudioHost compiles with ServerHandler trait
- **Committed in:** f4e2d41 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both auto-fixes necessary for compilation. No scope creep.

## Issues Encountered
None -- plan executed cleanly after auto-fixes.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Audio processing pipeline is hardened and ready for verification testing (02-02)
- All five success criteria addressed: bus count, ProcessContext, pre-allocation, denormals, sample rate errors
- Existing tests continue to pass with no regressions

## Self-Check: PASSED

- All 3 modified files exist on disk
- Commit f4e2d41 (Task 1) verified in git log
- Commit 4cede32 (Task 2) verified in git log

---
*Phase: 02-audio-processing*
*Completed: 2026-02-15*
