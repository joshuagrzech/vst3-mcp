# Mixing: Percussion & Drums Execution Manual

## 1. Kick Drum Processing

Step 1: Insert an EQ plugin. Set a Highpass filter `cutoff` to `0.05` (~20Hz - 30Hz) to remove sub-harmonic rumble that eats up headroom.

Step 2: Apply a Bell EQ cut to remove muddiness. Set the `frequency` to `0.2` (~250Hz), the `q_factor` (width) to `0.6` (relatively narrow), and the `gain` to `0.3` (representing a -3dB to -5dB cut).

Step 3: Apply a High-Shelf EQ boost for the "click" or beater noise. Set the `frequency` to `0.7` (~5kHz) and the `gain` to `0.6` (+2dB to +4dB).

Step 4: Insert a Compressor. Set `attack` to `0.4` (~15ms to 30ms) to let the transient punch through. Set `decay` (release) to `0.3` (~100ms) to recover before the next hit. Set `ratio` to `0.5` (typically 4:1) and lower the `threshold` until you achieve around 3dB to 6dB of gain reduction.

Routing & Rationale: The kick drum anchors the track. Cutting the 250Hz region removes "boxiness" and makes room for the bassline and snare body. Using a relatively slow compressor attack is critical; if the attack is too fast (`0.0`), the compressor will clamp down instantly and destroy the kick's punch. Routing the EQ before the compressor ensures you aren't triggering the compressor with frequencies you intend to cut anyway.

## 2. Snare Drum Processing

Step 1: Insert an EQ plugin. Set a Highpass filter `cutoff` to `0.15` (~100Hz - 150Hz) to ensure the snare doesn't clash with the kick or sub-bass.

Step 2: Apply a Bell EQ boost for the "crack." Set the `frequency` to `0.4` (~2kHz), the `q_factor` to `0.5`, and the `gain` to `0.65` (+3dB to +5dB).

Step 3: Insert a Saturation/Distortion plugin. Set the `drive` to `0.3` and the `mix` to `0.4`.

Routing & Rationale: Snares need to cut through dense mixes. Highpassing the snare ensures total low-end clarity. Routing the snare through parallel or mixed saturation adds dense harmonic overtones, making the snare appear louder and thicker in the mix without actually increasing its peak volume level.

## 3. Drum Bus Glue Compression

Step 1:  Route all individual drum tracks (Kick, Snare, Hats, Toms) to a single Drum Bus channel.

Step 2: Insert a VCA-style Bus Compressor on the Drum Bus. Set `attack` to `0.7` (~30ms) to preserve all drum transients.

Step 3: Set the `ratio` to `0.2` (2:1 or lower) for gentle control. 

Step 4: Set the `decay` (release) to `0.1` (Auto-release or very fast) so the compressor breathes with the tempo of the track.

Step 5: Adjust the `threshold` to achieve a maximum of 1dB to 2dB of gain reduction.

Routing & Rationale: Bus compression is not about volume control; it is about "glue." By routing all drums through a single, gently reacting compressor, the micro-dynamics of the kit are unified. The slow attack lets the hits punch through, while the fast release brings up the quiet tail of the drums, making the entire kit sound cohesive and aggressive.