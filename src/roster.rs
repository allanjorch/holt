use crate::model::{humps, ActivityPattern, PhraseShape, Preset, Species, VoiceKind, MAX_SPECIES};
use crate::rng::Rng;

pub fn build(preset: Preset, rng: &mut Rng, out: &mut [Species; MAX_SPECIES]) -> u8 {
    let mut n = 0u8;
    let plan = plan(preset, rng);
    let mut push = |species: Species| {
        if (n as usize) < MAX_SPECIES {
            out[n as usize] = species;
            n += 1;
        }
    };

    for _ in 0..plan.dawn_song {
        push(song_dawn(rng));
    }
    for _ in 0..plan.day_song {
        push(song_day(rng));
    }
    for _ in 0..plan.edge_song {
        push(song_edge(rng));
    }
    for _ in 0..plan.cricket {
        push(cricket(rng));
    }
    for _ in 0..plan.insect {
        push(insect(rng));
    }
    for band in plan.frogs {
        push(frog(
            rng,
            *band,
            plan.frog_tolerance,
            plan.frog_capacity,
            plan.frog_pattern,
        ));
    }
    for _ in 0..plan.tap {
        push(tap(rng));
    }

    separate(&mut out[..n as usize]);
    n
}

struct Plan {
    dawn_song: u8,
    day_song: u8,
    edge_song: u8,
    cricket: u8,
    insect: u8,
    frogs: &'static [(f32, f32)],
    frog_tolerance: f32,
    frog_capacity: u8,
    frog_pattern: ActivityPattern,
    tap: u8,
}

fn plan(preset: Preset, rng: &mut Rng) -> Plan {
    match preset {
        Preset::Broadleaf => Plan {
            dawn_song: 1,
            day_song: 1,
            edge_song: if rng.f32() < 0.55 { 1 } else { 0 },
            cricket: 1,
            insect: 0,
            frogs: &[(220.0, 480.0)],
            frog_tolerance: 0.12,
            frog_capacity: 2,
            frog_pattern: ActivityPattern::Crepuscular,
            tap: 1,
        },
        Preset::Wetwood => Plan {
            dawn_song: 1,
            day_song: 0,
            edge_song: 0,
            cricket: 2,
            insect: 0,
            frogs: &[(200.0, 340.0), (380.0, 620.0)],
            frog_tolerance: 0.22,
            frog_capacity: 3,
            frog_pattern: ActivityPattern::Nocturnal,
            tap: 0,
        },
        Preset::Dryridge => Plan {
            dawn_song: 1,
            day_song: 0,
            edge_song: 1,
            cricket: 0,
            insect: 1,
            frogs: &[],
            frog_tolerance: 0.1,
            frog_capacity: 1,
            frog_pattern: ActivityPattern::Dusk,
            tap: 1,
        },
        Preset::Nightmarsh => Plan {
            dawn_song: 0,
            day_song: 0,
            edge_song: 0,
            cricket: 2,
            insect: 0,
            frogs: &[(180.0, 320.0), (360.0, 640.0)],
            frog_tolerance: 0.28,
            frog_capacity: 3,
            frog_pattern: ActivityPattern::Nocturnal,
            tap: 0,
        },
        Preset::Clearing => Plan {
            dawn_song: 0,
            day_song: 0,
            edge_song: 1,
            cricket: 0,
            insect: 1,
            frogs: &[],
            frog_tolerance: 0.1,
            frog_capacity: 1,
            frog_pattern: ActivityPattern::Dusk,
            tap: if rng.f32() < 0.5 { 1 } else { 0 },
        },
    }
}

fn between(rng: &mut Rng, a: f32, b: f32) -> f32 {
    rng.gen_range(a, b)
}

