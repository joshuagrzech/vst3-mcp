# Phase 3: Parameter Control - Research

**Researched:** 2026-02-15
**Domain:** VST3 parameter enumeration, read/write, and sample-accurate automation via IParameterChanges
**Confidence:** HIGH

## Summary

Phase 3 implements complete parameter control for child VST3 plugins, enabling enumeration, reading, writing, and sample-accurate automation of plugin parameters. The core challenge is implementing host-side COM interfaces (`IParameterChanges` and `IParamValueQueue`) that deliver parameter changes to the plugin's processor with sample-accurate timing during the `process()` call.

The current codebase already implements parameter enumeration (`getParameterCount`, `getParameterInfo`) and reading (`getParamNormalized`), but parameter writing is incomplete: there's a TODO on line 628-631 in `plugin.rs` that passes `null` for `inputParameterChanges`, meaning parameter changes queued via `queue_parameter_change()` are discarded. This phase fills that gap by implementing the VST3 parameter change delivery mechanism, adding human-readable display strings, filtering read-only parameters, and validating that parameter writes produce audible changes.

**Primary recommendation:** Implement `IParameterChanges` and `IParamValueQueue` as pre-allocated COM objects (no runtime allocation), deliver queued parameter changes at sample-accurate offsets during `process()`, and validate with plugins that rely on parameter automation (compressors, EQs, synths with modulation).

## Standard Stack

### Core Dependencies (Already in Cargo.toml)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| vst3 | 0.3.0 | VST3 COM bindings for hosting child plugins | Provides `IEditController`, `IComponent`, `IParameterChanges` interfaces needed for parameter control |
| vst3::com_scrape_types | 0.3.0 | COM pointer wrappers and COM class implementation | Provides `ComPtr`, `ComWrapper` for implementing host-side COM interfaces |

### Supporting (No new dependencies required)

All parameter control functionality can be implemented using existing dependencies. The VST3 spec is complete and stable.

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| vst3 crate (0.3.0) | vst3-sys (RustAudio) | vst3-sys is lower-level raw bindings; vst3 provides better ergonomics and is already proven in this codebase |
| Pre-allocated parameter change queues | Dynamic allocation per process() call | Dynamic allocation violates real-time safety; current offline-only code tolerates it but establishes bad patterns |

**Installation:**
No new dependencies required. Phase 3 builds on existing `vst3 = "0.3.0"`.

## Architecture Patterns

### Recommended Project Structure

```
src/hosting/
├── plugin.rs           # PluginInstance with parameter queue delivery
├── param_changes.rs    # IParameterChanges and IParamValueQueue COM impl (NEW)
├── types.rs            # ParamInfo with flag interpretation helpers
└── mod.rs              # Public API
```

### Pattern 1: Pre-Allocated Parameter Change Queues

**What:** Allocate fixed-size parameter change structures during `setup()` and reuse them across all `process()` calls. No heap allocation during audio processing.

**When to use:** Always, for both offline and real-time processing. Establishes correct patterns early.

**Example:**
```rust
// In PluginInstance struct
struct PluginInstance {
    // Existing fields...
    param_changes: VecDeque<ParameterChange>,

    // NEW: Pre-allocated COM objects for delivering parameter changes
    param_changes_impl: ComWrapper<ParameterChanges>,
    param_queues: Vec<ComWrapper<ParamValueQueue>>,
    max_params_per_block: usize,
}

// In setup()
pub fn setup(&mut self, sample_rate: f64, max_block_size: i32) -> Result<(), HostError> {
    // Existing setup code...

    // Pre-allocate parameter change infrastructure
    self.max_params_per_block = 32; // Reasonable limit
    self.param_queues = (0..self.max_params_per_block)
        .map(|_| ParamValueQueue::new())
        .collect();
    self.param_changes_impl = ParameterChanges::new(&self.param_queues);

    // ...rest of setup
}

// In process()
pub fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], num_samples: i32) -> Result<(), HostError> {
    // Clear previous block's changes (reset count to 0, keep capacity)
    self.param_changes_impl.clear();

    // Populate changes from queue
    while let Some(change) = self.param_changes.pop_front() {
        if let Some(queue) = self.param_changes_impl.add_parameter(change.id) {
            queue.add_point(0, change.value); // Sample offset 0 = start of block
        }
    }

    // Set inputParameterChanges in ProcessData
    process_data.inputParameterChanges = self.param_changes_impl.to_com_ptr();

    // Existing process call...
}
```

