//! Deterministic RNG and weighted sampling (no external dependency, reproducible by seed).

#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    /// SplitMix64.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_f64() * n as f64) as usize % n.max(1)
    }
}

/// Picks an index with probability proportional to `weights`. Returns None if all are zero.
pub fn weighted_index(weights: &[f32], rng: &mut Rng) -> Option<usize> {
    let total: f64 = weights.iter().map(|&w| w.max(0.0) as f64).sum();
    if total <= 0.0 {
        return None;
    }
    let mut x = rng.next_f64() * total;
    let mut last = None;
    for (i, &w) in weights.iter().enumerate() {
        if w <= 0.0 {
            continue;
        }
        last = Some(i);
        x -= w as f64;
        if x < 0.0 {
            return Some(i);
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_sampling_respects_weights_and_skips_zero() {
        let w = [0.0, 3.0, 1.0, 0.0];
        let mut rng = Rng::new(42);
        let mut counts = [0usize; 4];
        for _ in 0..40_000 {
            counts[weighted_index(&w, &mut rng).unwrap()] += 1;
        }
        assert_eq!(counts[0], 0);
        assert_eq!(counts[3], 0);
        let ratio = counts[1] as f64 / counts[2] as f64;
        assert!((ratio - 3.0).abs() < 0.15, "ratio {ratio}");
        assert_eq!(weighted_index(&[0.0, 0.0], &mut rng), None);
    }
}
