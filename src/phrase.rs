use crate::model::{Species, MAX_SYLLABLES};
use crate::rng::Rng;

/// One note of a phrase. The synth plays it once and then drops it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Syllable {
    pub start_hz: f32,
    pub end_hz: f32,
    pub duration_s: f32,
    pub amplitude: f32,
    pub noise: f32,
    pub gap_after_s: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Phrase {
    pub syllables: [Syllable; MAX_SYLLABLES],
    pub count: u8,
    pub pan: f32,
    pub distance: f32,
    pub brightness: f32,
}

impl Phrase {
    pub fn duration_s(&self) -> f32 {
        let mut t = 0.0;
        for i in 0..self.count as usize {
            t += self.syllables[i].duration_s;
            if (i as u8) + 1 < self.count {
                t += self.syllables[i].gap_after_s;
            }
        }
        t
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Singer {
    pub pitch_ratio: f32,
    pub tempo_ratio: f32,
    pub contour_offset: f32,
    pub brightness_offset: f32,
    pub pan: f32,
    pub distance: f32,
}

pub fn compose(
    rng: &mut Rng,
    species: &Species,
    singer: Singer,
    reply_hint: Option<f32>,
) -> Phrase {
    let mut center = species.pitch_hz * singer.pitch_ratio * (1.0 + rng.gaussian() * 0.012);
    if let Some(hint) = reply_hint {
        if hint > 1.0 && center > 1.0 {
            center = (center.ln() * 0.6 + hint.ln() * 0.4).exp();
        }
    }

    let mut count = rng.gen_range_u8(species.syllables_min, species.syllables_max);
    if reply_hint.is_some() {
        count = match species.shape {
            crate::model::PhraseShape::Paired => count.max(2),
            crate::model::PhraseShape::Slur => 1,
            crate::model::PhraseShape::Motif => (count / 2).max(2).min(count).max(1),
            _ => (count / 2).max(1),
        };
    }
    count = count.clamp(1, MAX_SYLLABLES as u8);

    let tempo = singer.tempo_ratio.clamp(0.75, 1.3);
    // The species accent stays recognizable. Each performance leans a little
    // further or a little less, so the gesture is not a copy.
    let contour_scale = 1.0 + rng.gaussian() * 0.22;
    let mut syllables = [Syllable {
        start_hz: 0.0,
        end_hz: 0.0,
        duration_s: 0.0,
        amplitude: 0.0,
        noise: 0.0,
        gap_after_s: 0.0,
    }; MAX_SYLLABLES];

    for (i, syllable) in syllables.iter_mut().enumerate().take(count as usize) {
        let hz = syllable_hz(
            rng,
            species,
            center,
            count,
            i as u8,
            singer.contour_offset,
            contour_scale,
        );
        let sweep = if matches!(species.shape, crate::model::PhraseShape::Slur) {
            let wobble = ratio_from_cents(rng.gaussian() * 80.0);
            (species.sweep_ratio * wobble).clamp(0.5, 1.7)
        } else {
            let wobble = ratio_from_cents(rng.gaussian() * 18.0);
            (1.0 + (species.sweep_ratio - 1.0) * 0.35) * wobble
        };
        let dur = (rng.log_normal(species.syllable_median_s, species.syllable_sigma) / tempo)
            .clamp(0.012, 0.8);
        let t = if count == 1 {
            0.0
        } else {
            i as f32 / (count - 1) as f32
        };
        let gap = if (i as u8) + 1 == count {
            0.0
        } else {
            rng.log_normal(species.syllable_gap_median, species.syllable_gap_sigma)
                .clamp(0.012, 0.35)
                / tempo
        };
        *syllable = Syllable {
            start_hz: hz,
            end_hz: hz * sweep,
            duration_s: dur,
            amplitude: species.level * (1.0 - 0.12 * t),
            noise: species.noise_attack,
            gap_after_s: gap,
        };
    }

    Phrase {
        syllables,
        count,
        pan: singer.pan,
        distance: singer.distance,
        brightness: (species.brightness + singer.brightness_offset).clamp(0.0, 1.0),
    }
}

fn syllable_hz(
    rng: &mut Rng,
    species: &Species,
    center: f32,
    count: u8,
    index: u8,
    contour_offset: f32,
    contour_scale: f32,
) -> f32 {
    let cents = match species.shape {
        crate::model::PhraseShape::Slur => rng.gaussian() * species.step_scatter.max(20.0),
        crate::model::PhraseShape::Repeated => rng.gaussian() * species.step_scatter,
        crate::model::PhraseShape::Paired => {
            if index == 0 {
                rng.gaussian() * 8.0
            } else {
                species.step_cents * contour_scale + rng.gaussian() * species.step_scatter
            }
        }
        crate::model::PhraseShape::Motif => {
            let t = if count <= 1 {
                0.5
            } else {
                index as f32 / (count - 1) as f32
            };
            let mid = (count - 1) as f32 / 2.0;
            species.contour_cents * contour_scale * t
                + species.step_cents * (index as f32 - mid) * 0.15
                + rng.gaussian() * species.step_scatter
                + contour_offset * t
        }
    };
    center * ratio_from_cents(cents)
}

pub fn contour_cents(phrase: &Phrase) -> i16 {
    if phrase.count == 0 {
        return 0;
    }
    let a = phrase.syllables[0].start_hz;
    let b = if phrase.count == 1 {
        phrase.syllables[0].end_hz
    } else {
        phrase.syllables[phrase.count as usize - 1].start_hz
    };
    cents_between(a, b).round() as i16
}

pub fn signature(phrase: &Phrase, species: u8) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    h = mix(h, u64::from(species));
    h = mix(h, u64::from(phrase.count));
    for i in 0..phrase.count as usize {
        let start = phrase.syllables[i].start_hz.round() as u64;
        let end = phrase.syllables[i].end_hz.round() as u64;
        let dur = (phrase.syllables[i].duration_s * 200.0).round() as u64;
        let gap = (phrase.syllables[i].gap_after_s * 200.0).round() as u64;
        h = mix(h, start);
        h = mix(h, end);
        h = mix(h, dur);
        h = mix(h, gap);
    }
    h
}

fn mix(h: u64, v: u64) -> u64 {
    h ^ v.wrapping_mul(0x100_0000_01b3)
}

pub fn cents_between(a: f32, b: f32) -> f32 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    1200.0 * (b / a).ln() / std::f32::consts::LN_2
}

pub fn ratio_from_cents(cents: f32) -> f32 {
    (cents / 1200.0 * std::f32::consts::LN_2).exp()
}

/// A sampled silence. Clamped so a single draw cannot open a hole of many minutes
/// or a gap short enough to become a trill by accident. The cap is still long
/// enough that a singer can go quiet for a while.
pub fn gap(rng: &mut Rng, median: f32, sigma: f32, min_s: f32, max_s: f32) -> f32 {
    rng.log_normal(median.max(min_s), sigma).clamp(min_s, max_s)
}
