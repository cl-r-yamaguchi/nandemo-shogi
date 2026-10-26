use super::*;
use crate::action::{Action, apply};
use crate::geometry::{Facing, Pos};
use crate::pieces::Catalog;
use crate::state::{GameState, Piece, PieceId};

fn catalog() -> Catalog {
    Catalog::builtin().unwrap()
}

/// 対戦フェーズの盤。全員が城を置き終えている。手番は PlayerId(0)。
fn battle(catalog: &Catalog, players: usize) -> GameState {
    let mut state = GameState::new(catalog, players, PlayerId(0)).unwrap();
    state.phase = Phase::Battle;
    state.players.iter_mut().for_each(|p| p.hand.clear());
    state
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

fn set_hp(state: &mut GameState, id: PieceId, hp: u32) {
    state.pieces.iter_mut().find(|p| p.id == id).unwrap().hp = hp;
}

fn mv(piece: PieceId, x: i32, y: i32) -> Action {
    Action::Move {
        piece,
        to: Pos::new(x, y),
        promote: false,
    }
}

fn rotate(piece: PieceId) -> Action {
    Action::Rotate {
        piece,
        facing: Facing::East,
    }
}

fn ids(list: &[&[u8]]) -> Vec<Vec<PlayerId>> {
    list.iter()
        .map(|group| group.iter().map(|p| PlayerId(*p)).collect())
        .collect()
}

fn outcome(state: &GameState) -> &Outcome {
    state.outcome.as_ref().expect("終局していること")
}

/// 4 人対戦。各プレイヤーが城を 1 つずつ持ち、プレイヤー 0 は、プレイヤー 1 の城を攻撃できる位置にライオンがいる。
fn four_players(catalog: &Catalog) -> (GameState, PieceId) {
    let mut state = battle(catalog, 4);
    put(&mut state, catalog, "castle", 0, 0, 0);
    let castle1 = put(&mut state, catalog, "castle", 1, 13, 0);
    put(&mut state, catalog, "castle", 2, 0, 13);
    put(&mut state, catalog, "castle", 3, 13, 13);
    put(&mut state, catalog, "lion", 0, 12, 0);
    set_hp(&mut state, castle1, 4); // ライオンの攻撃力 4 で、1 回の攻撃で捕食できる
    (state, castle1)
}

// ---- 脱落（R-90・R-91） ----

#[test]
fn r90_r91_losing_the_last_castle_eliminates_the_player_and_clears_their_assets() {
    let catalog = catalog();
    let (mut state, castle1) = four_players(&catalog);
    put(&mut state, &catalog, "bear", 1, 8, 8);
    *state.players[1]
        .hand
        .entry(catalog.kind_of("lion").unwrap())
        .or_insert(0) += 2;
    let lion = state
        .pieces
        .iter()
        .find(|p| p.owner == PlayerId(0) && p.kind == catalog.kind_of("lion").unwrap())
        .unwrap()
        .id;
    let (next, events) = apply(&catalog, &state, &mv(lion, 13, 0)).unwrap();
    assert!(next.piece(castle1).is_none());
    assert!(next.players[1].eliminated);
    assert!(next.players[1].hand.is_empty());
    assert!(next.pieces.iter().all(|p| p.owner != PlayerId(1)));
    assert!(events.contains(&Event::Eliminated {
        player: PlayerId(1)
    }));
    assert_eq!(next.phase, Phase::Battle);
    assert_eq!(next.out_batches, [vec![PlayerId(1)]]);
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r90_the_turn_skips_the_eliminated_seat() {
    let catalog = catalog();
    let (state, _) = four_players(&catalog);
    let lion = state
        .pieces
        .iter()
        .find(|p| p.owner == PlayerId(0) && p.kind == catalog.kind_of("lion").unwrap())
        .unwrap()
        .id;
    let (next, _) = apply(&catalog, &state, &mv(lion, 13, 0)).unwrap();
    assert_eq!(next.current, PlayerId(2));
    assert_eq!(
        next.alive_players(),
        [PlayerId(0), PlayerId(2), PlayerId(3)]
    );
}

#[test]
fn r90_a_player_who_has_not_placed_a_castle_is_not_eliminated() {
    let catalog = catalog();
    // 配置フェーズ: プレイヤー 1 はまだ城を置いていない
    let state = GameState::new(&catalog, 3, PlayerId(0)).unwrap();
    let castle = Action::Place {
        kind: catalog.castle(),
        anchor: Pos::new(0, 0),
        facing: Facing::North,
    };
    let (next, events) = apply(&catalog, &state, &castle).unwrap();
    assert!(next.players.iter().all(|p| !p.eliminated));
    assert!(!events.iter().any(|e| matches!(e, Event::Eliminated { .. })));
}

// ---- 終局（R-93・R-94） ----

#[test]
fn r93_the_game_ends_when_one_player_remains() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "castle", 0, 0, 0);
    let castle1 = put(&mut state, &catalog, "castle", 1, 13, 0);
    let lion = put(&mut state, &catalog, "lion", 0, 12, 0);
    set_hp(&mut state, castle1, 4);
    let (next, events) = apply(&catalog, &state, &mv(lion, 13, 0)).unwrap();
    assert_eq!(next.phase, Phase::Finished);
    let result = outcome(&next);
    assert_eq!(result.reason, EndReason::LastStanding);
    assert_eq!(result.ranking, ids(&[&[0], &[1]]));
    assert_eq!(result.winners(), [PlayerId(0)]);
    assert!(!result.is_draw());
    assert!(matches!(events.last(), Some(Event::Finished { .. })));
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r93_actions_are_refused_after_the_game_has_ended() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "castle", 0, 0, 0);
    let castle1 = put(&mut state, &catalog, "castle", 1, 13, 0);
    let lion = put(&mut state, &catalog, "lion", 0, 12, 0);
    set_hp(&mut state, castle1, 4);
    let (next, _) = apply(&catalog, &state, &mv(lion, 13, 0)).unwrap();
    assert!(apply(&catalog, &next, &rotate(lion)).is_err());
    assert!(resign(&catalog, &next, PlayerId(0)).is_err());
}

