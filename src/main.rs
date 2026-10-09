mod play;

use holt::{
    clock, ActivityPattern, Config, PhraseShape, Preset, Species, VoiceKind, World, WorldEvent,
    BLOCK_MS, SAMPLE_RATE,
};
use std::collections::HashSet;
use std::env;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by Ctrl-C during playback, so the loop can finish the tally and close the device.
static STOP: AtomicBool = AtomicBool::new(false);

fn main() -> ExitCode {
    match run() {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::from(130),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, String> {
    let args = Args::parse(env::args().skip(1))?;
    if args.help {
        print!("{HELP}");
        return Ok(false);
    }
    if args.trace && args.for_ms.is_none() {
        return Err(
            "--trace needs --for, so the text has an end. Try: holt --trace --for 2m".to_string(),
        );
    }

    let mut world = World::new(Config {
        seed: args.seed.unwrap_or_else(random_seed),
        preset: args.preset,
        from_seedling: args.from_seedling,
        world_rate: args.world_rate,
    });
    if args.at_ms > 0 {
        world.fast_forward(args.at_ms / 1000);
    }
    if args.rain {
        world.open_in_rain();
    }

    let labels = labels(world.roster());
    print_place(&world, &labels);
    if args.for_ms == Some(0) {
        return Ok(false);
    }

    let mut score = Score::new(world.animal_ms());
    let listened = if args.trace {
        let for_ms = args.for_ms.unwrap_or(0);
        trace(&mut world, for_ms, &labels, &mut score)
    } else {
        play_for(&mut world, args.for_ms, &labels, &mut score)?
    };

    println!();
    println!(
        "{} listened   {} phrases   {} replies   {} exact repeats   {} births   {} deaths   {} storms",
        stamp(listened),
        score.phrases,
        score.replies,
        score.repeats,
        score.births,
        score.deaths,
        score.storms,
    );
    Ok(STOP.load(Ordering::Relaxed))
}

struct Score {
    origin: u64,
    phrases: u32,
    replies: u32,
    births: u32,
    deaths: u32,
    storms: u32,
    repeats: u32,
    seen: HashSet<u64>,
}

impl Score {
    fn new(origin: u64) -> Self {
        Self {
            origin,
            phrases: 0,
            replies: 0,
            births: 0,
            deaths: 0,
            storms: 0,
            repeats: 0,
            seen: HashSet::new(),
        }
    }

    fn drain(&mut self, world: &mut World, labels: &[String], at: u64) {
        while let Some(event) = world.poll_event() {
            self.note(world, labels, event, at);
        }
    }

    fn note(&mut self, world: &World, labels: &[String], event: WorldEvent, at: u64) {
        match event {
            WorldEvent::PhraseStarted {
                species,
                pan,
                distance,
                pitch_hz,
                syllables,
                contour_cents,
                reply,
                signature,
                ..
            } => {
                self.phrases += 1;
                if reply {
                    self.replies += 1;
                }
                if !self.seen.insert(signature) {
                    self.repeats += 1;
                }
                let name = labels
                    .get(species as usize)
                    .map(String::as_str)
                    .unwrap_or("?");
                let shape = world
                    .roster()
                    .get(species as usize)
                    .map(|s| shape_word(s.shape))
                    .unwrap_or("song");
                let verb = if reply { "reply" } else { "sing " };
                let notes = if syllables == 1 { "note " } else { "notes" };
                println!(
                    "{}  {}  {:<12} pan {:+.2}  dist {:.2}  {:<7} {syllables} {notes}  {:<7} {}",
                    stamp(at),
                    verb,
                    name,
                    pan,
                    distance,
                    shape,
                    contour_word(contour_cents),
                    hz(pitch_hz),
                );
            }
            WorldEvent::Birth {
                species,
                pan,
                distance,
                ..
            } => {
                self.births += 1;
                let name = labels
                    .get(species as usize)
                    .map(String::as_str)
                    .unwrap_or("?");
                println!(
                    "{}  born   {:<12} pan {:+.2}  dist {:.2}",
                    stamp(at),
                    name,
                    pan,
                    distance
                );
            }
            WorldEvent::Death { species, .. } => {
                self.deaths += 1;
                let name = labels
                    .get(species as usize)
                    .map(String::as_str)
                    .unwrap_or("?");
                println!("{}  gone   {name}", stamp(at));
            }
            WorldEvent::Storm { intensity } => {
                self.storms += 1;
                println!("{}  storm  intensity {intensity:.2}", stamp(at));
            }
            WorldEvent::Shower { on, intensity } => {
                if on {
                    println!("{}  rain   intensity {intensity:.2}", stamp(at));
                } else {
                    println!("{}  rain   passed", stamp(at));
                }
            }
            WorldEvent::PhraseEnded { .. } => {}
        }
    }
}

fn trace(world: &mut World, for_ms: u64, labels: &[String], score: &mut Score) -> u64 {
    let mut listened = 0u64;
    let mut next_status = 30_000u64;
    while listened < for_ms {
        world.advance_block();
        listened += BLOCK_MS;
        let at = world.animal_ms().saturating_sub(score.origin);
        score.drain(world, labels, at);
        if for_ms >= 30_000 && listened >= next_status && listened < for_ms {
            print_status(world, listened);
            next_status += 30_000;
        }
    }
    listened
}

fn play_for(
    world: &mut World,
    for_ms: Option<u64>,
    labels: &[String],
    score: &mut Score,
) -> Result<u64, String> {
    let mut device = play::Device::open()?;
    arm_stop();
    match for_ms {
        Some(ms) => println!("playing for {}", stamp(ms)),
        None => println!("playing until you stop it"),
    }
    let _ = io::stdout().flush();
    let mut buf = [0.0f32; 960];
    let mut next_status = 30_000u64;
    let total_frames = for_ms.map(|ms| ms.saturating_mul(u64::from(SAMPLE_RATE)) / 1_000);
    let mut done = 0u64;
    while !STOP.load(Ordering::Relaxed) {
        let frames = match total_frames {
            Some(total) if done >= total => break,
            Some(total) => ((total - done) as usize).min(480),
            None => 480,
        };
        if frames == 0 {
            break;
        }
        let n = frames * 2;
        world.render(&mut buf[..n]);
        device.write(&buf[..n])?;
        done += frames as u64;
        let listened = done.saturating_mul(1_000) / u64::from(SAMPLE_RATE);
        let at = world.animal_ms().saturating_sub(score.origin);
        score.drain(world, labels, at);
        let limit = for_ms.unwrap_or(u64::MAX);
        if limit >= 30_000 && listened >= next_status && listened < limit {
            print_status(world, listened);
            next_status += 30_000;
        }
        let _ = io::stdout().flush();
    }
    Ok(done.saturating_mul(1_000) / u64::from(SAMPLE_RATE))
}

/// Ctrl-C ends an open listen after the current moment, instead of cutting the device off.
fn arm_stop() {
    STOP.store(false, Ordering::SeqCst);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    unsafe {
        linux_arm_stop();
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe fn linux_arm_stop() {
    // glibc `struct sigaction` on x86_64: handler, 128-byte mask, flags, pad, restorer.
    #[repr(C)]
    struct SigAction {
        handler: usize,
        mask: [u64; 16],
        flags: i32,
        _pad: i32,
        restorer: usize,
    }
    const _: () = assert!(std::mem::size_of::<SigAction>() == 152);

    unsafe extern "C" {
        fn sigaction(sig: i32, act: *const SigAction, old: *mut SigAction) -> i32;
    }

    let action = SigAction {
        handler: on_stop as *const () as usize,
        mask: [0; 16],
        flags: 0x1000_0000, // SA_RESTART, so an in-flight write finishes
        _pad: 0,
        restorer: 0,
    };
    unsafe {
        sigaction(2, &action, std::ptr::null_mut()); // SIGINT
        sigaction(15, &action, std::ptr::null_mut()); // SIGTERM
    }
}

extern "C" fn on_stop(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}

fn print_place(world: &World, labels: &[String]) {
    let snap = world.snapshot();
    let (hour, minute) = clock(snap.day_phase);
    let stand = if world.from_seedling() {
        "seedling"
    } else {
        "established"
    };
    let age = if snap.world_ms > 0 {
        format!("  age {}", stamp(snap.world_ms))
    } else {
        String::new()
    };
    let rate = world.world_rate();
    let rate_note = if (rate - 1.0).abs() > 0.001 {
        format!("  world ×{rate:.3}")
    } else {
        String::new()
    };
    println!(
        "holt  seed {}  {}  {stand}{age}{rate_note}",
        world.seed(),
        world.preset().name(),
    );
    let rain = if snap.shower >= 0.08 {
        format!("   rain {:.2}", snap.shower)
    } else {
        String::new()
    };
    println!(
        "      {hour:02}:{minute:02}   canopy {:.2}   moisture {:.2}   wind {:.2}   gust {:.2}{rain}   alive {}",
        snap.canopy, snap.moisture, snap.wind, snap.gust, snap.alive,
    );
    println!();
    for (i, species) in world.roster().iter().enumerate() {
        let room = world.capacity_of(i);
        let presence = if room == 0 {
            "no room".to_string()
        } else {
            format!("{} of {room}", world.alive_of(i))
        };
        println!(
            "      {:<12} {:<8} {:>8}   {presence:<8} gap ~{:.0}s",
            labels[i],
            species.pattern.name(),
            hz(species.pitch_hz),
            species.bout_gap_median,
        );
    }
    println!();
}

fn print_status(world: &World, listened_ms: u64) {
    let snap = world.snapshot();
    let (hour, minute) = clock(snap.day_phase);
    let singing = snap.individuals.iter().filter(|i| i.singing).count();
    let rain = if snap.shower >= 0.02 {
        format!("  rain {:.2}", snap.shower)
    } else {
        String::new()
    };
    println!(
        "{}  --     {hour:02}:{minute:02}  canopy {:.2}  moisture {:.2}  wind {:.2}  gust {:.2}{rain}  alive {}  singing {singing}",
        stamp(listened_ms),
        snap.canopy,
        snap.moisture,
        snap.wind,
        snap.gust,
        snap.alive,
    );
}

fn labels(roster: &[Species]) -> Vec<String> {
    let bases: Vec<&str> = roster.iter().map(base_name).collect();
    bases
        .iter()
        .enumerate()
        .map(|(i, base)| {
            let mut same: Vec<usize> = bases
                .iter()
                .enumerate()
                .filter(|(_, other)| *other == base)
                .map(|(j, _)| j)
                .collect();
            if same.len() == 1 {
                return (*base).to_string();
            }
            same.sort_by(|&a, &b| {
                roster[a]
                    .pitch_hz
                    .partial_cmp(&roster[b].pitch_hz)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let rank = same.iter().position(|&j| j == i).unwrap_or(0);
            let last = same.len() - 1;
            let height = if rank == 0 {
                "low"
            } else if rank == last {
                "high"
            } else {
                "mid"
            };
            format!("{height} {base}")
        })
        .collect()
}

fn base_name(species: &Species) -> &'static str {
    match species.kind {
        VoiceKind::Chirp => "song",
        VoiceKind::Trill => match species.pattern {
            ActivityPattern::Nocturnal => "cricket",
            ActivityPattern::Diurnal => "insect",
            _ => "trill",
        },
        VoiceKind::Pulse => "frog",
        VoiceKind::Tick => "tap",
    }
}

fn shape_word(shape: PhraseShape) -> &'static str {
    match shape {
        PhraseShape::Motif => "motif",
        PhraseShape::Slur => "slur",
        PhraseShape::Repeated => "repeats",
        PhraseShape::Paired => "pair",
    }
}

fn contour_word(cents: i16) -> &'static str {
    if cents <= -30 {
        "falling"
    } else if cents >= 30 {
        "rising"
    } else {
        "level"
    }
}

fn hz(pitch: f32) -> String {
    if pitch >= 1000.0 {
        format!("{:.2} kHz", pitch / 1000.0)
    } else {
        format!("{pitch:.0} Hz")
    }
}

fn stamp(ms: u64) -> String {
    let total_cs = ms / 10;
    let cs = total_cs % 100;
    let total_s = total_cs / 100;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{cs:02}")
    } else {
        format!("{m}:{s:02}.{cs:02}")
    }
}

struct Args {
    help: bool,
    /// None picks a new place for this run.
    seed: Option<u64>,
    preset: Preset,
    /// None plays until the listen is stopped. Some(0) prints the roster only.
    for_ms: Option<u64>,
    at_ms: u64,
    from_seedling: bool,
    world_rate: f32,
    rain: bool,
    trace: bool,
}

impl Args {
    fn parse(flags: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Self {
            help: false,
            seed: None,
            preset: Preset::Broadleaf,
            for_ms: None,
            at_ms: 0,
            from_seedling: false,
            world_rate: 1.0,
            rain: false,
            trace: false,
        };
        let mut flags = flags.peekable();
        while let Some(flag) = flags.next() {
            match flag.as_str() {
                "-h" | "--help" => args.help = true,
                "--seed" => args.seed = Some(parse_u64(&need(&mut flags, "--seed")?)?),
                "--preset" => {
                    let name = need(&mut flags, "--preset")?;
                    args.preset = Preset::parse(&name)
                        .ok_or_else(|| format!("unknown preset '{name}'. {}", preset_list()))?;
                }
                "--for" => args.for_ms = Some(parse_duration(&need(&mut flags, "--for")?)?),
                "--at" => args.at_ms = parse_duration(&need(&mut flags, "--at")?)?,
                "--from-seedling" => args.from_seedling = true,
                "--rain" => args.rain = true,
                "--trace" => args.trace = true,
                "--world-rate" => {
                    let raw = need(&mut flags, "--world-rate")?;
                    let rate: f32 = raw
                        .parse()
                        .map_err(|_| format!("world rate '{raw}' is not a number"))?;
                    if rate <= 0.0 {
                        return Err("world rate has to be above 0".to_string());
                    }
                    args.world_rate = rate;
                }
                other => {
                    return Err(format!("unknown argument '{other}'. Try --help"));
                }
            }
        }
        Ok(args)
    }
}

fn need(flags: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    flags.next().ok_or_else(|| format!("{name} needs a value"))
}

fn parse_u64(raw: &str) -> Result<u64, String> {
    raw.parse()
        .map_err(|_| format!("'{raw}' is not a whole number"))
}

fn parse_duration(raw: &str) -> Result<u64, String> {
    let raw = raw.trim();
    let (number, factor): (&str, f64) = if let Some(n) = raw.strip_suffix('h') {
        (n, 3_600_000.0)
    } else if let Some(n) = raw.strip_suffix('m') {
        (n, 60_000.0)
    } else if let Some(n) = raw.strip_suffix('s') {
        (n, 1_000.0)
    } else {
        (raw, 1_000.0)
    };
    let value: f64 = number
        .trim()
        .parse()
        .map_err(|_| format!("'{raw}' is not a duration. Use 90s, 2m, or 6h"))?;
    if value < 0.0 {
        return Err("duration has to be zero or more".to_string());
    }
    Ok((value * factor).round() as u64)
}

fn preset_list() -> String {
    "Use broadleaf, wetwood, dryridge, nightmarsh, or clearing".to_string()
}

/// A new place. Zero is refused because the generator treats it as a fixed stand-in.
fn random_seed() -> u64 {
    if let Some(seed) = urandom_u64() {
        return seed;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(Config::default().seed);
    let mixed = nanos ^ ((std::process::id() as u64) << 32) ^ 0x6A09_E667_F3BC_C909;
    if mixed == 0 {
        Config::default().seed
    } else {
        mixed
    }
}

fn urandom_u64() -> Option<u64> {
    let mut bytes = [0u8; 8];
    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    file.read_exact(&mut bytes).ok()?;
    let seed = u64::from_ne_bytes(bytes);
    if seed == 0 {
        None
    } else {
        Some(seed)
    }
}

const HELP: &str = "\
holt — a forest that is a place, not a tape

Usage:
  holt [--seed N] [--preset NAME] [--for DURATION] [--at DURATION]
       [--from-seedling] [--rain] [--world-rate N] [--trace]

With no --seed, holt picks a new place and prints its number.
With no --for, it plays until you stop it with Ctrl-C.
--for sets a length, like 90s, 2m, or 6h. --for 0 prints the roster and stops.
--seed returns to a place. The first morning is --seed 1847392.
--at grows the place before you listen.
--rain arrives while a shower is falling. It eases on its own, and the leaves
keep dripping after it passes.
--world-rate speeds the sun, the weather, and succession. The animals stay
at their own pace. --trace prints a stretch at once, with no sound, and needs --for.

Presets: broadleaf, wetwood, dryridge, nightmarsh, clearing
";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(flags: &[&str]) -> Args {
        Args::parse(flags.iter().map(|s| (*s).to_string())).unwrap()
    }

    #[test]
    fn omitted_seed_and_duration_stay_open() {
        let parsed = args(&[]);
        assert!(parsed.seed.is_none());
        assert!(parsed.for_ms.is_none());
    }

    #[test]
    fn an_explicit_seed_and_zero_length_stay_put() {
        let parsed = args(&["--seed", "1847392", "--for", "0"]);
        assert_eq!(parsed.seed, Some(1_847_392));
        assert_eq!(parsed.for_ms, Some(0));
    }

    #[test]
    fn rain_is_a_way_of_arriving() {
        let parsed = args(&["--rain", "--preset", "wetwood"]);
        assert!(parsed.rain);
        assert_eq!(parsed.preset, Preset::Wetwood);
    }

    #[test]
    fn a_new_seed_is_a_real_number() {
        let first = random_seed();
        let second = random_seed();
        assert_ne!(first, 0);
        assert_ne!(second, 0);
        assert_ne!(first, second);
    }
}
