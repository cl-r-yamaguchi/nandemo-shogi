use std::collections::BTreeSet;

use super::*;
use crate::action::{Action, Event, apply};
use crate::geometry::{Facing, Pos, Rect};
use crate::pieces::KindId;
use crate::rng::{Rng, SplitMix64};
use crate::state::{GameState, Phase, Piece, PlayerId, Stage};

fn catalog() -> Catalog {
    Catalog::builtin().unwrap()
}

/// 対戦フェーズの盤。手番や段階には依存しない。
fn battle(catalog: &Catalog, players: usize) -> GameState {
    let mut state = GameState::new(catalog, players, PlayerId(0)).unwrap();
    state.phase = Phase::Battle;
    state.stage = Stage::Act;
    state.players.iter_mut().for_each(|p| p.hand.clear());
    state
}

fn put_facing(
    state: &mut GameState,
    catalog: &Catalog,
    id: &str,
    owner: u8,
    (x, y): (i32, i32),
    facing: Facing,
) -> PieceId {
    let kind = catalog.kind_of(id).unwrap();
    let hp = catalog.get(kind).unwrap().hp;
    let piece_id = state.issue_piece_id();
    state.pieces.push(Piece {
        id: piece_id,
        kind,
        owner: PlayerId(owner),
        anchor: Pos::new(x, y),
        facing,
        hp,
    });
    piece_id
}

fn put(state: &mut GameState, catalog: &Catalog, id: &str, owner: u8, x: i32, y: i32) -> PieceId {
    put_facing(state, catalog, id, owner, (x, y), Facing::North)
}

fn set_hp(state: &mut GameState, id: PieceId, hp: u32) {
    state.pieces.iter_mut().find(|p| p.id == id).unwrap().hp = hp;
}

fn threatened(catalog: &Catalog, state: &GameState) -> BTreeSet<PieceId> {
    threatened_pieces(catalog, state)
}

#[test]
fn a_piece_that_can_be_captured_next_move_is_threatened() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "lion", 1, 5, 5); // 攻撃力 4
    let victim = put(&mut state, &catalog, "kid-elephant", 0, 5, 4); // HP 1、ライオンの隣
    let result = threats(&catalog, &state);
    assert_eq!(result.keys().copied().collect::<Vec<_>>(), [victim]);
    assert_eq!(threatened(&catalog, &state), BTreeSet::from([victim]));
}

#[test]
fn a_piece_out_of_range_is_not_threatened() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "lion", 1, 5, 5);
    put(&mut state, &catalog, "kid-elephant", 0, 5, 8); // ライオンは 1 マスしか動けない
    assert!(threatened(&catalog, &state).is_empty());
}

#[test]
fn damage_below_the_hp_does_not_make_a_piece_threatened() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "lion", 1, 5, 5); // 攻撃力 4
    let bear = put(&mut state, &catalog, "bear", 0, 5, 4); // HP 6: 1 回では、捕食されない
    assert!(!threatened(&catalog, &state).contains(&bear));
    set_hp(&mut state, bear, 4);
    assert!(threatened(&catalog, &state).contains(&bear));
}

#[test]
fn your_own_pieces_are_not_a_threat() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "lion", 0, 5, 5);
    put(&mut state, &catalog, "kid-elephant", 0, 5, 4);
    assert!(threatened(&catalog, &state).is_empty());
}

#[test]
fn any_other_player_counts_as_an_attacker_not_only_the_next_seat() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    put(&mut state, &catalog, "lion", 2, 5, 5); // 手番は 0 だが、席 2 の駒が狙う
    let victim = put(&mut state, &catalog, "kid-elephant", 1, 5, 4);
    assert_eq!(threatened(&catalog, &state), BTreeSet::from([victim]));
}

#[test]
fn eliminated_players_pieces_do_not_threaten() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    let victim = put(&mut state, &catalog, "kid-elephant", 0, 5, 4);
    put(&mut state, &catalog, "lion", 1, 5, 5);
    assert_eq!(threatened(&catalog, &state), BTreeSet::from([victim]));
    // 脱落したプレイヤーの駒は、盤から消える。残った状態では、脅威がない
    state.pieces.retain(|p| p.owner != PlayerId(1));
    state.players[1].eliminated = true;
    state.out_batches.push(vec![PlayerId(1)]);
    assert!(threatened(&catalog, &state).is_empty());
}

