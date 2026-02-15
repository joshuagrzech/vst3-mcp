# Phase 2: Audio Processing - Research

**Researched:** 2026-02-15
**Domain:** Offline audio rendering through VST3 child plugins -- decode, buffer conversion, process, encode, sample rate matching, multi-channel handling
**Confidence:** HIGH (existing implementation covers ~80% of requirements; gaps are well-understood hardening and correctness issues)

## Summary

Phase 2 validates that audio files can be correctly processed through a hosted VST3 plugin in offline mode. The good news: the existing codebase already has a working end-to-end pipeline. `audio/decode.rs` decodes multi-format audio via symphonia, `audio/buffers.rs` handles interleave/deinterleave conversion, `audio/process.rs` implements block-based offline rendering with tail handling, `audio/encode.rs` writes 32-bit float WAV via hound, and `server.rs` ties it all together with sample-rate-aware re-setup. The MCP `process_audio` tool already works.

What remains is hardening and verification against the success criteria: (1) confirming bit-identical passthrough for transparent plugins (AUDIO-02), (2) verifying stereo channel preservation with no swap or collapse (AUDIO-03), (3) testing multi-bus plugins and plugins with sidechain inputs (AUDIO-03), (4) eliminating per-block allocations in `process()` to establish correct patterns (AUDIO-04), (5) ensuring sample rate matching works reliably including edge cases like re-setup failure (AUDIO-05), and (6) adding denormal flushing for CPU safety (not in requirements but critical for correctness with effect plugins processing near-silence).

There are two bugs in the current code that must be fixed: (a) the `process()` method in `plugin.rs` hardcodes `numInputs: 1, numOutputs: 1` regardless of actual bus count, which will fail for plugins with multiple audio buses or plugins that report zero input buses (generators/synths), and (b) the `process()` method allocates `Vec` on every call for channel pointer arrays, which is technically fine for offline but establishes a bad pattern that will break real-time in later phases.

**Primary recommendation:** Focus on verification and correctness fixes rather than new features. The pipeline works; now prove it works correctly with real plugins and edge cases. Fix the bus-count bug, add denormal flushing, eliminate per-block allocations, and write verification tests that cover all five success criteria.

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `symphonia` | 0.5.5 | Multi-format audio decoding (WAV, FLAC, MP3, OGG) | Pure Rust, no FFmpeg dependency. Supports all common audio formats with multi-channel and arbitrary sample rate. Already in codebase. |
| `hound` | 3.5.1 | WAV file encoding (32-bit float output) | Simple, reliable WAV writer. Supports 32-bit float sample format which preserves full precision. Already in codebase. |
| `vst3` (coupler-rs) | 0.3.0 | VST3 COM interfaces for child plugin process() calls | Provides AudioBusBuffers, ProcessData, ProcessSetup structs. Already in codebase from Phase 1. |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `no_denormals` | latest | Flush denormals to zero during audio processing | Wrap the process loop to prevent CPU spikes when plugins process near-silence through IIR filters. Alternative: inline `_mm_setcsr` calls. |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `symphonia` | `hound` for decoding too | hound only reads WAV; symphonia reads WAV, FLAC, MP3, OGG, etc. |
| `hound` for encoding | `symphonia` (write support) | symphonia does not support encoding. hound is the standard Rust WAV writer. |
| `no_denormals` crate | Manual `_mm_setcsr` via `std::arch::x86_64` | `no_denormals` provides RAII guard and cross-platform support (x86 + aarch64). Manual approach avoids a dependency but requires platform-specific code. |
| 32-bit float WAV output | 16-bit or 24-bit integer WAV | Float preserves full precision from the plugin output. Integer formats require dithering/truncation. |

**Dependencies (Cargo.toml -- no changes needed for Phase 2):**

All required crates are already in the project's Cargo.toml:
```toml
hound = "3.5.1"
symphonia = { version = "0.5.5", features = ["all"] }
vst3 = "0.3.0"
```

Optional addition for denormal flushing:
```toml
no_denormals = "0.1"  # or inline _mm_setcsr
```

