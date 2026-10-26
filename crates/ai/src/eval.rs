//! 局面の評価。`me` から見た、有利さの値（大きいほど有利）。仕様は docs/specs/online.md の 7 章。

use rules::pieces::{Catalog, PieceDef};
use rules::state::{GameState, Phase, PlayerId};
use rules::threats::threatened_pieces;

/// 勝ち（負け）の値。通常の評価値より、十分に大きい。
pub const WIN: f64 = 1_000_000.0;

const OWN_CASTLE_HP: f64 = 5.0;
const ENEMY_CASTLE_HP: f64 = 4.0;
const HAND_WEIGHT: f64 = 0.5;
const THREATENED_OWN: f64 = 0.6;
const THREATENED_ENEMY: f64 = 0.3;

/// 駒の価値。攻撃力・大きさ・HP から決める。城は別に数えるので、ここでは扱わない。
pub fn piece_value(def: &PieceDef, hp: u32) -> f64 {
    let area = f64::from(def.size[0] * def.size[1]);
    1.0 + 0.8 * f64::from(def.attack) * area + 0.5 * f64::from(hp)
}

/// 駒の種類としての価値（HP は最大値）。ドローで引いた駒の比較に使う。
pub fn kind_value(def: &PieceDef) -> f64 {
    piece_value(def, def.hp)
}

/// `me` から見た局面の評価。終局していれば、勝ち・負け・引き分けの値。
pub fn evaluate(catalog: &Catalog, state: &GameState, me: PlayerId) -> f64 {
    if state.phase == Phase::Finished {
        return finished_value(state, me);
    }
    let opponents = state
        .alive_players()
        .into_iter()
        .filter(|p| *p != me)
        .count()
        .max(1) as f64;
    let mut score = 0.0;

    for piece in &state.pieces {
        let Some(def) = catalog.get(piece.kind) else {
            continue;
        };
        let mine = piece.owner == me;
        if def.is_castle() {
            score += if mine {
                OWN_CASTLE_HP * f64::from(piece.hp)
            } else {
                -ENEMY_CASTLE_HP * f64::from(piece.hp) / opponents
            };
        } else {
            let value = piece_value(def, piece.hp);
            score += if mine { value } else { -value / opponents };
        }
    }

    for (kind, count) in &state.players[usize::from(me.0)].hand {
        if let Some(def) = catalog.get(*kind)
            && !def.is_castle()
        {
            score += HAND_WEIGHT * kind_value(def) * f64::from(*count);
        }
    }

    for id in threatened_pieces(catalog, state) {
        let Some(piece) = state.piece(id) else {
            continue;
        };
        let Some(def) = catalog.get(piece.kind) else {
            continue;
        };
        let mine = piece.owner == me;
        let value = match (def.is_castle(), mine) {
            (true, true) => OWN_CASTLE_HP * f64::from(piece.hp),
            (true, false) => ENEMY_CASTLE_HP * f64::from(piece.hp),
            (false, _) => piece_value(def, piece.hp),
        };
        score += if mine {
            -THREATENED_OWN * value
        } else {
            THREATENED_ENEMY * value / opponents
        };
    }
    score
}

/// 終局の値。1 位が自分だけなら勝ち、1 位に自分を含む複数なら引き分け、それ以外は負け。
fn finished_value(state: &GameState, me: PlayerId) -> f64 {
    let Some(outcome) = &state.outcome else {
        return 0.0;
    };
    match (outcome.winners().contains(&me), outcome.winners().len()) {
        (true, 1) => WIN,
        (true, _) => 0.0,
        (false, _) => -WIN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rules::finish::resign;
    use rules::geometry::{Facing, Pos};
    use rules::state::{Piece, PieceId, Stage};

    fn catalog() -> Catalog {
        Catalog::builtin().unwrap()
    }

    fn board(catalog: &Catalog) -> GameState {
        let mut state = GameState::new(catalog, 2, PlayerId(0)).unwrap();
        state.phase = Phase::Battle;
        state.stage = Stage::Act;
        state.players.iter_mut().for_each(|p| p.hand.clear());
        put(&mut state, catalog, "castle", 0, 0, 0);
        put(&mut state, catalog, "castle", 1, 13, 13);
        state
    }

    fn put(
        state: &mut GameState,
        catalog: &Catalog,
        id: &str,
        owner: u8,
        x: i32,
        y: i32,
    ) -> PieceId {
        let kind = catalog.kind_of(id).unwrap();
        let hp = catalog.get(kind).unwrap().hp;
        let piece_id = state.issue_piece_id();
        state.pieces.push(Piece {
            id: piece_id,
            kind,
            owner: PlayerId(owner),
            anchor: Pos::new(x, y),
            facing: Facing::North,
            hp,
        });
        piece_id
    }

    #[test]
    fn a_symmetric_position_is_even() {
        let catalog = catalog();
        let state = board(&catalog);
        let a = evaluate(&catalog, &state, PlayerId(0));
        let b = evaluate(&catalog, &state, PlayerId(1));
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn more_pieces_castle_hp_and_hand_are_better_for_me() {
        let catalog = catalog();
        let base = board(&catalog);
        let me = PlayerId(0);
        let base_score = evaluate(&catalog, &base, me);

        let mut with_piece = base.clone();
        put(&mut with_piece, &catalog, "bear", 0, 8, 8);
        assert!(evaluate(&catalog, &with_piece, me) > base_score);

        let mut with_hand = base.clone();
        *with_hand.players[0]
            .hand
            .entry(catalog.kind_of("lion").unwrap())
            .or_insert(0) += 1;
        assert!(evaluate(&catalog, &with_hand, me) > base_score);

        let mut enemy_hurt = base.clone();
        enemy_hurt.pieces[1].hp = 3;
        assert!(evaluate(&catalog, &enemy_hurt, me) > base_score);

        let mut me_hurt = base.clone();
        me_hurt.pieces[0].hp = 3;
        assert!(evaluate(&catalog, &me_hurt, me) < base_score);
    }

    #[test]
    fn a_hanging_piece_is_worth_less() {
        let catalog = catalog();
        let mut safe = board(&catalog);
        put(&mut safe, &catalog, "kid-elephant", 0, 4, 4);
        put(&mut safe, &catalog, "lion", 1, 10, 10);
        let mut hanging = board(&catalog);
        put(&mut hanging, &catalog, "kid-elephant", 0, 4, 4);
        put(&mut hanging, &catalog, "lion", 1, 5, 5); // 子供象の隣: 捕食される
        let me = PlayerId(0);
        assert!(evaluate(&catalog, &hanging, me) < evaluate(&catalog, &safe, me));
    }

    #[test]
    fn a_finished_game_is_win_loss_or_draw() {
        let catalog = catalog();
        let state = board(&catalog);
        let (done, _) = resign(&catalog, &state, PlayerId(1)).unwrap();
        assert_eq!(evaluate(&catalog, &done, PlayerId(0)), WIN);
        assert_eq!(evaluate(&catalog, &done, PlayerId(1)), -WIN);
    }

    #[test]
    fn piece_value_grows_with_attack_size_and_hp() {
        let catalog = catalog();
        let def = |id: &str| catalog.get(catalog.kind_of(id).unwrap()).unwrap();
        assert!(kind_value(def("lion")) > kind_value(def("kid-lion")));
        assert!(kind_value(def("elephant")) > kind_value(def("kid-elephant")));
        assert!(piece_value(def("bear"), 6) > piece_value(def("bear"), 2));
    }
}