#[test]
fn attackers_are_listed_for_each_victim() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    let a = put(&mut state, &catalog, "lion", 1, 5, 5);
    let b = put(&mut state, &catalog, "lion", 2, 7, 3);
    let victim = put(&mut state, &catalog, "kid-elephant", 0, 6, 4);
    let result = threats(&catalog, &state);
    assert_eq!(result[&victim], [a, b]);
}

#[test]
fn big_pieces_deal_damage_per_overlapping_cell() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    // 象（攻撃力 2、2x2）は、2 マス先まで動ける。守る側の象を 3 マス先に置くと、
    // 届く最大の重なりは 2 マス（5,3 へ動いたとき、5..6 x 3..4 と 5..6 x 2..3 が 1 行で重なる）で、ダメージ 4
    put(&mut state, &catalog, "elephant", 1, 5, 5);
    let defender = put(&mut state, &catalog, "elephant", 0, 5, 2);
    set_hp(&mut state, defender, 4);
    assert!(threatened(&catalog, &state).contains(&defender));
    set_hp(&mut state, defender, 5);
    assert!(!threatened(&catalog, &state).contains(&defender));
}

#[test]
fn a_big_attacker_hits_a_small_piece_for_one_cell_only() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "elephant", 1, 5, 5); // 重なりは、小さい駒の 1 マスだけ。ダメージ 2
    let weak = put(&mut state, &catalog, "kid-lion", 0, 5, 3); // HP 1
    let bear = put(&mut state, &catalog, "bear", 0, 8, 5); // HP 6
    let result = threatened(&catalog, &state);
    assert!(result.contains(&weak));
    assert!(!result.contains(&bear));
}

#[test]
fn a_charging_penguin_threatens_the_first_piece_in_front() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "jet-penguin", 1, 5, 12); // 攻撃力 3
    let near = put(&mut state, &catalog, "lion", 0, 5, 6); // HP 2
    put(&mut state, &catalog, "kid-elephant", 0, 5, 2); // 手前の駒の向こう側: 当たらない
    assert_eq!(threatened(&catalog, &state), BTreeSet::from([near]));
}

#[test]
fn a_castle_can_be_threatened_too() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let castle = put(&mut state, &catalog, "castle", 0, 5, 3);
    put(&mut state, &catalog, "lion", 1, 5, 5);
    set_hp(&mut state, castle, 4);
    assert_eq!(threatened(&catalog, &state), BTreeSet::from([castle]));
}

#[test]
fn facing_changes_who_is_threatened() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let victim = put(&mut state, &catalog, "kid-elephant", 0, 5, 4);
    put_facing(&mut state, &catalog, "kid-lion", 1, (5, 6), Facing::North); // 前に 1 マス: 5,5 まで。届かない
    assert!(!threatened(&catalog, &state).contains(&victim));
    state.pieces.pop();
    put_facing(&mut state, &catalog, "kid-lion", 1, (5, 5), Facing::North); // 前（北）は 5,4
    assert!(threatened(&catalog, &state).contains(&victim));
    state.pieces.pop();
    put_facing(&mut state, &catalog, "kid-lion", 1, (5, 5), Facing::South); // 後ろ向き: 届かない
    assert!(!threatened(&catalog, &state).contains(&victim));
}

#[test]
fn a_move_that_also_overlaps_the_attackers_own_piece_still_threatens_the_enemy() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    // 象（攻撃力 2）が 5,3 へ動くと 5..6 x 3..4。敵の子供ライオンと、自分のクマの両方に重なる。
    // クマは生き残るので、象は動けないが、ダメージは先に与えるので、子供ライオンは捕食される
    put(&mut state, &catalog, "elephant", 1, 5, 5);
    let enemy = put(&mut state, &catalog, "kid-lion", 0, 5, 3);
    put(&mut state, &catalog, "bear", 1, 6, 3);
    assert!(threatened(&catalog, &state).contains(&enemy));
}

