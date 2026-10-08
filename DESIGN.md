# Holt

A generative forest that is a place, not a tape.

Working name: **holt** (a wood, a grove). The binary and the crate are `holt`.
Alternative if the name should mean the process rather than the place: **sere**, the ecological word for a sequence of communities. (`sere` is also an English adjective meaning dry and withered, which fights the feeling we want.)

## The listening problem

Nature-sound apps, videos, and albums usually play a recording, or a small set of recordings, on a loop. After a while the ear matches an earlier stretch exactly: the same phrase, the same gust, the same drip, in the same order. A real forest does not do that on a human timescale.

A real hour in one place is still the same place. What changes is who is singing, how the wind is moving, and the small fact that this performance of a song has never happened before and will not happen again.

Holt is built around that.

### Invariants

1. No audio buffer is ever replayed. Wind, water, and insect haze are synthesized while they sound.
2. A phrase exists only while it is being performed. The species remembers a dialect, not a take.
3. Species identity lives in ranges and tendencies: pitch region, contour, grouping, tone. Two renditions are recognizably the same kind of singer.
4. Local rhythm is part of nature. A cricket bout, a two-note frog, a falling four-note song: all welcome. The bout ends, the gap is irregular, and the next bout is a new draw.
5. The scheduler has no bar, no scene list, and no cycle length.
6. Animal behavior runs in real seconds. A faster world-clock never chipmunks the singers.
7. The seed fixes the place and the rules. It does not fix a performance.

## Two clocks

| Clock | What it advances | Default rate |
|---|---|---|
| Animal time | Phrases, gaps, bouts, replies, gust texture, drip texture | Always 1 real second per real second |
| World time | Day phase, moisture, canopy, showers, storms, birth and death | 1× (a day lasts 24 hours) |

`--world-rate 24` plays a day in an hour. Singers still phrase at real speed. The dawn chorus simply thins because morning is passing, not because the birds were sped up.

Gusts and leaf-noise stay on animal time, so a fast world-clock does not turn the wind nervous. Weather fronts and succession move on world time.

## Runtime shape

One thread. One fixed allocation at startup. No audio is rendered for time that is skipped.

On each audio block (~10 ms):

1. If one or more world-seconds have come due, run that many ecology steps (climate, births, deaths, storms).
2. If an animal-second has come due, advance gusts and the slow drift of positions.
3. Start any phrases whose time has arrived and for which a voice is free. Finish any phrase that has ended.
4. Render active voices, the beds, and the shared space into the block.

Ecology steps draw a fixed pattern of random numbers. Living through an hour and fast-forwarding that hour therefore grow the same place. Fast-forward skips the performances themselves and then places the next songs in the future, so the first moment you hear is not every singer starting at once. `--at` is that fast-forward, lived at 1 world-second per 1 animal-second. `--world-rate` applies only while you listen.

An ecology step over 64 individuals is a few microseconds. Doing it on the audio thread keeps the program one thread and keeps the memory a single struct. The step allocates nothing.

```
World (fixed)                render
  climate, weather             beds (always on)
  species roster               chirp voices (pool)
  individuals (slots)          transient voices (pool)
  schedule on each singer      one short shared space
```

Suggested ceilings:

```rust
const MAX_SPECIES: usize = 12;
const MAX_INDIVIDUALS: usize = 64;
const MAX_SYLLABLES: usize = 8;
const CHIRP_VOICES: usize = 24;
const TRANSIENT_VOICES: usize = 16; // drips, ticks, wing flicks
const SAMPLE_RATE: u32 = 48_000;
```

Memory, roughly: the world under 32 KB, delay lines for the space under 100 KB, device buffers a small extra. Resident size stays well under 2 MB, and under 512 KB is the aim. CPU is a small slice of one core, because most slots are silent.

The audio backend should be a single embedded C file (miniaudio or similar): WASAPI, Core Audio, and ALSA, with no separate install. WAV export uses the same render path and can fast-forward the ecology to a requested age before the first sample.

## A world

A world is a pure function of `(seed, preset, age)` plus everything the clocks have done since.

```rust
struct World {
    seed: u64,
    preset: Preset,
    animal_time_s: f64,
    world_time_s: f64,
    climate: Climate,
    weather: Weather,
    species: [Species; MAX_SPECIES],
    species_count: u8,
    individuals: [Individual; MAX_INDIVIDUALS],
    voices: Voices,
    space: Space,
    rng: Rng, // ecology stream only; audio noise has its own stream
}
```