**Source:** Adapted from VST3 SDK patterns and real-time audio best practices.

### Pattern 2: Parameter Display String Conversion

**What:** Convert normalized parameter values (0.0-1.0) to human-readable strings using the plugin's own formatting logic.

**When to use:** Whenever displaying parameter values to users or AI tools (MCP integration).

**Example:**
```rust
pub fn get_parameter_display(&self, id: u32) -> Result<String, HostError> {
    let ctrl = self.controller.as_ref()
        .ok_or_else(|| HostError::InvalidState("no edit controller".to_string()))?;

    let normalized_value = unsafe { ctrl.getParamNormalized(id) };

    // Get human-readable string from plugin
    let mut string128: [u16; 128] = [0; 128];
    let result = unsafe {
        ctrl.getParamStringByValue(id, normalized_value, string128.as_mut_ptr())
    };

    if result == kResultOk {
        Ok(string128_to_string(&string128))
    } else {
        // Fallback: return normalized value as string
        Ok(format!("{:.3}", normalized_value))
    }
}
```

**Source:** [VST 3 Interfaces: IEditController](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IEditController.html)

### Pattern 3: Parameter Flag Interpretation

**What:** Filter parameters based on flags to exclude read-only, hidden, or program-change parameters from AI control.

**When to use:** Before exposing parameters to MCP tools or Focus Mode.

**Example:**
```rust
impl ParamInfo {
    /// Check if this parameter can be written by the host.
    pub fn is_writable(&self) -> bool {
        const K_CAN_AUTOMATE: u32 = 1 << 0;
        const K_IS_READ_ONLY: u32 = 1 << 1;

        // Must have kCanAutomate and must NOT have kIsReadOnly
        (self.flags & K_CAN_AUTOMATE != 0) && (self.flags & K_IS_READ_ONLY == 0)
    }

    /// Check if this parameter should be hidden from UI.
    pub fn is_hidden(&self) -> bool {
        const K_IS_HIDDEN: u32 = 1 << 5;
        self.flags & K_IS_HIDDEN != 0
    }

    /// Check if this is a bypass parameter.
    pub fn is_bypass(&self) -> bool {
        const K_IS_BYPASS: u32 = 1 << 4;
        self.flags & K_IS_BYPASS != 0
    }
}

// Usage: filter parameters for AI control
pub fn get_controllable_parameters(&self) -> Vec<ParamInfo> {
    (0..self.get_parameter_count())
        .filter_map(|i| self.get_parameter_info(i).ok())
        .filter(|p| p.is_writable() && !p.is_hidden())
        .collect()
}
```

**Source:** [VST 3 Interfaces: ParameterInfo](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ParameterInfo.html)

### Anti-Patterns to Avoid

- **Calling `setParamNormalized` on controller instead of using `IParameterChanges`:** The controller method only updates the UI, not the processor. Changes may not reach the DSP algorithm. Always use `IParameterChanges` for automation.
- **Allocating parameter change structures in `process()`:** Creates heap allocations on every audio block. Pre-allocate during `setup()`.
- **Ignoring sample offsets in `IParamValueQueue`:** All points at offset 0 causes discontinuous jumps. For smooth automation, distribute points across the buffer.
- **Not clearing parameter changes between blocks:** Reusing the same changes across multiple `process()` calls causes stale data. Clear and repopulate each block.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| COM object implementation | Manual vtable construction, reference counting | `ComWrapper` from vst3 crate | Edge cases: COM aggregation, QueryInterface chains, thread-safe ref counting |
| Parameter value smoothing | Custom interpolation logic in process() | Plugin's own smoothing (via IParamValueQueue sample offsets) | Plugins know their DSP characteristics; host provides raw automation points |
| Parameter unit conversion | Custom dB/Hz/% conversion logic | `normalizedParamToPlain` / `plainParamToNormalized` from plugin | Plugins define their own scaling (linear, log, custom curves); host should not assume |

**Key insight:** Parameter handling in VST3 is designed for the plugin to own conversion logic and smoothing. The host's job is to deliver normalized values at precise sample offsets; the plugin handles the rest.

## Common Pitfalls

### Pitfall 1: Missing IParameterChanges Delivery

**What goes wrong:** Parameter changes are queued but never delivered to the plugin because `inputParameterChanges` is null. The plugin's processor never receives the new values, so parameter writes have no effect.