## Architecture Patterns

### Recommended Project Structure (Phase 2 scope)

```
src/
  audio/
    mod.rs              # Re-exports
    decode.rs           # symphonia-based multi-format decoding (EXISTING)
    encode.rs           # hound-based WAV encoding (EXISTING)
    buffers.rs          # Interleave/deinterleave conversion (EXISTING)
    process.rs          # Block-based offline rendering with tail (EXISTING)
  hosting/
    plugin.rs           # PluginInstance::process() -- FIX bus count + pre-alloc buffers
```

No new files needed. Phase 2 is about hardening and verifying existing code.

### Pattern 1: VST3 ProcessData Construction for Offline Processing

**What:** Building the `ProcessData` struct correctly for each `process()` call. The host must provide `AudioBusBuffers` for every audio bus the plugin has (including deactivated buses except trailing ones), with `numInputs` and `numOutputs` matching the plugin's bus count from `getBusCount()`.

**When to use:** Every process() call.

**Critical detail -- numInputs/numOutputs is BUS count, NOT channel count:**

The current code hardcodes `numInputs: 1, numOutputs: 1` when inputs/outputs are non-empty. This is correct for the common case (most effect plugins have exactly 1 main input bus and 1 main output bus) but WRONG for:
- Plugins with sidechain input (2 input buses)
- Plugins with multiple output buses (e.g., drum machines with separate outputs)
- Synth/generator plugins with 0 input buses but 1+ output buses

**Correct approach:**
```rust
// Query actual bus counts from the plugin
let num_input_buses = unsafe {
    self.component.getBusCount(kAudio as i32, kInput as i32)
};
let num_output_buses = unsafe {
    self.component.getBusCount(kAudio as i32, kOutput as i32)
};

// Build one AudioBusBuffers per bus
let mut input_buses: Vec<AudioBusBuffers> = Vec::with_capacity(num_input_buses as usize);
let mut output_buses: Vec<AudioBusBuffers> = Vec::with_capacity(num_output_buses as usize);

// For bus 0 (main): use the actual audio data
// For bus 1+ (aux/sidechain): provide silence buffers with correct channel count
```

**Source:** [Steinberg VST3 API - ProcessData](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ProcessData.html) -- "numInputs: number of audio input busses", [Steinberg Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html) -- "the host must provide buffer data - even for inactive busses"

### Pattern 2: Pre-allocated Process Buffers

**What:** Allocate channel pointer arrays and auxiliary bus silence buffers once during `setup()` and reuse them across all `process()` calls. This eliminates per-block heap allocation.

**When to use:** Always. Even for offline processing, this establishes the correct pattern for real-time safety in later phases.

**Example:**
```rust
pub struct PluginInstance {
    // ... existing fields ...

    // Pre-allocated buffers for process() (allocated in setup(), reused in process())
    input_channel_ptrs: Vec<*mut f32>,
    output_channel_ptrs: Vec<*mut f32>,
    input_buses: Vec<AudioBusBuffers>,
    output_buses: Vec<AudioBusBuffers>,
    // Silence buffers for auxiliary buses (sidechain, etc.)
    aux_silence: Vec<Vec<f32>>,
    aux_silence_ptrs: Vec<*mut f32>,
}
```

**Source:** [Pitfalls Research - Pitfall 1: Memory Allocation on Audio Thread](../../../.planning/research/PITFALLS.md)

### Pattern 3: Sample Rate Matching via re_setup()

**What:** When the input audio file's sample rate differs from the plugin's current setup rate, the plugin must be re-configured: `stop_processing() -> deactivate() -> setup(new_rate) -> activate() -> start_processing()`. The existing `re_setup()` method does this correctly.

**When to use:** Before processing each audio file, compare the decoded sample rate against the current setup rate.

**Current implementation status:** Already implemented in `plugin.rs` as `re_setup()`. The `server.rs` `process_audio` tool calls it. However, the error handling has a gap: if `re_setup()` fails, the current code logs a debug message and continues processing with the wrong sample rate. This should be a hard error for Phase 2 success criteria #4.

