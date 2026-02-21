# Physical Modeling Synthesis Execution Manual

## 1. Plucked String (Karplus-Strong Emulation)

Step 1: Configure the Exciter. To emulate a guitar pick or finger pluck, generate a very brief burst of broadband energy. Set the exciter to White Noise with an Amp Envelope `attack` of `0.0`, `decay` of `0.02` (extremely fast), `sustain` of `0.0`, and `release` of `0.0`. 

Step 2: Route the Exciter into a tuned Resonator, specifically a short Delay Line with high feedback. Set the delay time to perfectly match the period of the desired fundamental frequency (e.g., if tuning to A4/440Hz, the delay time is ~2.27ms). 

Step 3: Set the delay line's `feedback_amount` to a very high value, typically `0.95` to `0.99`. This parameter governs the sustain/decay of the string.

Step 4: Insert a Lowpass filter inside the delay feedback loop to act as the dampening material. Set the `dampening_cutoff` to `0.7`. 

Step 5: Tie the synthesizer's velocity parameter to the Exciter's volume and the filter's `dampening_cutoff`. Harder keystrokes should increase the noise burst volume and push the cutoff closer to `1.0`.

Routing & Rationale: Karplus-Strong synthesis relies on feeding a tiny click of noise into a delay line that loops hundreds of times a second. The delay time dictates the pitch, and the high feedback keeps the "string" ringing. Crucially, routing a lowpass filter *inside* the feedback loop ensures that with every repetition, the highest frequencies decay faster than the low ones. This perfectly mirrors the physics of acoustic strings losing high-frequency energy to friction over time.

## 2. Struck Mallet / Bell (Modal Synthesis)

Step 1: Set the Exciter to simulate a hard wooden or felt mallet strike. Use a very short Sine or Triangle burst (`decay`: `0.05`) mixed with a subtle metallic click (`decay`: `0.01`).

Step 2:  Route the Exciter into a Resonator Bank (Modal Resonator) consisting of 4 to 8 parallel bandpass filters or sine oscillators.

Step 3: Detune the partials (modes) non-harmonically. Unlike a sawtooth wave where harmonics are strict integer multiples, a bell's overtones are chaotic. Set Mode 1 (Fundamental) to `1.0x` frequency, Mode 2 to `2.76x`, Mode 3 to `5.4x`, and Mode 4 to `8.9x`.

Step 4: Configure the individual decay times (stiffness/loss) for each mode. Set the fundamental `decay` to `0.8` (long ringing). Set the higher, dissonant modes to decay much faster (Mode 2 `decay`: `0.4`, Mode 3 `decay`: `0.2`).

Step 5: Randomize the strike position. Route a subtle Random LFO (`amount`: `0.15`) to the amplitude mix of the upper modes.

Routing & Rationale: Modal synthesis treats physical objects as a collection of independent resonant frequencies (modes). By striking a parallel bank of sharply tuned, non-harmonic filters with an exciter, you force them all to "ring out" at once. Routing different decay times to each mode simulates the stiffness and physical mass of a metal or wooden bar. Randomizing the amplitudes of the upper modes mimics the fact that hitting a xylophone slightly off-center excites different overtones than hitting it dead in the middle.

## 3. Wind / Breath Instrument (Tube Resonator)

Step 1: Set the primary Exciter to continuous White/Pink Noise to simulate breath airflow. 

Step 2: Shape the breath envelope to be more organic than a percussive strike. Set `attack` to `0.15` (a slight fade-in), `decay` to `0.4`, `sustain` to `0.7`, and `release` to `0.2`.

Step 3: Route the noise into a Tube Resonator or comb filter. Tune the resonator to the desired fundamental pitch.

Step 4: Modulate the breath pressure. Route a slow, subtle Sine LFO (`rate`: `0.5`, `amount`: `0.1`) to both the Exciter's amplitude and the Resonator's `brightness` or `cutoff` parameter.

Step 5: Introduce turbulence (overblowing). Map the Mod Wheel or Aftertouch (`0.0` to `1.0`) to increase the Exciter's noise level by `+0.4` and push the Resonator's pitch slightly sharp by `+0.05` semitones.

Routing & Rationale: Wind instruments are simply columns of air excited by turbulent breath. Routing sustained noise into a tuned tube resonator forces the noise to take on the resonant pitch of the tube. Tying an LFO to the breath pressure parameters creates human-like lung variations. Routing performance controls (like aftertouch) to push the volume and pitch sharp perfectly models the physical "overblowing" effect where a player forces too much air into a flute or saxophone, causing the pitch to bend and the timbre to become raspy.