Subseeds, via SplitMix64, so the streams stay independent:

* roster
* initial individuals
* ecology events
* audio noise

Ecology randomness is a sequential stream advanced once per world-second. `--at 6h` runs `6 * 3600` ecology steps at startup and then starts sound. That is deterministic and cheap. Audio noise is sequential too. A live performance and a later WAV of the same seed share the place and diverge in the exact noise, which is acceptable. Persisting `World` would make them match. v1 does not need that.

## Climate and weather

```rust
struct Climate {
    moisture: f32,     // 0 dry .. 1 saturated
    canopy: f32,       // 0 open .. 1 closed
    daylight: f32,     // 0 night .. 1 full day, derived from day_phase
    day_phase: f32,    // 0 at midnight, 0.25 at 06:00, 0.5 at noon
    disturbance: f32,  // 0 calm .. 1 just after a storm
}

struct Weather {
    gust: f32,         // animal-time, slow noise around 0..1
    shower: f32,       // 0..1, world-time
    held_water: f32,   // canopy storage that feeds drips after rain
}

struct Equilibrium {
    moisture: f32,
    canopy_ceiling: f32,
    wind_exposure: f32, // how windy the place is when the canopy is open
    warmth: f32,
}
```

`Equilibrium` comes from the preset, lightly perturbed by the seed, so two seeds of `broadleaf` are cousins.

Heard wind:

```
openness = 1 - canopy * 0.65
wind = equilibrium.wind_exposure * openness * (0.35 + 0.65 * gust)
```

### Integration, per world-second

Rates are chosen so that forty real minutes at 1× barely moves the canopy, while a morning started at dawn has become late morning four hours later.

```
moisture     += (equilibrium.moisture - moisture) * 1.5e-5
             +  shower * 0.01
             -  wind * (1 - moisture) * 2e-6

canopy       += 0.8e-5 * moisture * (canopy_ceiling - canopy)
             -  disturbance * canopy * 0.002

disturbance  *= 0.999  // about a 15-minute decay of a storm's shock
held_water   += shower * 0.05
held_water   *= 0.998  // drips continue after the shower stops

day_phase    += 1 / 86_400
```

`daylight` is a smooth hump: near 0 at midnight, near 1 at noon. `warmth_now = equilibrium.warmth * (0.6 + 0.4 * daylight)`.

### Storms

A Poisson draw once per world-hour, probability around 0.04 at typical climate, a little higher when moisture is already high. On a hit:

* `disturbance = 1`
* `shower` rises to a drawn intensity (a passing gust of rain, or a real shower) and then decays over a few world-minutes
* singing stretches: every live individual's `next_at` is pushed by 10–90 animal seconds, scaled by how exposed their species is
* a small chance that one exposed individual dies

Afterward, `held_water` keeps the drips going, frogs and other moisture-loving singers become more willing, and the birds return on their own schedules.

## Species

A species is a genotype. It is generated from the seed at the start and then left alone. Individuals carry the small differences.