fn song_dawn(rng: &mut Rng) -> Species {
    let pattern = ActivityPattern::Dawn;
    Species {
        kind: VoiceKind::Chirp,
        shape: PhraseShape::Motif,
        pattern,
        moisture_center: between(rng, 0.38, 0.55),
        moisture_tolerance: 0.36,
        canopy_center: between(rng, 0.32, 0.58),
        canopy_tolerance: 0.42,
        activity: humps(pattern),
        pitch_hz: between(rng, 3000.0, 4200.0),
        pitch_scatter: 0.025,
        syllable_median_s: 0.085,
        syllable_sigma: 0.22,
        syllables_min: 3,
        syllables_max: 6,
        phrases_min: 1,
        phrases_max: 1,
        syllable_gap_median: 0.05,
        syllable_gap_sigma: 0.28,
        phrase_gap_median: 0.2,
        phrase_gap_sigma: 0.2,
        bout_gap_median: between(rng, 16.0, 34.0),
        bout_gap_sigma: 0.62,
        step_cents: between(rng, -80.0, -25.0),
        step_scatter: 28.0,
        contour_cents: between(rng, -240.0, -90.0),
        attack_s: 0.012,
        decay_share: 0.62,
        noise_attack: 0.12,
        sweep_ratio: 0.97,
        sweep_curve: 1.0,
        brightness: 0.84,
        level: 0.7,
        distance_median: 0.46,
        replies: between(rng, 0.28, 0.48),
        reply_window_median: 0.75,
        capacity: rng.gen_range_u8(2, 3),
        birth_per_hour: 0.45,
        death_per_hour: 0.7,
        wind_shy: 0.75,
        territorial: 0.72,
        ..Species::default()
    }
}

fn song_day(rng: &mut Rng) -> Species {
    let pattern = ActivityPattern::Diurnal;
    let rising = rng.f32() < 0.22;
    Species {
        kind: VoiceKind::Chirp,
        shape: PhraseShape::Slur,
        pattern,
        moisture_center: between(rng, 0.40, 0.62),
        moisture_tolerance: 0.34,
        canopy_center: between(rng, 0.40, 0.62),
        canopy_tolerance: 0.35,
        activity: humps(pattern),
        pitch_hz: between(rng, 1400.0, 2300.0),
        pitch_scatter: 0.03,
        syllable_median_s: 0.38,
        syllable_sigma: 0.2,
        syllables_min: 1,
        syllables_max: 1,
        phrases_min: 1,
        phrases_max: 2,
        syllable_gap_median: 0.08,
        syllable_gap_sigma: 0.2,
        phrase_gap_median: 0.55,
        phrase_gap_sigma: 0.3,
        bout_gap_median: between(rng, 22.0, 48.0),
        bout_gap_sigma: 0.6,
        step_cents: 0.0,
        step_scatter: 10.0,
        contour_cents: if rising { 140.0 } else { -160.0 },
        attack_s: 0.04,
        decay_share: 0.55,
        noise_attack: 0.05,
        sweep_ratio: if rising {
            between(rng, 1.08, 1.22)
        } else {
            between(rng, 0.72, 0.9)
        },
        sweep_curve: between(rng, 0.7, 1.3),
        brightness: 0.55,
        level: 0.78,
        distance_median: 0.32,
        replies: between(rng, 0.08, 0.2),
        reply_window_median: 0.9,
        capacity: 2,
        birth_per_hour: 0.35,
        death_per_hour: 0.6,
        wind_shy: 0.62,
        territorial: 0.4,
        ..Species::default()
    }
}

fn song_edge(rng: &mut Rng) -> Species {
    let pattern = ActivityPattern::Dawn;
    Species {
        kind: VoiceKind::Chirp,
        shape: PhraseShape::Motif,
        pattern,
        moisture_center: between(rng, 0.28, 0.48),
        moisture_tolerance: 0.4,
        canopy_center: between(rng, 0.12, 0.32),
        canopy_tolerance: 0.28,
        activity: humps(pattern),
        pitch_hz: between(rng, 2200.0, 3200.0),
        pitch_scatter: 0.028,
        syllable_median_s: 0.07,
        syllable_sigma: 0.25,
        syllables_min: 2,
        syllables_max: 4,
        phrases_min: 1,
        phrases_max: 1,
        syllable_gap_median: 0.055,
        syllable_gap_sigma: 0.3,
        phrase_gap_median: 0.2,
        phrase_gap_sigma: 0.2,
        bout_gap_median: between(rng, 26.0, 52.0),
        bout_gap_sigma: 0.65,
        step_cents: between(rng, -40.0, 30.0),
        step_scatter: 34.0,
        contour_cents: between(rng, -80.0, 80.0),
        attack_s: 0.01,
        decay_share: 0.58,
        noise_attack: 0.18,
        sweep_ratio: 0.98,
        sweep_curve: 1.0,
        brightness: 0.74,
        level: 0.62,
        distance_median: 0.4,
        replies: 0.22,
        reply_window_median: 0.8,
        capacity: rng.gen_range_u8(1, 2),
        birth_per_hour: 0.4,
        death_per_hour: 0.65,
        wind_shy: 0.55,
        territorial: 0.5,
        ..Species::default()
    }
}

