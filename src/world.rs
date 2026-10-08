use crate::dsp::{Mixer, Voice};
use crate::model::{
    daylight, BedGains, Config, IndividualView, Preset, Snapshot, Species, WorldEvent, BLOCK_MS,
    CHIRP_VOICES, DAY_MS, MAX_INDIVIDUALS, MAX_SPECIES, SAMPLE_RATE, STREAM_AUDIO, STREAM_ECO,
    STREAM_PHRASE, STREAM_PLACE, STREAM_ROSTER,
};
use crate::phrase::{self, Singer};
use crate::rng::Rng;
use crate::roster;

const EVENT_CAP: usize = 128;

#[derive(Clone, Copy, Debug, Default)]
struct Individual {
    alive: bool,
    species: u8,
    pitch_ratio: f32,
    tempo_ratio: f32,
    brightness_offset: f32,
    gap_ratio: f32,
    contour_offset: f32,
    pan: f32,
    distance: f32,
    age_s: u32,
    next_at_ms: u64,
    phrases_left: u8,
    in_bout: bool,
    reply_pending: bool,
    reply_hint_hz: f32,
    singing: bool,
    this_was_reply: bool,
    phrase_start_ms: u64,
    phrase_end_ms: u64,
    current_hz: f32,
    last_hz: f32,
}

pub struct World {
    seed: u64,
    preset: Preset,
    from_seedling: bool,
    world_rate_milli: u64,
    phase_origin_ms: u64,
    animal_ms: u64,
    world_ms: u64,
    moisture: f32,
    canopy: f32,
    disturbance: f32,
    gust: f32,
    shower: f32,
    held_water: f32,
    shower_latched: bool,
    equilibrium_moisture: f32,
    canopy_ceiling: f32,
    wind_exposure: f32,
    warmth: f32,
    species: [Species; MAX_SPECIES],
    species_count: u8,
    individuals: [Individual; MAX_INDIVIDUALS],
    voices: [Voice; CHIRP_VOICES],
    mixer: Mixer,
    eco: Rng,
    phrase_rng: Rng,
    /// Beds, drips, and branch ticks. Performances and ecology do not draw from it.
    audio: Rng,
    audio_origin_ms: u64,
    audio_played: u64,
    audio_armed: bool,
    events: [Option<WorldEvent>; EVENT_CAP],
    event_read: usize,
    event_write: usize,
    event_len: usize,
    events_dropped: u64,
    emitting: bool,
    steals: u32,
}

