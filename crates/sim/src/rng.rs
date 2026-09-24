use serde::{Deserialize, Serialize};

/// xoshiro256** seeded via `SplitMix64`. Implemented here (not `StdRng`) so the algorithm
/// can never change under us: changing it invalidates every recorded replay.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        let mut sm = seed;
        Self {
            s: [
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
            ],
        }
    }

    pub const fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.s;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s1 << 17;
        let s2 = s2 ^ s0;
        let s3 = s3 ^ s1;
        let s1 = s1 ^ s2;
        let s0 = s0 ^ s3;
        self.s = [s0, s1, s2 ^ t, s3.rotate_left(45)];
        result
    }

    /// Uniform-enough integer in `0..n` (0 when `n` is 0): the top 32 bits scaled by `n`.
    pub fn below(&mut self, n: u32) -> u32 {
        let scaled = (self.next_u64() >> 32).wrapping_mul(u64::from(n)) >> 32;
        u32::try_from(scaled).unwrap_or(0)
    }
}

const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known answers from the reference algorithms; guards against accidental edits.
    #[test]
    fn matches_reference_algorithms() {
        let mut sm = 0;
        assert_eq!(splitmix64(&mut sm), 0xE220_A839_7B1D_CDAF);
        let mut rng = Rng { s: [1, 2, 3, 4] };
        assert_eq!(rng.next_u64(), 11520);
    }
}
