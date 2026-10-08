# holt

A generative forest you can leave running. It is a place, not a tape.

Nature-sound albums usually loop. After a while the ear catches the exact stretch coming back: the same phrase, the same gust, in the same order. An hour in a real wood is still that wood. What changes is who is singing, and that this particular song has not been sung before.

Holt grows one place from a seed. Birds, frogs, insects, rain, and air are made as they sound. A phrase is kept only while it is being sung. The same seed brings you back to the same place and the same rules. It does not bring back the same performance.

## Listen

Rust is enough to build it. On Linux the program plays through ALSA, which Omarchy already provides through PipeWire.

```bash
cargo build --release
./target/release/holt
```

It plays until you press Ctrl-C. The first line is the seed for this place. To return to it:

```bash
holt --seed 12345
```

The first morning this project was heard with is `--seed 1847392`.

## Ways to listen

- `--for 2m` listens for two minutes. Durations look like `90s`, `2m`, or `6h`. `--for 0` prints who lives here and stops.
- `--preset broadleaf` is the morning you get by default. The others are `wetwood`, `dryridge`, `nightmarsh`, and `clearing`.
- `--at 6h` lets the place grow before you listen. `--from-seedling` starts with a thin canopy.
- `--world-rate 24` moves the sun, the weather, and the slow growth of the wood. The animals stay at their own pace, so a faster day does not turn the songs into chipmunks.
- `--trace --for 2m` prints that stretch at once, with no sound. `--trace` needs a `--for`.

On Windows the program builds and runs. Playback there is still to come, so `--trace` is the way to watch a place.

The model behind the forest — two clocks, species, succession — is written out in [DESIGN.md](DESIGN.md).

## License

MIT OR Apache-2.0
