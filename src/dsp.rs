//! The sounding forest. Ecology and phrase rolls never draw from here, and
//! nothing in `fill` allocates. A phrase is played from the voice that stored
//! it, then forgotten.

use crate::model::{BedGains, Species, VoiceKind, SAMPLE_RATE};
use crate::phrase::{Phrase, Syllable};
use crate::rng::Rng;

const SR: f32 = SAMPLE_RATE as f32;
const TAU: f32 = std::f32::consts::TAU;
const CHUNK: usize = 480;
const SPACE_LEN: usize = 9_600;
const FADE_SAMPLES: u64 = 4_800;
const MASTER: f32 = 1.45;

const EARLY: [usize; 6] = [384, 640, 960, 1_344, 1_728, 2_112];
const EARLY_GAIN: [f32; 6] = [0.22, 0.16, 0.12, 0.08, 0.05, 0.03];
const TAIL: usize = 7_680;
const TAIL_FEEDBACK: f32 = 0.22;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Voice {
    pub active: bool,
    pub slot: u8,
    /// `species.level * (1 - distance)^2`. The quietest active voice yields.
    pub gain: f32,
    kind: VoiceKind,
    attack_s: f32,
    decay_share: f32,
    sweep_curve: f32,
    vibrato_hz: f32,
    vibrato_cents: f32,
    level: f32,
    phrase: Phrase,
    syl: u8,
    syl_pos: u32,
    phase: f32,
    vib_phase: f32,
    filter: f32,
    noise: u32,
}

impl Voice {
    pub(crate) fn begin(slot: u8, gain: f32, species: &Species, phrase: Phrase) -> Self {
        let hz_bits = phrase
            .syllables
            .first()
            .map(|s| s.start_hz.to_bits())
            .unwrap_or(0);
        Self {
            active: phrase.count > 0,
            slot,
            gain,
            kind: species.kind,
            attack_s: species.attack_s,
            decay_share: species.decay_share,
            sweep_curve: if species.sweep_curve <= 0.05 {
                1.0
            } else {
                species.sweep_curve
            },
            vibrato_hz: species.vibrato_hz,
            vibrato_cents: species.vibrato_cents,
            level: species.level.max(1.0e-4),
            phrase,
            syl: 0,
            syl_pos: 0,
            phase: 0.0,
            vib_phase: 0.0,
            filter: 0.0,
            // Hashed so the scheduler does not have to draw from the audio generator.
            noise: hash_noise(slot, hz_bits, gain.to_bits()),
        }
    }

    fn render(&mut self, canopy: f32, out: &mut [f32], wet: &mut [f32]) {
        if !self.active || self.phrase.count == 0 {
            self.active = false;
            return;
        }
        let frames = out.len() / 2;
        let pan = self.phrase.pan.clamp(-1.0, 1.0);
        let left = ((1.0 - pan) * 0.5).sqrt();
        let right = ((1.0 + pan) * 0.5).sqrt();
        let send = 0.05 + 0.45 * self.phrase.distance.clamp(0.0, 1.0);
        let near = (1.0 - self.phrase.distance).clamp(0.0, 1.0);
        let bright = self.phrase.brightness.clamp(0.12, 1.0);
        let mut base_hz = (700.0 + 11_300.0 * near) * (1.0 - canopy.clamp(0.0, 1.0) * 0.4) * bright;
        if matches!(self.kind, VoiceKind::Tick) {
            base_hz = base_hz.max(3_200.0);
        }

        for i in 0..frames {
            if !self.active {
                break;
            }
            let Some(sample) = self.tick(base_hz) else {
                continue;
            };
            out[i * 2] += sample * left;
            out[i * 2 + 1] += sample * right;
            wet[i] += sample * send;
        }
    }

