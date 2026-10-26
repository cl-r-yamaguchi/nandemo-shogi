//! 強さ 3 の探索。自分の行動を根として、相手の行動を数手先まで読む。
//! 仕様は docs/specs/online.md の 7 章。
//!
//! - 相手が複数でも、全員が自分の評価を下げるように動くとして読む（最悪の場合を想定する）。
//! - ドローの乱数は読めないので、相手は、盤上の駒の移動だけを行うものとして読む。
//! - 反復深化: 浅い探索から深くしていく。締め切りを過ぎたら、終えた最も深い結果で指す。

use std::time::Instant;

use rules::action::{Action, apply, legal_targets, strike_damage};
use rules::pieces::Catalog;
use rules::state::{GameState, Phase, PlayerId, Stage};

use crate::TIE;
use crate::eval::{ENEMY_CASTLE_HP, evaluate, piece_value};

/// 探索の最大の深さ（手数。自分の手と、相手の手を、合わせて数える）。
pub const MAX_DEPTH: u32 = 3;

/// 相手の手番（と、根より深い自分の手番）で、読む手の数の上限。捕食を優先して並べた上位だけを読む。
const MAX_BRANCH: usize = 10;

/// 根の手と、その評価値。
pub(crate) type Scored = Vec<(Action, f64)>;

/// 根の手 `actions` を、深さ 2 から `MAX_DEPTH` まで、反復深化で読む。
/// `first` は、深さ 1（直後の局面の評価）の結果。締め切りまでに終えた最も深い結果を返す。
pub(crate) fn deepen(
    catalog: &Catalog,
    state: &GameState,
    first: Scored,
    me: PlayerId,
    deadline: Instant,
) -> Scored {
    let mut best = first;
    for depth in 2..=MAX_DEPTH {
        // 前の深さで評価の高かった手から読む（枝刈りが効きやすい）
        let mut order = best.clone();
        order.sort_by(|a, b| b.1.total_cmp(&a.1));
        match search_root(catalog, state, order, depth, me, deadline) {
            Some(scored) => best = scored,
            None => break, // 時間切れ: この深さの途中の結果は捨てる
        }
    }
    best
}

/// 根の手のそれぞれを、`depth` 手先まで読む。締め切りを過ぎたら `None`。
fn search_root(
    catalog: &Catalog,
    state: &GameState,
    order: Scored,
    depth: u32,
    me: PlayerId,
    deadline: Instant,
) -> Option<Scored> {
    let mut result: Scored = Vec::with_capacity(order.len());
    let mut best = f64::NEG_INFINITY;
    for (action, _) in order {
        let value = match apply(catalog, state, &action) {
            // 同点の手も正確な値を得るため、最善より少しだけ低い値を、下限にする
            Ok((child, _)) => search(
                catalog,
                child,
                depth - 1,
                best - TIE,
                f64::INFINITY,
                me,
                deadline,
            )?,
            Err(_) => f64::NEG_INFINITY,
        };
        best = best.max(value);
        result.push((action, value));
    }
    Some(result)
}

/// アルファ・ベータ法。`me` から見た値を返す。締め切りを過ぎたら `None`。
fn search(
    catalog: &Catalog,
    mut state: GameState,
    depth: u32,
    mut alpha: f64,
    mut beta: f64,
    me: PlayerId,
    deadline: Instant,
) -> Option<f64> {
    if Instant::now() >= deadline {
        return None;
    }
    if state.phase == Phase::Finished {
        // 早い勝ちを好み、遅い負けを好む
        // 残りの深さが大きいほど、早く決着している。引き分け（0）には、加減しない
        let value = evaluate(catalog, &state, me);
        let bonus = if value > 0.0 {
            f64::from(depth)
        } else if value < 0.0 {
            -f64::from(depth)
        } else {
            0.0
        };
        return Some(value + bonus);
    }
    if depth == 0 {
        return Some(evaluate(catalog, &state, me));
    }
    // 相手の手は、盤上の駒の移動だけを読む。ドロー・選択は、行動待ちとして扱う
    state.stage = Stage::Act;
    let moves = ordered_moves(catalog, &state);
    if moves.is_empty() {
        // 動かせる駒がない: 手番を飛ばす
        state.advance_current();
        return search(catalog, state, depth - 1, alpha, beta, me, deadline);
    }
    let maximizing = state.current == me;
    let mut value = if maximizing {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    };
    for action in moves {
        let Ok((child, _)) = apply(catalog, &state, &action) else {
            continue;
        };
        let child_value = search(catalog, child, depth - 1, alpha, beta, me, deadline)?;
        if maximizing {
            value = value.max(child_value);
            alpha = alpha.max(value);
        } else {
            value = value.min(child_value);
            beta = beta.min(value);
        }
        if alpha >= beta {
            break;
        }
    }
    Some(value)
}

/// 手番のプレイヤーの、盤上の駒の移動を、読む価値の高い順に並べた上位 `MAX_BRANCH` 個。
/// 価値は、捕食できる敵の駒の価値（味方の捕食は減点）。同じ価値なら、元の順序を保つ。
fn ordered_moves(catalog: &Catalog, state: &GameState) -> Vec<Action> {
    let current = state.current;
    let mut moves: Vec<(Action, f64)> = Vec::new();
    for piece in state.pieces.iter().filter(|p| p.owner == current) {
        for to in legal_targets(catalog, state, piece.id) {
            let mut gain = 0.0;
            for (victim_id, damage) in strike_damage(catalog, state, piece, to) {
                let Some(victim) = state.piece(victim_id) else {
                    continue;
                };
                let Some(def) = catalog.get(victim.kind) else {
                    continue;
                };
                let value = if def.is_castle() {
                    ENEMY_CASTLE_HP * f64::from(victim.hp.min(damage))
                } else {
                    piece_value(def, victim.hp)
                };
                // 殺せる駒の価値。殺せないときは、ダメージの割合だけ
                let share = (f64::from(damage) / f64::from(victim.hp)).min(1.0);
                let signed = if victim.owner == current { -1.0 } else { 1.0 };
                gain += signed * share * value;
            }
            moves.push((
                Action::Move {
                    piece: piece.id,
                    to,
                    promote: false,
                },
                gain,
            ));
        }
    }
    moves.sort_by(|a, b| b.1.total_cmp(&a.1));
    moves.truncate(MAX_BRANCH);
    moves.into_iter().map(|(action, _)| action).collect()
}
