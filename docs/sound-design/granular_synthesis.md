# Granular Synthesis Execution Manual

## 1. Time-Stretched Ambient Pad

Step 1: Load a complex, tonal audio sample (such as a vocal choir, an orchestra chord, or a sustained piano note) into the primary granular engine.

Step 2: Set the grain size to be relatively large. Set `grain_size` to `0.6` (typically representing 100ms to 250ms per grain). 

Step 3: Increase the grain density to create a smooth, continuous wall of sound. Set `grain_density` or `grain_overlap` to `0.8` (triggering many grains simultaneously).

Step 4: Freeze or drastically slow down the playhead. Set `playback_speed` to `0.05` to slowly crawl through the sample, or `0.0` to freeze it entirely at a specific `sample_position`.

Step 5: Apply a slow Amp Envelope to fade the texture in and out. Set `attack` to `0.6`, `sustain` to `1.0`, and `release` to `0.7`.

Routing & Rationale: Granular synthesis works by chopping a sample into tiny micro-snippets (grains) and playing them back. By using large, overlapping grains and slowing the playback speed to near zero, the engine constantly crossfades between variations of the same tiny slice of audio. This entirely removes the original sample's transients and rhythm, transforming a brief, recognizable sound into an infinite, evolving, and smeared atmospheric pad.

## 2. Glitchy Rhythmic Stutter (IDM / Glitch)

Step 1: Load a transient-heavy, percussive audio file (such as a drum loop, mechanical foley, or beatbox recording) into the granular engine.

Step 2: Shrink the grain size drastically. Set `grain_size` to `0.1` (representing around 5ms to 15ms). 

Step 3: Lower the grain density to prevent overlapping. Set `grain_density` to `0.3` so individual grains fire distinctly with microscopic gaps between them.

Step 4: Create a Random or Sample & Hold LFO. Set the LFO `rate` to `0.5` (synced to something like 1/16th or 1/8th notes).

Step 5: Route the Random LFO to the `sample_position` parameter. Set the modulation `amount` to `0.4`.

Routing & Rationale: Tiny grains naturally impart a robotic, "buzzing" or "zipper" quality to the sound. By keeping the density low, we emphasize this chopped-up texture. Routing a randomized LFO to the playhead position forces the synthesizer to sporadically jump around the audio file, playing a hi-hat slice one moment and a snare slice the next. This generates a completely chaotic, unpredictable stutter effect perfect for IDM or cyberpunk soundscapes.

## 3. Metallic Grain Cloud (Cinematic / Tension)

Step 1: Load an atonal or highly resonant metallic strike (such as a struck cymbal, a bell, or scraped metal) into the engine.

Step 2: Set `grain_size` to `0.3` and `grain_density` to `0.6` for a moderate, scattered flow of grains.

Step 3: Apply extreme randomization to the pitch of each individual grain. Route a fast, un-synced Random LFO (or the engine's built-in grain pitch randomizer) to `grain_pitch`. Set the `amount` to `0.7` (+/- 12 to 24 semitones).

Step 4: Maximize the stereo width by randomizing grain placement. Set `pan_randomization` to `1.0` so each new grain fires in a different location in the left/right field.

Step 5: Route the master output through a Highpass filter (`cutoff`: `0.5`) and heavily drench it in Reverb (`mix`: `0.7`, `decay`: `0.8`).

Routing & Rationale: This patch is designed to build anxiety and tension. By randomizing the pitch of every single grain as it spawns, you completely destroy the original harmonic structure of the sample, replacing it with a dissonant, terrifying swarm of micro-sounds. Randomizing the pan creates a massive, enveloping stereo field, making it feel like the sound is swarming around the listener's head. The highpass filter removes low-end mud, ensuring the "cloud" sits cleanly over the top of a cinematic mix.