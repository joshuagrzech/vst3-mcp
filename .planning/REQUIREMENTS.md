# Requirements: VST3 MCP Host (Headless)

**Defined:** 2026-02-14
**Core Value:** Safe, conversational control of professional audio plugins for AI agents, with crash isolation that keeps the system stable even when plugins fail.

## v1 Requirements

Requirements for initial release. Each maps to roadmap phases.

### Architecture

- [ ] **ARCH-01**: Multi-process supervisor-worker architecture (supervisor handles MCP, worker loads plugins)
- [ ] **ARCH-02**: Crash isolation (plugin crashes only kill worker, not supervisor)

### Discovery

- [ ] **DISC-01**: Plugin scanner discovers VST3s and generates catalog with UIDs
- [ ] **DISC-02**: Blocklist system automatically skips plugins that previously crashed

### Hosting

- [ ] **HOST-01**: Worker process safely loads VST3 SDK (vst3-sys) and plugins ⚠️ **HYPOTHESIS:** coupler-rs/vst3 maturity unknown, SDK 3.8.0 compat needs verification

### Processing

- [ ] **PROC-01**: Offline audio processing pipeline (file in → VST process → file out)
- [ ] **PROC-02**: Transparent audio quality (preserve sample rate, bit depth, plugin native characteristics)

### Parameters

- [ ] **PARAM-01**: Dynamic schema generation (scan plugin parameters at runtime → MCP tool JSON schema) ⚠️ **HYPOTHESIS:** Novel approach, no precedent found
- [ ] **PARAM-02**: Focus mode ("wiggle") - list_parameters accepts .vstpreset mask, exposes only params that differ from default ⚠️ **HYPOTHESIS:** Novel AI-specific feature, IUnitInfo adoption rate unknown

### Presets

- [ ] **PRES-01**: Preset management (save/load .vstpreset files)

### Integration

- [ ] **INTEG-01**: MCP integration over stdio (Claude can call tools, get results)
- [ ] **INTEG-02**: Demo conversation works end-to-end ("brighten this vocal" → success without manual intervention) ⚠️ **HYPOTHESIS:** Full system validation target

## v2 Requirements

Deferred to future release. Tracked but not in current roadmap.

### Processing

- **PROC-03**: Plugin chains (multi-effect routing with latency compensation)
- **PROC-04**: MIDI event support for instrument plugins
- **PROC-05**: Parameter automation curves (time-based modulation)

### Analysis

- **ANLY-01**: Audio analysis feedback (LUFS, spectrum, dynamics)
- **ANLY-02**: Batch rendering (process multiple files with same settings)

### Compatibility

- **COMPAT-01**: Plugin compatibility tracking and reporting
- **COMPAT-02**: VST2 bridge for legacy plugins

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Real-time processing or live audio streams | All rendering is offline (batched) — real-time adds latency/buffer complexity not needed for interactive experimentation workflow |
| Plugin GUIs or editors | Headless only — no UI windows, parameter control is purely API-driven |
| VST2 support | VST3 only for v1 — VST2 is legacy, VST3 SDK is now MIT licensed |
| DAW features (timeline, tracks, mixing) | Single-plugin processor, not a full DAW — use existing DAWs for complex workflows |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| ARCH-01 | TBD | Pending |
| ARCH-02 | TBD | Pending |
| DISC-01 | TBD | Pending |
| DISC-02 | TBD | Pending |
| HOST-01 | TBD | Pending |
| PROC-01 | TBD | Pending |
| PROC-02 | TBD | Pending |
| PARAM-01 | TBD | Pending |
| PARAM-02 | TBD | Pending |
| PRES-01 | TBD | Pending |
| INTEG-01 | TBD | Pending |
| INTEG-02 | TBD | Pending |

**Coverage:**
- v1 requirements: 12 total
- Mapped to phases: 0 (roadmap not yet created)
- Unmapped: 12 ⚠️

---
*Requirements defined: 2026-02-14*
*Last updated: 2026-02-14 after initial definition*
