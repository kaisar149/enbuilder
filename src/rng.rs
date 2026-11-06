//! Small deterministic PRNG (SplitMix64). No external crates; identical on all platforms.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self { Rng(seed) }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 { (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 { lo + (hi - lo) * self.next_f64() }
    pub fn unit_vec3(&mut self) -> [f64; 3] {
        let z = self.range(-1.0, 1.0);
        let phi = self.range(0.0, std::f64::consts::TAU);
        let r = (1.0 - z * z).max(0.0).sqrt();
        [r * phi.cos(), r * phi.sin(), z]
    }
    /// k distinct values from 0..n (partial Fisher-Yates), in random order.
    pub fn choose_distinct(&mut self, n: usize, k: usize) -> Vec<usize> {
        let k = k.min(n);
        let mut idx: Vec<u32> = (0..n as u32).collect();
        for i in 0..k {
            let j = i + ((self.next_f64() * (n - i) as f64) as usize).min(n - i - 1);
            idx.swap(i, j);
        }
        idx[..k].iter().map(|&v| v as usize).collect()
    }
}
