# Melodic & Harmonic Elements Execution Manual

## 1. Analog Pluck / Arp (House / Trance)

Step 1: Choose a Square or Sawtooth waveform for the main oscillator to provide a harmonically rich starting point.

Step 2: Shape the Amp Envelope for a tight transient. Set `attack` to `0.0`, `decay` to `0.2`, `sustain` to `0.0`, and `release` to `0.2`.

Step 3: Route the oscillator through a 24dB/octave Lowpass filter. Set the base `cutoff` entirely closed (`0.1`) and `resonance` to `0.3`.

Step 4: Create a dedicated Filter Envelope. Set the `attack` to `0.0` and the `decay` to `0.15` (slightly faster than the amp decay).

Step 5: Route the Filter Envelope to modulate the Filter Cutoff. Set the modulation `amount` to `0.6`.

Routing & Rationale: Plucks rely heavily on spectral decay rather than just volume decay. By starting with the filter closed and using a snappy envelope to quickly open and shut it, you simulate the physical action of plucking a string. Routing the filter envelope to be slightly faster than the amp envelope ensures the bright harmonics die out just before the fundamental volume fades, creating a realistic, percussive tonal strike.

## 2. Supersaw / Epic Pad (EDM / Cinematic)

Step 1: Select a Sawtooth waveform as the core oscillator.

Step 2: Activate the Unison engine. Set the voice count between `7` and `9`. Set the `detune_amount` between `0.4` and `0.6`, and push the `stereo_spread` to `1.0` (100% wide).

Step 3: Configure the Amp Envelope for pad duties. Set `attack` to `0.4` (slow fade in), `decay` to `0.5`, `sustain` to `0.8`, and `release` to `0.5`.

Step 4: Route the output through a Lowpass filter with a static `cutoff` of `0.6` to tame the harshest high frequencies.

Step 5: Apply spatial effects. Add Reverb (`mix`: `0.4`, `size`: `0.8`) and Chorus (`mix`: `0.3`).

Routing & Rationale: The supersaw relies on stacking many identical waveforms and detuning them to create a massive, phase-smeared wall of sound. Pushing the unison stereo spread to maximum forces the detuned voices to the extreme left and right channels, leaving the center clear for drums and vocals. Routing this massive block of oscillators through heavy reverb and chorus further diffuses the sound, transforming aggressive sawtooth waves into a lush, cinematic texture.