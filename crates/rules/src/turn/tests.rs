use std::collections::BTreeMap;

use super::*;
use crate::finish::{EndReason, TURNS_PER_PLAYER, resign};
use crate::geometry::{Facing, Pos};
use crate::pieces::Catalog;
use crate::rng::SplitMix64;
use crate::state::{GameState, Piece, PieceId, PlayerId, Violation};

fn catalog() -> Catalog {
    Catalog::builtin().unwrap()
}

fn put(state: &mut GameState, catalog: &Catalog, id: &str, owner: u8, x: i32, y: i32) -> PieceId {
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

/// 対戦フェーズ。全員が城を置き終えていて、手番は PlayerId(0)。段階は、ドロー待ち。
fn battle(catalog: &Catalog, players: usize) -> GameState {
    let mut state = GameState::new(catalog, players, PlayerId(0)).unwrap();
    state.phase = Phase::Battle;
    state.players.iter_mut().for_each(|p| p.hand.clear());
    let spots = [(0, 0), (13, 0), (0, 13), (13, 13)];
    for (seat, (x, y)) in spots.into_iter().take(players).enumerate() {
        put(&mut state, catalog, "castle", seat as u8, x, y);
    }
    state.stage = Stage::Draw;
    state
}

fn hand_total(state: &GameState, owner: u8) -> u32 {
    state.players[usize::from(owner)].hand.values().sum()
}

fn cards(state: &GameState) -> Vec<KindId> {
    match &state.stage {
        Stage::Pick(cards) => cards.clone(),
        other => panic!("選択待ちではありません: {other:?}"),
    }
}

// ---- ドロー（R-05） ----

#[test]
fn r05_draw_gives_four_cards_and_waits_for_a_pick() {
    let catalog = catalog();
    let state = battle(&catalog, 4);
    let (next, events) = draw_cards(&catalog, &state, &mut SplitMix64(1)).unwrap();
    let drawn = cards(&next);
    assert_eq!(drawn.len(), DRAW_COUNT);
    assert_eq!(
        events,
        [Event::Drawn {
            player: PlayerId(0),
            cards: drawn
        }]
    );
    assert_eq!(next.validate(&catalog), Ok(()));
    assert_eq!(state.stage, Stage::Draw); // 元の状態は変わらない
}

#[test]
fn r05_the_castle_is_never_drawn() {
    let catalog = catalog();
    let state = battle(&catalog, 2);
    let mut rng = SplitMix64(99);
    for _ in 0..2000 {
        let (next, _) = draw_cards(&catalog, &state, &mut rng).unwrap();
        assert!(cards(&next).iter().all(|k| *k != catalog.castle()));
    }
}

#[test]
fn r05_draw_frequencies_follow_the_weights() {
    let catalog = catalog();
    let state = battle(&catalog, 2);
    let mut rng = SplitMix64(2024);
    let rounds = 5000;
    let mut counts: BTreeMap<KindId, f64> = BTreeMap::new();
    for _ in 0..rounds {
        let (next, _) = draw_cards(&catalog, &state, &mut rng).unwrap();
        for kind in cards(&next) {
            *counts.entry(kind).or_default() += 1.0;
        }
    }
    let total_weight: f64 = catalog.iter().map(|(_, d)| f64::from(d.weight)).sum();
    let draws = (rounds * DRAW_COUNT) as f64;
    for (kind, def) in catalog.iter() {
        let expected = draws * f64::from(def.weight) / total_weight;
        let actual = counts.get(&kind).copied().unwrap_or(0.0);
        if def.weight == 0 {
            assert_eq!(actual, 0.0, "{} は引かれない", def.id);
        } else {
            assert!(
                (actual - expected).abs() <= expected * 0.15,
                "{}: 期待 {expected:.0}、実際 {actual}",
                def.id
            );
        }
    }
}

#[test]
fn r05_cards_are_independent_so_duplicates_can_appear() {
    let catalog = catalog();
    let state = battle(&catalog, 2);
    let mut rng = SplitMix64(5);
    let has_duplicate = (0..200).any(|_| {
        let (next, _) = draw_cards(&catalog, &state, &mut rng).unwrap();
        let drawn = cards(&next);
        drawn
            .iter()
            .any(|k| drawn.iter().filter(|o| *o == k).count() > 1)
    });
    assert!(has_duplicate);
}

#[test]
fn draw_is_reproducible_from_the_seed() {
    let catalog = catalog();
    let state = battle(&catalog, 3);
    let draw = |seed| {
        cards(
            &draw_cards(&catalog, &state, &mut SplitMix64(seed))
                .unwrap()
                .0,
        )
    };
    assert_eq!(draw(11), draw(11));
    assert!((0..20).any(|s| draw(s) != draw(11)));
}

#[test]
fn draw_is_only_allowed_when_waiting_for_a_draw_in_battle() {
    let catalog = catalog();
    let mut rng = SplitMix64(1);
    let placement = GameState::new(&catalog, 2, PlayerId(0)).unwrap();
    assert_eq!(
        draw_cards(&catalog, &placement, &mut rng).unwrap_err(),
        ActionError::WrongPhase
    );
    let mut state = battle(&catalog, 2);
    state.stage = Stage::Act;
    assert_eq!(
        draw_cards(&catalog, &state, &mut rng).unwrap_err(),
        ActionError::WrongStage
    );
    state.stage = Stage::Draw;
    let (picking, _) = draw_cards(&catalog, &state, &mut rng).unwrap();
    assert_eq!(
        draw_cards(&catalog, &picking, &mut rng).unwrap_err(),
        ActionError::WrongStage
    );
}

// ---- 選択（R-06・R-20・R-21） ----

fn picking(catalog: &Catalog) -> GameState {
    draw_cards(catalog, &battle(catalog, 4), &mut SplitMix64(3))
        .unwrap()
        .0
}

#[test]
fn r06_picking_adds_one_card_to_the_hand_and_discards_the_rest() {
    let catalog = catalog();
    let state = picking(&catalog);
    let drawn = cards(&state);
    let (next, events) = apply(&catalog, &state, &Action::Pick { index: 2 }).unwrap();
    assert_eq!(next.stage, Stage::Act);
    assert_eq!(hand_total(&next, 0), 1);
    assert_eq!(next.players[0].hand[&drawn[2]], 1);
    assert_eq!(
        events,
        [Event::Picked {
            player: PlayerId(0),
            kind: drawn[2],
            index: 2
        }]
    );
    assert_eq!(next.current, PlayerId(0)); // 選択では、手番は進まない
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r06_picking_the_same_kind_twice_increases_the_count() {
    let catalog = catalog();
    let mut state = picking(&catalog);
    let kind = cards(&state)[0];
    *state.players[0].hand.entry(kind).or_insert(0) += 1;
    let (next, _) = apply(&catalog, &state, &Action::Pick { index: 0 }).unwrap();
    assert_eq!(next.players[0].hand[&kind], 2);
}

#[test]
fn r06_pick_index_must_be_in_range() {
    let catalog = catalog();
    let state = picking(&catalog);
    assert_eq!(
        apply(&catalog, &state, &Action::Pick { index: DRAW_COUNT }).unwrap_err(),
        ActionError::InvalidPick
    );
}

#[test]
fn r21_a_pick_cannot_be_changed() {
    let catalog = catalog();
    let (picked, _) = apply(&catalog, &picking(&catalog), &Action::Pick { index: 0 }).unwrap();
    assert_eq!(
        apply(&catalog, &picked, &Action::Pick { index: 1 }).unwrap_err(),
        ActionError::WrongStage
    );
}

#[test]
fn r06_actions_and_picks_must_follow_the_stage_order() {
    let catalog = catalog();
    let waiting_draw = battle(&catalog, 4);
    let lion = {
        let mut s = waiting_draw.clone();
        put(&mut s, &catalog, "lion", 0, 5, 5)
    };
    let mut with_lion = waiting_draw.clone();
    put(&mut with_lion, &catalog, "lion", 0, 5, 5);
    let rotate = Action::Rotate {
        piece: lion,
        facing: Facing::East,
    };
    // ドロー前: 選択も行動もできない
    assert_eq!(
        apply(&catalog, &with_lion, &Action::Pick { index: 0 }).unwrap_err(),
        ActionError::WrongStage
    );
    assert_eq!(
        apply(&catalog, &with_lion, &rotate).unwrap_err(),
        ActionError::WrongStage
    );
    assert_eq!(
        apply(&catalog, &with_lion, &Action::Pass).unwrap_err(),
        ActionError::WrongStage
    );
    // 選択前: 行動できない
    let (mut choosing, _) = draw_cards(&catalog, &with_lion, &mut SplitMix64(1)).unwrap();
    assert_eq!(
        apply(&catalog, &choosing, &rotate).unwrap_err(),
        ActionError::WrongStage
    );
    assert_eq!(
        apply(&catalog, &choosing, &Action::Pass).unwrap_err(),
        ActionError::WrongStage
    );
    // 選択後: 行動できる
    choosing = apply(&catalog, &choosing, &Action::Pick { index: 0 })
        .unwrap()
        .0;
    assert!(apply(&catalog, &choosing, &rotate).is_ok());
}

#[test]
fn pick_is_refused_outside_battle() {
    let catalog = catalog();
    let state = GameState::new(&catalog, 2, PlayerId(0)).unwrap();
    assert_eq!(
        apply(&catalog, &state, &Action::Pick { index: 0 }).unwrap_err(),
        ActionError::WrongPhase
    );
}

#[test]
fn r20_the_picked_piece_can_be_placed_in_the_same_turn() {
    let catalog = catalog();
    let mut state = picking(&catalog);
    let kind = cards(&state)[1];
    let (acted, _) = apply(&catalog, &state, &Action::Pick { index: 1 }).unwrap();
    state = acted;
    let spots = crate::action::legal_placements(&catalog, &state, kind);
    assert!(!spots.is_empty());
    let (anchor, facing) = spots[0];
    let (next, events) = apply(
        &catalog,
        &state,
        &Action::Place {
            kind,
            anchor,
            facing,
        },
    )
    .unwrap();
    assert!(matches!(events[0], Event::Placed { .. }));
    assert_eq!(hand_total(&next, 0), 0);
}

#[test]
fn r80_pass_after_picking_keeps_the_picked_card_in_hand() {
    let catalog = catalog();
    // 手番のプレイヤー 0 は、盤上に城だけ。選んだ駒が置ける場所を、ふさいでおく
    let mut state = battle(&catalog, 2);
    for (x, y) in [(2, 0), (2, 1), (2, 2), (1, 2), (0, 2)] {
        put(&mut state, &catalog, "bear", 1, x, y);
    }
    let (state, _) = draw_cards(&catalog, &state, &mut SplitMix64(8)).unwrap();
    let (state, _) = apply(&catalog, &state, &Action::Pick { index: 0 }).unwrap();
    let (next, events) = apply(&catalog, &state, &Action::Pass).unwrap();
    assert!(matches!(events[0], Event::Passed { .. }));
    assert_eq!(hand_total(&next, 0), 1);
    assert_eq!(next.current, PlayerId(1));
}

// ---- 手番の進行 ----

#[test]
fn each_turn_goes_draw_then_pick_then_act_then_the_next_players_draw() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5);
    assert_eq!(state.stage, Stage::Draw);
    let (state, _) = draw_cards(&catalog, &state, &mut SplitMix64(4)).unwrap();
    assert!(matches!(state.stage, Stage::Pick(_)));
    let (state, _) = apply(&catalog, &state, &Action::Pick { index: 0 }).unwrap();
    assert_eq!(state.stage, Stage::Act);
    let (state, _) = apply(
        &catalog,
        &state,
        &Action::Rotate {
            piece: lion,
            facing: Facing::East,
        },
    )
    .unwrap();
    assert_eq!(state.stage, Stage::Draw);
    assert_eq!(state.current, PlayerId(1));
    assert_eq!(state.turn, 1);
}