fn cricket(rng: &mut Rng) -> Species {
    let pattern = ActivityPattern::Nocturnal;
    Species {
        kind: VoiceKind::Trill,
        shape: PhraseShape::Repeated,
        pattern,
        moisture_center: between(rng, 0.42, 0.7),
        moisture_tolerance: 0.38,
        canopy_center: between(rng, 0.25, 0.6),
        canopy_tolerance: 0.45,
        activity: humps(pattern),
        pitch_hz: between(rng, 4300.0, 5600.0),
        pitch_scatter: 0.012,
        syllable_median_s: 0.026,
        syllable_sigma: 0.12,
        syllables_min: 4,
        syllables_max: 8,
        phrases_min: 4,
        phrases_max: 10,
        syllable_gap_median: 0.022,
        syllable_gap_sigma: 0.15,
        phrase_gap_median: 0.24,
        phrase_gap_sigma: 0.25,
        bout_gap_median: between(rng, 12.0, 26.0),
        bout_gap_sigma: 0.55,
        step_cents: 0.0,
        step_scatter: 4.0,
        contour_cents: 0.0,
        attack_s: 0.004,
        decay_share: 0.4,
        noise_attack: 0.02,
        sweep_ratio: 1.0,
        sweep_curve: 1.0,
        brightness: 0.95,
        level: 0.22,
        distance_median: 0.74,
        replies: 0.04,
        reply_window_median: 0.5,
        capacity: rng.gen_range_u8(2, 4),
        birth_per_hour: 0.3,
        death_per_hour: 0.5,
        wind_shy: 0.5,
        territorial: 0.25,
        ..Species::default()
    }
}

fn insect(rng: &mut Rng) -> Species {
    let pattern = ActivityPattern::Diurnal;
    Species {
        kind: VoiceKind::Trill,
        shape: PhraseShape::Repeated,
        pattern,
        moisture_center: between(rng, 0.2, 0.4),
        moisture_tolerance: 0.4,
        canopy_center: between(rng, 0.1, 0.35),
        canopy_tolerance: 0.4,
        activity: humps(pattern),
        pitch_hz: between(rng, 5000.0, 6800.0),
        pitch_scatter: 0.015,
        syllable_median_s: 0.02,
        syllable_sigma: 0.15,
        syllables_min: 2,
        syllables_max: 5,
        phrases_min: 2,
        phrases_max: 4,
        syllable_gap_median: 0.03,
        syllable_gap_sigma: 0.2,
        phrase_gap_median: 0.4,
        phrase_gap_sigma: 0.3,
        bout_gap_median: between(rng, 28.0, 60.0),
        bout_gap_sigma: 0.6,
        step_cents: 0.0,
        step_scatter: 6.0,
        contour_cents: 0.0,
        attack_s: 0.003,
        decay_share: 0.45,
        noise_attack: 0.08,
        sweep_ratio: 1.0,
        sweep_curve: 1.0,
        brightness: 0.9,
        level: 0.16,
        distance_median: 0.62,
        replies: 0.0,
        reply_window_median: 0.6,
        capacity: 2,
        birth_per_hour: 0.25,
        death_per_hour: 0.45,
        wind_shy: 0.7,
        territorial: 0.2,
        ..Species::default()
    }
}

