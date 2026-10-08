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
    air_delay: [f32; 32],
    air_delay_i: usize,
    water_lp: f32,
    water_hp_x: f32,
    water_hp_y: f32,
    insect_hp_x: f32,
    insect_hp_y: f32,
    specks: [Speck; 12],
    drip_wait: u32,
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
            air_delay: [0.0; 32],
            air_delay_i: 0,
            water_lp: 0.0,
            water_hp_x: 0.0,
            water_hp_y: 0.0,
            insect_hp_x: 0.0,
            insect_hp_y: 0.0,
            specks: [Speck::default(); 12],
            drip_wait: 0,
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
        self.air_delay = [0.0; 32];
        self.air_delay_i = 0;
        self.water_lp = 0.0;
        self.water_hp_x = 0.0;
        self.water_hp_y = 0.0;
        self.insect_hp_x = 0.0;
        self.insect_hp_y = 0.0;
        self.specks = [Speck::default(); 12];
        self.drip_wait = 0;
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

        let air_amp = 0.12 + beds.air.clamp(0.0, 1.5) * 0.30;
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
                spawn(&mut self.specks[..8], rng, SpeckKind::Drip);
            }
            if poisson_due(&mut self.tick_wait, rng, beds.tick_hz) {
                spawn(&mut self.specks[8..], rng, SpeckKind::Tick);
            }

            let mut speck_l = 0.0;
            let mut speck_r = 0.0;
            let mut speck_w = 0.0;
            for speck in &mut self.specks {
                if speck.env <= 0.0 {
                    continue;
                }
                let n = white(&mut speck.noise);
                let tone = (speck.phase * TAU).sin();
                speck.phase = (speck.phase + speck.inc).fract();
                let x = tone * (1.0 - speck.noise_mix) + n * speck.noise_mix;
                let y = speck.z + speck.pole * (x - speck.z);
                speck.z = flush(y);
                let s = y * speck.env * speck.amp;
                speck.env *= speck.decay;
                if speck.env < 1.0e-4 {
                    speck.env = 0.0;
                }
                speck_l += s * speck.pan_l;
                speck_r += s * speck.pan_r;
                speck_w += s * speck.wet;
            }

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

            let room = self.reflect(wet[i] + speck_w, gust_wobble);
            let mut left = out[i * 2] + speck_l + air_l + water + insect + room;
            let mut right = out[i * 2 + 1] + speck_r + air_r + water * 0.92 + insect + room;

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
}

#[derive(Clone, Copy)]
enum SpeckKind {
    Drip,
    Tick,
}

fn spawn(specks: &mut [Speck], rng: &mut Rng, kind: SpeckKind) {
    let slot = specks
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.env
                .partial_cmp(&b.env)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let pan = rng.gen_range(-0.85, 0.85);
    let (hz, tau, noise_mix, amp, wet, pole_hz) = match kind {
        SpeckKind::Drip => (
            rng.gen_range(1_400.0, 4_200.0),
            0.042,
            0.22,
            0.20,
            0.28,
            4_800.0,
        ),
        SpeckKind::Tick => (
            rng.gen_range(650.0, 1_700.0),
            0.011,
            0.82,
            0.32,
            0.16,
            3_400.0,
        ),
    };
    specks[slot] = Speck {
        env: 1.0,
        decay: (-1.0 / (tau * SR)).exp(),
        phase: rng.f32(),
        inc: hz / SR,
        pan_l: ((1.0 - pan) * 0.5).sqrt(),
        pan_r: ((1.0 + pan) * 0.5).sqrt(),
        noise: hash_noise(slot as u8, hz.to_bits(), rng.next_u64() as u32),
        noise_mix,
        amp,
        wet,
        z: 0.0,
        pole: one_pole_a(pole_hz),
    };
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