#[test]
fn r94_simultaneous_elimination_leaves_the_survivor_as_the_winner() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    put(&mut state, &catalog, "castle", 0, 0, 0);
    let a = put(&mut state, &catalog, "castle", 1, 3, 2); // 3..4 x 2..3
    let b = put(&mut state, &catalog, "castle", 2, 5, 2); // 5..6 x 2..3
    let elephant = put(&mut state, &catalog, "elephant", 0, 4, 5);
    set_hp(&mut state, a, 2);
    set_hp(&mut state, b, 2);
    // 象（攻撃力 2）が、4,3 へ動くと、4..5 x 3..4 になり、両方の城に 1 マスずつ重なる
    let (next, _) = apply(&catalog, &state, &mv(elephant, 4, 3)).unwrap();
    assert_eq!(next.out_batches, [vec![PlayerId(1), PlayerId(2)]]);
    let result = outcome(&next);
    assert_eq!(result.reason, EndReason::LastStanding);
    assert_eq!(result.ranking, ids(&[&[0], &[1, 2]]));
}

#[test]
fn r94_everyone_eliminated_at_once_is_a_draw() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let mine = put(&mut state, &catalog, "castle", 0, 3, 2);
    let theirs = put(&mut state, &catalog, "castle", 1, 5, 2);
    let elephant = put(&mut state, &catalog, "elephant", 0, 4, 5);
    set_hp(&mut state, mine, 2);
    set_hp(&mut state, theirs, 2);
    let (next, _) = apply(&catalog, &state, &mv(elephant, 4, 3)).unwrap();
    let result = outcome(&next);
    assert_eq!(result.reason, EndReason::AllEliminated);
    assert_eq!(result.ranking, ids(&[&[0, 1]]));
    assert!(result.is_draw());
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn attacking_your_own_last_castle_eliminates_you() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let mine = put(&mut state, &catalog, "castle", 0, 3, 2);
    put(&mut state, &catalog, "castle", 1, 10, 10);
    let lion = put(&mut state, &catalog, "lion", 0, 3, 4);
    set_hp(&mut state, mine, 4);
    let (next, _) = apply(&catalog, &state, &mv(lion, 3, 3)).unwrap();
    let result = outcome(&next);
    assert_eq!(result.winners(), [PlayerId(1)]);
    assert_eq!(result.ranking, ids(&[&[1], &[0]]));
}

