//! 脱落・終局・順位・投了。仕様は docs/specs/game-rules.md の 5 章（R-90〜R-97）。

use std::collections::BTreeMap;

use crate::action::{ActionError, Event};
use crate::pieces::Catalog;
use crate::state::{GameState, Phase, PlayerId, Stage};

/// R-95: 手番数の上限は、1 人あたりこの回数。上限は「開始時の人数 × この回数」で、
/// 途中で脱落者が出ても変わらない。
pub const TURNS_PER_PLAYER: u32 = 100;

/// 終局の理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// 生き残りが 1 人になった（投了を含む）（R-93）。
    LastStanding,
    /// 同時に脱落して、生き残りがいなくなった（R-94）。
    AllEliminated,
    /// 手番数の上限に達した（R-95）。
    TurnLimit,
}

/// 終局の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub reason: EndReason,
    /// 順位ごとの組。先頭が 1 位。同順位の人は、同じ組に席の順に入る（R-97）。
    pub ranking: Vec<Vec<PlayerId>>,
}

impl Outcome {
    /// 1 位の人たち。1 人なら勝者、複数なら引き分け。
    pub fn winners(&self) -> &[PlayerId] {
        self.ranking.first().map_or(&[], Vec::as_slice)
    }

    pub fn is_draw(&self) -> bool {
        self.winners().len() != 1
    }
}

/// 脱落した人の駒と持ち駒を消し、脱落の記録に加える（R-91）。
fn eliminate(state: &mut GameState, players: &[PlayerId], events: &mut Vec<Event>) {
    // 存在しない席と、脱落済みの席は無視する（二重に脱落させない）
    let players: Vec<PlayerId> = players
        .iter()
        .copied()
        .filter(|p| {
            state
                .players
                .get(usize::from(p.0))
                .is_some_and(|s| !s.eliminated)
        })
        .collect();
    if players.is_empty() {
        return;
    }
    let players = players.as_slice();
    for player in players {
        state.players[usize::from(player.0)].eliminated = true;
        state.players[usize::from(player.0)].hand.clear();
        events.push(Event::Eliminated { player: *player });
    }
    state.pieces.retain(|p| !players.contains(&p.owner));
    state.out_batches.push(players.to_vec());
}

/// プレイヤーの城の HP（盤上になければ 0）。
fn castle_hp(catalog: &Catalog, state: &GameState, player: PlayerId) -> u32 {
    state
        .pieces
        .iter()
        .filter(|p| p.owner == player && p.kind == catalog.castle())
        .map(|p| p.hp)
        .sum()
}

fn finish(catalog: &Catalog, state: &mut GameState, reason: EndReason, events: &mut Vec<Event>) {
    let mut ranking: Vec<Vec<PlayerId>> = Vec::new();
    let alive = state.alive_players();
    if !alive.is_empty() {
        if reason == EndReason::TurnLimit {
            // R-95: 残っている城の HP が大きい順。同じ値は同順位。
            let mut by_hp: BTreeMap<u32, Vec<PlayerId>> = BTreeMap::new();
            for player in alive {
                by_hp
                    .entry(castle_hp(catalog, state, player))
                    .or_default()
                    .push(player);
            }
            ranking.extend(by_hp.into_values().rev());
        } else {
            ranking.push(alive);
        }
    }
    // R-97: 脱落した人は、遅く脱落した順。同時に脱落した人は同順位。
    ranking.extend(state.out_batches.iter().rev().cloned().map(|mut batch| {
        batch.sort();
        batch
    }));
    let outcome = Outcome { reason, ranking };
    state.phase = Phase::Finished;
    // 終局後は、手番の段階を使わない（ドロー待ち・選択待ちを残さない）
    state.stage = Stage::Act;
    state.outcome = Some(outcome.clone());
    events.push(Event::Finished { outcome });
}