#[test]
fn the_last_castle_placement_starts_the_battle_waiting_for_a_draw() {
    let catalog = catalog();
    let mut state = GameState::new(&catalog, 2, PlayerId(0)).unwrap();
    for x in [0, 8] {
        let place = Action::Place {
            kind: catalog.castle(),
            anchor: Pos::new(x, 0),
            facing: Facing::North,
        };
        assert_eq!(state.stage, Stage::Act); // 配置フェーズは、ドローしない（R-12）
        state = apply(&catalog, &state, &place).unwrap().0;
    }
    assert_eq!(state.phase, Phase::Battle);
    assert_eq!(state.stage, Stage::Draw);
    assert_eq!(state.current, PlayerId(0));
}

#[test]
fn resigning_on_your_turn_hands_the_draw_to_the_next_player() {
    let catalog = catalog();
    let (state, _) = draw_cards(&catalog, &battle(&catalog, 3), &mut SplitMix64(1)).unwrap();
    let (next, _) = resign(&catalog, &state, PlayerId(0)).unwrap();
    assert_eq!(next.current, PlayerId(1));
    assert_eq!(next.stage, Stage::Draw);
    // 手番でない人の投了は、段階を変えない
    let (other, _) = resign(&catalog, &state, PlayerId(2)).unwrap();
    assert_eq!(other.current, PlayerId(0));
    assert_eq!(other.stage, state.stage);
}

