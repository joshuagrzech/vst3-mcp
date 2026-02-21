# Percussion & Drums Execution Manual

## 1. Synthesized Kick Drum (Electronic/Dance)

Step 1: Initialize the core sound source by selecting a Sine wave for the main oscillator (`osc_shape` = `0.0`). This provides the fundamental low-frequency energy without harsh harmonics.

Step 2: Shape the amplitude envelope to control the volume over time. Set `attack` to `0.0`, `decay` to `0.3`, `sustain` to `0.0`, and `release` to `0.1`. This creates a short, punchy burst of sound that immediately dies away.

Step 3: Configure the pitch envelope, which is the most critical step for a kick drum. Set the pitch envelope `attack` to `0.0` and `decay` very fast (between `0.05` and `0.1`). 

Step 4: Route the pitch envelope to the Oscillator Pitch. Set the `modulation_amount` to `0.8` (representing a drop of +36 to +48 semitones). 

Routing & Rationale: Routing a lightning-fast envelope to the oscillator's pitch creates the "click" and "thump" of the beater hitting the drum skin. The rapid sweep from a high frequency down to the sub-frequency sine wave tricks the human ear into perceiving a powerful, transient-heavy physical impact.

## 2. Synthesized Snare Drum

Step 1: Set up Oscillator 1 (the body) using a Sine or Triangle wave. Tune the pitch to roughly 200Hz to simulate the resonance of the physical drum shell.

Step 2: Set up Oscillator 2 (the rattle) using White Noise. Set its level to `0.8`. 

Step 3: Route Oscillator 2 through a Highpass filter. Set the filter `cutoff` to `0.4` (~1kHz). 

Step 4: Configure the global Amp Envelope. Set `attack` to `0.0`, `decay` to `0.25`, `sustain` to `0.0`, and `release` to `0.15`.

Step 5: Create a pitch envelope for Oscillator 1. Set the `decay` to `0.04` and route it to Osc 1 pitch with an `amount` of `0.3`.

Routing & Rationale: A snare consists of a tuned shell and loose metal wires (snares) underneath. Oscillator 1 provides the tonal "crack" of the shell, aided by the fast pitch envelope to create impact. Routing the white noise through a highpass filter ensures it only occupies the high frequencies, perfectly mimicking the chaotic rattle of the metal snare wires without muddying the low-end body of the drum.