**Source:** [Steinberg Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html) -- "The max. sample block size can change during the plug-in lifetime, but NOT while the audio component is active."

### Pattern 4: Tail Processing with Early Termination

**What:** After processing all input frames, continue feeding silence to the plugin to capture effect tails (reverb decay, delay echoes). Monitor the output for silence and stop early if the tail has decayed below a threshold.

**When to use:** For effect plugins with non-zero tail length. The current implementation processes the full reported tail length, which is correct but potentially wasteful for plugins reporting `kInfiniteTail`.

**Current implementation status:** Already implemented in `process.rs`. Uses `plugin.get_tail_samples()` with a 30-second cap for infinite tails. No early termination based on output silence level yet (optional optimization, not required for Phase 2).

### Pattern 5: ProcessContext for Offline Processing

**What:** The VST3 spec says processContext is "optional, but most welcome." For offline processing, a minimal ProcessContext with `sampleRate` and `projectTimeSamples` should be provided since those fields are "always valid" per the spec. Many plugins check `processContext` for null before reading it.

**When to use:** Every process() call. Passing null is risky -- some plugins dereference it without null-checking.

**Example:**
```rust
let mut context = ProcessContext {
    state: 0,  // not playing
    sampleRate: sample_rate,
    projectTimeSamples: current_sample_position as i64,
    // ... zero all other fields ...
};
process_data.processContext = &mut context;
```

**Source:** [Steinberg ProcessContext Reference](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ProcessContext.html) -- "sampleRate and projectTimeSamples: always valid"

### Anti-Patterns to Avoid

- **Assuming all effect plugins have exactly 1 input and 1 output bus:** Some plugins have sidechain buses, multiple output buses, or zero input buses (generators). Always query `getBusCount()` and provide the correct number of `AudioBusBuffers`.

- **Skipping buffers for deactivated buses:** The VST3 spec requires buffers for ALL buses (including deactivated ones) unless they are trailing deactivated buses. Omitting middle buses corrupts the bus index mapping.

- **Ignoring re_setup() failure:** If the plugin does not support the input file's sample rate, processing should fail explicitly rather than silently producing wrong-rate output.

- **Processing at arbitrary block sizes:** The `numSamples` in each `process()` call must be between 1 and `maxSamplesPerBlock` (set during `setupProcessing()`). The current code respects this with `DEFAULT_BLOCK_SIZE = 4096` which matches the setup call.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Audio format detection | Custom magic-byte parsers | `symphonia` probe | Handles dozens of formats, metadata, codec detection |
| WAV writing | Manual RIFF chunk assembly | `hound` | Correct RIFF header generation, sample format handling |
| Interleave/deinterleave | Nothing -- already hand-rolled | Keep existing `buffers.rs` | Simple enough to implement correctly. The existing code is tested and correct. |
| Denormal flushing | Manual inline asm | `no_denormals` crate or `std::arch::x86_64::_mm_setcsr` | Cross-platform (x86 + ARM), RAII guard pattern |
| Sample rate conversion | Custom resampler | Don't do it at all | For offline MVP, reject mismatched rates or require plugin to support re-setup. Sample rate conversion is a separate complex problem (Phase 2 does not require it). |
| Audio quality measurement | Custom RMS/peak analysis | Simple f32 comparison with epsilon tolerance | For verification only. Bit-identical comparison for passthrough test; RMS-based for quality checks. |

**Key insight:** The audio processing pipeline itself is straightforward -- decode, deinterleave, process in blocks, interleave, encode. The complexity is in the VST3 hosting details: correct ProcessData construction, bus management, sample rate lifecycle, and buffer lifetime.

## Common Pitfalls

### Pitfall 1: Bus Count Mismatch in ProcessData

**What goes wrong:** Hardcoding `numInputs: 1, numOutputs: 1` in ProcessData when the plugin has a different bus layout. The plugin reads bus buffers by index -- if the host says there are N buses but provides fewer, the plugin reads garbage memory.