/// R-93・R-94: 生き残りが 1 人以下なら、終局にする。終局したら `true`。
fn finish_if_decided(catalog: &Catalog, state: &mut GameState, events: &mut Vec<Event>) -> bool {
    match state.alive_players().len() {
        0 => finish(catalog, state, EndReason::AllEliminated, events),
        1 => finish(catalog, state, EndReason::LastStanding, events),
        _ => return false,
    }
    true
}

/// 行動の適用のあとの処理: 脱落・終局の判定、手番を回す、手番数の上限（R-22, R-90〜R-95）。
pub(crate) fn settle_after_action(
    catalog: &Catalog,
    state: &mut GameState,
    events: &mut Vec<Event>,
) {
    // R-90: 対戦フェーズでは、城が 1 つもないプレイヤーは脱落する。
    if state.phase == Phase::Battle {
        let castle = catalog.castle();
        let castleless: Vec<PlayerId> = state
            .alive_players()
            .into_iter()
            .filter(|player| {
                !state
                    .pieces
                    .iter()
                    .any(|p| p.owner == *player && p.kind == castle)
            })
            .collect();
        eliminate(state, &castleless, events);
    }
    if finish_if_decided(catalog, state, events) {
        return;
    }
    advance_phase_and_turn(catalog, state);
    // R-95
    let limit = state.players.len() as u32 * TURNS_PER_PLAYER;
    if state.phase == Phase::Battle && state.turn >= limit {
        finish(catalog, state, EndReason::TurnLimit, events);
    }
}

/// 全員が城を置いたら、対戦フェーズへ進める（R-12）。生き残りの席は、持ち駒に城が残っていない。
fn start_battle_if_all_placed(catalog: &Catalog, state: &mut GameState) {
    let castle = catalog.castle();
    let all_placed = state
        .players
        .iter()
        .all(|p| p.hand.get(&castle).copied().unwrap_or(0) == 0);
    if state.phase == Phase::Placement && all_placed {
        state.phase = Phase::Battle;
        state.turn = 0;
    }
}

/// R-11・R-12・R-22: フェーズの移行と、次の席への手番の移動。
fn advance_phase_and_turn(catalog: &Catalog, state: &mut GameState) {
    match state.phase {
        Phase::Placement => start_battle_if_all_placed(catalog, state),
        Phase::Battle => state.turn += 1,
        Phase::Finished => return,
    }
    state.advance_current();
    reset_stage(state);
}

/// 手番が変わったときの、手番の段階。対戦フェーズでは、新しい手番は、ドローから始まる（3.2）。
/// 配置フェーズは、段階を使わない。
fn reset_stage(state: &mut GameState) {
    state.stage = if state.phase == Phase::Battle {
        Stage::Draw
    } else {
        Stage::Act
    };
}

/// R-96: プレイヤーが投了する。手番でなくても、いつでもできる（終局後を除く）。
/// 手番のプレイヤーが投了したときは、手番を次へ回す。
pub fn resign(
    catalog: &Catalog,
    state: &GameState,
    player: PlayerId,
) -> Result<(GameState, Vec<Event>), ActionError> {
    if state.phase == Phase::Finished {
        return Err(ActionError::WrongPhase);
    }
    let alive = state
        .players
        .get(usize::from(player.0))
        .map(|p| !p.eliminated)
        .ok_or(ActionError::UnknownPlayer)?;
    if !alive {
        return Err(ActionError::UnknownPlayer);
    }
    let mut next = state.clone();
    let mut events = Vec::new();
    eliminate(&mut next, &[player], &mut events);
    if !finish_if_decided(catalog, &mut next, &mut events) {
        let was_current = next.current == player;
        if was_current {
            next.advance_current();
        }
        // 配置フェーズで、残りの全員が城を置き終えていたら、対戦フェーズへ進む
        start_battle_if_all_placed(catalog, &mut next);
        // 手番のプレイヤーが変わった、または対戦が始まったときは、ドローから始める
        if was_current || next.phase != state.phase {
            reset_stage(&mut next);
        }
    }
    debug_assert_eq!(next.validate(catalog), Ok(()));
    Ok((next, events))
}

#[cfg(test)]
mod tests;
