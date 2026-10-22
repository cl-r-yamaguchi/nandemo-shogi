//! ゲームルール（純粋な計算）。仕様は docs/specs/game-rules.md。
//!
//! 入出力・時刻・乱数の発生源に依存しない。

pub mod geometry;
pub mod pieces;
pub mod state;

/// 盤の一辺のマス数。
pub const BOARD_SIZE: u32 = 16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_is_16x16() {
        assert_eq!(BOARD_SIZE, 16);
    }
}
