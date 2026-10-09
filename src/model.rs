pub const MAX_SPECIES: usize = 12;
pub const MAX_INDIVIDUALS: usize = 64;
pub const MAX_SYLLABLES: usize = 8;
pub const CHIRP_VOICES: usize = 24;
pub const BLOCK_MS: u64 = 10;
pub const DAY_MS: u64 = 86_400_000;
pub const SAMPLE_RATE: u32 = 48_000;

pub const STREAM_ROSTER: u64 = 1;
pub const STREAM_PLACE: u64 = 2;
pub const STREAM_ECO: u64 = 3;
pub const STREAM_PHRASE: u64 = 4;
pub const STREAM_AUDIO: u64 = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VoiceKind {
    #[default]
    Chirp,
    Trill,
    Pulse,
    Tick,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhraseShape {
    #[default]
    Motif,
    Slur,
    Repeated,
    Paired,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityPattern {
    #[default]
    Diurnal,
    Dawn,
    Dusk,
    Nocturnal,
    Crepuscular,
}

impl ActivityPattern {
    pub fn name(self) -> &'static str {
        match self {
            Self::Diurnal => "day",
            Self::Dawn => "dawn",
            Self::Dusk => "dusk",
            Self::Nocturnal => "night",
            Self::Crepuscular => "twilight",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hump {
    pub phase: f32,
    pub width: f32,
    pub weight: f32,
}

pub fn humps(pattern: ActivityPattern) -> [Hump; 2] {
    match pattern {
        ActivityPattern::Dawn => [
            Hump {
                phase: 0.27,
                width: 0.055,
                weight: 1.0,
            },
            Hump::default(),
        ],
        ActivityPattern::Diurnal => [
            Hump {
                phase: 0.50,
                width: 0.20,
                weight: 1.0,
            },
            Hump::default(),
        ],
        ActivityPattern::Dusk => [
            Hump {
                phase: 0.80,
                width: 0.06,
                weight: 1.0,
            },
            Hump::default(),
        ],
        ActivityPattern::Nocturnal => [
            Hump {
                phase: 0.00,
                width: 0.18,
                weight: 1.0,
            },
            Hump::default(),
        ],
        ActivityPattern::Crepuscular => [
            Hump {
                phase: 0.27,
                width: 0.05,
                weight: 1.0,
            },
            Hump {
                phase: 0.80,
                width: 0.05,
                weight: 0.85,
            },
        ],
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    #[default]
    Broadleaf,
    Wetwood,
    Dryridge,
    Nightmarsh,
    Clearing,
}

impl Preset {
    pub fn name(self) -> &'static str {
        match self {
            Self::Broadleaf => "broadleaf",
            Self::Wetwood => "wetwood",
            Self::Dryridge => "dryridge",
            Self::Nightmarsh => "nightmarsh",
            Self::Clearing => "clearing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "broadleaf" => Some(Self::Broadleaf),
            "wetwood" => Some(Self::Wetwood),
            "dryridge" => Some(Self::Dryridge),
            "nightmarsh" => Some(Self::Nightmarsh),
            "clearing" => Some(Self::Clearing),
            _ => None,
        }
    }

    pub fn equilibrium(self) -> Equilibrium {
        match self {
            Self::Broadleaf => Equilibrium {
                moisture: 0.45,
                canopy_ceiling: 0.75,
                wind_exposure: 0.35,
                warmth: 0.55,
            },
            Self::Wetwood => Equilibrium {
                moisture: 0.80,
                canopy_ceiling: 0.90,
                wind_exposure: 0.25,
                warmth: 0.70,
            },
            Self::Dryridge => Equilibrium {
                moisture: 0.18,
                canopy_ceiling: 0.30,
                wind_exposure: 0.70,
                warmth: 0.60,
            },
            Self::Nightmarsh => Equilibrium {
                moisture: 0.85,
                canopy_ceiling: 0.40,
                wind_exposure: 0.20,
                warmth: 0.50,
            },
            Self::Clearing => Equilibrium {
                moisture: 0.35,
                canopy_ceiling: 0.25,
                wind_exposure: 0.55,
                warmth: 0.55,
            },
        }
    }

    pub fn start_phase(self) -> f32 {
        match self {
            Self::Broadleaf => 0.28,
            Self::Wetwood => 0.22,
            Self::Dryridge => 0.35,
            Self::Nightmarsh => 0.92,
            Self::Clearing => 0.27,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Equilibrium {
    pub moisture: f32,
    pub canopy_ceiling: f32,
    pub wind_exposure: f32,
    pub warmth: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Species {
    pub kind: VoiceKind,
    pub shape: PhraseShape,
    pub pattern: ActivityPattern,
    pub moisture_center: f32,
    pub moisture_tolerance: f32,
    pub canopy_center: f32,
    pub canopy_tolerance: f32,
    pub activity: [Hump; 2],
    pub pitch_hz: f32,
    pub pitch_scatter: f32,
    pub syllable_median_s: f32,
    pub syllable_sigma: f32,
    pub syllables_min: u8,
    pub syllables_max: u8,
    pub phrases_min: u8,
    pub phrases_max: u8,
    pub syllable_gap_median: f32,
    pub syllable_gap_sigma: f32,
    pub phrase_gap_median: f32,
    pub phrase_gap_sigma: f32,
    pub bout_gap_median: f32,
    pub bout_gap_sigma: f32,
    pub step_cents: f32,
    pub step_scatter: f32,
    pub contour_cents: f32,
    pub attack_s: f32,
    pub decay_share: f32,
    pub noise_attack: f32,
    pub vibrato_hz: f32,
    pub vibrato_cents: f32,
    pub sweep_ratio: f32,
    pub sweep_curve: f32,
    pub brightness: f32,
    pub level: f32,
    pub distance_median: f32,
    pub replies: f32,
    pub reply_window_median: f32,
    pub capacity: u8,
    pub birth_per_hour: f32,
    pub death_per_hour: f32,
    pub wind_shy: f32,
    pub territorial: f32,
}

impl Species {
    pub fn activity_at(&self, phase: f32) -> f32 {
        let mut a = 0.0;
        for hump in self.activity {
            if hump.weight <= 0.0 || hump.width <= 0.0 {
                continue;
            }
            let mut d = (phase - hump.phase).abs();
            if d > 0.5 {
                d = 1.0 - d;
            }
            let z = d / hump.width;
            a += (-z * z).exp() * hump.weight;
        }
        a.clamp(0.0, 1.0)
    }

    pub fn habitat(&self, moisture: f32, canopy: f32) -> f32 {
        affinity(self.moisture_center, self.moisture_tolerance, moisture)
            * affinity(self.canopy_center, self.canopy_tolerance, canopy)
    }

    pub fn capacity_at(&self, moisture: f32, canopy: f32) -> u8 {
        (self.capacity as f32 * self.habitat(moisture, canopy)).round() as u8
    }
}

fn affinity(center: f32, tolerance: f32, x: f32) -> f32 {
    let t = tolerance.max(0.02);
    let z = (x - center) / t;
    (-z * z).exp()
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub seed: u64,
    pub preset: Preset,
    pub from_seedling: bool,
    pub world_rate: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 1_847_392,
            preset: Preset::Broadleaf,
            from_seedling: false,
            world_rate: 1.0,
        }
    }
}

/// Polled view of the place. A picture copies this. It contains no pointers.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub animal_ms: u64,
    pub world_ms: u64,
    pub day_phase: f32,
    pub daylight: f32,
    pub moisture: f32,
    pub canopy: f32,
    pub wind: f32,
    pub gust: f32,
    pub shower: f32,
    pub held_water: f32,
    pub disturbance: f32,
    pub beds: BedGains,
    pub individuals: [IndividualView; MAX_INDIVIDUALS],
    pub alive: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IndividualView {
    pub slot: u8,
    pub species: u8,
    pub kind: VoiceKind,
    pub alive: bool,
    pub pan: f32,
    pub distance: f32,
    pub singing: bool,
    pub phrase_phase: f32,
    pub pitch_hz: f32,
    pub level: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BedGains {
    pub air: f32,
    pub water: f32,
    pub insects: f32,
    pub drip_hz: f32,
    pub tick_hz: f32,
    /// 0 dry .. 1 a hard shower. The rain texture follows this.
    pub shower: f32,
}

/// Edges. Drips and gusts stay off this list; their rates are on the snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldEvent {
    PhraseStarted {
        slot: u8,
        species: u8,
        pan: f32,
        distance: f32,
        pitch_hz: f32,
        duration_s: f32,
        syllables: u8,
        contour_cents: i16,
        reply: bool,
        signature: u64,
    },
    PhraseEnded {
        slot: u8,
        species: u8,
    },
    Birth {
        slot: u8,
        species: u8,
        pan: f32,
        distance: f32,
    },
    Death {
        slot: u8,
        species: u8,
    },
    Storm {
        intensity: f32,
    },
    Shower {
        on: bool,
        intensity: f32,
    },
}

pub fn daylight(phase: f32) -> f32 {
    ((phase - 0.5) * std::f32::consts::TAU).cos() * 0.5 + 0.5
}

pub fn clock(phase: f32) -> (u32, u32) {
    let mins = (phase * 24.0 * 60.0).round() as u32 % (24 * 60);
    (mins / 60, mins % 60)
}
