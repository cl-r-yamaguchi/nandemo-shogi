//! 乱数の窓口。`rules` は、乱数の発生源に依存せず、呼び出し側が `Rng` を渡す（architecture.md の 4.1）。

/// 乱数の発生源。
pub trait Rng {
    fn next_u64(&mut self) -> u64;

    /// `0..bound` の一様な整数（`bound` は 1 以上）。
    fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "bound は 1 以上が必要です");
        ((u128::from(self.next_u64()) * u128::from(bound)) >> 64) as u64
    }
}

/// 同じシードから同じ列を出す、軽量な乱数（SplitMix64）。再現と、テストに使う。
#[derive(Debug, Clone)]
pub struct SplitMix64(pub u64);

impl Rng for SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_the_same_sequence() {
        let mut a = SplitMix64(42);
        let mut b = SplitMix64(42);
        let xs: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        assert_ne!(SplitMix64(1).next_u64(), SplitMix64(2).next_u64());
    }

    #[test]
    fn below_stays_in_range_and_covers_all_values() {
        let mut rng = SplitMix64(7);
        let mut seen = [false; 5];
        for _ in 0..200 {
            let n = rng.below(5);
            assert!(n < 5);
            seen[n as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(rng.below(1), 0);
    }
}