```rust
enum VoiceKind {
    Chirp,  // songbirds, slurred singers
    Trill,  // crickets, some warblers, stridulation
    Pulse,  // frogs, toads
    Tick,   // woodpecker, branch, beak
}

enum PhraseShape {
    Slur,      // one syllable, an expressive pitch scoop
    Motif,     // a few syllables around a center, with an accent
    Repeated,  // nearly identical notes in a group
    Paired,    // two pulses, the classic amphibian interval
}

struct Niche {
    center: f32,     // 0..1 along one climate axis
    tolerance: f32,  // width of the hump
}

struct RangeF {
    median: f32,
    sigma: f32, // log-normal
}

struct RangeU {
    min: u8,
    max: u8,
}

struct Accent {
    step_cents: f32,     // typical interval between syllables
    step_scatter: f32,   // cents of variation around that interval
    contour_cents: f32,  // overall rise (positive) or fall across the phrase
    attack_s: f32,
    decay_share: f32,    // fraction of the syllable that is release
    noise_attack: f32,   // 0..1, a speck of noise at the onset
    vibrato_hz: f32,
    vibrato_cents: f32,
    sweep_ratio: f32,    // for Slur: end_hz / start_hz, e.g. 0.7 falls
    sweep_curve: f32,    // <1 sweeps early, >1 sweeps late
}

struct Species {
    kind: VoiceKind,
    shape: PhraseShape,
    moisture: Niche,
    canopy: Niche,
    /// Activity over day_phase. Sum of up to two wrapped humps.
    /// Dawn chorus, diurnal, dusk, nocturnal, or crepuscular.
    activity: [Hump; 2],
    pitch_hz: f32,
    pitch_scatter: f32,      // individual-to-individual ratio, e.g. 0.03
    syllable_s: RangeF,
    grouping: Grouping,
    accent: Accent,
    brightness: f32,         // 0 dull .. 1 bright
    level: f32,
    distance: RangeF,        // where this species prefers to sit
    replies: f32,            // 0..1 chance a neighbor answers
    reply_window_s: RangeF,
    capacity: u8,            // individuals when niche affinity is 1
    birth_per_hour: f32,     // world-hours, at full deficit
    death_per_hour: f32,     // world-hours, at full surplus
    wind_shy: f32,           // how much wind and rain suppress calling
    territorial: f32,        // how strongly a neighbor's song delays this one
}

struct Grouping {
    syllables_per_phrase: RangeU,
    phrases_per_bout: RangeU,
    syllable_gap_s: RangeF, // short, inside the phrase
    phrase_gap_s: RangeF,   // inside the bout
    bout_gap_s: RangeF,     // the long silence
}

struct Hump {
    phase: f32,   // center, 0..1 around the day
    width: f32,
    weight: f32,
}
```

Niche affinity is a Gaussian hump:

```
fn affinity(n: Niche, x: f32) -> f32 {
    let z = (x - n.center) / n.tolerance;
    exp(-z * z)
}

habitat = affinity(moisture_niche, climate.moisture)
        * affinity(canopy_niche, climate.canopy)
```

`habitat` decides how many individuals the place can hold.
`activity(day_phase)` decides how willing those individuals are to sing.
The two stay separate, so a dawn singer still lives here at noon and is simply quiet.

Pitch centers for species of the same `VoiceKind` are drawn with a minimum separation of about 150 cents, so two singers share a family and keep their own register. Frog pulses sit between about 180 and 700 Hz. High song sits between about 2 and 5 kHz. Trills may sit higher and thinner.

### Activity humps

| Pattern | Humps (phase, the day) |
|---|---|
| Dawn chorus | one hump around 0.27 (about 06:30), narrow |
| Diurnal | one broad hump around 0.50 |
| Dusk | one hump around 0.80 |
| Nocturnal | one hump around 0.00, wide enough to cover the night |
| Crepuscular | dawn hump plus dusk hump |

## Individuals

Fixed slots. Birth switches a slot on. Death switches it off. The array never grows.

```rust
struct Individual {
    alive: bool,
    species: u8,
    pitch_ratio: f32,      // dialect, near 1
    tempo_ratio: f32,
    brightness_offset: f32,
    gap_ratio: f32,
    contour_offset: f32,   // a personal lean on the species accent
    pan: f32,              // -1..1
    distance: f32,         // 0 near .. 1 far
    next_at_s: f64,        // animal time
    phrases_left: u8,      // remaining in this bout, after the one in progress
    reply_generation: u8,  // 0 normal, 1 this bout is an answer, 2 would be an answer to an answer and is never scheduled
    singing: bool,
}
```

Dialect is drawn at birth from the species scatter and then held. A child of an existing singer is placed nearby in pan and distance, and its dialect is the parent's plus a small mutation:

* pitch ratio: parent ± up to about 40 cents, clamped to the species scatter
* tempo, gap, brightness, contour: a small step, clamped

A founder, arriving when the species has no one alive, is placed at the species' preferred distance with a random pan.

Movement is animal-time and very slow. Pan and distance drift on a slow random walk, pulled back toward the species' preferred distance. Crossing the stereo field takes minutes. Positions wrap gently at the edges.

## Phrase generation

Called at the moment of singing. The result is written into a voice and discarded when the voice releases.

```rust
struct Syllable {
    start_hz: f32,
    end_hz: f32,
    duration_s: f32,
    amplitude: f32,
    noise: f32,
    gap_after_s: f32,
}

struct Phrase {
    syllables: [Syllable; MAX_SYLLABLES],
    count: u8,
    pan: f32,
    distance: f32,
    brightness: f32,
    kind: VoiceKind,
}
```

