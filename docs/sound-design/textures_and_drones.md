# Textures & Drones Execution Manual

## 1. Evolving Ambient Drone

Step 1: Initialize two oscillators using Complex Wavetables or FM Pairs.

Step 2: Create a glacial Amp Envelope. Set `attack` to `0.8`, `decay` to `1.0`, `sustain` to `1.0`, and `release` to `0.8`.

Step 3: Configure LFO 1 as a Triangle wave running at an extremely slow `rate` of `0.05`. Route this LFO to the Wavetable Position (or FM Index) of Oscillator 1 with an `amount` of `0.5`.

Step 4: Configure LFO 2 as a Sine wave running at a `rate` of `0.08`. Route this to the Filter Cutoff with an `amount` of `0.3`.

Step 5: Route the entire signal into a 100% wet Reverb (`mix`: `1.0`, `decay_time`: `0.9`).

Routing & Rationale: Drones require continuous, unpredictable internal movement to prevent them from sounding stagnant. Routing multiple, asynchronous LFOs running at incredibly slow speeds to different timbral parameters (like wavetable position and filter cutoff) ensures the harmonic structure is constantly shifting. Routing the final output into a 100% wet, heavily decayed reverb washes away the transients, transforming the raw waveforms into a smeared, atmospheric background bed.