impl World {
    pub fn new(config: Config) -> Self {
        let rate = config.world_rate.clamp(0.05, 240.0);
        let eq = config.preset.equilibrium();
        let start_phase = config.preset.start_phase();
        let (moisture, canopy, held_water) = if config.from_seedling {
            (eq.moisture * 0.70, 0.05, 0.0)
        } else {
            let held = if config.preset == Preset::Wetwood {
                0.18
            } else {
                0.0
            };
            (eq.moisture, eq.canopy_ceiling * 0.55, held)
        };

        let mut world = Self {
            seed: config.seed,
            preset: config.preset,
            from_seedling: config.from_seedling,
            world_rate_milli: (rate * 1000.0).round() as u64,
            phase_origin_ms: (f64::from(start_phase) * DAY_MS as f64).round() as u64,
            animal_ms: 0,
            world_ms: 0,
            moisture,
            canopy,
            disturbance: 0.0,
            gust: 0.42,
            shower: 0.0,
            held_water,
            shower_latched: false,
            equilibrium_moisture: eq.moisture,
            canopy_ceiling: eq.canopy_ceiling,
            wind_exposure: eq.wind_exposure,
            warmth: eq.warmth,
            species: [Species::default(); MAX_SPECIES],
            species_count: 0,
            individuals: [Individual::default(); MAX_INDIVIDUALS],
            voices: [Voice::default(); CHIRP_VOICES],
            mixer: Mixer::new(),
            eco: Rng::stream(config.seed, STREAM_ECO),
            phrase_rng: Rng::stream(config.seed, STREAM_PHRASE),
            audio: Rng::stream(config.seed, STREAM_AUDIO),
            audio_origin_ms: 0,
            audio_played: 0,
            audio_armed: false,
            events: [None; EVENT_CAP],
            event_read: 0,
            event_write: 0,
            event_len: 0,
            events_dropped: 0,
            emitting: true,
            steals: 0,
        };

        let mut roster_rng = Rng::stream(config.seed, STREAM_ROSTER);
        world.species_count = roster::build(config.preset, &mut roster_rng, &mut world.species);
        let mut place_rng = Rng::stream(config.seed, STREAM_PLACE);
        world.place_initial(&mut place_rng);
        world
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn preset(&self) -> Preset {
        self.preset
    }

    pub fn from_seedling(&self) -> bool {
        self.from_seedling
    }

    pub fn world_rate(&self) -> f32 {
        self.world_rate_milli as f32 / 1000.0
    }

    pub fn animal_ms(&self) -> u64 {
        self.animal_ms
    }

    pub fn world_ms(&self) -> u64 {
        self.world_ms
    }

    pub fn roster(&self) -> &[Species] {
        &self.species[..self.species_count as usize]
    }

    pub fn events_dropped(&self) -> u64 {
        self.events_dropped
    }

    pub fn steals(&self) -> u32 {
        self.steals
    }

    pub fn day_phase(&self) -> f32 {
        ((self.phase_origin_ms + self.world_ms) % DAY_MS) as f32 / DAY_MS as f32
    }

    pub fn alive_of(&self, species: usize) -> u8 {
        self.individuals
            .iter()
            .filter(|ind| ind.alive && ind.species as usize == species)
            .count() as u8
    }

    pub fn capacity_of(&self, species: usize) -> u8 {
        self.species
            .get(species)
            .map(|s| s.capacity_at(self.moisture, self.canopy))
            .unwrap_or(0)
    }

    /// Grow the place by `seconds` of life at 1×, without sounding the songs.
    /// The next phrases are then placed in the future.
    pub fn fast_forward(&mut self, seconds: u64) {
        let was = self.emitting;
        self.emitting = false;
        for _ in 0..seconds {
            self.animal_ms += 1000;
            self.world_ms += 1000;
            self.world_second();
            self.animal_second();
        }
        self.emitting = was;
        self.silence();
        self.reschedule_all();
        self.clear_events();
    }

    pub fn advance_block(&mut self) {
        self.advance_ms(BLOCK_MS);
    }

    /// Fill interleaved stereo at 48 kHz, then move animal time by those samples.
    ///
    /// The buffer is synthesized from the voices already singing, so a phrase
    /// that begins during this step is heard on the next buffer. The first call
    /// anchors the audio clock to the current animal time, which is what makes
    /// `--at` the start of listening rather than a second pass through the songs.
    pub fn render(&mut self, interleaved: &mut [f32]) {
        let frames = interleaved.len() / 2;
        if frames == 0 {
            return;
        }
        if !self.audio_armed {
            self.audio_origin_ms = self.animal_ms;
            self.audio_played = 0;
            self.audio_armed = true;
        }
        let canopy = self.canopy;
        let gust = self.gust;
        let beds = self.bed_gains(self.heard_wind());
        self.mixer.fill(
            &mut self.voices,
            &mut self.audio,
            canopy,
            gust,
            &beds,
            interleaved,
        );
        self.audio_played = self.audio_played.saturating_add(frames as u64);
        let target = self
            .audio_origin_ms
            .saturating_add(self.audio_played.saturating_mul(1_000) / u64::from(SAMPLE_RATE));
        if target > self.animal_ms {
            self.advance_ms(target - self.animal_ms);
        }
    }

    pub fn advance_ms(&mut self, animal_ms: u64) {
        let prev_animal = self.animal_ms / 1000;
        let prev_world = self.world_ms / 1000;
        self.animal_ms += animal_ms;
        self.world_ms += animal_ms.saturating_mul(self.world_rate_milli) / 1000;
        let world_steps = self.world_ms / 1000 - prev_world;
        let animal_steps = self.animal_ms / 1000 - prev_animal;
        for _ in 0..world_steps {
            self.world_second();
        }
        for _ in 0..animal_steps {
            self.animal_second();
        }
        self.service_phrases();
    }

    pub fn poll_event(&mut self) -> Option<WorldEvent> {
        if self.event_len == 0 {
            return None;
        }
        let ev = self.events[self.event_read].take();
        self.event_read = (self.event_read + 1) % EVENT_CAP;
        self.event_len -= 1;
        ev
    }

    pub fn snapshot(&self) -> Snapshot {
        let phase = self.day_phase();
        let wind = self.heard_wind();
        let mut individuals = [IndividualView::default(); MAX_INDIVIDUALS];
        let mut alive = 0u8;
        for (i, ind) in self.individuals.iter().enumerate() {
            let kind = self
                .species
                .get(ind.species as usize)
                .map(|s| s.kind)
                .unwrap_or_default();
            let phrase_phase = if ind.singing && ind.phrase_end_ms > ind.phrase_start_ms {
                let span = (ind.phrase_end_ms - ind.phrase_start_ms) as f32;
                ((self.animal_ms.saturating_sub(ind.phrase_start_ms)) as f32 / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if ind.alive {
                alive += 1;
            }
            individuals[i] = IndividualView {
                slot: i as u8,
                species: ind.species,
                kind,
                alive: ind.alive,
                pan: ind.pan,
                distance: ind.distance,
                singing: ind.singing,
                phrase_phase,
                pitch_hz: ind.current_hz,
                level: if ind.alive {
                    self.species
                        .get(ind.species as usize)
                        .map(|s| s.level)
                        .unwrap_or(0.0)
                } else {
                    0.0
                },
            };
        }
        Snapshot {
            animal_ms: self.animal_ms,
            world_ms: self.world_ms,
            day_phase: phase,
            daylight: daylight(phase),
            moisture: self.moisture,
            canopy: self.canopy,
            wind,
            gust: self.gust,
            shower: self.shower,
            held_water: self.held_water,
            disturbance: self.disturbance,
            beds: self.bed_gains(wind),
            individuals,
            alive,
        }
    }

    fn bed_gains(&self, wind: f32) -> BedGains {
        let night = 1.0 - daylight(self.day_phase());
        let warm = self.warmth * (0.55 + 0.45 * daylight(self.day_phase()));
        BedGains {
            air: wind.clamp(0.0, 1.0),
            water: (self.moisture * 0.55 + self.held_water * 0.45).clamp(0.0, 1.0),
            insects: (self.moisture * night * warm * (1.0 - 0.5 * wind)).clamp(0.0, 1.0),
            drip_hz: self.held_water * 6.0,
            tick_hz: self.canopy * wind * 0.05,
        }
    }

    fn heard_wind(&self) -> f32 {
        let openness = 1.0 - self.canopy * 0.65;
        let gust = self.gust.clamp(0.0, 1.0);
        self.wind_exposure * openness * (0.35 + 0.65 * gust)
    }

    fn place_initial(&mut self, rng: &mut Rng) {
        let phase = self.day_phase();
        for s in 0..self.species_count as usize {
            let species = self.species[s];
            let cap = species.capacity_at(self.moisture, self.canopy);
            let act = species.activity_at(phase);
            if act <= 0.25 || cap == 0 {
                continue;
            }
            let mut n = ((cap as f32) * 0.6).round() as u8;
            if n == 0 {
                n = 1;
            }
            n = n.min(cap);
            for _ in 0..n {
                let species = self.species[s];
                let arrival = draw_arrival(rng, &species, None);
                self.install(s as u8, arrival, false);
            }
        }
        self.spread_initial(rng);
    }

    /// The first chorus is spread across the stereo field. Later births sit
    /// near a parent, so a family still clusters.
    fn spread_initial(&mut self, rng: &mut Rng) {
        let mut alive = [0u8; MAX_INDIVIDUALS];
        let mut n = 0;
        for (i, ind) in self.individuals.iter().enumerate() {
            if ind.alive {
                alive[n] = i as u8;
                n += 1;
            }
        }
        if n == 0 {
            return;
        }
        if n == 1 {
            self.individuals[alive[0] as usize].pan = rng.gen_range(-0.15, 0.15);
            return;
        }
        for (k, slot) in alive.iter().enumerate().take(n) {
            let t = k as f32 / (n - 1) as f32;
            let pan = -0.78 + t * 1.56 + rng.gen_range(-0.05, 0.05);
            self.individuals[*slot as usize].pan = pan.clamp(-1.0, 1.0);
        }
    }

    fn install(&mut self, species_index: u8, arrival: Arrival, announce: bool) {
        let Some(slot) = self.individuals.iter().position(|ind| !ind.alive) else {
            return;
        };
        self.individuals[slot] = Individual {
            alive: true,
            species: species_index,
            pitch_ratio: arrival.pitch_ratio,
            tempo_ratio: arrival.tempo_ratio,
            brightness_offset: arrival.brightness_offset,
            gap_ratio: arrival.gap_ratio,
            contour_offset: arrival.contour_offset,
            pan: arrival.pan,
            distance: arrival.distance,
            age_s: 0,
            next_at_ms: self.animal_ms + ms(arrival.first_wait_s),
            ..Individual::default()
        };
        if announce {
            self.emit(WorldEvent::Birth {
                slot: slot as u8,
                species: species_index,
                pan: arrival.pan,
                distance: arrival.distance,
            });
        }
    }

    fn world_second(&mut self) {
        let storm_roll = self.eco.f32();
        let hourly = 0.03 + 0.03 * self.moisture;
        if storm_roll < hourly / 3600.0 {
            self.strike_storm();
        }

        self.shower *= (-1.0 / 180.0_f32).exp();
        if self.shower < 1.0e-4 {
            self.shower = 0.0;
        }
        let heard = self.shower >= 0.08;
        if heard != self.shower_latched {
            self.shower_latched = heard;
            self.emit(WorldEvent::Shower {
                on: heard,
                intensity: self.shower,
            });
        }

        let wind = self.heard_wind();
        self.moisture += (self.equilibrium_moisture - self.moisture) * 1.5e-5 + self.shower * 0.01
            - wind * (1.0 - self.moisture) * 2.0e-6;
        self.canopy += 0.8e-5 * self.moisture * (self.canopy_ceiling - self.canopy)
            - self.disturbance * self.canopy * 0.002;
        self.disturbance *= (-0.001_f32).exp();
        self.held_water += self.shower * 0.05;
        self.held_water *= (-0.002_f32).exp();

        self.moisture = self.moisture.clamp(0.0, 1.0);
        self.canopy = self.canopy.clamp(0.0, self.canopy_ceiling.max(0.0));
        self.held_water = self.held_water.clamp(0.0, 1.0);
        self.disturbance = self.disturbance.clamp(0.0, 1.0);

        let count = self.species_count as usize;
        for s in 0..count {
            let species = self.species[s];
            let cap = species.capacity_at(self.moisture, self.canopy);
            let alive = self.alive_of(s);
            let roll_b = self.eco.f32();
            let roll_d = self.eco.f32();
            if alive < cap {
                let frac = (cap - alive) as f32 / cap as f32;
                let mut per_second = species.birth_per_hour / 3600.0 * frac;
                if alive == 0 {
                    per_second *= 0.5;
                }
                if roll_b < per_second {
                    let parent = self.lowest_alive(s as u8).map(|p| self.individuals[p]);
                    let species = self.species[s];
                    let arrival = draw_arrival(&mut self.eco, &species, parent.as_ref());
                    self.install(s as u8, arrival, true);
                }
            }
            if alive > cap {
                let frac = (alive - cap) as f32 / alive as f32;
                let per_second = species.death_per_hour / 3600.0 * frac;
                if roll_d < per_second {
                    if let Some(slot) = self.oldest(s as u8) {
                        self.kill(slot);
                    }
                }
            }
        }
    }

    fn strike_storm(&mut self) {
        let intensity = 0.08 + self.eco.f32().powi(2) * 0.75;
        self.disturbance = 1.0;
        self.shower = self.shower.max(intensity);
        let push = 10_000 + (self.eco.f32() * 80_000.0) as u64;
        let count = self.species_count as usize;
        for slot in 0..MAX_INDIVIDUALS {
            let ind = &self.individuals[slot];
            if !ind.alive || ind.singing {
                continue;
            }
            let shy = self
                .species
                .get(ind.species as usize)
                .filter(|_| (ind.species as usize) < count)
                .map(|s| s.wind_shy)
                .unwrap_or(0.5);
            let extra = push + (slot as u64 * 173) + (shy * 20_000.0) as u64;
            self.individuals[slot].next_at_ms = self.animal_ms.saturating_add(extra);
            self.individuals[slot].in_bout = false;
            self.individuals[slot].phrases_left = 0;
        }
        if self.eco.f32() < 0.25 {
            if let Some(slot) = self.most_exposed() {
                self.kill(slot);
            }
        }
        self.emit(WorldEvent::Storm { intensity });
    }

    fn most_exposed(&self) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, ind) in self.individuals.iter().enumerate() {
            if !ind.alive {
                continue;
            }
            let shy = self.species[ind.species as usize].wind_shy;
            let score = shy + ind.distance * 0.15;
            if best.map(|(_, s)| score > s).unwrap_or(true) {
                best = Some((i, score));
            }
        }
        best.map(|(i, _)| i)
    }

    fn animal_second(&mut self) {
        // One gaussian per slot, alive or not, so a birth never shifts later draws.
        let z = self.eco.gaussian();
        let tau = 8.0_f32;
        let a = (-1.0 / tau).exp();
        let mean = 0.45;
        self.gust = mean + (self.gust - mean) * a + 0.18 * (1.0 - a * a).sqrt() * z;
        self.gust = self.gust.clamp(0.0, 1.0);

        for slot in 0..MAX_INDIVIDUALS {
            let zp = self.eco.gaussian();
            let zd = self.eco.gaussian();
            if !self.individuals[slot].alive {
                continue;
            }
            let preferred = self.species[self.individuals[slot].species as usize].distance_median;
            let ind = &mut self.individuals[slot];
            ind.pan = (ind.pan + zp * 0.015).clamp(-1.0, 1.0);
            let distance = ind.distance + zd * 0.008 + (preferred - ind.distance) * 0.02;
            ind.distance = distance.clamp(0.05, 0.95);
            ind.age_s = ind.age_s.saturating_add(1);
        }
    }

    fn service_phrases(&mut self) {
        self.finish_due();
        self.start_due();
    }

    fn finish_due(&mut self) {
        let now = self.animal_ms;
        let mut due = [0u8; MAX_INDIVIDUALS];
        let mut n = 0;
        for (i, ind) in self.individuals.iter().enumerate() {
            if ind.alive && ind.singing && ind.phrase_end_ms <= now {
                due[n] = i as u8;
                n += 1;
            }
        }
        for slot in due.into_iter().take(n) {
            self.finish_phrase(slot as usize);
        }
    }

    fn start_due(&mut self) {
        let now = self.animal_ms;
        let mut due = [(0u64, 0u8); MAX_INDIVIDUALS];
        let mut n = 0;
        for (i, ind) in self.individuals.iter().enumerate() {
            if ind.alive && !ind.singing && ind.next_at_ms <= now {
                due[n] = (ind.next_at_ms, i as u8);
                n += 1;
            }
        }
        due[..n].sort_unstable();
        for &(_, slot) in due[..n].iter() {
            let slot = slot as usize;
            let ind = self.individuals[slot];
            if !(ind.alive && !ind.singing && ind.next_at_ms <= self.animal_ms) {
                continue;
            }
            let species = self.species[ind.species as usize];
            if !ind.reply_pending && species.activity_at(self.day_phase()) < 0.08 {
                let wait = self.phrase_rng.gen_range(40.0, 110.0);
                self.individuals[slot].next_at_ms = self.animal_ms + ms(wait);
                continue;
            }
            self.start_phrase(slot);
        }
    }

    fn start_phrase(&mut self, slot: usize) {
        let ind = self.individuals[slot];
        let species = self.species[ind.species as usize];
        let reply = ind.reply_pending;
        let hint = if reply && ind.reply_hint_hz > 1.0 {
            Some(ind.reply_hint_hz)
        } else {
            None
        };
        let singer = Singer {
            pitch_ratio: ind.pitch_ratio,
            tempo_ratio: ind.tempo_ratio,
            contour_offset: ind.contour_offset,
            brightness_offset: ind.brightness_offset,
            pan: ind.pan,
            distance: ind.distance,
        };
        let phrase = phrase::compose(&mut self.phrase_rng, &species, singer, hint);
        let dur_ms = ms(phrase.duration_s()).max(40);
        let gain = species.level * (1.0 - ind.distance).powi(2);
        let pitch_hz = phrase.syllables[0].start_hz;
        let syllables = phrase.count;
        let contour = phrase::contour_cents(&phrase);
        let signature = phrase::signature(&phrase, ind.species);
        self.alloc_voice(slot as u8, gain, &species, phrase);

        if reply {
            self.individuals[slot].this_was_reply = true;
            self.individuals[slot].in_bout = false;
            self.individuals[slot].phrases_left = 0;
        } else if !ind.in_bout {
            let n = self
                .phrase_rng
                .gen_range_u8(species.phrases_min, species.phrases_max);
            self.individuals[slot].phrases_left = n.saturating_sub(1);
            self.individuals[slot].in_bout = n > 1;
            self.individuals[slot].this_was_reply = false;
        } else {
            self.individuals[slot].phrases_left = ind.phrases_left.saturating_sub(1);
            self.individuals[slot].this_was_reply = false;
        }
        self.individuals[slot].reply_pending = false;
        self.individuals[slot].singing = true;
        self.individuals[slot].phrase_start_ms = self.animal_ms;
        self.individuals[slot].phrase_end_ms = self.animal_ms + dur_ms;
        self.individuals[slot].current_hz = pitch_hz;

        self.emit(WorldEvent::PhraseStarted {
            slot: slot as u8,
            species: ind.species,
            pan: ind.pan,
            distance: ind.distance,
            pitch_hz,
            duration_s: dur_ms as f32 / 1000.0,
            syllables,
            contour_cents: contour,
            reply,
            signature,
        });
        self.territorial_push(slot);
    }

    fn finish_phrase(&mut self, slot: usize) {
        let ind = self.individuals[slot];
        if !ind.alive || !ind.singing {
            return;
        }
        let species = self.species[ind.species as usize];
        self.release_voice(slot as u8);
        let last = ind.current_hz;
        self.individuals[slot].singing = false;
        self.individuals[slot].last_hz = last;
        self.individuals[slot].current_hz = last;

        if ind.this_was_reply {
            self.individuals[slot].this_was_reply = false;
            self.individuals[slot].in_bout = false;
            self.individuals[slot].phrases_left = 0;
            let gap_s = self.bout_gap(&species, slot);
            self.individuals[slot].next_at_ms = self.animal_ms + ms(gap_s);
        } else if self.individuals[slot].phrases_left > 0 {
            let gap_s = phrase::gap(
                &mut self.phrase_rng,
                species.phrase_gap_median,
                species.phrase_gap_sigma,
                0.08,
                2.5,
            ) * ind.gap_ratio;
            self.individuals[slot].next_at_ms = self.animal_ms + ms(gap_s);
        } else {
            self.individuals[slot].in_bout = false;
            let gap_s = self.bout_gap(&species, slot);
            self.individuals[slot].next_at_ms = self.animal_ms + ms(gap_s);
        }

        self.emit(WorldEvent::PhraseEnded {
            slot: slot as u8,
            species: ind.species,
        });
        if !ind.this_was_reply {
            self.maybe_reply(slot, last);
        }
    }

    fn bout_gap(&mut self, species: &Species, slot: usize) -> f32 {
        let ind = self.individuals[slot];
        let act = species.activity_at(self.day_phase()).clamp(0.05, 1.0);
        let weather = (1.0 - species.wind_shy * self.heard_wind().max(self.shower)).clamp(0.2, 1.0);
        let scale = 1.0 / (act * weather);
        let raw = phrase::gap(
            &mut self.phrase_rng,
            species.bout_gap_median,
            species.bout_gap_sigma,
            3.0,
            (species.bout_gap_median * 8.0).clamp(30.0, 240.0),
        );
        raw * ind.gap_ratio.clamp(0.75, 1.3) * scale
    }

    fn maybe_reply(&mut self, singer: usize, last_hz: f32) {
        let species_index = self.individuals[singer].species;
        let species = self.species[species_index as usize];
        let act = species.activity_at(self.day_phase());
        let weather = (1.0 - species.wind_shy * self.heard_wind().max(self.shower)).clamp(0.0, 1.0);
        let p = (species.replies * act * weather).clamp(0.0, 0.8);
        if p <= 0.0 {
            return;
        }
        let origin = self.individuals[singer].pan;
        let now = self.animal_ms;
        let mut best: Option<(usize, f32)> = None;
        for (i, ind) in self.individuals.iter().enumerate() {
            if i == singer || !ind.alive || ind.species != species_index {
                continue;
            }
            if ind.singing || ind.reply_pending || ind.next_at_ms <= now + 1500 {
                continue;
            }
            let dp = (ind.pan - origin).abs();
            if dp > 0.75 {
                continue;
            }
            if best.map(|(_, d)| dp < d).unwrap_or(true) {
                best = Some((i, dp));
            }
        }
        let Some((slot, _)) = best else {
            return;
        };
        if self.phrase_rng.f32() >= p {
            return;
        }
        let window = self
            .phrase_rng
            .log_normal(species.reply_window_median, 0.22)
            .clamp(0.35, 1.8);
        self.individuals[slot].reply_pending = true;
        self.individuals[slot].reply_hint_hz = last_hz;
        self.individuals[slot].next_at_ms = now + ms(window);
        self.individuals[slot].in_bout = false;
        self.individuals[slot].phrases_left = 0;
    }

    fn territorial_push(&mut self, singer: usize) {
        let species_index = self.individuals[singer].species;
        let territorial = self.species[species_index as usize].territorial;
        if territorial <= 0.0 {
            return;
        }
        let origin = self.individuals[singer].pan;
        let radius = 0.15 + 0.55 * territorial;
        let factor = 1.0 + territorial;
        let now = self.animal_ms;
        for i in 0..MAX_INDIVIDUALS {
            if i == singer {
                continue;
            }
            let ind = self.individuals[i];
            if !ind.alive || ind.species != species_index || ind.singing || ind.reply_pending {
                continue;
            }
            if (ind.pan - origin).abs() > radius {
                continue;
            }
            let remain = ind.next_at_ms.saturating_sub(now);
            if remain > 0 && remain < 12_000 {
                let stretched = ((remain as f32) * factor) as u64;
                self.individuals[i].next_at_ms = now + stretched.min(60_000);
            }
        }
    }

    fn alloc_voice(&mut self, slot: u8, gain: f32, species: &Species, phrase: phrase::Phrase) {
        let voice = Voice::begin(slot, gain, species, phrase);
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            self.voices[i] = voice;
            return;
        }
        let mut quiet = 0usize;
        let mut quiet_gain = f32::MAX;
        for (i, voice) in self.voices.iter().enumerate() {
            if voice.gain < quiet_gain {
                quiet_gain = voice.gain;
                quiet = i;
            }
        }
        let stolen = self.voices[quiet].slot as usize;
        self.steals += 1;
        self.interrupt(stolen);
        self.voices[quiet] = voice;
    }