    /// One sample, or nothing during the pause between syllables.
    fn tick(&mut self, base_hz: f32) -> Option<f32> {
        if self.syl >= self.phrase.count {
            self.active = false;
            return None;
        }
        let syl = self.phrase.syllables[self.syl as usize];
        let dur = frames_of(syl.duration_s).max(1);
        let gap = if self.syl + 1 < self.phrase.count {
            frames_of(syl.gap_after_s)
        } else {
            0
        };
        let into = self.syl_pos;
        self.syl_pos += 1;
        if self.syl_pos >= dur + gap {
            self.syl += 1;
            self.syl_pos = 0;
            if self.syl >= self.phrase.count {
                self.active = false;
            }
        }
        if into >= dur {
            return None;
        }

        let dur_s = syl.duration_s.max(1.0 / SR);
        let u = (into as f32 / dur as f32).clamp(0.0, 1.0);
        let t = into as f32 / SR;
        let (env, pitch, noise_mix, harmonic) = self.tone(syl, u, t, dur_s);

        let cents = if self.vibrato_cents > 0.5 {
            self.vibrato_cents
        } else if matches!(self.kind, VoiceKind::Chirp) && dur_s > 0.08 {
            // A long whistle with no written vibrato still breathes a little.
            6.0
        } else {
            0.0
        };
        let mut hz = pitch.clamp(40.0, 16_000.0);
        if cents > 0.0 {
            let rate = if self.vibrato_hz > 0.1 {
                self.vibrato_hz
            } else {
                5.2
            };
            self.vib_phase = (self.vib_phase + rate / SR).fract();
            let vib = (self.vib_phase * TAU).sin() * cents;
            hz *= (vib / 1_200.0 * std::f32::consts::LN_2).exp();
        }
        self.phase = (self.phase + hz / SR).fract();
        let cycle = self.phase * TAU;
        let fundamental = cycle.sin();
        let tone = if harmonic > 0.0 {
            (1.0 - harmonic) * fundamental + harmonic * (cycle * 2.0).sin()
        } else {
            fundamental
        };
        let onset = (1.0 - u) * (1.0 - u);
        let nse = white(&mut self.noise) * noise_mix.clamp(0.0, 1.0) * (0.25 + 0.75 * onset);
        let mixed = tone * (1.0 - noise_mix.clamp(0.0, 0.9)) + nse;
        let cutoff = (base_hz * (0.32 + 0.68 * env)).clamp(160.0, 15_000.0);
        let filtered = lowpass(&mut self.filter, mixed, cutoff);
        let taper = (syl.amplitude / self.level).clamp(0.0, 1.4);
        Some(filtered * env * self.gain * taper)
    }

    fn tone(&self, syl: Syllable, u: f32, t: f32, dur_s: f32) -> (f32, f32, f32, f32) {
        match self.kind {
            VoiceKind::Pulse => {
                let attack = (t / self.attack_s.max(0.001)).min(1.0);
                let release = ((dur_s - t) / 0.01).clamp(0.0, 1.0);
                let env = attack * (-t / 0.09).exp() * release;
                (env, syl.start_hz, syl.noise * 0.25, 0.0)
            }
            VoiceKind::Tick => {
                let release = ((dur_s - t) / 0.004).clamp(0.0, 1.0);
                let env = (-t / 0.012).exp() * release;
                (env, syl.start_hz, syl.noise.max(0.75), 0.08)
            }
            VoiceKind::Chirp | VoiceKind::Trill => {
                let env = envelope(u, self.attack_s, dur_s, self.decay_share);
                let bent = bend(u, self.sweep_curve);
                let pitch = syl.start_hz + (syl.end_hz - syl.start_hz) * bent;
                let pure = matches!(self.kind, VoiceKind::Trill) && syl.noise < 0.05;
                let harmonic = if pure {
                    0.0
                } else {
                    0.16 * self.phrase.brightness.clamp(0.0, 1.0)
                };
                (env, pitch, syl.noise, harmonic)
            }
        }
    }
}

pub(crate) struct Mixer {
    air: Pink,
    water: Pink,
    air_lp: f32,
    /// Glides toward the gust's level so a new gust does not step.
    air_follow: f32,
    air_delay: [f32; 32],
    air_delay_i: usize,
    water_lp: f32,
    water_hp_x: f32,
    water_hp_y: f32,
    insect_hp_x: f32,
    insect_hp_y: f32,
    rain_wait: u32,
    rain_hit: f32,
    rain_pan_l: f32,
    rain_pan_r: f32,
    rain_lp: f32,
    rain_hp_x: f32,
    rain_hp_y: f32,
    rain_tick_x: f32,
    rain_tick_y: f32,
    specks: [Speck; 12],
    drip_wait: u32,
    /// Follow-up drops shaken loose by a drip. `wait == 0` is an empty slot.
    shakes: [Shake; 4],
    tick_wait: u32,
    space: Vec<f32>,
    space_i: usize,
    space_in: f32,
    space_damp: f32,
    dc_x: [f32; 2],
    dc_y: [f32; 2],
    played: u64,
}