**Why it happens:** Most simple effect plugins have exactly 1 input and 1 output bus, so hardcoded values work in testing. The bug surfaces when testing with plugins that have sidechain inputs, multiple outputs, or zero inputs (synths).

**How to avoid:** Query `getBusCount(kAudio, kInput)` and `getBusCount(kAudio, kOutput)` at setup time. Pre-allocate the correct number of `AudioBusBuffers`. For non-main buses (sidechain, aux), provide silence buffers with the correct channel count from `getBusInfo()`.

**Warning signs:** Crashes in specific plugins that work fine in other hosts. Corrupted audio output. ASAN reports about out-of-bounds reads.

**Current status:** BUG -- `plugin.rs` lines 418-429 hardcode `numInputs` and `numOutputs` to 0 or 1 based on whether the caller provided any input/output data, rather than matching the plugin's actual bus count.

### Pitfall 2: Missing Denormal Flushing

**What goes wrong:** Processing audio through IIR filter plugins (EQ, reverb, compressor) with near-silence input causes denormalized floating-point values. The CPU spends orders of magnitude more time processing denormals than normal floats, causing massive performance degradation.

**Why it happens:** The host is responsible for setting the FTZ (Flush-To-Zero) and DAZ (Denormals-Are-Zero) flags on the processing thread. In a DAW, nih-plug handles this automatically for the wrapper plugin. In our offline processing binary, nobody sets these flags.

**How to avoid:** Set FTZ and DAZ flags before entering the process loop. Use the `no_denormals` crate or inline `_mm_setcsr` calls. Restore flags after processing.

**Warning signs:** CPU usage spikes dramatically when processing silence or very quiet audio through reverb/delay plugins. Processing a 10-second file takes minutes instead of seconds.

**Current status:** NOT IMPLEMENTED. The `render_offline()` function does not set denormal flushing flags. This must be added.

