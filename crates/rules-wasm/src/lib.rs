//! `rules` を wasm にして、ブラウザから呼ぶ入口。ルールは書き直さず、`rules` をそのまま使う。
//!
//! 状態は、サーバーが送る盤面の写し（`StateView`）の JSON。結果も JSON。ここの関数は、読み取りの
//! 問い合わせだけで、状態を持たない。不正な入力は、例外ではなく、エラーの文字列として返す。

mod queries;

use wasm_bindgen::prelude::{JsError, wasm_bindgen};

/// 盤の一辺のマス数。
#[wasm_bindgen]
pub fn board_size() -> u32 {
    rules::BOARD_SIZE
}

/// 駒の種類の定義の一覧（JSON）。種類の番号は、配列の位置。
#[wasm_bindgen]
pub fn piece_kinds() -> Result<String, JsError> {
    queries::piece_kinds().map_err(|e| JsError::new(&e))
}

/// 手番のプレイヤーの、合法な行動の一覧（JSON）。
#[wasm_bindgen]
pub fn legal_actions(state: &str) -> Result<String, JsError> {
    queries::legal_actions(state).map_err(|e| JsError::new(&e))
}

/// 盤上の駒 `piece` の、移動先のアンカーの一覧（JSON）。
#[wasm_bindgen]
pub fn legal_targets(state: &str, piece: u32) -> Result<String, JsError> {
    queries::legal_targets(state, piece).map_err(|e| JsError::new(&e))
}

/// 種類 `kind` の駒を置ける（アンカー, 向き）の一覧（JSON）。
#[wasm_bindgen]
pub fn legal_placements(state: &str, kind: u32) -> Result<String, JsError> {
    queries::legal_placements(state, kind).map_err(|e| JsError::new(&e))
}

/// 行動（JSON）を適用した結果（出来事と、その後の状態）のプレビュー（JSON）。
/// 脱落の順位は近似（読み取り用）。
#[wasm_bindgen]
pub fn preview(state: &str, action: &str) -> Result<String, JsError> {
    queries::preview(state, action).map_err(|e| JsError::new(&e))
}

/// 捕食される危険のある駒の番号の一覧（JSON）。
#[wasm_bindgen]
pub fn threatened(state: &str) -> Result<String, JsError> {
    queries::threatened(state).map_err(|e| JsError::new(&e))
}

#[cfg(test)]
mod tests;
