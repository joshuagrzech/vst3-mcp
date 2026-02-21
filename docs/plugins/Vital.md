# Vital VST3 Parameter & Execution Manual

## Overview & General Constraints

Vital is a spectral warping wavetable synthesizer. It features 3 wavetable oscillators, 1 sample oscillator, 2 main filters, 6 envelopes, 8 LFOs, and a highly modular drag-and-drop modulation matrix.



All standard continuous parameters expect normalized values `[0.0, 1.0]` via VST3 `batch_set` unless mapped to integer-based selectors (like Unison Voices or Waveform Index). 

Vital relies heavily on bipolar modulation. Ensure your agent calculates offsets correctly when mapping LFOs or Macros to base parameters.

## 1. Oscillators (Osc 1, Osc 2, Osc 3)

The core wavetable generators. Each oscillator has identical parameter structures, denoted by their index (e.g., `osc_1`, `osc_2`).

`osc_1_level`: normalized `[0.0, 1.0]`. Controls the amplitude.

`osc_1_pan`: Bipolar. `0.5` is center. `0.0` is hard left, `1.0` is hard right.

`osc_1_pitch`: The base pitch offset. Often represented in semitones but normalized in the VST3 domain. 

`osc_1_frame`: normalized `[0.0, 1.0]`. Controls the 3D wavetable position. Crucial for movement.

`osc_1_spectral_morph`: The amount of spectral warping applied to the wavetable.

`osc_1_spectral_morph_type`: Integer/Discrete selector (e.g., Vocode, Formant Scale, Harmonic Stretch).

`osc_1_distortion`: Amount of phase distortion/warp applied.

`osc_1_distortion_type`: Integer/Discrete selector (e.g., Sync, FM, RM, Bend).

Quirk: To perform FM synthesis, you must set one oscillator's `distortion_type` to "FM" and route another oscillator's output directly into it using the routing matrix, bypassing the filter.

## 2. Sample/Noise Oscillator (SMP)

Used for transients, noise beds, or acoustic layers.

`sample_level`: normalized `[0.0, 1.0]`. 

`sample_pan`: Bipolar, `0.5` is center.

`sample_pitch`: Controls the pitch of the loaded noise sample.

`sample_random_phase`: Toggle `[0.0` or `1.0]`. When enabled, the sample starts at a random phase each keystroke, good for analog noise. When disabled, it starts consistently, crucial for drum transients.

## 3. Filters (Filter 1 & Filter 2)

Vital has two primary filters that can operate in series, parallel, or independently per oscillator.

`filter_1_cutoff`: The frequency cutoff. Normalized `[0.0, 1.0]`.

`filter_1_resonance`: The peak resonance at the cutoff point.

`filter_1_drive`: Adds analog-style saturation inside the filter circuit.

`filter_1_blend`: Morphs between different filter shapes if a dual-shape filter is selected.

`filter_1_mix`: Dry/wet mix of the filter.

Quirk - Routing: Oscillator routing to filters is explicit. You must toggle `osc_1_to_filter_1` (boolean `0.0` or `1.0`) to send audio to the filter. By default, oscillators might bypass the filters entirely if these toggles are missing from your patch payload.

## 4. Envelopes (Env 1 to 6)

Vital uses a 5-stage envelope structure (AHDSR). 

`env_1_attack`: Time to reach maximum amplitude.

`env_1_hold`: Time the envelope stays at maximum amplitude before the decay phase begins. This is an unusual parameter; keep at `0.0` for classic ADSR behavior.

`env_1_decay`: Time to drop to the sustain level.

`env_1_sustain`: The resting amplitude level while the key is held.

`env_1_release`: Time to fade to zero after the key is released.

Quirk - Hardwiring: `env_1` is strictly hardwired to the Master Amplitude of the synthesizer. Do not use `env_1` for pitch drops or filter sweeps unless you want the entire volume of the synth to follow that exact curve. Use `env_2` or `env_3` for utility modulations.

## 5. LFOs (LFO 1 to 8)

Highly customizable low-frequency oscillators that can act as repeating modulators or one-shot envelopes.

`lfo_1_frequency`: The speed of the LFO. If tempo sync is on, this acts as an integer selector for note divisions (e.g., 1/4, 1/8, 1/16).

`lfo_1_tempo_sync`: Boolean toggle `[0.0` or `1.0]`. 

`lfo_1_smooth`: Adds a lowpass filter to the LFO output, rounding off sharp edges.

Quirk - Envelope Mode: LFOs can be set to "Envelope" mode via the discrete `lfo_1_mode` parameter. In this mode, the LFO plays exactly once per keystroke, effectively giving you up to 8 additional customizable envelopes.

## 6. Modulation Matrix



Vital relies on a routing matrix to connect modulators (Envs, LFOs, Macros) to targets (Cutoff, Pitch, Wavetable Frame). 

`mod_1_source`: Integer identifying the source (e.g., LFO 1, Env 2, Macro 1).

`mod_1_destination`: Integer identifying the target parameter (e.g., Filter 1 Cutoff).

`mod_1_amount`: Bipolar parameter `[-1.0, 1.0]`. Dictates how strongly the source affects the destination. `0.0` means no modulation.

`mod_1_bipolar_toggle`: Boolean `[0.0` or `1.0]`. Dictates whether the modulation pushes the parameter in one direction from its base value (unipolar) or pushes it in both directions symmetrically (bipolar).

Quirk - Visual Disconnect: In the UI, modulations appear as colored rings around the destination knobs. Programmatically via VST3, they exclusively exist as numbered slots in the modulation matrix. Always ensure your agent occupies an empty `mod_x` slot when establishing a new routing.

## 7. Macros (Macro 1 to 4)

Four global controllers designed for live performance or broad patch changes.

`macro_1`: Normalized `[0.0, 1.0]`.

`macro_2`: Normalized `[0.0, 1.0]`.

Quirk - Value Ranges: Macros themselves do nothing until routed via the Modulation Matrix. Always route a macro to a destination with a specific `mod_amount` to make it functional.

## 8. Effects Chain

Vital features a modular effects rack.

`chorus_mix`, `chorus_voices`, `chorus_delay`.
`delay_mix`, `delay_feedback`, `delay_time`.
`reverb_mix`, `reverb_time`, `reverb_size`, `reverb_high_cut`.
`distortion_mix`, `distortion_drive`, `distortion_type` (Integer selector).
`compressor_mix`, `compressor_band_ratio` (Multiband compression).

Quirk - Effects Ordering: The audio passes through the effects in the order they are activated in the UI. While the exact VST3 parameter IDs for changing effect order dynamically can be finicky depending on the DAW wrapper, enabling an effect using its `_on` toggle (e.g., `reverb_on` = `1.0`) enables it in the chain.