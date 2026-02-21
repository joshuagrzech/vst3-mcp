# Bass Foundation Execution Manual

## 1. Pure Sub Bass (D&B / Dubstep / House)

Step 1: Select a Sine or Triangle wave for the primary oscillator. Tune the pitch down by exactly two octaves (`-24.0` semitones).

Step 2: Route the oscillator through a 24dB/octave Lowpass filter. Set the `cutoff` to `0.15` (targeting roughly 100Hz). 

Step 3: Force the synthesizer's output to absolute mono by setting the stereo width to `0.0`. 

Step 4: Apply a gentle Tube or Soft Clip saturation effect. Set the `drive` to `0.2`.

Routing & Rationale: Sub bass must translate evenly on massive club sound systems. Filtering a pure sine/triangle wave ensures no rogue high frequencies interfere with the mix. Forcing the signal to mono prevents phase cancellation issues in large speaker arrays. Finally, routing the clean sub through mild saturation generates 2nd and 3rd-order harmonics; this allows the bassline to remain audible on smaller consumer speakers (like phones or laptops) that cannot reproduce frequencies below 60Hz.

## 2. Reese / Neuro Mid-Bass

Step 1: Initialize Oscillator 1 and Oscillator 2, setting both to a Sawtooth waveform. Drop the pitch of both oscillators by one octave (`-12.0` semitones).

Step 2: Detune Oscillator 2 by setting `detune` to `0.55`. 

Step 3: Disable oscillator phase randomization (set to `0.0`). 

Step 4: Route both oscillators to a Bandpass or Notch filter. Set `cutoff` to `0.4` and `resonance` to `0.75`.

Step 5: Route the post-filter signal into an Asymmetrical or Diode distortion unit. Set `drive` to `0.8` and `mix` to `1.0`.

Routing & Rationale: The core of the Reese bass is the phasing movement caused by playing two slightly detuned waveforms against each other. Disabling phase randomization ensures the low-frequency phase aligns consistently on every keystroke, preventing random volume drops. Routing the detuned signal into a highly resonant notch filter creates vocal-like "formant" peaks. Smashing that filtered signal through heavy distortion compresses the peaks and vastly multiplies the harmonic density, resulting in an aggressive, tearing texture.