**Why it happens:** Implementing `IParameterChanges` and `IParamValueQueue` as COM interfaces is non-trivial. The current codebase defers this (line 628-631 TODO in `plugin.rs`), so parameter changes are discarded.

**How to avoid:**
- Implement `IParameterChanges` and `IParamValueQueue` as host-side COM objects using `vst3::Class` trait and `ComWrapper`
- Pre-allocate these objects during `setup()` to avoid allocation in `process()`
- Populate queues from the `param_changes: VecDeque` before each `process()` call
- Set `ProcessData.inputParameterChanges` to a valid COM pointer

**Warning signs:**
- Calling `queue_parameter_change()` has no audible effect on output
- Plugins ignore parameter changes during processing
- Integration tests that modify parameters and compare output audio fail

### Pitfall 2: Incorrect Parameter Flag Interpretation

**What goes wrong:** Writing to read-only parameters causes plugin errors or crashes. Exposing hidden parameters to AI overwhelms users with irrelevant controls.

**Why it happens:** The VST3 flag constants are not well-documented in Rust bindings. Developers assume all parameters are writable, or check the wrong flag bits.

**How to avoid:**
- Define flag constants explicitly: `kCanAutomate = 1 << 0`, `kIsReadOnly = 1 << 1`, etc.
- A writable parameter requires `kCanAutomate` AND NOT `kIsReadOnly`
- Filter out `kIsHidden` parameters from MCP tools and Focus Mode
- Identify bypass parameters with `kIsBypass` for special handling

**Warning signs:**
- Plugin returns errors when setting certain parameters
- MCP tools list hundreds of parameters including internal/hidden ones
- Parameter writes succeed but have no effect (writing to read-only params)

### Pitfall 3: Zipper Noise from Discontinuous Parameter Changes

**What goes wrong:** All parameter changes are delivered at sample offset 0, causing the plugin to apply them instantly. This creates audible stepping artifacts ("zipper noise") during parameter sweeps.

**Why it happens:** The simplest `IParamValueQueue` implementation adds all points at offset 0. For offline processing with one change per block, this is acceptable. But parameter sweeps (gradual changes) need multiple points distributed across the buffer.

**How to avoid:**
- For single parameter changes: one point at offset 0 is sufficient
- For parameter sweeps: generate multiple points (e.g., one per 64 samples) with interpolated values
- Let the plugin handle smoothing internally—don't try to smooth in the host
- Test with sweeps of filter cutoff, gain, or pitch parameters (most audibly sensitive)

**Warning signs:**
- Audible stepping/clicking during parameter automation
- Plugins sound "choppy" when parameters change continuously
- Integration tests for parameter sweeps produce discontinuous output

### Pitfall 4: Not Syncing Controller State After Process

**What goes wrong:** Parameter values written to the processor via `IParameterChanges` are not reflected in the controller. Calling `getParamNormalized()` returns stale values, breaking assumptions about parameter state.

**Why it happens:** The VST3 architecture separates processor (DSP) and controller (UI/parameter management). Changes to the processor don't automatically update the controller. The plugin may send updates via `outputParameterChanges`, which the host must apply to the controller.

**How to avoid:**
- After each `process()` call, check `ProcessData.outputParameterChanges`
- Iterate output queues and call `setParamNormalized()` on controller for each changed parameter
- This is required for plugins with split Component/Controller architecture
- Unified plugins (controller == component) may not need this, but checking is harmless

**Warning signs:**
- Parameter reads return old values after writes
- Plugin's internal state diverges from controller's view
- Split-architecture plugins (Vital, some Steinberg plugins) misbehave

## Code Examples

Verified patterns from VST3 SDK and Rust implementations:

### Host-Side IParameterChanges Implementation