**Source:** [JUCE denormal prevention discussion](https://forum.juce.com/t/state-of-the-art-denormal-prevention/16802), [no_denormals crate](https://docs.rs/no_denormals/latest/no_denormals/)

### Pitfall 3: Channel Swap or Mono Collapse

**What goes wrong:** When converting between interleaved (file) and planar (VST3) formats, a bug in the interleave/deinterleave logic causes left and right channels to swap, or both channels to receive the same data (mono collapse).

**Why it happens:** Off-by-one errors in channel indexing. Or, the plugin has a different number of output channels than input channels, and the host does not handle the mismatch.

**How to avoid:** The existing `buffers.rs` has comprehensive tests including a `test_known_signal_stereo` that verifies L/R channel separation. Additional verification needed: process a file with known different content on L and R channels through a bypass/transparent plugin and verify channels match.

**Warning signs:** Stereo imaging is wrong. Left ear hears what should be right. Both channels identical when they should differ.

**Current status:** The interleave/deinterleave functions are tested and correct. The risk is in the `process()` method's channel pointer array construction, which must match the file's channel layout to the plugin's bus channel layout.

### Pitfall 4: Sample Rate Mismatch Silently Produces Wrong Output

**What goes wrong:** The plugin is set up at 44100 Hz but the input file is 48000 Hz. The plugin processes the audio at the wrong rate, causing pitch shift and duration change. Worse, some plugins (e.g., delay with time-based settings) produce completely wrong output.

**Why it happens:** The `re_setup()` call in `server.rs` catches the error but continues processing with the old rate on failure. The user gets a file that sounds wrong with no error indication.

**How to avoid:** Make sample rate mismatch a hard error. If `re_setup()` fails, return an error to the caller. Never process audio at a different rate than the input file's native rate.

**Warning signs:** Output sounds pitched up/down. Time-based effects (delay, reverb) sound wrong. Output duration is correct but pitch is wrong.

**Current status:** Partially handled. `server.rs` calls `re_setup()` but swallows the error with a debug log. Must be changed to return an error.

### Pitfall 5: ProcessContext Null Pointer Dereference in Plugin

**What goes wrong:** Passing `processContext: std::ptr::null_mut()` in ProcessData causes some plugins to crash when they dereference the pointer without null-checking.

**Why it happens:** The VST3 spec says processContext is "optional, but most welcome." Many plugin developers assume the host always provides it and access `processContext->sampleRate` without checking for null.

**How to avoid:** Always provide a valid ProcessContext, even for offline processing. Set `sampleRate` and `projectTimeSamples` to correct values. Zero all other fields. Set `state = 0` (not playing).

**Warning signs:** Segfaults during process() in specific plugins. Crashes that only happen with certain plugins but not others.

**Current status:** BUG -- `plugin.rs` line 437 sets `processContext: std::ptr::null_mut()`. Should provide a minimal ProcessContext with at least `sampleRate` and advancing `projectTimeSamples`.

### Pitfall 6: Per-Block Allocation in process()

**What goes wrong:** The current `process()` method creates `Vec<*mut f32>` for channel pointers on every call. For offline processing this is a performance drag; for future real-time use it will cause audio glitches.

**Why it happens:** Simpler code -- allocating fresh vectors each call avoids managing persistent state.

**How to avoid:** Pre-allocate channel pointer arrays during `setup()`. Store as fields on `PluginInstance`. Reuse by writing new pointer values into existing vectors each process() call.

**Warning signs:** For offline: slower than necessary on large files. For future real-time: `assert_process_allocs` panics, audio glitches under load.

**Current status:** TECH DEBT -- `plugin.rs` lines 383-396 allocate two `Vec` per process() call. Should be pre-allocated.

## Code Examples

### Correct Multi-Bus ProcessData Construction

```rust
// Source: Steinberg VST3 API Documentation

// During setup(), query and store bus layout:
let num_input_audio_buses = unsafe {
    self.component.getBusCount(kAudio as i32, kInput as i32)
};
let num_output_audio_buses = unsafe {
    self.component.getBusCount(kAudio as i32, kOutput as i32)
};

// For each bus, query channel count via getBusInfo()
// Pre-allocate AudioBusBuffers arrays

// During process():
let mut process_data = ProcessData {
    processMode: kOffline as i32,
    symbolicSampleSize: kSample32 as i32,
    numSamples: block_size,
    numInputs: num_input_audio_buses,   // actual bus count, NOT channel count
    numOutputs: num_output_audio_buses,  // actual bus count, NOT channel count
    inputs: input_buses.as_mut_ptr(),
    outputs: output_buses.as_mut_ptr(),
    inputParameterChanges: std::ptr::null_mut(),
    outputParameterChanges: std::ptr::null_mut(),
    inputEvents: std::ptr::null_mut(),
    outputEvents: std::ptr::null_mut(),
    processContext: &mut context,  // NOT null
};
```

### Minimal ProcessContext for Offline Processing

```rust
// Source: Steinberg ProcessContext documentation
// sampleRate and projectTimeSamples are "always valid"

use std::mem::zeroed;
use vst3::Steinberg::Vst::ProcessContext;

fn make_offline_context(sample_rate: f64, sample_position: i64) -> ProcessContext {
    let mut ctx: ProcessContext = unsafe { zeroed() };
    ctx.sampleRate = sample_rate;
    ctx.projectTimeSamples = sample_position;
    ctx.state = 0;  // not playing, no transport flags set
    ctx
}

// In the process loop, advance projectTimeSamples:
let mut sample_position: i64 = 0;
for block in blocks {
    let mut ctx = make_offline_context(sample_rate, sample_position);
    process_data.processContext = &mut ctx;
    // ... call plugin.process() ...
    sample_position += block_size as i64;
}
```

### Denormal Flushing for Offline Process Loop

```rust
// Option A: Using no_denormals crate
use no_denormals::no_denormals;

pub fn render_offline(plugin: &mut PluginInstance, decoded: &DecodedAudio) -> Result<Vec<f32>> {
    // Wrap entire processing in denormal-free scope
    unsafe {
        no_denormals(|| {
            render_offline_inner(plugin, decoded)
        })
    }
}

// Option B: Manual MXCSR (x86_64 only)
#[cfg(target_arch = "x86_64")]
fn set_denormal_flushing() -> u32 {
    unsafe {
        let old = std::arch::x86_64::_mm_getcsr();
        // Set FTZ (bit 15) and DAZ (bit 6)
        std::arch::x86_64::_mm_setcsr(old | (1 << 15) | (1 << 6));
        old
    }
}

#[cfg(target_arch = "x86_64")]
fn restore_mxcsr(old: u32) {
    unsafe {
        std::arch::x86_64::_mm_setcsr(old);
    }
}
```

### Verification: Bit-Identical Passthrough Test

```rust
// For success criterion #2: bypass/transparent plugin produces identical output

fn verify_passthrough(input: &[f32], output: &[f32], channels: usize) -> bool {
    // For a true bypass plugin, output should be bit-identical to input
    // (no floating-point operations applied)
    if input.len() != output.len() {
        return false;
    }

    input.iter().zip(output.iter()).all(|(a, b)| a.to_bits() == b.to_bits())
}

// For near-identical (plugins that apply identity gain):
fn verify_near_identical(input: &[f32], output: &[f32], max_error: f32) -> bool {
    if input.len() != output.len() {
        return false;
    }

    input.iter().zip(output.iter()).all(|(a, b)| (a - b).abs() <= max_error)
}
```

### Verification: Channel Preservation Test

```rust
// For success criterion #3: stereo channels preserved, no swap or collapse

fn verify_channels_preserved(
    input_left: &[f32],
    input_right: &[f32],
    output_left: &[f32],
    output_right: &[f32],
) -> bool {
    // Check no channel swap: output L correlates with input L, not input R
    let lr_correlation = cross_correlate(input_left, output_left);
    let rl_correlation = cross_correlate(input_left, output_right);

    // Left output should match left input more than right input
    lr_correlation > rl_correlation
}

fn cross_correlate(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len());
    let sum: f64 = a.iter().zip(b.iter())
        .map(|(x, y)| (*x as f64) * (*y as f64))
        .sum();
    sum / n as f64
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual WAV parsing | symphonia for decoding, hound for encoding | Established | Reliable multi-format support |
| Ignoring denormals | FTZ+DAZ flag management in host | Established best practice | Prevents CPU spikes with near-silent audio |
| Null processContext | Always provide minimal ProcessContext | VST3 3.7.0 (IProcessContextRequirements) | Plugins can declare what context they need; null risks crashes |
| Single bus assumption | Query getBusCount() for actual layout | Always required by spec | Correct handling of sidechain, multi-out plugins |

**Deprecated/outdated:**
- Passing null for `processContext` -- while technically "optional" per spec, many plugins crash on null. Always provide at least `sampleRate` and `projectTimeSamples`.

## Open Questions

1. **Which effect plugin to use for passthrough/transparent testing?**
   - What we know: Need a plugin that passes audio through unchanged (or with minimal processing) to verify no corruption. A gain plugin set to 0 dB, or a bypass-capable plugin.
   - What's unclear: Which freely available Linux VST3 plugin provides true bypass or transparent passthrough for testing.
   - Recommendation: Use a plugin from the test system (e.g., Surge XT with all effects off, or a simple JUCE-built gain plugin). Alternatively, test with the plugin in bypass state if it supports software bypass via `IAudioProcessor`.

2. **Should we support plugins with 0 input buses (generators/synths) in Phase 2?**
   - What we know: The phase requirements say "process audio file through child plugin." Generators don't process input -- they generate output. AUDIO-01 says "process audio file through child plugin (offline rendering)" which implies an effect plugin, not a generator.
   - What's unclear: Whether the bus-count fix should also handle 0-input-bus plugins or whether that is deferred.
   - Recommendation: Fix the bus count to be correct regardless, but Phase 2 testing focuses on effect plugins (1+ input buses). Generator support is incidental correctness, not a success criterion.

3. **What happens when a plugin doesn't support the input file's sample rate?**
   - What we know: `setupProcessing()` returns `kResultOk` or an error code. If it fails, the plugin cannot process at that rate.
   - What's unclear: Whether any common plugins reject standard sample rates (44100, 48000, 96000).
   - Recommendation: Make this a hard error with a clear message: "Plugin does not support sample rate {X} Hz." Most plugins support all standard rates, so this is an edge case.

4. **Should the no_denormals crate be added as a dependency, or use inline _mm_setcsr?**
   - What we know: `no_denormals` provides cross-platform (x86 + ARM) RAII guard. The alternative is manual `_mm_setcsr` which is x86-only.
   - What's unclear: Whether the `no_denormals` crate's UB caveat (per Rust docs on MXCSR modification) is a practical concern.
   - Recommendation: Use inline `_mm_setcsr` for now (x86-only is fine since the platform is Linux x86_64). This avoids a new dependency and the UB concern. Wrap it in a helper function with RAII-style save/restore.

## Sources

### Primary (HIGH confidence)
- [Steinberg VST3 API - ProcessData Struct](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ProcessData.html) -- numInputs/numOutputs are bus counts, buffer requirements
- [Steinberg VST3 API - AudioBusBuffers Struct](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1AudioBusBuffers.html) -- numChannels, channelBuffers32, silenceFlags
- [Steinberg VST3 API - ProcessContext Struct](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ProcessContext.html) -- sampleRate and projectTimeSamples always valid, state flags
- [Steinberg Processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html) -- bus buffer requirements, offline mode, parameter flushing
- [Steinberg IProcessContextRequirements](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Change+History/3.7.0/IProcessContextRequirements.html) -- plugins declare context needs
- Existing codebase: `src/audio/*.rs`, `src/hosting/plugin.rs`, `src/server.rs` -- working implementation, verified against real plugins

### Secondary (MEDIUM confidence)
- [SONAR X3 VST3 Internals](https://noelborthwick.com/2014/01/22/developer-notes-sonar-x3-vst3-internals/) -- real-world host implementation details, parameter automation timing
- [JUCE State of the Art Denormal Prevention](https://forum.juce.com/t/state-of-the-art-denormal-prevention/16802) -- FTZ/DAZ best practices for audio hosts
- [no_denormals crate docs](https://docs.rs/no_denormals/latest/no_denormals/) -- RAII denormal flushing for Rust
- [hound crate](https://github.com/ruuda/hound) -- WAV 32-bit float encoding
- [symphonia crate](https://github.com/pdeljanov/Symphonia) -- multi-format audio decoding
- [Steinberg Multiple Dynamic I/O Support](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Change+History/3.0.0/Multiple+Dynamic+IO.html) -- multi-bus architecture

### Tertiary (LOW confidence)
- [JUCE forum: VST3 offline rendering produces no change](https://forum.juce.com/t/why-does-offline-rendering-with-vst3-plugin-show-no-audible-or-waveform-change-tracktion-engine-juce-host-no-ui-and-plugin-ui-both-tried/66771) -- known pitfall, no resolution found
- [Rust stdarch MXCSR issue #852](https://github.com/rust-lang/stdarch/issues/852) -- DAZ constant not in std::arch, UB concerns

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all crates verified, already in use in codebase, no new dependencies needed
- Architecture: HIGH -- existing pipeline works end-to-end, issues are well-understood correctness fixes
- Buffer conversion: HIGH -- existing tests cover interleave/deinterleave, planar format matches VST3 spec
- Bus management: HIGH -- Steinberg spec is clear on numInputs/numOutputs semantics, fix is straightforward
- Pitfalls: HIGH -- denormal flushing and null ProcessContext are well-documented across multiple sources
- Multi-channel handling: MEDIUM -- existing code works for stereo; up-to-8-channel support needs testing but the data structures are correct

**Research date:** 2026-02-15
**Valid until:** 2026-03-15 (30 days -- domain is stable, libraries are mature)