    fn interrupt(&mut self, slot: usize) {
        if !self.individuals[slot].singing {
            self.release_voice(slot as u8);
            return;
        }
        let species = self.individuals[slot].species;
        let sp = self.species[species as usize];
        self.release_voice(slot as u8);
        self.individuals[slot].singing = false;
        self.individuals[slot].in_bout = false;
        self.individuals[slot].phrases_left = 0;
        self.individuals[slot].this_was_reply = false;
        self.individuals[slot].reply_pending = false;
        let gap_s = self.bout_gap(&sp, slot);
        self.individuals[slot].next_at_ms = self.animal_ms + ms(gap_s);
        self.emit(WorldEvent::PhraseEnded {
            slot: slot as u8,
            species,
        });
    }

    fn release_voice(&mut self, slot: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.slot == slot {
                *voice = Voice::default();
            }
        }
    }

    fn silence(&mut self) {
        for ind in &mut self.individuals {
            ind.singing = false;
            ind.in_bout = false;
            ind.phrases_left = 0;
            ind.reply_pending = false;
            ind.this_was_reply = false;
        }
        self.voices = [Voice::default(); CHIRP_VOICES];
        self.mixer.quiet();
        self.audio_armed = false;
        self.audio_played = 0;
    }

    fn reschedule_all(&mut self) {
        for slot in 0..MAX_INDIVIDUALS {
            if !self.individuals[slot].alive {
                continue;
            }
            let species = self.individuals[slot].species as usize;
            let median = self.species[species].bout_gap_median.clamp(4.0, 90.0);
            let wait = self.phrase_rng.gen_range(0.8, median);
            self.individuals[slot].next_at_ms = self.animal_ms + ms(wait);
        }
    }

    fn lowest_alive(&self, species: u8) -> Option<usize> {
        self.individuals
            .iter()
            .position(|ind| ind.alive && ind.species == species)
    }

    fn oldest(&self, species: u8) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (i, ind) in self.individuals.iter().enumerate() {
            if !ind.alive || ind.species != species {
                continue;
            }
            if best.map(|(_, age)| ind.age_s >= age).unwrap_or(true) {
                best = Some((i, ind.age_s));
            }
        }
        best.map(|(i, _)| i)
    }

    fn kill(&mut self, slot: usize) {
        if !self.individuals[slot].alive {
            return;
        }
        let species = self.individuals[slot].species;
        if self.individuals[slot].singing {
            self.release_voice(slot as u8);
            self.emit(WorldEvent::PhraseEnded {
                slot: slot as u8,
                species,
            });
        }
        self.individuals[slot].alive = false;
        self.individuals[slot].singing = false;
        self.emit(WorldEvent::Death {
            slot: slot as u8,
            species,
        });
    }

    fn emit(&mut self, event: WorldEvent) {
        if !self.emitting {
            return;
        }
        if self.event_len == EVENT_CAP {
            self.event_read = (self.event_read + 1) % EVENT_CAP;
            self.event_len -= 1;
            self.events_dropped += 1;
        }
        self.events[self.event_write] = Some(event);
        self.event_write = (self.event_write + 1) % EVENT_CAP;
        self.event_len += 1;
    }

    fn clear_events(&mut self) {
        self.events = [None; EVENT_CAP];
        self.event_read = 0;
        self.event_write = 0;
        self.event_len = 0;
        self.events_dropped = 0;
    }
}