Draw, for a center pitch `center = species.pitch_hz * individual.pitch_ratio * phrase_jitter` where `phrase_jitter` is a few percent:

* **Slur.** One syllable. `start_hz = center`, `end_hz = center * sweep_ratio`, duration from `syllable_s`. The sweep is the song.
* **Motif.** `k` syllables from `syllables_per_phrase`. Syllable `i` sits at

  ```
  cents = contour_cents * (i / (k - 1))
        + step_cents * (i - (k - 1) / 2) * 0.15
        + gaussian(0, step_scatter)
        + individual.contour_offset
  hz = center * 2^(cents / 1200)
  ```

  The phrase stays around `center`. It does not random-walk out of the species.
* **Repeated.** `k` syllables on `center`, each detuned by only a few cents. Gaps are short and slightly uneven. This is a cricket chirp or a dove.
* **Paired.** Two syllables. The interval is `step_cents` plus a little scatter (often a third or a fourth). A tight gap inside the pair.

Amplitudes within a phrase fall or rise slightly with the contour, and the first syllable is a little stronger. `noise` is highest on the attack portion, from `accent.noise_attack`.

Bout logic, stored on the individual:

* Starting from silence, draw `phrases_per_bout`, write it to `phrases_left`, and start one phrase now.
* When the phrase ends, if `phrases_left > 0`, decrement it and set `next_at = now + phrase_gap * gap_ratio`.
* When the bout ends, set `next_at = now + bout_gap * gap_ratio` and clear `phrases_left`.

`bout_gap` is log-normal with a long tail. Medians, by character:

| Character | Syllables | Phrases in a bout | Bout gap median |
|---|---|---|---|
| High dawn singer | 3–6 | 1 | 15–40 s |
| Slurred mid singer | 1 | 1–2 | 20–50 s |
| Cricket | 4–8 | 4–12 | 8–25 s |
| Frog | 2 | 3–8 | 12–40 s |
| Woodpecker tick | 1–4 | 1 | 40–120 s |

These are generation ranges, not constants shared by every member of a preset. Each species draws its own medians inside the range.

### Call and response

When a phrase ends, look at up to two living conspecifics that are silent, within a pan distance scaled by `territorial`, with `reply_generation == 0`. With probability `species.replies * activity`, schedule one of them:

* `next_at = now + reply_window` (about 0.4–1.5 s)
* `reply_generation = 1`
* the reply phrase is generated shorter (fewer syllables, or one pair) and its center starts near the last syllable the neighbor just sang, then leans back toward the replier's own pitch

An answer does not produce an answer. `reply_generation` stops the chain.

### Territorial delay

If a conspecific within the territorial radius began a phrase in the last few seconds, multiply this individual's upcoming gap by about `1 + territorial`. Songs space themselves without a grid.

### Wind and rain suppress song

```
shy = 1 - wind_shy * max(wind, shower)
```

`shy` stretches bout gaps. Frogs use a low `wind_shy` and a high moisture niche, so a shower can wake them while the high singers wait it out.

## Beds

Beds are the place itself. They are always present, and they are not individuals.

```rust
struct Beds {
    air: NoiseBed,
    water: NoiseBed,
    insects: NoiseBed,
}
```

