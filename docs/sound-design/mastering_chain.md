# Mastering Chain Execution Manual

## 1. Master Bus Equalization

Step 1: Insert a clean, linear-phase EQ on the Master channel.

Step 2: Apply a Highpass filter `cutoff` at `0.05` (~20Hz) with a steep slope (24dB/oct) to clean up subsonic rumble.

Step 3: Apply a gentle Low-Shelf EQ cut to control mud. Set `frequency` to `0.25` (~150Hz) and `gain` to `0.45` (-1dB to -2dB max).

Step 4: Apply a gentle High-Shelf EQ boost for "air." Set `frequency` to `0.8` (~10kHz) and `gain` to `0.55` (+1dB to +2dB max).

Routing & Rationale: Mastering EQ should be incredibly subtle. The goal is not to fix bad sounds, but to gently tilt the overall tonal balance of the fully mixed track. A linear-phase EQ is routed first in the chain because it alters frequencies without introducing phase smearing, preserving the punch of the mix. 

## 2. Multiband Compression

Step 1:  Insert a Multiband Compressor.

Step 2: Set the Low crossover to `0.25` (~150Hz) and the High crossover to `0.7` (~5kHz).

Step 3: Target the Low band (Sub/Bass). Set a slow `attack` (`0.6`) and fast `decay` (`0.2`). Set the `threshold` to lightly touch the peaks, yielding roughly 1dB to 2dB of gain reduction.

Step 4: Target the Mid band (Vocals/Synths). Set a moderate `attack` (`0.4`) and moderate `decay` (`0.4`). Adjust the `threshold` for a maximum of 1dB of gain reduction.

Routing & Rationale: A multiband compressor splits the audio into independent frequency zones. Routing the master mix through this allows you to compress the volatile low-end (kick and bass) without accidentally squashing the high-end (hi-hats and vocals). Gentle settings here act as "glue," ensuring that no single frequency band jumps out and hurts the listener's ears.

## 3. Brickwall Limiting

Step 1: Insert a Brickwall Limiter at the absolute end of the mastering chain.

Step 2: Set the Output Ceiling (True Peak limit) to `0.9` (-1.0 dBTP). 

Step 3: Set the Limiter `attack` to `0.0` (Lookahead enabled, instant clamping). Set the `decay` (release) to `0.3` (~50ms to 100ms) to avoid distortion while maximizing loudness.

Step 4: Slowly push the Limiter's Input Gain (or lower the Threshold) until the track reaches the desired perceived loudness (typically -14 LUFS to -8 LUFS depending on the genre). This usually translates to moving the `input_gain` parameter from `0.5` up to `0.7` or `0.8`.

Routing & Rationale: The limiter is the final safety net before the audio is exported. It acts as an infinite-ratio compressor that strictly forbids audio from crossing the set ceiling. Setting the output ceiling slightly below 0.0dB (-1.0 dBTP) prevents inter-sample peaking when the audio is compressed into streaming formats like MP3 or AAC. Pushing the gain into the limiter raises the quietest parts of the track, achieving commercial loudness.