fn frog(
    rng: &mut Rng,
    band: (f32, f32),
    tolerance: f32,
    capacity: u8,
    pattern: ActivityPattern,
) -> Species {
    Species {
        kind: VoiceKind::Pulse,
        shape: PhraseShape::Paired,
        pattern,
        moisture_center: between(rng, 0.78, 0.9),
        moisture_tolerance: tolerance,
        canopy_center: between(rng, 0.35, 0.7),
        canopy_tolerance: 0.4,
        activity: humps(pattern),
        pitch_hz: between(rng, band.0, band.1),
        pitch_scatter: 0.04,
        syllable_median_s: 0.11,
        syllable_sigma: 0.15,
        syllables_min: 2,
        syllables_max: 2,
        phrases_min: 3,
        phrases_max: 7,
        syllable_gap_median: 0.09,
        syllable_gap_sigma: 0.2,
        phrase_gap_median: 0.38,
        phrase_gap_sigma: 0.25,
        bout_gap_median: between(rng, 14.0, 36.0),
        bout_gap_sigma: 0.58,
        step_cents: between(rng, 280.0, 520.0),
        step_scatter: 18.0,
        contour_cents: 0.0,
        attack_s: 0.008,
        decay_share: 0.72,
        noise_attack: 0.04,
        sweep_ratio: 0.92,
        sweep_curve: 0.8,
        brightness: 0.32,
        level: 0.86,
        distance_median: 0.56,
        replies: between(rng, 0.3, 0.55),
        reply_window_median: 0.7,
        capacity,
        birth_per_hour: 0.4,
        death_per_hour: 0.55,
        wind_shy: 0.22,
        territorial: 0.35,
        ..Species::default()
    }
}

fn tap(rng: &mut Rng) -> Species {
    let pattern = ActivityPattern::Diurnal;
    Species {
        kind: VoiceKind::Tick,
        shape: PhraseShape::Repeated,
        pattern,
        moisture_center: 0.5,
        moisture_tolerance: 0.55,
        canopy_center: between(rng, 0.35, 0.7),
        canopy_tolerance: 0.4,
        activity: humps(pattern),
        pitch_hz: between(rng, 700.0, 1400.0),
        pitch_scatter: 0.02,
        syllable_median_s: 0.028,
        syllable_sigma: 0.2,
        syllables_min: 1,
        syllables_max: 4,
        phrases_min: 1,
        phrases_max: 1,
        syllable_gap_median: 0.12,
        syllable_gap_sigma: 0.25,
        phrase_gap_median: 0.3,
        phrase_gap_sigma: 0.2,
        bout_gap_median: between(rng, 45.0, 120.0),
        bout_gap_sigma: 0.55,
        step_cents: 0.0,
        step_scatter: 8.0,
        contour_cents: 0.0,
        attack_s: 0.002,
        decay_share: 0.8,
        noise_attack: 0.85,
        sweep_ratio: 1.0,
        sweep_curve: 1.0,
        brightness: 0.4,
        level: 0.58,
        distance_median: 0.42,
        replies: 0.0,
        reply_window_median: 0.5,
        capacity: 1,
        birth_per_hour: 0.2,
        death_per_hour: 0.35,
        wind_shy: 0.15,
        territorial: 0.85,
        ..Species::default()
    }
}

fn separate(species: &mut [Species]) {
    let kinds = [
        VoiceKind::Chirp,
        VoiceKind::Trill,
        VoiceKind::Pulse,
        VoiceKind::Tick,
    ];
    for kind in kinds {
        let mut ix = [0usize; MAX_SPECIES];
        let mut m = 0;
        for (i, species) in species.iter().enumerate() {
            if species.kind == kind {
                ix[m] = i;
                m += 1;
            }
        }
        for a in 1..m {
            let mut b = a;
            while b > 0 && species[ix[b]].pitch_hz < species[ix[b - 1]].pitch_hz {
                ix.swap(b, b - 1);
                b -= 1;
            }
        }
        for a in 1..m {
            let min = species[ix[a - 1]].pitch_hz * 1.0905;
            if species[ix[a]].pitch_hz < min {
                species[ix[a]].pitch_hz = min;
            }
        }
    }
}