// ---- 時間切れ（R-14・R-31） ----

#[test]
fn r31_time_out_before_the_draw_draws_picks_at_random_and_passes() {
    let catalog = catalog();
    let state = battle(&catalog, 3);
    let (next, events) = time_out(&catalog, &state, &mut SplitMix64(6)).unwrap();
    assert!(matches!(events[0], Event::Drawn { .. }));
    assert!(matches!(events[1], Event::Picked { .. }));
    assert_eq!(
        events[2],
        Event::TimedOut {
            player: PlayerId(0)
        }
    );
    assert_eq!(hand_total(&next, 0), 1);
    assert_eq!(next.current, PlayerId(1));
    assert_eq!(next.stage, Stage::Draw);
    assert_eq!(next.turn, 1);
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r31_time_out_while_picking_picks_one_of_the_four_and_passes() {
    let catalog = catalog();
    let state = picking(&catalog);
    let drawn = cards(&state);
    let (next, events) = time_out(&catalog, &state, &mut SplitMix64(9)).unwrap();
    assert_eq!(events.len(), 2);
    let Event::Picked { kind, index, .. } = events[0] else {
        panic!("先頭は選択のはず: {events:?}")
    };
    assert_eq!(drawn[index], kind);
    assert_eq!(
        events[1],
        Event::TimedOut {
            player: PlayerId(0)
        }
    );
    assert_eq!(next.players[0].hand[&kind], 1);
    assert_eq!(next.current, PlayerId(1));
}

#[test]
fn r31_time_out_after_picking_just_passes_even_if_actions_exist() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    put(&mut state, &catalog, "lion", 0, 5, 5); // 動ける駒がある
    let (state, _) = draw_cards(&catalog, &state, &mut SplitMix64(2)).unwrap();
    let (state, _) = apply(&catalog, &state, &Action::Pick { index: 0 }).unwrap();
    let before_hand = hand_total(&state, 0);
    let (next, events) = time_out(&catalog, &state, &mut SplitMix64(2)).unwrap();
    assert_eq!(
        events,
        [Event::TimedOut {
            player: PlayerId(0)
        }]
    );
    assert_eq!(hand_total(&next, 0), before_hand);
    assert_eq!(next.pieces.len(), state.pieces.len());
    assert_eq!(next.current, PlayerId(1));
}