#[test]
fn pieces_of_eliminated_players_are_never_reported() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    put(&mut state, &catalog, "lion", 0, 5, 5);
    put(&mut state, &catalog, "kid-elephant", 1, 5, 4);
    state.players[1].eliminated = true; // 駒が残っていても、守られる側として数えない
    assert!(threatened(&catalog, &state).is_empty());
}

#[test]
fn an_empty_board_has_no_threats() {
    let catalog = catalog();
    let state = battle(&catalog, 3);
    assert!(threats(&catalog, &state).is_empty());
}

#[test]
fn threats_do_not_change_the_state() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "lion", 1, 5, 5);
    put(&mut state, &catalog, "kid-elephant", 0, 5, 4);
    let before = state.clone();
    let _ = threats(&catalog, &state);
    assert_eq!(state, before);
}

// ---- 判定と、実際の捕食の一致 ----

/// 駒を、重ならないように、ランダムに撒いた盤。3 人対戦で、各自の城がある。
fn scattered(catalog: &Catalog, seed: u64) -> GameState {
    let mut rng = SplitMix64(seed);
    let mut state = battle(catalog, 3);
    for (seat, (x, y)) in [(0, 0), (13, 0), (0, 13)].into_iter().enumerate() {
        put(&mut state, catalog, "castle", seat as u8, x, y);
    }
    let kinds: Vec<KindId> = catalog
        .iter()
        .filter(|(_, d)| !d.is_castle())
        .map(|(k, _)| k)
        .collect();
    for _ in 0..60 {
        let kind = kinds[rng.below(kinds.len() as u64) as usize];
        let facing = Facing::ALL[rng.below(4) as usize];
        let anchor = Pos::new(rng.below(16) as i32, rng.below(16) as i32);
        let def = catalog.get(kind).unwrap();
        let rect = def.footprint(anchor, facing);
        let free = rect.is_inside(16)
            && state
                .pieces
                .iter()
                .filter_map(|p| state.footprint(catalog, p))
                .all(|other| !other.overlaps(&rect));
        if !free {
            continue;
        }
        let owner = rng.below(3) as u8;
        let id = state.issue_piece_id();
        let hp = 1 + rng.below(u64::from(def.hp)) as u32;
        state.pieces.push(Piece {
            id,
            kind,
            owner: PlayerId(owner),
            anchor,
            facing,
            hp,
        });
    }
    state
}

/// 実際に、すべての他プレイヤーの駒を、あらゆる移動先へ動かしてみて、捕食される駒を集める。
fn captured_by_real_moves(catalog: &Catalog, state: &GameState) -> BTreeSet<PieceId> {
    let mut captured = BTreeSet::new();
    for attacker in &state.pieces {
        for target in legal_targets(catalog, state, attacker.id) {
            let mut turn = state.clone();
            turn.current = attacker.owner;
            let action = Action::Move {
                piece: attacker.id,
                to: target,
                promote: false,
            };
            let (_, events) = apply(catalog, &turn, &action).unwrap();
            for event in events {
                if let Event::Captured { piece, taker, .. } = event {
                    let victim_owner = state.piece(piece).unwrap().owner;
                    if victim_owner != taker {
                        captured.insert(piece);
                    }
                }
            }
        }
    }
    captured
}

#[test]
fn the_judgement_matches_what_apply_really_captures() {
    let catalog = catalog();
    let mut nonempty = 0;
    for seed in 0..40 {
        let state = scattered(&catalog, seed);
        assert_eq!(state.validate(&catalog), Ok(()));
        let expected = captured_by_real_moves(&catalog, &state);
        assert_eq!(threatened_pieces(&catalog, &state), expected, "seed {seed}");
        if !expected.is_empty() {
            nonempty += 1;
        }
    }
    assert!(nonempty > 5, "判定が空ばかりでは、比較にならない");
}

#[test]
fn the_overlap_used_for_damage_is_the_footprint_overlap() {
    // 補助: Rect の重なりと、ダメージの計算の前提を確かめる
    let a = Rect {
        x: 0,
        y: 0,
        w: 2,
        h: 2,
    };
    let b = Rect {
        x: 1,
        y: 1,
        w: 2,
        h: 2,
    };
    assert_eq!(a.overlap_area(&b), 1);
}
