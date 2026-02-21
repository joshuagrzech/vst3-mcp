# Serum quick reference

## LFO routing (matrix workflow)

Use the Matrix tab for most modulation routing. Set Source to `LFO 1` (or any LFO), then set
Destination to a target like `Filter Cutoff`, `WT Pos`, or `FX Mix`. Raise Amount for positive
modulation or drag negative for inverse movement.

If routing sounds too strong, reduce the Matrix Amount first before changing the base knob.
This keeps the static tone while reducing modulation depth.

## LFO mode quirks

`TRIG` retriggers the LFO phase on note-on, which is useful for per-note consistency.
`ENV` mode turns the LFO into a one-shot envelope and is common for plucks and drops.
When designing tempo-locked movement, use BPM sync and verify triplet/dotted states.

## Filter and envelope interaction

When the filter sounds unstable, check whether both Env 2 and an LFO are modulating cutoff.
Stacked modulation can make the same knob look correct while sounding exaggerated.

For smoother bass movement, use lower resonance and slightly slower attack in Env 2, then
add a small LFO modulation amount for motion rather than full-range sweeps.