#[test]
fn r31_time_out_is_reproducible_from_the_seed() {
    let catalog = catalog();
    let state = battle(&catalog, 3);
    let run = |seed| time_out(&catalog, &state, &mut SplitMix64(seed)).unwrap();
    assert_eq!(run(5), run(5));
}

#[test]
fn r14_r31_time_out_in_placement_puts_the_castle_at_a_legal_random_spot() {
    let catalog = catalog();
    let mut state = GameState::new(&catalog, 3, PlayerId(0)).unwrap();
    let mut rng = SplitMix64(77);
    let mut anchors = Vec::new();
    for seat in 0..3u8 {
        assert_eq!(state.current, PlayerId(seat));
        let (next, events) = time_out(&catalog, &state, &mut rng).unwrap();
        assert_eq!(
            events[0],
            Event::TimedOut {
                player: PlayerId(seat)
            }
        );
        let Event::Placed { anchor, facing, .. } = events[1] else {
            panic!("城が置かれるはず: {events:?}")
        };
        assert_eq!(facing, Facing::North);
        anchors.push(anchor);
        assert_eq!(next.validate(&catalog), Ok(()));
        state = next;
    }
    assert_eq!(state.phase, Phase::Battle);
    assert_eq!(state.stage, Stage::Draw);
    assert_eq!(state.pieces.len(), 3);
    anchors.sort();
    anchors.dedup();
    assert_eq!(anchors.len(), 3);
}