// ---- 順位（R-97） ----

#[test]
fn r97_later_eliminations_rank_higher() {
    let catalog = catalog();
    let (mut state, castle1) = four_players(&catalog);
    let lion = state
        .pieces
        .iter()
        .find(|p| p.owner == PlayerId(0) && p.kind == catalog.kind_of("lion").unwrap())
        .unwrap()
        .id;
    // プレイヤー 1 → 3 → 2 の順に脱落させる。手番は、毎回プレイヤー 0 に戻るよう、ほかの人は投了で進める
    let (next, _) = apply(&catalog, &state, &mv(lion, 13, 0)).unwrap();
    assert!(next.piece(castle1).is_none());
    state = next;
    let (next, _) = resign(&catalog, &state, PlayerId(3)).unwrap();
    let (next, _) = resign(&catalog, &next, PlayerId(2)).unwrap();
    let result = outcome(&next);
    assert_eq!(result.reason, EndReason::LastStanding);
    assert_eq!(result.ranking, ids(&[&[0], &[2], &[3], &[1]]));
    assert_eq!(result.winners(), [PlayerId(0)]);
}

// ---- 手番数の上限（R-95） ----

fn limit_game(catalog: &Catalog) -> (GameState, PieceId, PieceId, PieceId) {
    let mut state = battle(catalog, 2);
    let c0 = put(&mut state, catalog, "castle", 0, 0, 0);
    let c1 = put(&mut state, catalog, "castle", 1, 13, 13);
    let lion = put(&mut state, catalog, "lion", 0, 5, 5);
    (state, c0, c1, lion)
}

#[test]
fn r95_the_game_continues_until_the_turn_limit() {
    let catalog = catalog();
    let (mut state, _, _, lion) = limit_game(&catalog);
    state.turn = 2 * TURNS_PER_PLAYER - 2;
    let (next, _) = apply(&catalog, &state, &rotate(lion)).unwrap();
    assert_eq!(next.phase, Phase::Battle);
    assert_eq!(next.turn, 2 * TURNS_PER_PLAYER - 1);
}

#[test]
fn r95_at_the_limit_the_larger_castle_hp_wins() {
    let catalog = catalog();
    let (mut state, _, c1, lion) = limit_game(&catalog);
    set_hp(&mut state, c1, 6);
    state.turn = 2 * TURNS_PER_PLAYER - 1;
    let (next, events) = apply(&catalog, &state, &rotate(lion)).unwrap();
    assert_eq!(next.phase, Phase::Finished);
    let result = outcome(&next);
    assert_eq!(result.reason, EndReason::TurnLimit);
    assert_eq!(result.ranking, ids(&[&[0], &[1]]));
    assert!(matches!(events.last(), Some(Event::Finished { .. })));
}

#[test]
fn r95_equal_castle_hp_is_a_draw() {
    let catalog = catalog();
    let (mut state, _, _, lion) = limit_game(&catalog);
    state.turn = 2 * TURNS_PER_PLAYER - 1;
    let (next, _) = apply(&catalog, &state, &rotate(lion)).unwrap();
    let result = outcome(&next);
    assert_eq!(result.ranking, ids(&[&[0, 1]]));
    assert!(result.is_draw());
}

#[test]
fn r95_eliminated_players_rank_below_survivors_at_the_limit() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    put(&mut state, &catalog, "castle", 0, 0, 0);
    let c1 = put(&mut state, &catalog, "castle", 1, 13, 13);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5);
    set_hp(&mut state, c1, 3);
    state.players[2].eliminated = true;
    state.out_batches.push(vec![PlayerId(2)]);
    state.turn = 3 * TURNS_PER_PLAYER - 1;
    let (next, _) = apply(&catalog, &state, &rotate(lion)).unwrap();
    assert_eq!(outcome(&next).ranking, ids(&[&[0], &[1], &[2]]));
}

