//! AI。仕様は docs/specs/online.md の 7 章。`rules` だけに依存し、人間と同じルールで動く。
//!
//! - `choose_pick`: 引いた 4 枚から、持ち駒にする 1 枚を選ぶ。
//! - `choose_action`: 配置・移動・回転・パスのうち、行動を 1 つ選ぶ。
//!
//! 乱数は呼び出し側から渡す（同じシードと同じ状態からは、同じ手になる）。

pub mod eval;

use std::time::Instant;

use rules::action::{Action, Event, apply, legal_actions};
use rules::pieces::Catalog;
use rules::rng::Rng;
use rules::state::{GameState, Phase, PlayerId, Stage};

use crate::eval::{evaluate, kind_value};

/// AI の強さ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 合法な行動から、ほぼランダムに選ぶ。捕食できる行動があれば、その中から選ぶ。
    One,
    /// 自分の 1 手先を、評価関数で読んで、最もよい行動を選ぶ。
    Two,
    /// 数手先まで探索する（まだ実装していない。いまは `Two` と同じ動き）。
    Three,
}

impl Level {
    /// 設定の値（1〜3）から作る。
    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            _ => None,
        }
    }
}

/// 引いた 4 枚から、持ち駒にする 1 枚の番号を選ぶ。選択待ちの段階でなければ `None`。
pub fn choose_pick(
    catalog: &Catalog,
    state: &GameState,
    level: Level,
    rng: &mut impl Rng,
) -> Option<usize> {
    let Stage::Pick(cards) = &state.stage else {
        return None;
    };
    if cards.is_empty() {
        return None;
    }
    match level {
        Level::One => Some(rng.below(cards.len() as u64) as usize),
        Level::Two | Level::Three => {
            // 評価の高い駒。同じ値なら、ランダムに選ぶ
            let values: Vec<f64> = cards
                .iter()
                .map(|kind| catalog.get(*kind).map_or(f64::MIN, kind_value))
                .collect();
            Some(pick_best(&values, rng))
        }
    }
}

/// 行動を 1 つ選ぶ。合法な行動がなければ（ドロー待ち、終局後など）`None`。
/// `deadline` を過ぎたら、そこまでに調べた中で最もよい行動を返す。
pub fn choose_action(
    catalog: &Catalog,
    state: &GameState,
    level: Level,
    deadline: Instant,
    rng: &mut impl Rng,
) -> Option<Action> {
    let actions = legal_actions(catalog, state);
    if actions.is_empty() {
        return None;
    }
    let me = state.current;
    match level {
        Level::One => Some(random_with_capture_preference(
            catalog, state, &actions, me, rng,
        )),
        Level::Two | Level::Three => {
            Some(best_by_one_ply(catalog, state, actions, me, deadline, rng))
        }
    }
}

/// 値の最大のものの番号。最大が複数あるときは、ランダムに 1 つ。
fn pick_best(values: &[f64], rng: &mut impl Rng) -> usize {
    let best = values.iter().copied().fold(f64::MIN, f64::max);
    let ties: Vec<usize> = values
        .iter()
        .enumerate()
        .filter(|(_, v)| (best - **v).abs() < 1e-9)
        .map(|(i, _)| i)
        .collect();
    ties[rng.below(ties.len() as u64) as usize]
}

/// 強さ 1: 敵の駒を捕食できる行動があれば、その中から。なければ、すべての行動から、ランダムに選ぶ。
fn random_with_capture_preference(
    catalog: &Catalog,
    state: &GameState,
    actions: &[Action],
    me: PlayerId,
    rng: &mut impl Rng,
) -> Action {
    let captures: Vec<&Action> = actions
        .iter()
        .filter(|action| captures_an_enemy(catalog, state, action, me))
        .collect();
    let pool: Vec<&Action> = if captures.is_empty() {
        actions.iter().collect()
    } else {
        captures
    };
    pool[rng.below(pool.len() as u64) as usize].clone()
}

