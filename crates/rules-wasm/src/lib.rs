//! `rules` を wasm にして、ブラウザから呼ぶ入口。ルールは書き直さず、`rules` をそのまま使う。

use wasm_bindgen::prelude::wasm_bindgen;

/// 盤の一辺のマス数。
#[wasm_bindgen]
pub fn board_size() -> u32 {
    rules::BOARD_SIZE
}

#[cfg(test)]
mod tests {
    #[test]
    fn board_size_matches_rules() {
        assert_eq!(super::board_size(), rules::BOARD_SIZE);
    }
}
