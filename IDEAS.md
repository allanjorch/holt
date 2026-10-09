# Ideas

Later shapes for holt. The place described in [DESIGN.md](DESIGN.md) stays as it is. These are ways it might grow.

## WAV export

Write a stretch of the forest to a file on the same render path the speakers use. The ecology can be fast-forwarded to a requested age before the first sample, so a file can open on a morning that has already been lived.

A live sitting and a later file of the same seed share the place. The exact noise is drawn again, so the two do not match sample for sample. Persisting `World` would make them match, if a file ever needs to be a continuation of a sitting rather than another visit.

## Walking to another preset

`--walk` moves the equilibrium slowly toward another preset. The ear hears turnover. Gaps stretch for species the new climate discourages, founders of the better-suited species appear, and the beds follow because their gains follow the climate. There is no crossfade of recordings.

## MIDI

The engine stays the instrument. A MIDI mirror of phrase starts would let someone record the score, or send the songs to another synth. Hearing the forest does not depend on it.

## A picture of the place

A live picture reads the world and never tells it what to do. The snapshot and the event ring are already the two surfaces. Sound does not wait on the picture, and the picture does not replay an audio buffer.

The snapshot is enough to draw the place. Canopy is a density. Rain rate is `drip_hz`. A singer is a mark in the stereo field that brightens for the length of a phrase, and a slot keeps its index so the same creature can be followed until it goes quiet.

Events are the punctuations: a phrase, a hatch, a death, a storm, a shower crossing a threshold. Drips and gusts stay off the ring. Their rates live on the snapshot, and the picture makes its own particles from those rates.

When the picture lives on a second thread, the audio thread publishes one double-buffered snapshot per block and a single-producer ring of events. A windowing toolkit stays out of the binary until there is a picture worth linking. It would dominate the size.

## A version inside the seed

Worth exploring before the roster grows. A seed could carry the version of the forest that grew it, so a later holt, with new animals or new biomes, mints its own seeds. An older seed would still open the place it named, with the singers and the rules of that version, and would not be reinterpreted by everything added since.
