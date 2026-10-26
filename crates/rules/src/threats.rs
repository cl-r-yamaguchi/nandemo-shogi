//! 危険な駒（次の一手で捕食されうる駒）の判定。仕様は docs/specs/game-rules.md の 8 章（R-A0〜R-A2）。
//! 画面の「！マーク」と、AI の評価が使う。状態は変更しない。

use std::collections::{BTreeMap, BTreeSet};

use crate::action::{legal_targets, rects, strike_damage_in};
use crate::pieces::Catalog;
use crate::state::{GameState, PieceId};

/// 捕食されうる駒ごとに、その駒を 1 回の移動で捕食できる攻撃側の駒の一覧。
///
/// 攻撃側は、脱落していない、持ち主以外のプレイヤーの盤上の駒。その駒が動ける先（`legal_targets`）の
/// うち、重なった駒に、（重なったマス数）×（攻撃力）のダメージを与えて、その駒の現在の HP 以上に
/// なるものがあれば、その駒は、捕食されうる。手番や、手番の段階は見ない。
pub fn threats(catalog: &Catalog, state: &GameState) -> BTreeMap<PieceId, Vec<PieceId>> {
    let mut found: BTreeMap<PieceId, BTreeSet<PieceId>> = BTreeMap::new();
    // 占有範囲は、盤面が同じ間は変わらないので、1 回だけ作って、使い回す
    let all = rects(catalog, state);
    for attacker in &state.pieces {
        if state.players[usize::from(attacker.owner.0)].eliminated {
            continue;
        }
        for target in legal_targets(catalog, state, attacker.id) {
            for (victim_id, damage) in strike_damage_in(catalog, &all, attacker, target) {
                let Some(victim) = state.piece(victim_id) else {
                    continue;
                };
                let victim_alive = !state.players[usize::from(victim.owner.0)].eliminated;
                if victim_alive && victim.owner != attacker.owner && damage >= victim.hp {
                    found.entry(victim_id).or_default().insert(attacker.id);
                }
            }
        }
    }
    found
        .into_iter()
        .map(|(victim, attackers)| (victim, attackers.into_iter().collect()))
        .collect()
}

/// 次の一手で捕食されうる駒（画面の「！マーク」を付ける駒）。
pub fn threatened_pieces(catalog: &Catalog, state: &GameState) -> BTreeSet<PieceId> {
    threats(catalog, state).into_keys().collect()
}

#[cfg(test)]
mod tests;
