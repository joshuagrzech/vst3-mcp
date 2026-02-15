# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-15)

**Core value:** AI can reliably read and write any exposed plugin parameter in real-time while the user makes music in their DAW
**Current focus:** Phase 3 - Parameter Control (IN PROGRESS)

## Current Position

Phase: 3 of 6 (Parameter Control)
Plan: 2 of 2 in current phase (all plans complete)
Status: Phase Complete
Last activity: 2026-02-15 -- Completed 03-02 (parameter control integration tests)

Progress: [██████░░░░] 50%

## Performance Metrics

**Velocity:**
- Total plans completed: 6
- Average duration: 9min
- Total execution time: 0.90 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-plugin-hosting | 2 | 34min | 17min |
| 02-audio-processing | 2 | 11min | 6min |
| 03-parameter-control | 2 | 8min | 4min |

**Recent Trend:**
- Last 5 plans: 4min, 4min, 4min, 30min, 8min
- Trend: accelerating (clean implementations, no deviations)

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- [Roadmap]: Offline MVP first, real-time DAW integration deferred to v2
- [Roadmap]: 6 phases derived from 30 v1 requirements across 6 categories
- [01-01]: Used std::process::Command + stdout JSON for scanner IPC
- [01-01]: Arc<VstModule> stored in PluginInstance to enforce module lifetime structurally
- [01-01]: ManuallyDrop for factory in VstModule to make drop order explicit before ExitDll
- [01-01]: Option::take() in PluginInstance::Drop for COM pointer release ordering
- [01-02]: Out-of-process scanner used in integration test to match production code path
- [01-02]: Controller classId failing as IComponent is scanner filtering issue, deferred
- [01-02]: Single plugin brand (Vital) sufficient -- yabridge needs Wine bridge
- [02-01]: Inline asm for MXCSR instead of deprecated _mm_getcsr/_mm_setcsr
- [02-01]: unsafe impl Send for PluginInstance -- raw pointers single-threaded behind Mutex
- [02-01]: Auxiliary buses get pre-allocated silence/scratch buffers during setup()
- [02-02]: std::env::temp_dir() over tempfile crate to avoid extra dependencies
- [02-02]: Tests skip with return+eprintln rather than #[ignore] for CI visibility
- [02-02]: Cross-correlation for channel swap detection, generous thresholds for diverse plugins
- [03-01]: Pre-allocate 32 parameter queues with 16 points each to eliminate process() allocation
- [03-01]: RefCell for interior mutability in COM objects (VST3 traits require &self)
- [03-01]: Sample offset 0 for all parameter points sufficient for Phase 3 (sweeps deferred)
- [03-01]: getParamStringByValue with fallback to normalized value - plugin knows best formatting
- [03-01]: Flag constants as local const values (not exposed in vst3 crate API)
- [03-02]: Tests use plugin.process() directly instead of render_offline for simpler parameter validation
- [03-02]: Max abs diff threshold 0.001 for parameter audibility testing (conservative, measurable)
- [03-02]: 8 sweep steps sufficient for crash detection, zipper noise FFT analysis deferred
- [03-02]: Tests skip gracefully ensuring CI passes without VST3 plugins installed

### Pending Todos

- Scanner should filter discovered classes to only IComponent-compatible entries (discovered during 01-02 integration testing)

### Blockers/Concerns

- [Research]: stdio transport inside DAW plugin may not work -- DAWs may redirect stdin/stdout. SSE fallback planned for Phase 4.
- [Research]: vst3 crate vs nih-plug's vst3-sys fork coexistence needs validation in Phase 1.

## Session Continuity

Last session: 2026-02-15
Stopped at: Completed 03-02-PLAN.md (parameter control integration tests) -- Phase 3 complete
Resume file: None