```rust
use vst3::Steinberg::Vst::{IParameterChanges, IParamValueQueue, ParamID, ParamValue};
use vst3::com_scrape_types::{ComWrapper, ComPtr};
use std::cell::RefCell;

/// Host-side implementation of IParameterChanges.
pub struct ParameterChanges {
    queues: RefCell<Vec<ComWrapper<ParamValueQueue>>>,
    active_count: RefCell<usize>,
}

impl ParameterChanges {
    pub fn new(capacity: usize) -> ComWrapper<Self> {
        ComWrapper::new(ParameterChanges {
            queues: RefCell::new((0..capacity).map(|_| ParamValueQueue::new()).collect()),
            active_count: RefCell::new(0),
        })
    }

    pub fn clear(&self) {
        *self.active_count.borrow_mut() = 0;
        for queue in self.queues.borrow().iter() {
            queue.clear();
        }
    }

    pub fn add_parameter(&self, id: ParamID) -> Option<&ComWrapper<ParamValueQueue>> {
        let mut count = self.active_count.borrow_mut();
        let queues = self.queues.borrow();

        if *count < queues.len() {
            queues[*count].set_parameter_id(id);
            let idx = *count;
            *count += 1;
            Some(&queues[idx])
        } else {
            None // Exceeded capacity
        }
    }
}

impl vst3::Class for ParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParameterChanges {
    unsafe fn getParameterCount(&self) -> i32 {
        *self.active_count.borrow() as i32
    }

    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        let queues = self.queues.borrow();
        if index >= 0 && (index as usize) < *self.active_count.borrow() {
            queues[index as usize].to_com_ptr().unwrap().as_ptr()
        } else {
            std::ptr::null_mut()
        }
    }

    unsafe fn addParameterData(&self, id: &ParamID, index: *mut i32) -> *mut IParamValueQueue {
        if let Some(queue) = self.add_parameter(*id) {
            let mut count = self.active_count.borrow_mut();
            if !index.is_null() {
                *index = (*count - 1) as i32;
            }
            queue.to_com_ptr().unwrap().as_ptr()
        } else {
            std::ptr::null_mut()
        }
    }
}
```

**Source:** Adapted from VST3 SDK patterns and vst3 crate COM implementation examples.

### Host-Side IParamValueQueue Implementation

```rust
pub struct ParamValueQueue {
    param_id: RefCell<ParamID>,
    points: RefCell<Vec<(i32, ParamValue)>>, // (sampleOffset, value)
}

impl ParamValueQueue {
    pub fn new() -> ComWrapper<Self> {
        ComWrapper::new(ParamValueQueue {
            param_id: RefCell::new(0),
            points: RefCell::new(Vec::with_capacity(16)), // Pre-allocate
        })
    }

    pub fn set_parameter_id(&self, id: ParamID) {
        *self.param_id.borrow_mut() = id;
    }

    pub fn clear(&self) {
        self.points.borrow_mut().clear(); // Keep capacity
    }

    pub fn add_point(&self, offset: i32, value: ParamValue) {
        self.points.borrow_mut().push((offset, value));
    }
}

impl vst3::Class for ParamValueQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamValueQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        *self.param_id.borrow()
    }

    unsafe fn getPointCount(&self) -> i32 {
        self.points.borrow().len() as i32
    }

    unsafe fn getPoint(&self, index: i32, sample_offset: *mut i32, value: *mut ParamValue) -> i32 {
        let points = self.points.borrow();
        if index >= 0 && (index as usize) < points.len() {
            let (offset, val) = points[index as usize];
            if !sample_offset.is_null() { *sample_offset = offset; }
            if !value.is_null() { *value = val; }
            vst3::Steinberg::kResultOk
        } else {
            vst3::Steinberg::kInvalidArgument
        }
    }

    unsafe fn addPoint(&self, sample_offset: i32, value: ParamValue, index: *mut i32) -> i32 {
        self.add_point(sample_offset, value);
        if !index.is_null() {
            *index = (self.points.borrow().len() - 1) as i32;
        }
        vst3::Steinberg::kResultOk
    }
}
```

**Source:** Adapted from [IParamValueQueue Interface](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IParamValueQueue.html).

### Parameter Sweep with Multiple Points

```rust
/// Queue a parameter sweep from start_value to end_value over num_samples.
pub fn queue_parameter_sweep(&mut self, id: ParamID, start_value: f64, end_value: f64, num_samples: i32) {
    // For smooth sweeps, add multiple points across the buffer
    const POINTS_PER_SWEEP: i32 = 8; // More points = smoother
    let step = num_samples / POINTS_PER_SWEEP;

    for i in 0..POINTS_PER_SWEEP {
        let offset = i * step;
        let t = i as f64 / (POINTS_PER_SWEEP - 1) as f64; // 0.0 to 1.0
        let value = start_value + t * (end_value - start_value);

        self.param_changes.push_back(ParameterChange {
            id,
            value,
            sample_offset: offset,
        });
    }
}
```