struct Arrival {
    pitch_ratio: f32,
    tempo_ratio: f32,
    brightness_offset: f32,
    gap_ratio: f32,
    contour_offset: f32,
    pan: f32,
    distance: f32,
    first_wait_s: f32,
}

fn draw_arrival(rng: &mut Rng, species: &Species, parent: Option<&Individual>) -> Arrival {
    let (pitch_ratio, tempo_ratio, gap_ratio, brightness_offset, contour_offset, pan, distance) =
        if let Some(parent) = parent {
            (
                (parent.pitch_ratio * phrase::ratio_from_cents(rng.gaussian() * 25.0))
                    .clamp(0.94, 1.06),
                (parent.tempo_ratio + rng.gaussian() * 0.03).clamp(0.85, 1.15),
                (parent.gap_ratio + rng.gaussian() * 0.05).clamp(0.75, 1.3),
                (parent.brightness_offset + rng.gaussian() * 0.03).clamp(-0.15, 0.15),
                (parent.contour_offset + rng.gaussian() * 8.0).clamp(-40.0, 40.0),
                (parent.pan + rng.gaussian() * 0.12).clamp(-1.0, 1.0),
                (parent.distance + rng.gaussian() * 0.05).clamp(0.08, 0.95),
            )
        } else {
            (
                (1.0 + rng.gaussian() * species.pitch_scatter).clamp(0.9, 1.12),
                (1.0 + rng.gaussian() * 0.04).clamp(0.85, 1.15),
                (1.0 + rng.gaussian() * 0.08).clamp(0.75, 1.3),
                (rng.gaussian() * 0.04).clamp(-0.12, 0.12),
                (rng.gaussian() * 12.0).clamp(-30.0, 30.0),
                rng.gen_range(-0.75, 0.75),
                (species.distance_median * (1.0 + rng.gaussian() * 0.08)).clamp(0.08, 0.95),
            )
        };
    let median = species.bout_gap_median.clamp(4.0, 90.0);
    Arrival {
        pitch_ratio,
        tempo_ratio,
        gap_ratio,
        brightness_offset,
        contour_offset,
        pan,
        distance,
        first_wait_s: rng.gen_range(0.4, median),
    }
}

