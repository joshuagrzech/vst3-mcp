# Mixing: Bass Foundation Execution Manual

## 1. Sub Bass Clarity

Step 1: Insert an EQ plugin. Set a Highpass filter `cutoff` to `0.05` (~20Hz).

Step 2: Set a Lowpass filter `cutoff` to `0.25` (~150Hz). 

Step 3: Insert a Utility/Imaging plugin. Set the `stereo_width` to `0.0` to force the signal into absolute mono.

Routing & Rationale: Sub bass must be heavily restricted. Highpassing at 20Hz removes inaudible DC offset and subsonic rumble that damages speakers. Lowpassing at 150Hz ensures the sub doesn't bleed into the mid-range and clash with vocals or synths. Forcing it to mono is mandatory for vinyl pressing and large club systems, preventing phase cancellation in the low end.

## 2. Mid-Bass Stereo Imaging

Step 1: Insert an EQ plugin on the Mid-Bass (Reese, Acid, etc.). Set a Highpass filter `cutoff` to `0.2` (~120Hz).

Step 2: Insert a Chorus or Dimension Expander plugin. Set `mix` to `0.5` and `stereo_width` to `0.8`.

Step 3: Insert a Saturation plugin. Set `drive` to `0.5` to enhance upper harmonics.

Routing & Rationale: Mid-bass provides the audible "growl" of the bassline. Because we have a dedicated Sub Bass handling the low frequencies, we must highpass the Mid-Bass at 120Hz to prevent a massive, muddy buildup of low frequencies. Once the low end is removed, routing the Mid-Bass through wide stereo effects pushes it to the sides of the speakers, wrapping around the mono kick and sub-bass perfectly.

## 3. Kick/Bass Sidechain Compression

Step 1:  Insert a Compressor on the Sub Bass and Mid-Bass bus.

Step 2: Route the audio output of the Kick Drum channel into the Compressor's "Sidechain Input" or "Key Input".

Step 3: Set the Compressor `attack` to `0.0` (Instantaneous).

Step 4: Set the `ratio` to `1.0` (Infinity:1 or Brickwall).

Step 5: Set the `decay` (release) to `0.2` (~50ms to 150ms). Tune this by ear so the bass volume recovers exactly in time with the track's tempo.

Routing & Rationale: Kick and Bass occupy the exact same frequency range. If they hit at the same time, the volume doubles, clipping the master bus. Sidechain compression solves this by using the Kick's volume to trigger the Bass's compressor. When the kick hits, the bass instantly ducks out of the way (due to the `0.0` attack), and smoothly swells back in (dictated by the release time). This creates the classic electronic music "pumping" groove while ensuring perfect low-end clarity.