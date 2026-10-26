//! 手番の最初の段階（ドロー）と、時間切れの処理。仕様は docs/specs/game-rules.md の 3.1〜3.3 章。
//!
//! 選択は `Action::Pick`（`action::apply`）で行う。時間そのものの計測は、サーバーが行う（R-32）。

use crate::action::{Action, ActionError, Event, apply, legal_placements};
use crate::finish;
use crate::pieces::{Catalog, KindId};
use crate::rng::Rng;
use crate::state::{GameState, Phase, Stage};

/// R-05: 1 回のドローで引く枚数。
pub const DRAW_COUNT: usize = 4;

/// 出現重みに比例した確率で、駒を 1 つ引く。出現重み 0 の駒（城）は引かれない。
fn draw_one(catalog: &Catalog, rng: &mut impl Rng) -> KindId {
    let total: u64 = catalog.iter().map(|(_, d)| u64::from(d.weight)).sum();
    // `Catalog` が、出現重みが 1 以上の駒を 1 つ以上持つことを検証済み
    let mut target = rng.below(total);
    for (kind, def) in catalog.iter() {
        let weight = u64::from(def.weight);
        if target < weight {
            return kind;
        }
        target -= weight;
    }
    unreachable!("出現重みの合計の範囲内で選ぶので、必ず見つかる")
}

/// R-05: 駒を毎回独立に `DRAW_COUNT` 枚引き、選択待ちの段階へ進める。
/// 対戦フェーズで、ドロー待ちの段階のときだけできる。
pub fn draw_cards(
    catalog: &Catalog,
    state: &GameState,
    rng: &mut impl Rng,
) -> Result<(GameState, Vec<Event>), ActionError> {
    if state.phase != Phase::Battle {
        return Err(ActionError::WrongPhase);
    }
    if state.stage != Stage::Draw {
        return Err(ActionError::WrongStage);
    }
    let mut next = state.clone();
    let cards: Vec<KindId> = (0..DRAW_COUNT).map(|_| draw_one(catalog, rng)).collect();
    let event = Event::Drawn {
        player: next.current,
        cards: cards.clone(),
    };
    next.stage = Stage::Pick(cards);
    debug_assert_eq!(next.validate(catalog), Ok(()));
    Ok((next, vec![event]))
}

/// R-31: 持ち時間を過ぎたときの処理。手番を強制的に終える。
///
/// サーバーは、手番ごとに、タイマーを 1 つだけ持ち、時間切れのときに 1 回だけ呼ぶ
/// （手番番号 `state.turn` と、手番のプレイヤーで、そのタイマーの手番か照合する）。
/// 行動が確定した直後に呼ぶと、次の手番を強制的に終えてしまう。
/// - 配置フェーズ: 城を置ける位置から、ランダムに 1 か所に置く（R-14）。
/// - 対戦フェーズ: 選択がまだなら、引いた 4 枚からランダムに 1 枚を持ち駒にし、行動はパスする。
pub fn time_out(
    catalog: &Catalog,
    state: &GameState,
    rng: &mut impl Rng,
) -> Result<(GameState, Vec<Event>), ActionError> {
    let player = state.current;
    match state.phase {
        Phase::Finished => Err(ActionError::WrongPhase),
        Phase::Placement => {
            let spots = legal_placements(catalog, state, catalog.castle());
            if spots.is_empty() {
                // 城を置けないまま手番を進めると、配置フェーズが終わらなくなる。
                // 呼び出し側が、そのプレイヤーを `resign` で脱落させる。
                return Err(ActionError::NoLegalPlacement);
            }
            let (anchor, facing) = spots[rng.below(spots.len() as u64) as usize];
            let place = Action::Place {
                kind: catalog.castle(),
                anchor,
                facing,
            };
            let (next, mut events) = apply(catalog, state, &place)?;
            events.insert(0, Event::TimedOut { player });
            Ok((next, events))
        }
        Phase::Battle => {
            let mut next = state.clone();
            let mut events = Vec::new();
            if next.stage == Stage::Draw {
                let (drawn, drawn_events) = draw_cards(catalog, &next, rng)?;
                next = drawn;
                events.extend(drawn_events);
            }
            if let Stage::Pick(cards) = &next.stage {
                let index = rng.below(cards.len() as u64) as usize;
                let (picked, picked_events) = apply(catalog, &next, &Action::Pick { index })?;
                next = picked;
                events.extend(picked_events);
            }
            // 行動はパスする（行える行動があっても、強制的に終える）
            events.push(Event::TimedOut { player });
            finish::settle_after_action(catalog, &mut next, &mut events);
            debug_assert_eq!(next.validate(catalog), Ok(()));
            Ok((next, events))
        }
    }
}

#[cfg(test)]
mod tests;