fn ms(seconds: f32) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActivityPattern, Config, Preset, VoiceKind, WorldEvent};
    use std::collections::HashSet;

    fn listen(world: &mut World, animal_ms: u64) -> Vec<WorldEvent> {
        let mut out = Vec::new();
        let mut left = animal_ms;
        while left > 0 {
            let step = left.min(BLOCK_MS);
            world.advance_ms(step);
            left -= step;
            while let Some(ev) = world.poll_event() {
                out.push(ev);
            }
        }
        out
    }

    #[test]
    fn same_seed_same_roster() {
        let a = World::new(Config::default());
        let b = World::new(Config::default());
        let pa: Vec<f32> = a.roster().iter().map(|s| s.pitch_hz).collect();
        let pb: Vec<f32> = b.roster().iter().map(|s| s.pitch_hz).collect();
        assert_eq!(pa, pb);
        assert!(a.roster().len() >= 4);
    }

    #[test]
    fn pitches_of_a_kind_stay_apart() {
        let world = World::new(Config::default());
        for kind in [
            VoiceKind::Chirp,
            VoiceKind::Trill,
            VoiceKind::Pulse,
            VoiceKind::Tick,
        ] {
            let mut pitches: Vec<f32> = world
                .roster()
                .iter()
                .filter(|s| s.kind == kind)
                .map(|s| s.pitch_hz)
                .collect();
            pitches.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in pitches.windows(2) {
                assert!(
                    pair[1] / pair[0] >= 1.08,
                    "{kind:?} pitches too close: {pitches:?}"
                );
            }
        }
    }

    #[test]
    fn broadleaf_morning_has_singers_and_no_frog_yet() {
        let world = World::new(Config::default());
        let mut frogs_alive = 0;
        let mut songs_alive = 0;
        for (i, species) in world.roster().iter().enumerate() {
            let alive = world.alive_of(i);
            match species.kind {
                VoiceKind::Pulse => {
                    assert_eq!(
                        world.capacity_of(i),
                        0,
                        "dry morning should not hold a frog"
                    );
                    frogs_alive += alive;
                }
                VoiceKind::Chirp => songs_alive += alive,
                _ => {}
            }
            assert!(alive <= species.capacity);
        }
        assert_eq!(frogs_alive, 0);
        assert!(songs_alive >= 2, "dawn should already have singers");
    }

    #[test]
    fn nightmarsh_is_awake_with_frogs() {
        let world = World::new(Config {
            preset: Preset::Nightmarsh,
            ..Config::default()
        });
        let frogs: u8 = world
            .roster()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind == VoiceKind::Pulse)
            .map(|(i, _)| world.alive_of(i))
            .sum();
        assert!(frogs >= 2, "a night marsh should already be peeping");
        let dawn_songs: u8 = world
            .roster()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.pattern == ActivityPattern::Dawn)
            .map(|(i, _)| world.alive_of(i))
            .sum();
        assert_eq!(dawn_songs, 0);
    }

    #[test]
    fn an_hour_of_morning_does_not_repeat_a_phrase() {
        let mut world = World::new(Config::default());
        let events = listen(&mut world, 60 * 60 * 1000);
        let mut seen = HashSet::new();
        let mut phrases = 0;
        let mut replies = 0;
        let mut collisions = Vec::new();
        for ev in &events {
            if let WorldEvent::PhraseStarted {
                signature,
                reply,
                species,
                pitch_hz,
                syllables,
                contour_cents,
                duration_s,
                ..
            } = ev
            {
                phrases += 1;
                if *reply {
                    replies += 1;
                }
                if !seen.insert(*signature) {
                    collisions.push((*species, *pitch_hz, *syllables, *contour_cents, *duration_s));
                }
            }
        }
        assert!(
            collisions.is_empty(),
            "exact phrase came back {phrases} phrases, {} collisions, first {:?}",
            collisions.len(),
            collisions.first()
        );
        assert!(phrases > 40, "too quiet: {phrases} phrases");
        assert!(phrases < 1200, "too crowded: {phrases} phrases");
        assert!(replies > 0, "dawn singers should sometimes answer");
        assert_eq!(world.steals(), 0);
        assert_eq!(world.events_dropped(), 0);
    }

    #[test]
    fn fast_forward_matches_a_lived_hour() {
        let cfg = Config {
            seed: 99,
            ..Config::default()
        };
        let mut lived = World::new(cfg);
        let mut jumped = World::new(cfg);
        let _ = listen(&mut lived, 30 * 60 * 1000);
        jumped.fast_forward(30 * 60);
        let a = lived.snapshot();
        let b = jumped.snapshot();
        assert_eq!(a.moisture.to_bits(), b.moisture.to_bits());
        assert_eq!(a.canopy.to_bits(), b.canopy.to_bits());
        assert_eq!(a.held_water.to_bits(), b.held_water.to_bits());
        assert_eq!(a.gust.to_bits(), b.gust.to_bits());
        assert_eq!(a.alive, b.alive);
        for (left, right) in a.individuals.iter().zip(b.individuals.iter()) {
            assert_eq!(left.alive, right.alive);
            assert_eq!(left.species, right.species);
            assert_eq!(left.pan.to_bits(), right.pan.to_bits());
            assert_eq!(left.distance.to_bits(), right.distance.to_bits());
        }
    }

    #[test]
    fn a_seedling_closes_a_little_canopy() {
        let mut world = World::new(Config {
            from_seedling: true,
            ..Config::default()
        });
        let before = world.snapshot().canopy;
        world.fast_forward(6 * 60 * 60);
        let after = world.snapshot().canopy;
        assert!(after > before + 0.01, "{before} -> {after}");
    }

    #[test]
    fn a_day_returns_the_sun_to_the_same_place() {
        let mut world = World::new(Config::default());
        let start = world.day_phase();
        world.fast_forward(24 * 60 * 60);
        let end = world.day_phase();
        assert!((start - end).abs() < 1.0e-5, "{start} vs {end}");
    }

    #[test]
    fn render_is_audible_and_finite() {
        let mut world = World::new(Config::default());
        let mut buf = vec![0.0f32; 480 * 2];
        let mut early_sq = 0.0f32;
        let mut early_n = 0usize;
        let mut early_peak = 0.0f32;
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f32;
        let mut n = 0usize;
        let blocks = 8 * 1_000 / BLOCK_MS;
        // This seed's first phrase is a little after two seconds.
        let early_blocks = 1_000 / BLOCK_MS;
        for block in 0..blocks {
            world.render(&mut buf);
            for sample in &buf {
                assert!(sample.is_finite(), "non-finite sample {sample}");
                let mag = sample.abs();
                peak = peak.max(mag);
                sum_sq += sample * sample;
                n += 1;
                if block < early_blocks {
                    early_peak = early_peak.max(mag);
                    early_sq += sample * sample;
                    early_n += 1;
                }
            }
        }
        let rms = (sum_sq / n as f32).sqrt();
        let early_rms = (early_sq / early_n as f32).sqrt();
        assert!(
            early_rms > 0.004,
            "air floor missing: early rms {early_rms} peak {early_peak}"
        );
        assert!(
            early_peak < 0.45,
            "air too loud before the first song: peak {early_peak} rms {early_rms}"
        );
        assert!(
            peak > 0.05 && peak < 0.95,
            "peak {peak} rms {rms} early rms {early_rms}"
        );
        assert!(rms > 0.008, "rms {rms} peak {peak}");
    }

    #[test]
    fn render_keeps_the_ecology_of_a_listen() {
        let cfg = Config::default();
        let mut heard = World::new(cfg);
        let mut lived = World::new(cfg);
        let mut buf = [0.0f32; 480 * 2];
        let ms = 15_000u64;
        let blocks = ms / BLOCK_MS;
        for _ in 0..blocks {
            heard.render(&mut buf);
        }
        let _ = listen(&mut lived, ms);
        let a = heard.snapshot();
        let b = lived.snapshot();
        assert_eq!(a.animal_ms, b.animal_ms);
        assert_eq!(a.world_ms, b.world_ms);
        assert_eq!(a.moisture.to_bits(), b.moisture.to_bits());
        assert_eq!(a.canopy.to_bits(), b.canopy.to_bits());
        assert_eq!(a.held_water.to_bits(), b.held_water.to_bits());
        assert_eq!(a.gust.to_bits(), b.gust.to_bits());
        assert_eq!(a.alive, b.alive);
        for (left, right) in a.individuals.iter().zip(b.individuals.iter()) {
            assert_eq!(left.alive, right.alive);
            assert_eq!(left.species, right.species);
            assert_eq!(left.singing, right.singing);
            assert_eq!(left.pan.to_bits(), right.pan.to_bits());
            assert_eq!(left.distance.to_bits(), right.distance.to_bits());
            assert_eq!(left.pitch_hz.to_bits(), right.pitch_hz.to_bits());
        }
    }

    #[test]
    fn render_after_a_jump_keeps_time() {
        let mut world = World::new(Config::default());
        world.fast_forward(30);
        let start = world.animal_ms();
        let mut buf = [0.0f32; 960];
        world.render(&mut buf);
        assert_eq!(world.animal_ms(), start + BLOCK_MS);
        assert!(buf.iter().all(|sample| sample.is_finite()));
    }
}