impl Mixer {
    pub(crate) fn new() -> Self {
        Self {
            air: Pink::default(),
            water: Pink::default(),
            air_lp: 0.0,
            air_follow: 0.0,
            air_delay: [0.0; 32],
            air_delay_i: 0,
            water_lp: 0.0,
            water_hp_x: 0.0,
            water_hp_y: 0.0,
            insect_hp_x: 0.0,
            insect_hp_y: 0.0,
            rain_wait: 0,
            rain_hit: 0.0,
            rain_pan_l: 0.707,
            rain_pan_r: 0.707,
            rain_lp: 0.0,
            rain_hp_x: 0.0,
            rain_hp_y: 0.0,
            rain_tick_x: 0.0,
            rain_tick_y: 0.0,
            specks: [Speck::default(); 12],
            drip_wait: 0,
            shakes: [Shake::default(); 4],
            tick_wait: 0,
            space: vec![0.0; SPACE_LEN],
            space_i: 0,
            space_in: 0.0,
            space_damp: 0.0,
            dc_x: [0.0; 2],
            dc_y: [0.0; 2],
            played: 0,
        }
    }

    /// Drop ringing audio after a jump in time. The delay line stays put.
    pub(crate) fn quiet(&mut self) {
        self.air = Pink::default();
        self.water = Pink::default();
        self.air_lp = 0.0;
        self.air_follow = 0.0;
        self.air_delay = [0.0; 32];
        self.air_delay_i = 0;
        self.water_lp = 0.0;
        self.water_hp_x = 0.0;
        self.water_hp_y = 0.0;
        self.insect_hp_x = 0.0;
        self.insect_hp_y = 0.0;
        self.rain_wait = 0;
        self.rain_hit = 0.0;
        self.rain_pan_l = 0.707;
        self.rain_pan_r = 0.707;
        self.rain_lp = 0.0;
        self.rain_hp_x = 0.0;
        self.rain_hp_y = 0.0;
        self.rain_tick_x = 0.0;
        self.rain_tick_y = 0.0;
        self.specks = [Speck::default(); 12];
        self.drip_wait = 0;
        self.shakes = [Shake::default(); 4];
        self.tick_wait = 0;
        self.space.fill(0.0);
        self.space_i = 0;
        self.space_in = 0.0;
        self.space_damp = 0.0;
        self.dc_x = [0.0; 2];
        self.dc_y = [0.0; 2];
        self.played = 0;
    }