#[test]
fn r95_the_limit_scales_with_the_number_of_players() {
    let catalog = catalog();
    let mut state = battle(&catalog, 4);
    for seat in 0..4 {
        put(
            &mut state,
            &catalog,
            "castle",
            seat,
            if seat % 2 == 0 { 0 } else { 13 },
            if seat < 2 { 0 } else { 13 },
        );
    }
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5);
    state.turn = 2 * TURNS_PER_PLAYER - 1; // 2 人なら上限だが、4 人ではまだ
    let (next, _) = apply(&catalog, &state, &rotate(lion)).unwrap();
    assert_eq!(next.phase, Phase::Battle);
}

// ---- 投了（R-96） ----

#[test]
fn r96_a_player_can_resign_out_of_turn() {
    let catalog = catalog();
    let (state, _) = four_players(&catalog);
    let (next, events) = resign(&catalog, &state, PlayerId(2)).unwrap();
    assert!(next.players[2].eliminated);
    assert!(next.pieces.iter().all(|p| p.owner != PlayerId(2)));
    assert_eq!(
        events,
        [Event::Eliminated {
            player: PlayerId(2)
        }]
    );
    assert_eq!(next.current, PlayerId(0)); // 手番は変わらない
    assert_eq!(next.turn, state.turn);
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r96_resigning_on_your_turn_passes_the_turn_on() {
    let catalog = catalog();
    let (state, _) = four_players(&catalog);
    let (next, _) = resign(&catalog, &state, PlayerId(0)).unwrap();
    assert_eq!(next.current, PlayerId(1));
}

#[test]
fn r96_resigning_in_a_two_player_game_ends_it() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "castle", 0, 0, 0);
    put(&mut state, &catalog, "castle", 1, 13, 13);
    let (next, events) = resign(&catalog, &state, PlayerId(1)).unwrap();
    assert_eq!(next.phase, Phase::Finished);
    assert_eq!(outcome(&next).winners(), [PlayerId(0)]);
    assert!(matches!(events.last(), Some(Event::Finished { .. })));
}

#[test]
fn r96_resigning_during_placement_removes_the_player_without_a_castle() {
    let catalog = catalog();
    let state = GameState::new(&catalog, 3, PlayerId(0)).unwrap();
    let (next, _) = resign(&catalog, &state, PlayerId(0)).unwrap();
    assert!(next.players[0].eliminated);
    assert_eq!(next.current, PlayerId(1));
    assert_eq!(next.phase, Phase::Placement);
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r96_placement_ends_when_the_remaining_players_have_all_placed() {
    let catalog = catalog();
    let state = GameState::new(&catalog, 3, PlayerId(0)).unwrap();
    let place = |x| Action::Place {
        kind: catalog.castle(),
        anchor: Pos::new(x, 0),
        facing: Facing::North,
    };
    let (state, _) = apply(&catalog, &state, &place(0)).unwrap();
    let (state, _) = apply(&catalog, &state, &place(6)).unwrap();
    // 最後に残ったプレイヤー 2 が、城を置く前に投了すると、残りの 2 人で対戦が始まる
    let (next, _) = resign(&catalog, &state, PlayerId(2)).unwrap();
    assert_eq!(next.phase, Phase::Battle);
    assert_eq!(next.turn, 0);
    assert_eq!(next.current, PlayerId(0));
}

#[test]
fn r96_unknown_or_already_eliminated_players_cannot_resign() {
    let catalog = catalog();
    let (state, _) = four_players(&catalog);
    assert_eq!(
        resign(&catalog, &state, PlayerId(9)).unwrap_err(),
        ActionError::UnknownPlayer
    );
    let (next, _) = resign(&catalog, &state, PlayerId(1)).unwrap();
    assert_eq!(
        resign(&catalog, &next, PlayerId(1)).unwrap_err(),
        ActionError::UnknownPlayer
    );
}

// ---- 状態の検証 ----

#[test]
fn validate_rejects_inconsistent_elimination_and_outcome() {
    let catalog = catalog();
    let (mut state, _) = four_players(&catalog);
    assert_eq!(state.validate(&catalog), Ok(()));

    let mut flagged = state.clone();
    flagged.players[3].eliminated = true; // 記録がない
    assert!(flagged.validate(&catalog).is_err());

    let mut has_assets = state.clone();
    has_assets.players[1].eliminated = true;
    has_assets.out_batches.push(vec![PlayerId(1)]);
    assert_eq!(
        has_assets.validate(&catalog),
        Err(crate::state::Violation::EliminatedHasAssets(PlayerId(1)))
    );

    state.phase = Phase::Finished; // 結果がない
    assert_eq!(
        state.validate(&catalog),
        Err(crate::state::Violation::OutcomeMismatch)
    );
}

#[test]
fn apply_does_not_change_the_original_state_when_it_ends_the_game() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "castle", 0, 0, 0);
    let castle1 = put(&mut state, &catalog, "castle", 1, 13, 0);
    let lion = put(&mut state, &catalog, "lion", 0, 12, 0);
    set_hp(&mut state, castle1, 4);
    let before = state.clone();
    let _ = apply(&catalog, &state, &mv(lion, 13, 0)).unwrap();
    assert_eq!(state, before);
}

