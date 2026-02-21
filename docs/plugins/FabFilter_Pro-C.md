# FabFilter Pro-C 2 VST3 Parameter & Execution Manual

## Overview & General Constraints

FabFilter Pro-C 2 is a highly versatile broadband compressor featuring eight distinct compression algorithms (Styles), advanced side-chaining, and mid/side processing capabilities.

All standard continuous parameters expect normalized values `[0.0, 1.0]` via VST3 `batch_set`. 

While Pro-C 2 has an incredibly visual UI for human users, automated agents must rely entirely on the parameter mappings to dictate the gain reduction envelope.

## 1. Main Dynamics Controls

These parameters define the threshold of compression and how aggressively the signal is clamped down.

`threshold`: Normalized `[0.0, 1.0]`. Controls the dB level at which the compressor engages. *Quirk: Because audio levels vary wildly, blind threshold setting by an LLM often fails. It is highly recommended to use a peak-reading pre-pass to determine the appropriate threshold before applying.*

`ratio`: Normalized `[0.0, 1.0]`. Controls the severity of the gain reduction. Maps logarithmically (e.g., 1:1 up to Infinity:1).

`knee`: Controls the transition curve from uncompressed to compressed. `0.0` is a hard knee (abrupt), while `1.0` represents a 72dB soft knee (very gradual, saturation-like).

`range`: Limits the maximum amount of gain reduction applied, regardless of how loud the input signal gets.
*Quirk - Range Scaling:* The `range` parameter does not just brickwall the gain reduction; it subtly scales the compressor's reaction curve as you approach the limit. If you set `range` to 9dB, a signal that would normally be compressed by 8.5dB will actually receive slightly less compression to smoothly transition into the hard limit. 

## 2. Style & Character

Pro-C 2 uses a discrete parameter to radically alter the internal algorithms, changing the harmonic distortion, attack/release curves, and knee behavior intrinsically.

`style`: Integer/Discrete selector. 
`0` = Clean (Transparent)
`1` = Classic (Feedback, vintage)
`2` = Opto (Slow, optical tube emulation)
`3` = Vocal (Program-dependent ratio)
`4` = Mastering (Extremely transparent, catches fast transients)
`5` = Bus (Glue compression)
`6` = Punch (Aggressive, fast attack)
`7` = Pumping (EDM style, highly noticeable artifacts)

## 3. Time & Envelope Controls

These define the shape and breathing of the compression over time.

`attack`: Time taken to reach maximum gain reduction. `0.0` is near-instantaneous (under 1ms).

`release`: Time taken to recover to 0dB of gain reduction. 

`auto_release`: Boolean `[0.0` or `1.0]`. When enabled, the release time becomes program-dependent, adapting to the audio material. If activated, manual `release` values are used as a general baseline rather than a strict time.

`hold`: normalized `[0.0, 1.0]`. Delays the onset of the release phase. Crucial for creating transparent bass compression or aggressive EDM pumping (holds the signal down up to 500ms).

`lookahead`: normalized `[0.0, 1.0]`. Delays the audio signal slightly so the compressor can "see" transients up to 20ms before they happen. *Quirk: Adding lookahead introduces plugin latency. If zero-latency processing is strictly required for live tracking, this must be `0.0`.*

## 4. Output & Mix Controls

`mix`: Dry/Wet control. Normalized `[0.0, 1.0]`. `0.5` represents 50% parallel compression. *Quirk: Pro-C 2 can scale the mix up to 200%. Be extremely careful mapping values above traditional 100% as it scales the gain change aggressively.*

`auto_gain`: Boolean `[0.0` or `1.0]`. Automatically calculates makeup gain based on the threshold and ratio. *Quirk: Auto-gain algorithms often overcompensate on transient-heavy material. For precise mastering, leave this at `0.0` and adjust output gain manually.*

`output_gain`: Manual makeup gain to restore perceived loudness.

`stereo_link`: Normalized `[0.0, 1.0]`. `0.0` processes left and right entirely independently (dual mono). `1.0` locks the gain reduction so both channels are compressed equally, preserving the stereo image center.

## 5. Side-Chain Routing

Pro-C 2 allows internal filtering of the detection signal, or entirely external triggering.

`sidechain_ext`: Boolean `[0.0` or `1.0]`. `0.0` triggers off the incoming track. `1.0` tells the plugin to listen to the DAW's external sidechain bus (e.g., listening to a Kick drum to compress a Bassline).

`sidechain_hp_cutoff`: Highpass filter applied *only* to the detection circuit. Use this to prevent heavy sub-bass from triggering the compressor unnecessarily.

`sidechain_lp_cutoff`: Lowpass filter applied *only* to the detection circuit.
