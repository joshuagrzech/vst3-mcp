---
phase: 01-plugin-hosting
plan: 02
subsystem: hosting
tags: [vst3, integration-test, lifecycle, teardown-stress, unified-split, vital]

# Dependency graph
requires:
  - phase: 01-01
    provides: "crash-safe scanner, hardened teardown, Arc<VstModule> lifetime enforcement"
provides:
  - Integration test binary (agent-audio-integration-test) exercising all Phase 1 success criteria
  - Real-world validation of full VST3 lifecycle (Created -> SetupDone -> Active -> Processing -> teardown)
  - Stress-tested 10-cycle load/unload without crashes or leaks
  - Split component/controller detection for Vital synth
  - hex_string_to_tuid() and is_controller_separate() utility methods
affects: [02-parameter-access, server, plugin-loading]

# Tech tracking
tech-stack:
  added: []
  patterns: [integration-test-binary, phase-based-test-reporting, hex-tuid-conversion]

key-files:
  created:
    - src/bin/integration_test.rs
  modified:
    - src/hosting/plugin.rs
    - src/hosting/scanner.rs
    - Cargo.toml

key-decisions:
  - "Out-of-process scanner used in integration test to match production code path"
  - "Explicit main binary target added to Cargo.toml alongside scanner and integration test"
  - "Controller classId correctly fails to load as IComponent -- scanner filtering deferred (not a hosting bug)"
  - "Single plugin brand (Vital) sufficient for Phase 1 -- yabridge plugins crash in chainloader (expected, Wine bridge dependency)"

patterns-established:
  - "Integration test binary pattern: phase-based PASS/FAIL reporting with optional CLI args for targeted testing"
  - "hex_string_to_tuid(): standard conversion for classId hex strings to TUID byte arrays"
  - "is_controller_separate(): runtime detection of unified vs split Component/Controller architecture"

# Metrics
duration: ~30min (includes human checkpoint verification)
completed: 2026-02-15
---

# Phase 1 Plan 2: Integration Testing with Real VST3 Plugins Summary

**Integration test binary validating full VST3 lifecycle (scan, load, process, teardown x10) against Vital synth with split component/controller detection**

## Performance

- **Duration:** ~30 min (includes human checkpoint verification)
- **Started:** 2026-02-15
- **Completed:** 2026-02-15
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Created 700+ line integration test binary exercising all 5 Phase 1 success criteria against real plugins
- Validated full lifecycle: Created -> SetupDone -> Active -> Processing -> Active -> SetupDone with Vital synth
- 10 load/unload stress test cycles completed cleanly -- no crashes, no leaks
- Detected Vital's split component/controller architecture (2855 parameters, stereo output + MIDI input)
- 9/11 test phases passed; 2 expected failures (controller classId not instantiable as IComponent -- scanner filtering issue, not hosting)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create integration test binary** - `a1df2db` (feat)
2. **Fix: Use out-of-process scanner in integration test** - `39769e2` (fix)
3. **Fix: Add explicit main binary target** - `5a95a66` (fix)
4. **Task 2: Human verification checkpoint** - approved (no commit, human-verify)

**Plan metadata:** (this commit)

## Files Created/Modified
- `src/bin/integration_test.rs` - Integration test binary with 5 test phases (A-E), CLI args, structured PASS/FAIL output
- `src/hosting/plugin.rs` - Added `is_controller_separate()` method and `hex_string_to_tuid()` helper
- `src/hosting/scanner.rs` - Made `scan_plugins_safe()` public for integration test consumption
- `Cargo.toml` - Added agent-audio-integration-test and explicit agent-audio main binary targets

## Decisions Made
- Used out-of-process scanner (`scan_plugins_safe()`) in integration test to match production code path, not the in-process scanner
- Added explicit `[[bin]]` target for `agent-audio` main binary -- Cargo infers it from `src/main.rs` but explicit entries are needed when other `[[bin]]` targets exist
- Accepted 1 plugin brand (Vital) as sufficient for Phase 1 validation -- yabridge plugins require Wine bridge and crash in chainloader as expected
- Controller classId failing to instantiate as IComponent is a scanner filtering issue (scanner lists all classes, not just IComponent ones), not a hosting layer bug -- deferred to future improvement

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Integration test used in-process scanner instead of out-of-process**
- **Found during:** Task 1 (integration test creation)
- **Issue:** Initial implementation called scan functions that loaded plugins in-process, but the production code path uses the out-of-process scanner binary
- **Fix:** Switched to `scan_plugins_safe()` which delegates to the scanner child process
- **Files modified:** src/bin/integration_test.rs, src/hosting/scanner.rs
- **Verification:** Integration test runs correctly with out-of-process scanning
- **Committed in:** 39769e2

**2. [Rule 3 - Blocking] Cargo binary target inference broken by explicit [[bin]] entries**
- **Found during:** Task 1 (post-commit build verification)
- **Issue:** Adding `[[bin]]` entries for scanner and integration test caused Cargo to stop auto-detecting `src/main.rs` as the main binary
- **Fix:** Added explicit `[[bin]] name = "agent-audio" path = "src/main.rs"` target
- **Files modified:** Cargo.toml
- **Verification:** `cargo build` succeeds for all three binary targets
- **Committed in:** 5a95a66

---

**Total deviations:** 2 auto-fixed (2 blocking issues)
**Impact on plan:** Both fixes necessary for correct build and test execution. No scope creep.

## Issues Encountered
- Only 1 plugin brand available for testing (Vital) -- yabridge-wrapped plugins crash in their chainloader because they need the Wine bridge running. This is expected behavior and not a hosting layer issue.
- 2 of 11 test cases show "expected failures" -- the scanner discovers controller classIds alongside component classIds, and controller classes cannot be instantiated as IComponent. Future improvement: filter scanner results to only IComponent classes.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All 5 Phase 1 success criteria validated against real hardware: scanning, loading, lifecycle, teardown stress, unified/split detection
- Hosting layer ready for Phase 2 (parameter access)
- Known limitation: scanner returns controller classIds that cannot be loaded as components -- filtering improvement deferred
- Integration test binary available at `cargo run --bin agent-audio-integration-test` for regression testing

## Self-Check: PASSED

All 4 artifact files exist. All 3 task commits verified (a1df2db, 39769e2, 5a95a66). Key patterns confirmed: integration test phases A-E, is_controller_separate(), hex_string_to_tuid(), scan_plugins_safe() public API.

---
*Phase: 01-plugin-hosting*
*Completed: 2026-02-15*