#[test]
fn r95_the_limit_uses_the_starting_player_count_even_after_eliminations() {
    let catalog = catalog();
    let mut state = battle(&catalog, 4);
    put(&mut state, &catalog, "castle", 0, 0, 0);
    put(&mut state, &catalog, "castle", 1, 13, 13);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5);
    for seat in [2u8, 3] {
        state.players[usize::from(seat)].eliminated = true;
    }
    state.out_batches.push(vec![PlayerId(3)]);
    state.out_batches.push(vec![PlayerId(2)]);
    state.turn = 2 * TURNS_PER_PLAYER; // 生き残りは 2 人だが、開始時は 4 人（上限 400）
    let (next, _) = apply(&catalog, &state, &rotate(lion)).unwrap();
    assert_eq!(next.phase, Phase::Battle);
}

#[test]
fn eliminating_the_same_player_twice_has_no_effect() {
    let catalog = catalog();
    let (state, _) = four_players(&catalog);
    let (next, _) = resign(&catalog, &state, PlayerId(1)).unwrap();
    let mut again = next.clone();
    let mut events = Vec::new();
    eliminate(&mut again, &[PlayerId(1), PlayerId(9)], &mut events);
    assert!(events.is_empty());
    assert_eq!(again, next);
}

#[test]
fn validate_rejects_duplicate_batches_and_incomplete_rankings() {
    let catalog = catalog();
    let (state, _) = four_players(&catalog);
    let (mut next, _) = resign(&catalog, &state, PlayerId(1)).unwrap();
    next.out_batches.push(vec![PlayerId(1)]);
    assert!(next.validate(&catalog).is_err());

    let mut two = battle(&catalog, 2);
    put(&mut two, &catalog, "castle", 0, 0, 0);
    put(&mut two, &catalog, "castle", 1, 13, 13);
    let (mut done, _) = resign(&catalog, &two, PlayerId(1)).unwrap();
    assert_eq!(done.validate(&catalog), Ok(()));
    done.outcome.as_mut().unwrap().ranking = ids(&[&[0]]); // 1 人が抜けている
    assert!(done.validate(&catalog).is_err());
}

#[test]
fn the_placement_turn_counts_castles_placed_and_resets_when_the_battle_starts() {
    let catalog = catalog();
    let mut state = GameState::new(&catalog, 3, PlayerId(0)).unwrap();
    assert_eq!(state.turn, 0);
    for (placed, x) in [0, 6, 12].into_iter().enumerate() {
        assert_eq!(state.turn, placed as u32);
        let place = Action::Place {
            kind: catalog.castle(),
            anchor: Pos::new(x, 0),
            facing: Facing::North,
        };
        state = apply(&catalog, &state, &place).unwrap().0;
    }
    assert_eq!(state.phase, Phase::Battle);
    assert_eq!(state.turn, 0);
}