#[test]
fn time_out_is_refused_after_the_game_has_ended() {
    let catalog = catalog();
    let mut two = battle(&catalog, 2);
    two.stage = Stage::Act;
    let (done, _) = resign(&catalog, &two, PlayerId(1)).unwrap();
    assert_eq!(
        time_out(&catalog, &done, &mut SplitMix64(1)).unwrap_err(),
        ActionError::WrongPhase
    );
}

#[test]
fn time_out_still_triggers_the_turn_limit() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    state.turn = 2 * TURNS_PER_PLAYER - 1;
    let (next, events) = time_out(&catalog, &state, &mut SplitMix64(1)).unwrap();
    assert_eq!(next.phase, Phase::Finished);
    let Some(Event::Finished { outcome }) = events.last() else {
        panic!("終局するはず: {events:?}")
    };
    assert_eq!(outcome.reason, EndReason::TurnLimit);
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn ending_the_game_clears_the_stage() {
    let catalog = catalog();
    // 選択待ちの途中で、手番でないプレイヤーが投了して終局する
    let mut two = battle(&catalog, 2);
    two.stage = Stage::Draw;
    let (picking, _) = draw_cards(&catalog, &two, &mut SplitMix64(1)).unwrap();
    let (done, _) = resign(&catalog, &picking, PlayerId(1)).unwrap();
    assert_eq!(done.phase, Phase::Finished);
    assert_eq!(done.stage, Stage::Act);
    assert_eq!(done.validate(&catalog), Ok(()));
}

#[test]
fn time_out_in_placement_without_any_legal_spot_is_an_error_and_resign_unblocks() {
    let catalog = catalog();
    // 盤を、大きな駒でふさいで、城を置ける場所をなくす
    let mut state = GameState::with_board_size(&catalog, 4, 2, PlayerId(0)).unwrap();
    put(&mut state, &catalog, "elephant", 1, 0, 0);
    put(&mut state, &catalog, "elephant", 1, 2, 0);
    put(&mut state, &catalog, "elephant", 1, 0, 2);
    put(&mut state, &catalog, "elephant", 1, 2, 2);
    assert_eq!(
        time_out(&catalog, &state, &mut SplitMix64(1)).unwrap_err(),
        ActionError::NoLegalPlacement
    );
    let (next, _) = resign(&catalog, &state, PlayerId(0)).unwrap();
    assert_eq!(next.phase, Phase::Finished);
}

// ---- 状態の検証 ----

#[test]
fn validate_checks_the_stage_against_the_phase_and_the_cards() {
    let catalog = catalog();
    let mut placement = GameState::new(&catalog, 2, PlayerId(0)).unwrap();
    placement.stage = Stage::Draw;
    assert_eq!(placement.validate(&catalog), Err(Violation::StageMismatch));

    let mut state = battle(&catalog, 2);
    state.stage = Stage::Pick(vec![catalog.kind_of("lion").unwrap()]); // 4 枚ではない
    assert_eq!(state.validate(&catalog), Err(Violation::StageMismatch));
    state.stage = Stage::Pick(vec![catalog.castle(); DRAW_COUNT]); // 城は引かれない
    assert_eq!(state.validate(&catalog), Err(Violation::StageMismatch));
    state.stage = Stage::Pick(vec![catalog.kind_of("lion").unwrap(); DRAW_COUNT]);
    assert_eq!(state.validate(&catalog), Ok(()));
}