Each bed is white noise (xorshift or similar, a few bytes of state) shaped by a one-pole or a short pink filter (Kellet's economy filter, a handful of floats), then a gentle bandpass. Gain and cutoff move with slow animal-time noise plus the climate. Nothing is read from a looped buffer.

| Bed | Gain follows | Color |
|---|---|---|
| Air | `wind`, as defined above | low and broad, duller as the canopy closes |
| Water | `moisture`, plus a slower term from `held_water` | mid, soft |
| Insects | `moisture * nocturnal_or_warm * (1 - 0.5 * wind)` | thin and high, quiet |

Drips and ticks are transient voices, Poisson in animal time:

```
drip_rate_hz = held_water * 6          // after rain, a brief patter, then sparse
tick_rate_hz = canopy * wind * 0.05    // a branch, very rare
```

A drip is a pulse voice with a short decay, high and small, panned at random. A branch tick is a noise burst of a few milliseconds.

There is a floor of air even in a still, dry moment, so the render never collapses to digital silence.

## Space

One shared space, short and dull. A forest is not a long reverb.

```rust
struct Space {
    early_s: [f32; 6],     // all under ~40 ms
    early_gain: [f32; 6],
    tail_s: f32,           // about 0.12 .. 0.22
    tail_feedback: f32,    // low
    hf_damp: f32,
}
```

Per voice, before the send:

```
near  = 1 - distance
gain  = near * near * level
hz    = lerp(700, 12_000, near) * (1 - canopy * 0.4) * brightness
delay = distance * 0.035
wet   = lerp(0.05, 0.5, distance)
```

Early-tap times drift by about ±1 ms with the gust signal, so the reflection pattern breathes. The tail is heavily damped. Delay memory stays under 100 KB.

## Voices

```rust
struct Voice {
    active: bool,
    kind: VoiceKind,
    phrase: Phrase,
    syllable: u8,
    t: f32,
    phase: f32,
    filter: OnePole,
    owner: Option<u8>, // individual slot; None for drips
}
```

Rendering one sample of a chirp syllable:

```
u     = t / duration
pitch = lerp(start_hz, end_hz, u ^ sweep_curve)
phase += pitch / SAMPLE_RATE
env   = attack_then_decay(u, attack_s, decay_share)
tone  = sin(phase + vibrato)
nse   = noise * noise_attack * onset(u)
x     = tone * (1 - noise_mix) + nse
y     = lowpass(x, cutoff_hz(env, brightness, distance, canopy)) * env
```

The lowpass opens a little with `env`, so the syllable is brighter at its peak.

* **Trill** is the same oscillator, amplitude-gated by a pulse train, or a motif of very short repeated syllables. A cricket is closer to a pure tone, low `noise_attack`, little sweep.
* **Pulse** is a resonator that is struck and rings: frequency fixed, amplitude an exponential decay. The species pitch is the resonator.
* **Tick** is noise through a short decay, sometimes a resonant tick for a woodpecker.

If the chirp pool is full, the quietest voice yields. Beds are never stolen. Natural densities should leave the pool with room; the dawn chorus is the busiest case and wants well under 24 simultaneous songs, because bout gaps are long.

## Succession

Once per world-second, for each species:

```
habitat  = moisture_affinity * canopy_affinity
capacity = round(species.capacity * habitat)
alive    = count of living individuals of this species
```

* If `alive < capacity`, a birth is drawn with rate `birth_per_hour * (capacity - alive) / capacity`. A birth fills a free slot. Prefer a child of a living member. If none are alive, it is a founder, and the rate is halved so arrival is an event.
* If `alive > capacity`, a death is drawn with rate `death_per_hour * (alive - capacity) / capacity`. The oldest, or any one of them, switches off at the end of its current phrase.
* If `habitat` is near 0, capacity is 0, and the species thins out over minutes of world time. It does not pop out of existence on a single sample.

Call willingness, used when drawing the next bout gap and when a phrase wants to start:

```
willingness = activity(day_phase) * (1 - wind_shy * max(wind, shower))
```

A low willingness stretches `next_at`. It does not kill anyone.

What forty quiet minutes at 1× should contain:

* the same roster and almost the same canopy
* gusts that come and go
* singers that take long irregular breaks
* possibly one birth, one extended silence from a single individual, or a light shower
* no sample, phrase, or stereo snapshot that matches an earlier one

What a morning started at dawn contains, four real hours later:

* the dawn chorus has eased into midday sparseness, because `day_phase` moved and activity followed
* canopy and moisture have crept
* the place is still itself

What `--from-seedling` contains:

* canopy near 0.05, moisture a little under the equilibrium
* one or two founder singers, wind relatively open
* over world-hours, colonists arrive as habitat allows, and the canopy closes toward its ceiling

`--from-seedling` is the audible succession. The default is already a place you can sit down in.

## Presets

A preset is an equilibrium, a bias on the roster draw, and a default day phase. It is not a loop and not a separate engine. The seed fills in the actual species.

Five presets:

| Preset | Equilibrium (moisture, canopy ceiling, wind, warmth) | Roster bias | Starts |
|---|---|---|---|
| `broadleaf` | 0.45, 0.75, 0.35, 0.55 | two or three chirps, one trill, one pulse with a narrow moisture niche, rare ticks | dawn, phase 0.28 |
| `wetwood` | 0.80, 0.90, 0.25, 0.70 | pulses and drips prominent, one high chirp, strong night insects | pre-dawn, phase 0.22 |
| `dryridge` | 0.18, 0.30, 0.70, 0.60 | one or two sparse chirps, a tick, thin insects, lots of air | morning, phase 0.35 |
| `nightmarsh` | 0.85, 0.40, 0.20, 0.50 | paired pulses, trills, quiet air, almost no diurnal chirps | night, phase 0.92 |
| `clearing` | 0.35, 0.25, 0.55, 0.55 | pioneers: one chirp, one trill, open wind. Canopy ceiling stays low. | dawn, phase 0.27 |

Roster generation, after the preset picks counts and ranges:

1. Draw pitch centers inside the kind's range, with a minimum separation.
2. Draw niches. Stratify them across moisture and canopy so the species do not all prefer the identical climate. A frog's moisture center is high. A pioneer chirp's canopy center is low. A shade singer's canopy center is high.
3. Draw accent, grouping, and capacity inside the preset's ranges.
4. Place the initial population at about 60% of current capacity for species whose activity at the start phase is above a small threshold. Others start absent and may colonize later.

Default preset is `broadleaf`.

A later `--walk` can move the equilibrium targets slowly toward another preset. The ear hears turnover: gaps stretch for species the new climate discourages, founders of the better-suited species appear, beds crossfade because their gains follow climate. There is no crossfade of recordings.

## Sharing a place

```
holt --seed 1847392
holt --seed 1847392 --preset wetwood
holt --seed 1847392 --at 6h
holt --seed 1847392 --from-seedling
holt --seed 1847392 --world-rate 24
```

The same seed and preset name the same roster, the same equilibrium, and the same initial singers. `--at` is how long the place has already been alive, grown at a natural 1×. `--world-rate` then applies only to the sitting you actually hear. The performance from the moment you press play is new every time, which is the point.

Listener controls stay small: seed, preset, start age, world rate, and a moisture nudge.

## Observers

The world has no knowledge of pixels. A picture is a reader, and the text trace is the first one. Two surfaces, both plain data:

1. **Snapshot**, polled whenever the reader wants. Time of day, daylight, moisture, canopy, wind, shower, water held in the leaves, disturbance, the bed gains (air, water, insects, drip rate), and every individual slot: alive or not, species, kind, pan, distance, whether they are inside a phrase, how far through that phrase, and the pitch they are on. A slot keeps its index for the life of that creature, so a picture can track the same singer until the slot goes quiet. This is enough to draw the place. Canopy is a density. Rain rate is `drip_hz`. A singer is a mark in the stereo field that brightens for the length of a phrase. Copying a snapshot is a few kilobytes and copies no pointers.

2. **Events**, a fixed ring of edges: phrase started, phrase ended, birth, death, storm, shower crossing a threshold. A picture uses these for punctuations such as a hatch or a change in the weather. If the reader falls behind, the oldest edges drop and a counter increments. The snapshot stays true. Drips and gusts are not events. Their rates live on the snapshot, and a picture makes its own particles from those rates, so the ring stays small and a visual never replays an audio buffer.

Nothing in the ecology calls back into a renderer. A later window, a terminal picture, or another process reads these same two surfaces. When that window lives on a second thread, the audio thread publishes one double-buffered snapshot per block and a single-producer ring of events. The types are plain `Copy` values. Sound never waits on the picture.

A windowing toolkit stays out of the binary until there is a picture worth linking. It would dominate the size.

## MIDI

The internal engine is the instrument. A MIDI mirror of phrase starts is a possible later output for people who want to record the score or drive another synth. It is not required to hear anything, and v1 does not include it.

## Defaults

* Name: `holt`
* Language: Rust. Audio backend: one embedded C file, compiled into the binary.
* Preset: `broadleaf`, already populated, dawn.
* World rate: 1×. Animal time: always real.
* Phrase performances are discarded.
* Single thread, fixed pools, allocation only at startup.
* MIDI and a graphical interface wait. The snapshot and the event ring are the door they will use.

## Build order, when code starts

1. The world struct, roster generation, and ecology step, with a text trace of who sings and when. The trace is the first observer: it drains the event ring and prints a snapshot on a slow cadence. Listen with your eyes for loops, crowding, and dead air.
2. Chirp, pulse, and the air bed, straight to the speakers.
3. The other beds, drips, distance, and the short space.
4. Presets, `--at`, `--from-seedling`, and `--world-rate`.
5. WAV export on the same path.