fn captures_an_enemy(catalog: &Catalog, state: &GameState, action: &Action, me: PlayerId) -> bool {
    if !matches!(action, Action::Move { .. }) {
        return false;
    }
    apply(catalog, state, action).is_ok_and(|(_, events)| {
        events.iter().any(|event| match event {
            Event::Captured { piece, .. } => state.piece(*piece).is_some_and(|p| p.owner != me),
            _ => false,
        })
    })
}

/// 強さ 2 が評価する配置の候補の上限。終盤は、持ち駒の種類 × 置ける場所 × 4 方向で数千通りになり、
/// 全部を評価すると、時間がかかりすぎる。配置の候補は、似た位置が多いので、超えたら間引く。
/// 移動と回転は、間引かない。
const MAX_PLACEMENTS: usize = 150;

/// 配置の候補が `MAX_PLACEMENTS` を超えるとき、ランダムに間引く（シードから決まる）。
fn thin_out_placements(actions: Vec<Action>, rng: &mut impl Rng) -> Vec<Action> {
    let place_count = actions
        .iter()
        .filter(|a| matches!(a, Action::Place { .. }))
        .count();
    if place_count <= MAX_PLACEMENTS {
        return actions;
    }
    let (mut places, others): (Vec<Action>, Vec<Action>) = actions
        .into_iter()
        .partition(|a| matches!(a, Action::Place { .. }));
    {
        // 先頭から MAX_PLACEMENTS 個を、ランダムに選ぶ（部分的なフィッシャー–イェーツのシャッフル）
        for i in 0..MAX_PLACEMENTS {
            let j = i + rng.below((places.len() - i) as u64) as usize;
            places.swap(i, j);
        }
        places.truncate(MAX_PLACEMENTS);
    }
    others.into_iter().chain(places).collect()
}

/// フィッシャー–イェーツのシャッフル（シードから決まる）。
fn shuffle<T>(items: &mut [T], rng: &mut impl Rng) {
    for i in (1..items.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

/// 強さ 2: それぞれの行動を適用した直後の局面を評価して、最もよいものを選ぶ。
fn best_by_one_ply(
    catalog: &Catalog,
    state: &GameState,
    actions: Vec<Action>,
    me: PlayerId,
    deadline: Instant,
    rng: &mut impl Rng,
) -> Action {
    // 配置フェーズの城は、位置の評価が別なので、間引かない（城の候補は少ない）
    let mut actions = if state.phase == Phase::Placement {
        actions
    } else {
        thin_out_placements(actions, rng)
    };
    // 評価する順序を、ランダムにする。時間切れで途中までしか評価できなくても、行動の種類に偏らない
    shuffle(&mut actions, rng);
    let mut scored: Vec<(Action, f64)> = Vec::with_capacity(actions.len());
    for action in actions {
        // 時間切れでも、最低 1 つは評価する
        if !scored.is_empty() && Instant::now() >= deadline {
            break;
        }
        let score = if state.phase == Phase::Placement {
            // 合法な行動だけを列挙しているので、適用して確かめる必要はない
            placement_score(catalog, state, &action)
        } else {
            match apply(catalog, state, &action) {
                Ok((next, _)) => evaluate(catalog, &next, me),
                Err(_) => f64::MIN,
            }
        };
        scored.push((action, score));
    }
    let values: Vec<f64> = scored.iter().map(|(_, s)| *s).collect();
    let index = pick_best(&values, rng);
    scored.swap_remove(index).0
}

/// 配置フェーズの城: ほかの城から離れた位置を選ぶ（すきまが大きいほどよい）。
fn placement_score(catalog: &Catalog, state: &GameState, action: &Action) -> f64 {
    let Action::Place {
        kind,
        anchor,
        facing,
    } = action
    else {
        return f64::MIN;
    };
    let Some(def) = catalog.get(*kind) else {
        return f64::MIN;
    };
    let rect = def.footprint(*anchor, *facing);
    state
        .pieces
        .iter()
        .filter(|p| p.kind == catalog.castle())
        .filter_map(|p| state.footprint(catalog, p))
        .map(|castle| f64::from(castle.gap(&rect)))
        .fold(f64::MAX, f64::min)
        .min(1e6)
}

#[cfg(test)]
mod tests;
