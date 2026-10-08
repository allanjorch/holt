/// Small deterministic generator. Streams are split from the world seed so
/// roster, placement, ecology, and phrase performance never share draws.

#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x000A_11CE_5EED } else { seed },
        }
    }

    pub fn stream(seed: u64, n: u64) -> Self {
        let mixed = splitmix64(seed ^ n.wrapping_mul(0xD1B5_4A32_D192_ED03));
        Self::new(mixed)
    }

    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*. A zero state would lock; `new` refuses zero.
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn f32(&mut self) -> f32 {
        let x = (self.next_u64() >> 40) as u32;
        x as f32 / 16_777_216.0
    }

    /// (0, 1], for logarithms in Box-Muller.
    fn f32_open(&mut self) -> f32 {
        let x = (self.next_u64() >> 40) as u32;
        (x as f32 + 1.0) / 16_777_217.0
    }

    pub fn gen_range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f32()
    }

    pub fn gen_range_u8(&mut self, min: u8, max: u8) -> u8 {
        if max <= min {
            return min;
        }
        let span = u32::from(max - min) + 1;
        min + (self.next_u64() as u32 % span) as u8
    }

    pub fn gaussian(&mut self) -> f32 {
        let u1 = self.f32_open();
        let u2 = self.f32();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = std::f32::consts::TAU * u2;
        r * theta.cos()
    }

    /// `median` is the median of the result. `sigma` is the log-space spread.
    pub fn log_normal(&mut self, median: f32, sigma: f32) -> f32 {
        median * (sigma * self.gaussian()).exp()
    }
}

pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
