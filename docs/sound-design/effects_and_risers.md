# Effects & Risers Execution Manual

## 1. White Noise Sweep (Build-up Riser)

Step 1: Set the primary oscillator to White Noise.

Step 2: Configure the Amp Envelope to dictate the length of the riser. Set `attack` to `0.8` (representing a long, multi-bar fade in), `sustain` to `1.0`, and `release` to `0.5`.

Step 3: Route the noise through a Lowpass or Bandpass filter. 

Step 4: Route a Macro controller (Macro 1) to both the `filter_cutoff` and a `reverb_mix` parameter simultaneously.

Step 5: Define the Macro ranges. As Macro 1 moves from `0.0` to `1.0`, it should sweep the filter cutoff from `0.1` up to `0.9`, and increase the reverb mix from `0.2` to `0.6`.

Routing & Rationale: A sweep is designed to build tension leading up to a structural change in the music. By routing a single Macro knob to both the filter cutoff and the reverb wetness, an agent or user can push one parameter to simultaneously make the sound brighter and push it further back into the simulated acoustic space. This combination mimics the psychoacoustic sensation of a massive wave of energy approaching and swelling over the listener.

## 2. "Laser" / "Pew" Zaps (Dubstep / Glitch)

Step 1: Choose a Sawtooth or Square wave for aggressive harmonic content.

Step 2: Set a very tight Amp Envelope. `attack`: `0.0`, `decay`: `0.15`, `sustain`: `0.0`, `release`: `0.05`.

Step 3: Configure a dedicated Pitch Envelope. Set `attack` to `0.0` and `decay` to `0.1`.

Step 4: Route the Pitch Envelope to the Oscillator Pitch. Set the modulation `amount` to `-0.8` to force a rapid 2 to 4 octave downward dive.

Step 5: Route the signal through a Highpass filter with the `cutoff` at `0.3`.

Routing & Rationale: Sci-fi laser effects are fundamentally extremely fast pitch drops applied to bright waveforms. Routing a fast, decaying envelope negatively to pitch creates this classic "pew" articulation. Because dropping the pitch rapidly pushes the oscillator down into the bass frequencies, routing the final output through a highpass filter is crucial; it cuts out the sub-bass artifacts generated at the tail end of the pitch drop, keeping the zap clean and preventing it from clashing with the track's actual bassline.