    pub(crate) fn fill(
        &mut self,
        voices: &mut [Voice],
        rng: &mut Rng,
        canopy: f32,
        gust: f32,
        beds: &BedGains,
        interleaved: &mut [f32],
    ) {
        let frames = interleaved.len() / 2;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            let end = (done + n) * 2;
            self.chunk(
                voices,
                rng,
                canopy,
                gust,
                beds,
                &mut interleaved[done * 2..end],
            );
            done += n;
        }
    }

    fn chunk(
        &mut self,
        voices: &mut [Voice],
        rng: &mut Rng,
        canopy: f32,
        gust: f32,
        beds: &BedGains,
        out: &mut [f32],
    ) {
        let frames = out.len() / 2;
        out.fill(0.0);
        let mut wet = [0.0f32; CHUNK];
        let wet = &mut wet[..frames];
        for voice in voices.iter_mut() {
            if voice.active {
                voice.render(canopy, out, wet);
            }
        }

        let air_target = (0.12 + beds.air.clamp(0.0, 1.5) * 0.30) * wind_breath(gust);
        if self.played == 0 {
            self.air_follow = air_target;
        }
        let air_glide = 1.0 - (-1.0 / (1.4 * SR)).exp();
        let air_hz = 200.0 + (1.0 - canopy.clamp(0.0, 1.0)) * 620.0;
        let air_a = one_pole_a(air_hz);
        let water_amp = beds.water.clamp(0.0, 1.0) * 0.07;
        let water_lp_a = one_pole_a(880.0);
        let water_hp_a = (-TAU * 260.0 / SR).exp();
        let insect_amp = beds.insects.clamp(0.0, 1.0) * 0.045;
        let insect_hp_a = (-TAU * 5_500.0 / SR).exp();
        let gust_wobble = ((gust - 0.45).clamp(-1.0, 1.0) * 36.0).round() as isize;

        for i in 0..frames {
            if poisson_due(&mut self.drip_wait, rng, beds.drip_hz) {
                self.leaf_drip(rng, None, None, true);
            }
            self.shake_drips(rng);
            if poisson_due(&mut self.tick_wait, rng, beds.tick_hz) {
                spawn_tick(&mut self.specks[8..], rng);
            }

            let mut speck_l = 0.0;
            let mut speck_r = 0.0;
            let mut speck_w = 0.0;
            for speck in &mut self.specks {
                if speck.env <= 0.0 && speck.click <= 0.0 {
                    continue;
                }
                let s = speck_sample(speck);
                speck_l += s * speck.pan_l;
                speck_r += s * speck.pan_r;
                speck_w += s * speck.wet;
            }

            self.air_follow += air_glide * (air_target - self.air_follow);
            let air_amp = self.air_follow;
            let air_white = rng.f32() * 2.0 - 1.0;
            let pink = self.air.next(air_white);
            self.air_lp += air_a * (pink - self.air_lp);
            self.air_lp = flush(self.air_lp);
            let air_now = self.air_lp;
            let air_l = air_now * air_amp;
            let air_r = self.delay_air(air_now) * air_amp;

            let water_white = rng.f32() * 2.0 - 1.0;
            let water_pink = self.water.next(water_white);
            self.water_lp += water_lp_a * (water_pink - self.water_lp);
            self.water_lp = flush(self.water_lp);
            let water_y = highpass(
                &mut self.water_hp_x,
                &mut self.water_hp_y,
                self.water_lp,
                water_hp_a,
            );
            let water = water_y * water_amp;

            let insect_white = rng.f32() * 2.0 - 1.0;
            let insect = highpass(
                &mut self.insect_hp_x,
                &mut self.insect_hp_y,
                insect_white,
                insect_hp_a,
            ) * insect_amp;

            let (rain_l, rain_r) = self.rain(rng, canopy, beds.shower);

            let room = self.reflect(wet[i] + speck_w, gust_wobble);
            let mut left = out[i * 2] + speck_l + air_l + water + insect + rain_l + room;
            let mut right =
                out[i * 2 + 1] + speck_r + air_r + water * 0.92 + insect + rain_r + room;

            let fade = if self.played < FADE_SAMPLES {
                self.played as f32 / FADE_SAMPLES as f32
            } else {
                1.0
            };
            self.played = self.played.saturating_add(1);
            left = dc(&mut self.dc_x[0], &mut self.dc_y[0], left) * fade * MASTER;
            right = dc(&mut self.dc_x[1], &mut self.dc_y[1], right) * fade * MASTER;
            out[i * 2] = soft(left);
            out[i * 2 + 1] = soft(right);
        }
    }

    /// Rain on the leaves. A light shower keeps a few drops separable. A harder
    /// one overlaps them into a rush, and a closed canopy dulls it.
    fn rain(&mut self, rng: &mut Rng, canopy: f32, shower: f32) -> (f32, f32) {
        let shower = shower.clamp(0.0, 1.0);
        if shower <= 0.001 {
            self.rain_hit *= (-1.0 / (0.02 * SR)).exp();
            if self.rain_hit < 1.0e-4 {
                self.rain_hit = 0.0;
                self.rain_wait = 0;
            }
            return (0.0, 0.0);
        }
        let bright = 1.0 - canopy.clamp(0.0, 1.0) * 0.45;
        let rate = shower * (22.0 + shower * 90.0);
        if poisson_due(&mut self.rain_wait, rng, rate) {
            self.rain_hit = (self.rain_hit + 0.45 + 0.55 * rng.f32()).min(1.6);
            let pan = rng.gen_range(-0.85, 0.85);
            self.rain_pan_l = ((1.0 - pan) * 0.5).sqrt();
            self.rain_pan_r = ((1.0 + pan) * 0.5).sqrt();
        }
        self.rain_hit *= (-1.0 / (0.016 * SR)).exp();

        let n = rng.f32() * 2.0 - 1.0;
        let body_a = one_pole_a(800.0 + bright * 2_400.0);
        self.rain_lp += body_a * (n - self.rain_lp);
        self.rain_lp = flush(self.rain_lp);
        let body = highpass(
            &mut self.rain_hp_x,
            &mut self.rain_hp_y,
            self.rain_lp,
            (-TAU * (320.0 + bright * 220.0) / SR).exp(),
        );
        let tick = highpass(
            &mut self.rain_tick_x,
            &mut self.rain_tick_y,
            n,
            (-TAU * (2_200.0 + bright * 2_400.0) / SR).exp(),
        );
        let steady = body * shower * 0.34;
        let drops = tick * self.rain_hit * shower * 0.22;
        (
            steady + drops * self.rain_pan_l,
            steady * 0.92 + drops * self.rain_pan_r,
        )
    }

    /// A fraction of a millisecond later, so the air is not a point in the middle.
    fn delay_air(&mut self, sample: f32) -> f32 {
        let i = self.air_delay_i;
        let delayed = self.air_delay[i];
        self.air_delay[i] = sample;
        self.air_delay_i = (i + 1) % self.air_delay.len();
        delayed
    }

    fn reflect(&mut self, wet: f32, wobble: isize) -> f32 {
        let len = self.space.len();
        let w = self.space_i;
        let mut early = 0.0;
        for (tap, gain) in EARLY.iter().zip(EARLY_GAIN) {
            let at = delay_index(w, *tap as isize + wobble, len);
            early += self.space[at] * gain;
        }
        let tail = self.space[delay_index(w, TAIL as isize + wobble, len)];
        self.space_damp += 0.28 * (tail - self.space_damp);
        self.space_damp = flush(self.space_damp);
        self.space_in += 0.42 * (wet - self.space_in);
        self.space_in = flush(self.space_in);
        self.space[w] = self.space_in + self.space_damp * TAIL_FEEDBACK;
        self.space_i = (w + 1) % len;
        (early + self.space_damp * 0.18) * 0.55
    }

    /// One leaf drip, and sometimes a softer drop or two shaken loose beside it.
    /// A shaken drop does not shake another, so a cluster stays a cluster.
    fn leaf_drip(&mut self, rng: &mut Rng, pan: Option<f32>, amp: Option<f32>, shake: bool) {
        let pan = pan
            .unwrap_or_else(|| rng.gen_range(-0.9, 0.9))
            .clamp(-0.95, 0.95);
        // Most drops are small. A few are close enough to notice on their own.
        let amp = amp.unwrap_or_else(|| 0.07 + 0.15 * rng.f32().powi(2));
        spawn_drip(&mut self.specks[..8], rng, pan, amp);
        if shake && rng.f32() < 0.42 {
            let amp2 = amp * rng.gen_range(0.40, 0.75);
            self.schedule_shake(rng, pan, amp2, 0.040, 0.130);
            if rng.f32() < 0.35 {
                let amp3 = amp * rng.gen_range(0.28, 0.55);
                self.schedule_shake(rng, pan, amp3, 0.075, 0.190);
            }
        }
    }

    fn schedule_shake(&mut self, rng: &mut Rng, pan: f32, amp: f32, min_s: f32, max_s: f32) {
        let Some(slot) = self.shakes.iter().position(|shake| shake.wait == 0) else {
            return;
        };
        let pan = (pan + rng.gen_range(-0.22, 0.22)).clamp(-0.95, 0.95);
        self.shakes[slot] = Shake {
            wait: frames_of(rng.gen_range(min_s, max_s)).max(1),
            pan,
            amp,
        };
    }

    fn shake_drips(&mut self, rng: &mut Rng) {
        for i in 0..self.shakes.len() {
            if self.shakes[i].wait == 0 {
                continue;
            }
            self.shakes[i].wait -= 1;
            if self.shakes[i].wait == 0 {
                let pan = self.shakes[i].pan;
                let amp = self.shakes[i].amp;
                self.leaf_drip(rng, Some(pan), Some(amp), false);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Speck {
    env: f32,
    decay: f32,
    phase: f32,
    inc: f32,
    pan_l: f32,
    pan_r: f32,
    noise: u32,
    noise_mix: f32,
    amp: f32,
    wet: f32,
    z: f32,
    pole: f32,
    /// Bright tick at the attack. Zero for a branch tick, which has no separate click.
    click: f32,
    click_decay: f32,
    hp_x: f32,
    hp_y: f32,
    hp_a: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Shake {
    wait: u32,
    pan: f32,
    amp: f32,
}

fn quietest(specks: &[Speck]) -> usize {
    specks
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.env
                .partial_cmp(&b.env)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn spawn_tick(specks: &mut [Speck], rng: &mut Rng) {
    let slot = quietest(specks);
    let pan = rng.gen_range(-0.85, 0.85);
    let hz = rng.gen_range(650.0, 1_700.0);
    specks[slot] = Speck {
        env: 1.0,
        decay: exp_decay(0.011),
        phase: rng.f32(),
        inc: hz / SR,
        pan_l: ((1.0 - pan) * 0.5).sqrt(),
        pan_r: ((1.0 + pan) * 0.5).sqrt(),
        noise: hash_noise(slot as u8, hz.to_bits(), rng.next_u64() as u32),
        noise_mix: 0.82,
        amp: 0.32,
        wet: 0.16,
        z: 0.0,
        pole: one_pole_a(3_400.0),
        click: 0.0,
        click_decay: 0.0,
        hp_x: 0.0,
        hp_y: 0.0,
        hp_a: 0.0,
    };
}

/// A leaf drip is a bright noise tick plus a dull, short body. There is no
/// held pitch, and the color changes from drop to drop.
fn spawn_drip(specks: &mut [Speck], rng: &mut Rng, pan: f32, amp: f32) {
    let slot = quietest(specks);
    let body_hz = rng.gen_range(420.0, 1_500.0);
    let click_hz = rng.gen_range(2_200.0, 5_200.0);
    let pan = pan.clamp(-1.0, 1.0);
    specks[slot] = Speck {
        env: rng.gen_range(0.28, 0.55),
        decay: exp_decay(rng.gen_range(0.008, 0.016)),
        phase: 0.0,
        inc: 0.0,
        pan_l: ((1.0 - pan) * 0.5).sqrt(),
        pan_r: ((1.0 + pan) * 0.5).sqrt(),
        noise: hash_noise(slot as u8, body_hz.to_bits(), rng.next_u64() as u32),
        noise_mix: 1.0,
        amp,
        wet: rng.gen_range(0.08, 0.20),
        z: 0.0,
        pole: one_pole_a(body_hz),
        click: rng.gen_range(0.55, 0.95),
        click_decay: exp_decay(rng.gen_range(0.0022, 0.0055)),
        hp_x: 0.0,
        hp_y: 0.0,
        hp_a: (-TAU * click_hz / SR).exp(),
    };
}

fn speck_sample(speck: &mut Speck) -> f32 {
    let n = white(&mut speck.noise);
    let tone = (speck.phase * TAU).sin();
    speck.phase = (speck.phase + speck.inc).fract();
    let x = tone * (1.0 - speck.noise_mix) + n * speck.noise_mix;
    let y = speck.z + speck.pole * (x - speck.z);
    speck.z = flush(y);
    let mut s = y * speck.env;
    if speck.click > 0.0 {
        let hp = highpass(&mut speck.hp_x, &mut speck.hp_y, n, speck.hp_a);
        s += hp * speck.click;
        speck.click *= speck.click_decay;
        if speck.click < 1.0e-4 {
            speck.click = 0.0;
        }
    }
    speck.env *= speck.decay;
    if speck.env < 1.0e-4 {
        speck.env = 0.0;
    }
    s * speck.amp
}

/// How the wind is breathing. A gust at rest (0.45) leaves the level alone.
fn wind_breath(gust: f32) -> f32 {
    let centered = (gust.clamp(0.0, 1.0) - 0.45) * 1.6;
    (1.0 + centered).clamp(0.4, 1.85)
}

fn exp_decay(tau: f32) -> f32 {
    (-1.0 / (tau.max(1.0 / SR) * SR)).exp()
}

fn poisson_due(wait: &mut u32, rng: &mut Rng, rate_hz: f32) -> bool {
    if rate_hz <= 0.001 {
        *wait = 0;
        return false;
    }
    if *wait == 0 {
        let u = rng.f32().clamp(1.0e-5, 1.0);
        let samples = (-u.ln() / rate_hz * SR).clamp(1.0, SR * 3_600.0);
        *wait = samples as u32;
    }
    *wait -= 1;
    *wait == 0
}

#[derive(Clone, Copy, Debug, Default)]
struct Pink {
    b0: f32,
    b1: f32,
    b2: f32,
}

impl Pink {
    fn next(&mut self, white: f32) -> f32 {
        // Paul Kellet's economy pink filter.
        self.b0 = 0.99765 * self.b0 + white * 0.0990460;
        self.b1 = 0.96300 * self.b1 + white * 0.2965164;
        self.b2 = 0.57000 * self.b2 + white * 1.0526913;
        // The economy filter sums well above unity. Bring it back to a soft noise.
        flush((self.b0 + self.b1 + self.b2 + white * 0.1848) * 0.11)
    }
}

fn envelope(u: f32, attack_s: f32, duration_s: f32, decay_share: f32) -> f32 {
    let attack = (attack_s / duration_s).clamp(0.002, 0.45);
    let decay = decay_share.clamp(0.08, 0.92);
    let release = (1.0 - decay).max(attack + 0.02).min(0.98);
    if u < attack {
        u / attack
    } else if u < release {
        1.0
    } else {
        let t = (u - release) / (1.0 - release);
        (1.0 - t).clamp(0.0, 1.0)
    }
}

fn bend(u: f32, curve: f32) -> f32 {
    if (curve - 1.0).abs() < 0.04 {
        u
    } else {
        u.powf(curve)
    }
}

fn lowpass(z: &mut f32, x: f32, cutoff: f32) -> f32 {
    let a = one_pole_a(cutoff);
    *z += a * (x - *z);
    *z = flush(*z);
    *z
}

fn one_pole_a(cutoff: f32) -> f32 {
    (1.0 - (-TAU * cutoff / SR).exp()).clamp(0.0, 1.0)
}

fn highpass(x_prev: &mut f32, y_prev: &mut f32, x: f32, a: f32) -> f32 {
    let y = a * (*y_prev + x - *x_prev);
    *x_prev = x;
    *y_prev = flush(y);
    y
}

fn dc(x_prev: &mut f32, y_prev: &mut f32, x: f32) -> f32 {
    let y = x - *x_prev + 0.995 * *y_prev;
    *x_prev = x;
    *y_prev = flush(y);
    y
}

fn soft(x: f32) -> f32 {
    x.clamp(-3.0, 3.0).tanh()
}

fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}

fn white(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    if x == 0 {
        x = 0x1234_5678;
    }
    *state = x;
    ((x >> 8) as f32 / 16_777_216.0) * 2.0 - 1.0
}

fn hash_noise(slot: u8, hz_bits: u32, extra: u32) -> u32 {
    let mut x = 0xA5A5_1234u32
        ^ u32::from(slot).wrapping_mul(0x9E37_79B9)
        ^ hz_bits.rotate_left(7)
        ^ extra.wrapping_mul(0x85EB_CA6B);
    if x == 0 {
        x = 1;
    }
    x
}

fn frames_of(seconds: f32) -> u32 {
    (seconds.max(0.0) * SR).round() as u32
}

fn delay_index(write: usize, delay: isize, len: usize) -> usize {
    let d = delay.clamp(1, (len as isize) - 1) as usize;
    (write + len - d) % len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_drip(seed: u64) -> (Speck, Vec<f32>) {
        let mut specks = [Speck::default(); 8];
        let mut rng = Rng::new(seed);
        spawn_drip(&mut specks, &mut rng, 0.2, 0.12);
        let slot = specks.iter().position(|speck| speck.amp > 0.0).unwrap();
        let speck = specks[slot];
        let mut live = specks[slot];
        let n = (0.12 * SR) as usize;
        let mut samples = Vec::with_capacity(n);
        for _ in 0..n {
            samples.push(speck_sample(&mut live));
        }
        (speck, samples)
    }

    #[test]
    fn a_leaf_drip_is_a_short_unpitched_tick() {
        let mut poles = Vec::new();
        let mut clicks = Vec::new();
        let hp_lo = (-TAU * 5_200.0 / SR).exp();
        let hp_hi = (-TAU * 2_200.0 / SR).exp();
        for seed in 1..17 {
            let (speck, samples) = one_drip(seed);
            assert_eq!(speck.noise_mix, 1.0, "a drip holds no tone");
            assert_eq!(speck.inc, 0.0);
            assert!(speck.click > 0.0);
            assert!(speck.pole + 1.0e-5 >= one_pole_a(420.0));
            assert!(speck.pole <= one_pole_a(1_500.0) + 1.0e-5);
            assert!((hp_lo..=hp_hi).contains(&speck.hp_a));
            poles.push(speck.pole.to_bits());
            clicks.push(speck.hp_a.to_bits());

            let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            assert!(peak > 0.01, "silent drip from seed {seed}");
            let peak_at = samples
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
                .unwrap()
                .0;
            assert!(
                peak_at < (0.012 * SR) as usize,
                "seed {seed} peaks late, at sample {peak_at}"
            );
            let last = samples.iter().rposition(|s| s.abs() > peak * 0.08).unwrap();
            assert!(
                last < (0.060 * SR) as usize,
                "seed {seed} still sounding at sample {last}"
            );
            let early: f32 = samples[..(0.020 * SR) as usize].iter().map(|s| s * s).sum();
            let late: f32 = samples[(0.070 * SR) as usize..].iter().map(|s| s * s).sum();
            assert!(
                late < early * 0.01,
                "seed {seed} rings on: tail {late} early {early}"
            );
        }
        let bodies = poles.iter().collect::<std::collections::HashSet<_>>().len();
        let ticks = clicks
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert!(bodies > 6, "bodies barely vary: {bodies}");
        assert!(ticks > 6, "ticks barely vary: {ticks}");
    }

    #[test]
    fn a_gust_changes_the_wind_without_changing_its_color() {
        assert!((wind_breath(0.45) - 1.0).abs() < 1.0e-4);
        let lull = air_rms(0.12);
        let rest = air_rms(0.45);
        let blow = air_rms(0.82);
        assert!(
            lull < rest * 0.75,
            "a lull is still the usual wind: {lull} vs {rest}"
        );
        assert!(
            blow > rest * 1.35,
            "a blow is still the usual wind: {blow} vs {rest}"
        );
        assert!(
            blow < rest * 2.2,
            "a blow became another sound: {blow} vs {rest}"
        );
    }

    fn air_rms(gust: f32) -> f32 {
        let mut mixer = Mixer::new();
        let mut voices = [Voice::default(); 1];
        let mut rng = Rng::new(1);
        let beds = BedGains {
            air: 0.11,
            ..BedGains::default()
        };
        let mut buf = [0.0f32; 2];
        let skip = (0.4 * SR) as usize;
        let keep = (0.4 * SR) as usize;
        for _ in 0..skip {
            mixer.chunk(&mut voices, &mut rng, 0.5, gust, &beds, &mut buf);
        }
        let mut sum = 0.0f32;
        for _ in 0..keep {
            mixer.chunk(&mut voices, &mut rng, 0.5, gust, &beds, &mut buf);
            sum += buf[0] * buf[0] + buf[1] * buf[1];
        }
        (sum / (keep * 2) as f32).sqrt()
    }

    #[test]
    fn some_drips_shake_a_neighbor_loose() {
        let mut mixer = Mixer::new();
        let mut rng = Rng::new(7);
        let mut voices = [Voice::default(); 1];
        let beds = BedGains {
            drip_hz: 3.0,
            ..BedGains::default()
        };
        let mut prev = [0.0f32; 8];
        let mut onsets = Vec::new();
        let mut buf = [0.0f32; 2];
        let frames = 8 * SR as usize;
        for t in 0..frames {
            mixer.chunk(&mut voices, &mut rng, 0.5, 0.4, &beds, &mut buf);
            for (s, speck) in mixer.specks[..8].iter().enumerate() {
                if speck.env > prev[s] + 0.05 {
                    onsets.push(t);
                }
                prev[s] = speck.env;
            }
        }
        let mut gaps = Vec::new();
        for pair in onsets.windows(2) {
            let gap = pair[1] - pair[0];
            if gap > 0 {
                gaps.push(gap);
            }
        }
        let short = gaps
            .iter()
            .filter(|gap| (0.025 * SR) as usize <= **gap && **gap <= (0.200 * SR) as usize)
            .count();
        let wide = gaps
            .iter()
            .filter(|gap| **gap > (0.250 * SR) as usize)
            .count();
        assert!(onsets.len() > 20, "too few drips: {}", onsets.len());
        assert!(short >= 3, "no shaken drops among {gaps:?}");
        assert!(wide >= 3, "the drips lost their long gaps among {gaps:?}");
    }
}