**Source:** Derived from VST3 automation best practices and zipper noise prevention techniques.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| setParamNormalized on controller for automation | IParameterChanges with sample-accurate offsets | VST 3.0 (2008) | Sample-accurate automation became mandatory; direct controller calls are for UI updates only |
| Per-block allocation of parameter queues | Pre-allocated queues with clear/reuse | Real-time best practices (2015+) | Eliminates allocation in audio path; critical for RT safety |
| Host-side parameter smoothing | Plugin-side smoothing via distributed automation points | VST3 spec design | Plugins know their DSP characteristics better than hosts; host provides raw points |

**Deprecated/outdated:**
- **VST2 `setParameter` (immediate parameter change):** VST2 had no sample-accurate automation. VST3 requires `IParameterChanges` for proper timing.
- **Ignoring `outputParameterChanges`:** Early VST3 hosts skipped output parameter sync. Modern hosts must process `outputParameterChanges` to sync controller state.

## Open Questions

1. **Should parameter sweeps use linear or custom interpolation?**
   - What we know: VST3 spec states hosts provide "linear approximation" of automation curves
   - What's unclear: Whether custom curves (logarithmic for frequency, etc.) should be applied host-side or plugin-side
   - Recommendation: Always use linear interpolation in host. Plugins handle non-linear scaling internally via `normalizedParamToPlain`.

2. **How many points per sweep are sufficient to avoid zipper noise?**
   - What we know: More points = smoother, but also more overhead for plugin to process
   - What's unclear: Optimal trade-off varies by parameter type (frequency vs gain vs mix)
   - Recommendation: Start with 8-16 points per block (every 64-256 samples at 512 sample blocks). Test with filter cutoff sweeps (most sensitive).

3. **Do all plugins correctly handle `outputParameterChanges`?**
   - What we know: Split architecture plugins (separate component/controller) need this for state sync
   - What's unclear: Whether unified plugins (controller == component) populate `outputParameterChanges` or assume host tracks state
   - Recommendation: Always process `outputParameterChanges` if non-null. Defensive programming.

## Sources

### Primary (HIGH confidence)
- [VST 3 Parameters and Automation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Parameters+Automation/Index.html) - Official Steinberg VST3 spec
- [IParameterChanges Class Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IParameterChanges.html) - Interface definition and usage
- [IParamValueQueue Class Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IParamValueQueue.html) - Automation point queue mechanics
- [IEditController Class Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IEditController.html) - Parameter value conversion methods
- [ParameterInfo Struct Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ParameterInfo.html) - Parameter flag definitions

### Secondary (MEDIUM confidence)
- [VST3 Sample Accurate Automation Example](https://gist.github.com/olilarkin/1a6e07a1d4ed6c9e20e81938fc1e1b2b) - iPlug2 implementation notes
- [Implementing sample accurate automation in vst3 - KVR Forum](https://www.kvraudio.com/forum/viewtopic.php?t=490162) - Community discussion on automation implementation
- [Clarification of parameter handling in VST 3 - Steinberg Forums](https://forums.steinberg.net/t/clarification-of-parameter-handling-in-vst-3/201914) - Official clarification on parameter sync
- [Best place to smooth parameter changes - iPlug2 Forum](https://iplug2.discourse.group/t/best-place-means-to-smooth-parameter-changes-to-eliminate-zipper-noise/758) - Zipper noise prevention discussion

### Tertiary (LOW confidence)
- [Zipper Noise when Automating Plugin - Apple Community](https://discussions.apple.com/thread/300525) - User-reported issues with automation
- [Request for clarification on VST3 parameter automation update frequency - Renoise Forums](https://forum.renoise.com/t/request-for-clarification-on-vst3-parameter-automation-update-frequency/74976) - DAW-specific automation behavior

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - vst3 crate is proven in current codebase, no new dependencies
- Architecture: HIGH - VST3 COM interface implementation is well-documented with SDK examples
- Pitfalls: HIGH - Missing IParameterChanges, flag misinterpretation, and zipper noise are well-known VST3 hosting issues

**Research date:** 2026-02-15
**Valid until:** 90 days (VST3 spec is stable; automation patterns unlikely to change)

---
*Research for Phase 3: Parameter Control*
*Focus: IParameterChanges implementation, parameter display, flag filtering, and sample-accurate automation*
