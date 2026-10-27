//! 対局の記録（純粋なデータ）と、記録からの再現。保存は `store`。仕様は docs/specs/online.md の 8 章。

use protocol::{RoomRules, SeatKind};
use rules::action::apply;
use rules::finish::{Outcome, resign};
use rules::pieces::Catalog;
use rules::rng::{Rng, SplitMix64};
use rules::state::{GameState, PlayerId};
use rules::turn::{draw_cards, time_out};

use crate::room::{HistoryEntry, Move};

/// 席の記録。プレイヤー ID（本人だけが知る合言葉）は、含めない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatRecord {
    pub kind: SeatKind,
    pub name: String,
}

/// 終わった（または、中断した）対局の記録。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRecord {
    pub rules: RoomRules,
    /// ドローと最初の手番の乱数のシード（`record::replay` が、ここから再現する）。
    pub seed: u64,
    pub seats: Vec<SeatRecord>,
    pub history: Vec<HistoryEntry>,
    /// 終局の結果。終局しないまま、ルームが閉じたとき（全員の切断）は、`None`（中断）。
    pub outcome: Option<Outcome>,
}

/// 再現できなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// 席の数が、ルールの人数と合わない。
    BadSeats,
    /// `index` 番目の操作が、規則に合わない・手番番号や席が合わない。
    Mismatch { index: usize },
    /// 再現した結果（出来事・終局の結果）が、記録と一致しない。
    Diverged { index: Option<usize> },
}

/// 記録から、シードと操作の列をもとに、対局を最初から再現し、記録された出来事と結果が
/// 一致することを確かめる。一致したら、最後の状態を返す。
pub fn replay(catalog: &Catalog, record: &GameRecord) -> Result<GameState, ReplayError> {
    let players = usize::from(record.rules.players);
    if record.seats.len() != players {
        return Err(ReplayError::BadSeats);
    }
    let mut rng = SplitMix64(record.seed);
    let first = PlayerId(rng.below(players as u64) as u8);
    let mut state = GameState::new(catalog, players, first).ok_or(ReplayError::BadSeats)?;
    for (index, entry) in record.history.iter().enumerate() {
        if entry.turn != state.turn {
            return Err(ReplayError::Mismatch { index });
        }
        let stepped = match &entry.mv {
            Move::Draw => draw_cards(catalog, &state, &mut rng),
            Move::Act { action } => apply(catalog, &state, action),
            Move::TimeOut => time_out(catalog, &state, &mut rng),
            Move::Resign => resign(catalog, &state, entry.seat),
        };
        let (next, events) = stepped.map_err(|_| ReplayError::Mismatch { index })?;
        // 行動・時間切れの席は、その時点の手番のプレイヤー（投了は、誰でもよい）
        if !matches!(entry.mv, Move::Resign) && entry.seat != state.current {
            return Err(ReplayError::Mismatch { index });
        }
        if events != entry.events {
            return Err(ReplayError::Diverged { index: Some(index) });
        }
        state = next;
    }
    if state.outcome != record.outcome {
        return Err(ReplayError::Diverged { index: None });
    }
    Ok(state)
}

#[cfg(test)]
mod tests;
