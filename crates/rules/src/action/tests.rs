use super::*;
use crate::pieces::Catalog;
use crate::state::{GameState, Phase, Piece, PieceId, PlayerId};

fn catalog() -> Catalog {
    Catalog::builtin().unwrap()
}

/// 対戦フェーズでは、城のないプレイヤーは脱落する（R-90）。
/// これらのテストは、駒を直接置いた盤を使うので、城のない（脱落していない）プレイヤーに、
/// 空いている場所へ城を補ってから、`super::apply` を呼ぶ。
fn with_castles(catalog: &Catalog, state: &GameState) -> GameState {
    let mut state = state.clone();
    if state.phase != Phase::Battle {
        return state;
    }
    for seat in 0..state.players.len() {
        let owner = PlayerId(seat as u8);
        let has_castle = state
            .pieces
            .iter()
            .any(|p| p.owner == owner && p.kind == catalog.castle());
        if state.players[seat].eliminated || has_castle {
            continue;
        }
        let size = state.board_size as i32;
        let spot = (0..size - 1)
            .rev()
            .flat_map(|y| (0..size - 1).rev().map(move |x| Pos::new(x, y)))
            .find(|anchor| {
                let rect = catalog
                    .get(catalog.castle())
                    .unwrap()
                    .footprint(*anchor, Facing::North);
                state
                    .pieces
                    .iter()
                    .filter_map(|p| state.footprint(catalog, p))
                    .all(|other| !other.overlaps(&rect))
            })
            .expect("城を置ける空きがあること");
        let id = state.issue_piece_id();
        state.pieces.insert(
            0,
            Piece {
                id,
                kind: catalog.castle(),
                owner,
                anchor: spot,
                facing: Facing::North,
                hp: catalog.get(catalog.castle()).unwrap().hp,
            },
        );
    }
    state
}

fn apply(
    catalog: &Catalog,
    state: &GameState,
    action: &Action,
) -> Result<(GameState, Vec<Event>), ActionError> {
    super::apply(catalog, &with_castles(catalog, state), action)
}

fn new_game(catalog: &Catalog, players: usize) -> GameState {
    GameState::new(catalog, players, PlayerId(0)).unwrap()
}

/// 対戦フェーズの盤を、駒を直接置いて作る。手番は PlayerId(0)。
fn battle(catalog: &Catalog) -> GameState {
    let mut state = new_game(catalog, 4);
    state.phase = Phase::Battle;
    // 城は置き終えている
    state.players.iter_mut().for_each(|p| p.hand.clear());
    state
}

fn put(
    state: &mut GameState,
    catalog: &Catalog,
    id: &str,
    owner: u8,
    x: i32,
    y: i32,
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

fn give(state: &mut GameState, catalog: &Catalog, owner: u8, id: &str) {
    let kind = catalog.kind_of(id).unwrap();
    *state.players[usize::from(owner)]
        .hand
        .entry(kind)
        .or_insert(0) += 1;
}

fn place_castle(catalog: &Catalog, x: i32, y: i32, facing: Facing) -> Action {
    Action::Place {
        kind: catalog.castle(),
        anchor: Pos::new(x, y),
        facing,
    }
}

fn mv(piece: PieceId, x: i32, y: i32) -> Action {
    Action::Move {
        piece,
        to: Pos::new(x, y),
        promote: false,
    }
}

fn hand_count(state: &GameState, catalog: &Catalog, owner: u8, id: &str) -> u32 {
    let kind = catalog.kind_of(id).unwrap();
    state.players[usize::from(owner)]
        .hand
        .get(&kind)
        .copied()
        .unwrap_or(0)
}

fn kind_name<'a>(catalog: &'a Catalog, state: &GameState, id: PieceId) -> &'a str {
    &catalog.get(state.piece(id).unwrap().kind).unwrap().id
}

// ---- 配置フェーズ（R-11, R-12, R-13, R-22） ----

#[test]
fn r12_castles_are_placed_in_seat_order_then_battle_begins() {
    let catalog = catalog();
    let mut state = new_game(&catalog, 4);
    let spots = [(0, 0), (13, 0), (0, 13), (13, 13)];
    for (seat, (x, y)) in spots.into_iter().enumerate() {
        assert_eq!(state.phase, Phase::Placement);
        assert_eq!(state.current, PlayerId(seat as u8));
        let (next, events) = apply(
            &catalog,
            &state,
            &place_castle(&catalog, x, y, Facing::North),
        )
        .unwrap();
        assert!(matches!(events[..], [Event::Placed { .. }]));
        state = next;
    }
    assert_eq!(state.phase, Phase::Battle);
    assert_eq!(state.turn, 0);
    assert_eq!(state.current, PlayerId(0));
    assert_eq!(state.pieces.len(), 4);
    assert_eq!(state.validate(&catalog), Ok(()));
    assert!(state.players.iter().all(|p| p.hand.is_empty()));
}

#[test]
fn r11_placement_starts_from_the_first_player_and_wraps() {
    let catalog = catalog();
    let mut state = GameState::new(&catalog, 3, PlayerId(2)).unwrap();
    let mut order = Vec::new();
    for (x, y) in [(0, 0), (8, 0), (0, 8)] {
        order.push(state.current.0);
        state = apply(
            &catalog,
            &state,
            &place_castle(&catalog, x, y, Facing::North),
        )
        .unwrap()
        .0;
    }
    assert_eq!(order, [2, 0, 1]);
    assert_eq!(state.current, PlayerId(2));
}

#[test]
fn r13_castles_need_a_gap_of_three() {
    let catalog = catalog();
    let state = apply(
        &catalog,
        &new_game(&catalog, 2),
        &place_castle(&catalog, 0, 0, Facing::North),
    )
    .unwrap()
    .0;
    let try_at = |x, y| {
        apply(
            &catalog,
            &state,
            &place_castle(&catalog, x, y, Facing::North),
        )
        .map(|_| ())
    };
    assert_eq!(try_at(4, 0), Err(ActionError::TooCloseToCastle)); // すきま 2
    assert_eq!(try_at(2, 2), Err(ActionError::TooCloseToCastle)); // 斜めに接する
    assert_eq!(try_at(1, 1), Err(ActionError::Overlaps));
    assert_eq!(try_at(5, 0), Ok(())); // すきま 3
    assert_eq!(try_at(0, 5), Ok(()));
    assert_eq!(try_at(5, 5), Ok(()));
}

#[test]
fn r13_castle_must_be_inside_the_board_and_face_north() {
    let catalog = catalog();
    let state = new_game(&catalog, 2);
    let place = |x, y, f| apply(&catalog, &state, &place_castle(&catalog, x, y, f)).map(|_| ());
    assert_eq!(place(15, 0, Facing::North), Err(ActionError::OutsideBoard));
    assert_eq!(place(-1, 0, Facing::North), Err(ActionError::OutsideBoard));
    assert_eq!(
        place(0, 0, Facing::East),
        Err(ActionError::CastleMustFaceNorth)
    );
    assert_eq!(place(14, 14, Facing::North), Ok(()));
}

#[test]
fn placement_phase_allows_only_castles() {
    let catalog = catalog();
    let mut state = new_game(&catalog, 2);
    give(&mut state, &catalog, 0, "lion");
    let lion = Action::Place {
        kind: catalog.kind_of("lion").unwrap(),
        anchor: Pos::new(0, 0),
        facing: Facing::North,
    };
    assert_eq!(
        apply(&catalog, &state, &lion).unwrap_err(),
        ActionError::WrongPhase
    );
    assert_eq!(
        apply(&catalog, &state, &Action::Pass).unwrap_err(),
        ActionError::WrongPhase
    );
    assert_eq!(
        apply(
            &catalog,
            &state,
            &Action::Rotate {
                piece: PieceId(0),
                facing: Facing::East
            }
        )
        .unwrap_err(),
        ActionError::WrongPhase
    );
}

// ---- 配置（R-40〜R-43） ----

#[test]
fn r42_pieces_are_placed_next_to_own_pieces_including_diagonals() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "castle", 0, 5, 5, Facing::North); // 5..6 x 5..6
    give(&mut state, &catalog, 0, "lion");
    let lion = catalog.kind_of("lion").unwrap();
    let place = |x, y| {
        apply(
            &catalog,
            &state,
            &Action::Place {
                kind: lion,
                anchor: Pos::new(x, y),
                facing: Facing::North,
            },
        )
        .map(|_| ())
    };
    assert_eq!(place(7, 5), Ok(())); // 辺で接する
    assert_eq!(place(7, 7), Ok(())); // 斜めで接する
    assert_eq!(place(4, 4), Ok(()));
    assert_eq!(place(8, 5), Err(ActionError::NotAdjacentToOwn));
    assert_eq!(place(6, 6), Err(ActionError::Overlaps));
    assert_eq!(place(0, 0), Err(ActionError::NotAdjacentToOwn));
}

#[test]
fn r42_adjacent_to_an_enemy_piece_does_not_count() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "castle", 0, 0, 0, Facing::North);
    put(&mut state, &catalog, "castle", 1, 10, 10, Facing::North);
    give(&mut state, &catalog, 0, "lion");
    let lion = catalog.kind_of("lion").unwrap();
    let action = Action::Place {
        kind: lion,
        anchor: Pos::new(12, 10),
        facing: Facing::North,
    };
    assert_eq!(
        apply(&catalog, &state, &action).unwrap_err(),
        ActionError::NotAdjacentToOwn
    );
}

#[test]
fn r43_placed_piece_has_full_hp_the_chosen_facing_and_leaves_the_hand() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "castle", 0, 0, 0, Facing::North);
    give(&mut state, &catalog, 0, "elephant");
    let kind = catalog.kind_of("elephant").unwrap();
    let action = Action::Place {
        kind,
        anchor: Pos::new(2, 0),
        facing: Facing::West,
    };
    let (next, events) = apply(&catalog, &state, &action).unwrap();
    let placed = next.pieces.last().unwrap();
    assert_eq!(
        (placed.hp, placed.facing, placed.owner),
        (8, Facing::West, PlayerId(0))
    );
    assert_eq!(hand_count(&next, &catalog, 0, "elephant"), 0);
    assert_eq!(
        events,
        [Event::Placed {
            piece: placed.id,
            kind,
            owner: PlayerId(0),
            anchor: Pos::new(2, 0),
            facing: Facing::West,
        }]
    );
    // 元の状態は変わらない
    assert_eq!(hand_count(&state, &catalog, 0, "elephant"), 1);
    assert_eq!(state.pieces.len(), 1);
}

#[test]
fn placing_requires_the_piece_in_hand_and_no_castles_in_battle() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "castle", 0, 0, 0, Facing::North);
    let lion = catalog.kind_of("lion").unwrap();
    let action = Action::Place {
        kind: lion,
        anchor: Pos::new(2, 0),
        facing: Facing::North,
    };
    assert_eq!(
        apply(&catalog, &state, &action).unwrap_err(),
        ActionError::NotInHand
    );
    give(&mut state, &catalog, 0, "castle");
    assert_eq!(
        apply(
            &catalog,
            &state,
            &place_castle(&catalog, 8, 8, Facing::North)
        )
        .unwrap_err(),
        ActionError::WrongPhase
    );
}

#[test]
fn r04_rotated_big_piece_must_fit_when_placed() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "castle", 0, 13, 13, Facing::North); // 13..14
    give(&mut state, &catalog, 0, "orca");
    let kind = catalog.kind_of("orca").unwrap();
    let place = |x, y, f| {
        apply(
            &catalog,
            &state,
            &Action::Place {
                kind,
                anchor: Pos::new(x, y),
                facing: f,
            },
        )
        .map(|_| ())
    };
    assert_eq!(place(15, 13, Facing::North), Ok(())); // 1x2 が x=15, y=13..14
    assert_eq!(place(15, 13, Facing::East), Err(ActionError::OutsideBoard)); // 2x1 は x=15..16
}

// ---- 移動の候補（R-50〜R-53） ----

#[test]
fn r50_targets_follow_the_facing_and_stay_inside_the_board() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let foal = put(&mut state, &catalog, "foal", 0, 5, 5, Facing::North);
    let mut targets = legal_targets(&catalog, &state, foal);
    targets.sort();
    assert_eq!(targets, [Pos::new(4, 3), Pos::new(6, 3)]);
    state.pieces[0].facing = Facing::East;
    let mut targets = legal_targets(&catalog, &state, foal);
    targets.sort();
    assert_eq!(targets, [Pos::new(7, 4), Pos::new(7, 6)]);
    // 盤の端では、外へ出る候補が消える
    state.pieces[0].anchor = Pos::new(0, 0);
    state.pieces[0].facing = Facing::South;
    assert_eq!(legal_targets(&catalog, &state, foal), [Pos::new(1, 2)]);
}

#[test]
fn r50_big_pieces_must_stay_inside_with_their_whole_footprint() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let elephant = put(&mut state, &catalog, "elephant", 0, 13, 13, Facing::North);
    let targets = legal_targets(&catalog, &state, elephant);
    assert!(
        targets
            .iter()
            .all(|t| t.x <= 14 && t.y <= 14 && t.x >= 0 && t.y >= 0)
    );
    assert!(targets.contains(&Pos::new(14, 14)));
    assert!(!targets.contains(&Pos::new(15, 13)));
}

#[test]
fn r50_castle_has_no_targets() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let castle = put(&mut state, &catalog, "castle", 0, 5, 5, Facing::North);
    assert!(legal_targets(&catalog, &state, castle).is_empty());
    assert_eq!(
        apply(&catalog, &state, &mv(castle, 6, 5)).unwrap_err(),
        ActionError::IllegalTarget
    );
}

#[test]
fn r52_pieces_in_the_path_do_not_block_movement() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let horse = put(&mut state, &catalog, "horse", 0, 5, 5, Facing::North);
    // 馬の周囲 1 マスを、敵の駒でふさぐ
    for (x, y) in [
        (4, 4),
        (5, 4),
        (6, 4),
        (4, 5),
        (6, 5),
        (4, 6),
        (5, 6),
        (6, 6),
    ] {
        put(&mut state, &catalog, "bear", 1, x, y, Facing::North);
    }
    let (next, events) = apply(&catalog, &state, &mv(horse, 5, 7)).unwrap();
    assert_eq!(next.piece(horse).unwrap().anchor, Pos::new(5, 7));
    assert_eq!(
        events,
        [Event::Moved {
            piece: horse,
            from: Pos::new(5, 5),
            to: Pos::new(5, 7)
        }]
    );
}

#[test]
fn r53_jet_penguin_stops_at_the_first_piece_in_front() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let jet = put(&mut state, &catalog, "jet-penguin", 0, 5, 10, Facing::North);
    put(&mut state, &catalog, "lion", 1, 5, 6, Facing::North);
    put(&mut state, &catalog, "lion", 1, 5, 3, Facing::North);
    assert_eq!(legal_targets(&catalog, &state, jet), [Pos::new(5, 6)]);
}

#[test]
fn r53_jet_penguin_stops_at_the_board_edge_without_obstacles() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let jet = put(&mut state, &catalog, "jet-penguin", 0, 5, 10, Facing::North);
    assert_eq!(legal_targets(&catalog, &state, jet), [Pos::new(5, 0)]);
    state.pieces[0].facing = Facing::East;
    assert_eq!(legal_targets(&catalog, &state, jet), [Pos::new(15, 10)]);
    // 前が盤の端なら、動けない
    state.pieces[0].anchor = Pos::new(15, 10);
    assert!(legal_targets(&catalog, &state, jet).is_empty());
}

#[test]
fn r53_jet_penguin_also_stops_at_own_pieces() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let jet = put(&mut state, &catalog, "jet-penguin", 0, 5, 10, Facing::North);
    put(&mut state, &catalog, "kid-lion", 0, 5, 7, Facing::North);
    assert_eq!(legal_targets(&catalog, &state, jet), [Pos::new(5, 7)]);
}

// ---- 攻撃と捕食（R-54〜R-59, R-5A） ----

#[test]
fn r54_r55_r56_r58_lion_captures_and_moves_and_takes_the_piece_into_hand() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    let victim = put(&mut state, &catalog, "kid-elephant", 1, 5, 4, Facing::North);
    let (next, events) = apply(&catalog, &state, &mv(lion, 5, 4)).unwrap();
    assert!(next.piece(victim).is_none());
    assert_eq!(next.piece(lion).unwrap().anchor, Pos::new(5, 4));
    assert_eq!(hand_count(&next, &catalog, 0, "kid-elephant"), 1);
    assert_eq!(
        events,
        [
            Event::Damaged {
                piece: victim,
                damage: 4,
                hp: 0
            },
            Event::Captured {
                piece: victim,
                kind: catalog.kind_of("kid-elephant").unwrap(),
                taker: PlayerId(0),
                to_hand: true
            },
            Event::Moved {
                piece: lion,
                from: Pos::new(5, 5),
                to: Pos::new(5, 4)
            },
        ]
    );
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn moving_to_an_empty_square_only_moves() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    let (next, events) = apply(&catalog, &state, &mv(lion, 6, 6)).unwrap();
    assert_eq!(
        events,
        [Event::Moved {
            piece: lion,
            from: Pos::new(5, 5),
            to: Pos::new(6, 6)
        }]
    );
    assert_eq!(next.piece(lion).unwrap().anchor, Pos::new(6, 6));
}

#[test]
fn r56_a_surviving_target_means_damage_only_and_no_move() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let horse = put(&mut state, &catalog, "horse", 0, 5, 5, Facing::North);
    let elephant = put(&mut state, &catalog, "elephant", 1, 4, 2, Facing::North); // 4..5 x 2..3
    let (next, events) = apply(&catalog, &state, &mv(horse, 5, 3)).unwrap();
    assert_eq!(next.piece(horse).unwrap().anchor, Pos::new(5, 5));
    assert_eq!(next.piece(elephant).unwrap().hp, 6); // 馬の攻撃力 2 × 重なり 1 マス
    assert_eq!(
        events,
        [Event::Damaged {
            piece: elephant,
            damage: 2,
            hp: 6
        }]
    );
}

#[test]
fn r54_damage_is_overlapping_cells_times_attack() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let attacker = put(&mut state, &catalog, "elephant", 0, 5, 5, Facing::North);
    let defender = put(&mut state, &catalog, "elephant", 1, 5, 3, Facing::North); // 5..6 x 3..4
    // 5,4 へ動くと、5..6 x 4..5 が、守る側の 5..6 x 4 の 2 マスに重なる
    let (next, events) = apply(&catalog, &state, &mv(attacker, 5, 4)).unwrap();
    assert_eq!(
        events,
        [Event::Damaged {
            piece: defender,
            damage: 4,
            hp: 4
        }]
    );
    assert_eq!(next.piece(defender).unwrap().hp, 4);
    assert_eq!(next.piece(attacker).unwrap().anchor, Pos::new(5, 5));
}

#[test]
fn r54_the_attacker_does_not_collide_with_itself() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let elephant = put(&mut state, &catalog, "elephant", 0, 5, 5, Facing::North);
    let (next, events) = apply(&catalog, &state, &mv(elephant, 5, 6)).unwrap();
    assert_eq!(
        events,
        [Event::Moved {
            piece: elephant,
            from: Pos::new(5, 5),
            to: Pos::new(5, 6)
        }]
    );
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r55_several_pieces_are_hit_at_once() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let elephant = put(&mut state, &catalog, "elephant", 0, 5, 5, Facing::North);
    let a = put(&mut state, &catalog, "kid-lion", 1, 5, 3, Facing::North);
    let b = put(&mut state, &catalog, "kid-lion", 2, 6, 3, Facing::North);
    let c = put(&mut state, &catalog, "elephant", 3, 7, 3, Facing::North); // 7..8 x 3..4
    // 5,3 へ動くと、5..6 x 3..4 になり、a と b は 1 マスずつ、c は重ならない
    let (next, events) = apply(&catalog, &state, &mv(elephant, 5, 3)).unwrap();
    assert!(next.piece(a).is_none() && next.piece(b).is_none());
    assert_eq!(next.piece(c).unwrap().hp, 8);
    assert_eq!(next.piece(elephant).unwrap().anchor, Pos::new(5, 3));
    assert_eq!(hand_count(&next, &catalog, 0, "kid-lion"), 2);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Captured { .. }))
            .count(),
        2
    );
}

#[test]
fn r55_if_one_target_survives_nothing_moves_but_the_other_is_still_captured() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let elephant = put(&mut state, &catalog, "elephant", 0, 5, 5, Facing::North);
    let weak = put(&mut state, &catalog, "kid-lion", 1, 5, 3, Facing::North);
    let strong = put(&mut state, &catalog, "elephant", 2, 6, 3, Facing::North); // 6..7 x 3..4
    let (next, _) = apply(&catalog, &state, &mv(elephant, 5, 3)).unwrap();
    assert!(next.piece(weak).is_none());
    assert_eq!(next.piece(strong).unwrap().hp, 4); // 2 マス重なる × 攻撃力 2
    assert_eq!(next.piece(elephant).unwrap().anchor, Pos::new(5, 5));
}

#[test]
fn r5a_capturing_your_own_piece_returns_it_to_your_hand() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    let own = put(&mut state, &catalog, "kid-lion", 0, 5, 4, Facing::North);
    let (next, _) = apply(&catalog, &state, &mv(lion, 5, 4)).unwrap();
    assert!(next.piece(own).is_none());
    assert_eq!(hand_count(&next, &catalog, 0, "kid-lion"), 1);
}

#[test]
fn r59_a_captured_castle_vanishes_and_is_not_added_to_the_hand() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    let castle = put(&mut state, &catalog, "castle", 1, 5, 3, Facing::North); // 5..6 x 3..4
    state.pieces.iter_mut().find(|p| p.id == castle).unwrap().hp = 4;
    let (next, events) = apply(&catalog, &state, &mv(lion, 5, 4)).unwrap();
    assert!(next.piece(castle).is_none());
    assert_eq!(hand_count(&next, &catalog, 0, "castle"), 0);
    assert!(events.contains(&Event::Captured {
        piece: castle,
        kind: catalog.castle(),
        taker: PlayerId(0),
        to_hand: false
    }));
}

#[test]
fn r59_your_own_castle_can_be_attacked_too() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    let castle = put(&mut state, &catalog, "castle", 0, 5, 3, Facing::North);
    let (next, events) = apply(&catalog, &state, &mv(lion, 5, 4)).unwrap();
    assert_eq!(next.piece(castle).unwrap().hp, 6);
    assert_eq!(
        events,
        [Event::Damaged {
            piece: castle,
            damage: 4,
            hp: 6
        }]
    );
}

#[test]
fn jet_penguin_charge_attacks_the_first_piece_it_meets() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let jet = put(&mut state, &catalog, "jet-penguin", 0, 5, 10, Facing::North);
    let victim = put(&mut state, &catalog, "lion", 1, 5, 6, Facing::North);
    let (next, events) = apply(&catalog, &state, &mv(jet, 5, 6)).unwrap();
    assert!(next.piece(victim).is_none());
    assert_eq!(next.piece(jet).unwrap().anchor, Pos::new(5, 6));
    assert_eq!(
        events[0],
        Event::Damaged {
            piece: victim,
            damage: 3,
            hp: 0
        }
    );
}

// ---- 成り（R-60〜R-63） ----

fn promotion_setup(catalog: &Catalog) -> (GameState, PieceId, PieceId) {
    let mut state = battle(catalog);
    let kid = put(&mut state, catalog, "kid-elephant", 0, 5, 5, Facing::North);
    let foal = put(&mut state, catalog, "foal", 1, 5, 4, Facing::North);
    (state, kid, foal)
}

fn promote_move(piece: PieceId, promote: bool) -> Action {
    Action::Move {
        piece,
        to: Pos::new(5, 4),
        promote,
    }
}

#[test]
fn r60_r61_capturing_lets_a_kid_promote_and_restores_full_hp() {
    let catalog = catalog();
    let (state, kid, foal) = promotion_setup(&catalog);
    let (next, events) = apply(&catalog, &state, &promote_move(kid, true)).unwrap();
    assert!(next.piece(foal).is_none());
    assert_eq!(kind_name(&catalog, &next, kid), "elephant");
    assert_eq!(next.piece(kid).unwrap().hp, 8);
    assert_eq!(next.piece(kid).unwrap().anchor, Pos::new(5, 4));
    assert!(matches!(events.last(), Some(Event::Promoted { .. })));
    assert_eq!(next.validate(&catalog), Ok(()));
}

#[test]
fn r60_promotion_is_optional() {
    let catalog = catalog();
    let (state, kid, _) = promotion_setup(&catalog);
    let (next, events) = apply(&catalog, &state, &promote_move(kid, false)).unwrap();
    assert_eq!(kind_name(&catalog, &next, kid), "kid-elephant");
    assert!(!events.iter().any(|e| matches!(e, Event::Promoted { .. })));
}

#[test]
fn r63_promote_without_a_capture_is_ignored() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let kid = put(&mut state, &catalog, "kid-elephant", 0, 5, 5, Facing::North);
    let (next, events) = apply(
        &catalog,
        &state,
        &Action::Move {
            piece: kid,
            to: Pos::new(5, 4),
            promote: true,
        },
    )
    .unwrap();
    assert_eq!(kind_name(&catalog, &next, kid), "kid-elephant");
    assert_eq!(events.len(), 1);
}

#[test]
fn r62_no_promotion_when_the_bigger_footprint_does_not_fit() {
    let catalog = catalog();
    let (mut state, kid, _) = promotion_setup(&catalog);
    put(&mut state, &catalog, "kid-lion", 2, 6, 4, Facing::North); // 成り先の範囲に、別の駒がいる
    let (next, events) = apply(&catalog, &state, &promote_move(kid, true)).unwrap();
    assert_eq!(kind_name(&catalog, &next, kid), "kid-elephant");
    assert!(!events.iter().any(|e| matches!(e, Event::Promoted { .. })));
}

#[test]
fn r62_no_promotion_at_the_board_edge_when_it_would_stick_out() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let kid = put(
        &mut state,
        &catalog,
        "kid-elephant",
        0,
        15,
        5,
        Facing::North,
    );
    put(&mut state, &catalog, "foal", 1, 15, 4, Facing::North);
    let action = Action::Move {
        piece: kid,
        to: Pos::new(15, 4),
        promote: true,
    };
    let (next, _) = apply(&catalog, &state, &action).unwrap();
    assert_eq!(kind_name(&catalog, &next, kid), "kid-elephant");
}

// ---- 移動の検証 ----

#[test]
fn move_is_rejected_for_wrong_owner_unknown_piece_or_illegal_target() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let mine = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    let theirs = put(&mut state, &catalog, "lion", 1, 10, 10, Facing::North);
    assert_eq!(
        apply(&catalog, &state, &mv(theirs, 10, 9)).unwrap_err(),
        ActionError::NotOwner
    );
    assert_eq!(
        apply(&catalog, &state, &mv(PieceId(99), 1, 1)).unwrap_err(),
        ActionError::UnknownPiece
    );
    assert_eq!(
        apply(&catalog, &state, &mv(mine, 5, 8)).unwrap_err(),
        ActionError::IllegalTarget
    );
    assert_eq!(
        apply(&catalog, &state, &mv(mine, 5, 5)).unwrap_err(),
        ActionError::IllegalTarget
    );
}

// ---- 回転（R-70〜R-72） ----

#[test]
fn r70_a_piece_turns_to_any_other_facing_in_one_action() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    for facing in [Facing::East, Facing::South, Facing::West] {
        let (next, events) = apply(
            &catalog,
            &state,
            &Action::Rotate {
                piece: lion,
                facing,
            },
        )
        .unwrap();
        assert_eq!(next.piece(lion).unwrap().facing, facing);
        assert_eq!(
            events,
            [Event::Rotated {
                piece: lion,
                facing
            }]
        );
        assert_eq!(next.piece(lion).unwrap().anchor, Pos::new(5, 5));
    }
}

#[test]
fn r70_same_facing_castles_and_other_players_pieces_cannot_rotate() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    let castle = put(&mut state, &catalog, "castle", 0, 10, 10, Facing::North);
    let theirs = put(&mut state, &catalog, "lion", 1, 0, 0, Facing::North);
    let rotate =
        |piece, facing| apply(&catalog, &state, &Action::Rotate { piece, facing }).map(|_| ());
    assert_eq!(rotate(lion, Facing::North), Err(ActionError::SameFacing));
    assert_eq!(rotate(castle, Facing::East), Err(ActionError::CannotRotate));
    assert_eq!(rotate(theirs, Facing::East), Err(ActionError::NotOwner));
    assert_eq!(
        rotate(PieceId(77), Facing::East),
        Err(ActionError::UnknownPiece)
    );
}

#[test]
fn r71_rotation_is_refused_when_the_new_footprint_does_not_fit() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let orca = put(&mut state, &catalog, "orca", 0, 15, 3, Facing::North); // 1x2: x=15, y=3..4
    put(&mut state, &catalog, "lion", 1, 7, 7, Facing::North);
    let rotate = |facing| {
        apply(
            &catalog,
            &state,
            &Action::Rotate {
                piece: orca,
                facing,
            },
        )
        .map(|_| ())
    };
    assert_eq!(rotate(Facing::East), Err(ActionError::OutsideBoard)); // 2x1 は x=15..16
    assert_eq!(rotate(Facing::South), Ok(()));
    // 別の駒に重なる向きにも回転できない
    let mut state = battle(&catalog);
    let orca = put(&mut state, &catalog, "orca", 0, 5, 5, Facing::North); // x=5, y=5..6
    put(&mut state, &catalog, "lion", 1, 6, 5, Facing::North);
    let rotate = |facing| {
        apply(
            &catalog,
            &state,
            &Action::Rotate {
                piece: orca,
                facing,
            },
        )
        .map(|_| ())
    };
    assert_eq!(rotate(Facing::East), Err(ActionError::Overlaps));
    assert_eq!(rotate(Facing::South), Ok(()));
}

// ---- パス（R-80）と手番の進行（R-22） ----

#[test]
fn r80_pass_is_refused_when_an_action_exists() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    assert!(has_legal_action(&catalog, &state));
    assert_eq!(
        apply(&catalog, &state, &Action::Pass).unwrap_err(),
        ActionError::PassNotAllowed
    );
}

#[test]
fn r80_pass_is_allowed_only_without_any_action_and_advances_the_turn() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    // 手番のプレイヤーには、駒も持ち駒もない
    put(&mut state, &catalog, "lion", 1, 5, 5, Facing::North);
    assert!(!has_legal_action(&catalog, &state));
    let (next, events) = apply(&catalog, &state, &Action::Pass).unwrap();
    assert_eq!(
        events,
        [Event::Passed {
            player: PlayerId(0)
        }]
    );
    assert_eq!(next.current, PlayerId(1));
    assert_eq!(next.turn, 1);
}

#[test]
fn r80_a_castle_alone_or_a_hand_with_no_legal_spot_means_pass() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "castle", 0, 0, 0, Facing::North);
    give(&mut state, &catalog, 0, "lion");
    assert!(has_legal_action(&catalog, &state)); // 城の隣に置ける
    // 周囲をすべて他の駒でふさぐと、置く場所がなくなる
    let mut blocked = battle(&catalog);
    put(&mut blocked, &catalog, "castle", 0, 0, 0, Facing::North);
    give(&mut blocked, &catalog, 0, "lion");
    for (x, y) in [(2, 0), (2, 1), (2, 2), (1, 2), (0, 2)] {
        put(&mut blocked, &catalog, "bear", 1, x, y, Facing::North);
    }
    assert!(legal_placements(&catalog, &blocked, catalog.kind_of("lion").unwrap()).is_empty());
    assert!(!has_legal_action(&catalog, &blocked));
}

#[test]
fn r22_turns_pass_in_seat_order_and_skip_eliminated_seats() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    put(&mut state, &catalog, "lion", 2, 10, 10, Facing::North);
    state.players[1].eliminated = true;
    state.out_batches.push(vec![PlayerId(1)]);
    let (next, _) = apply(
        &catalog,
        &state,
        &Action::Rotate {
            piece: lion,
            facing: Facing::East,
        },
    )
    .unwrap();
    assert_eq!(next.current, PlayerId(2));
    assert_eq!(next.turn, 1);
}

#[test]
fn r22_the_last_seat_wraps_around_to_the_first() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    state.current = PlayerId(3);
    let lion = put(&mut state, &catalog, "lion", 3, 5, 5, Facing::North);
    let (next, _) = apply(
        &catalog,
        &state,
        &Action::Rotate {
            piece: lion,
            facing: Facing::East,
        },
    )
    .unwrap();
    assert_eq!(next.current, PlayerId(0));
}

#[test]
fn finished_games_accept_no_actions() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    state.phase = Phase::Finished;
    assert_eq!(
        apply(
            &catalog,
            &state,
            &Action::Rotate {
                piece: lion,
                facing: Facing::East
            }
        )
        .unwrap_err(),
        ActionError::WrongPhase
    );
}

#[test]
fn moves_and_rotations_are_not_allowed_in_the_placement_phase() {
    let catalog = catalog();
    let mut state = new_game(&catalog, 2);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    assert_eq!(
        apply(&catalog, &state, &mv(lion, 5, 4)).unwrap_err(),
        ActionError::WrongPhase
    );
}

#[test]
fn apply_never_changes_the_original_state() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5, Facing::North);
    put(&mut state, &catalog, "kid-lion", 1, 5, 4, Facing::North);
    let before = state.clone();
    let _ = apply(&catalog, &state, &mv(lion, 5, 4)).unwrap();
    let _ = apply(&catalog, &state, &mv(lion, 9, 9));
    assert_eq!(state, before);
}

#[test]
fn legal_placements_lists_every_facing() {
    let catalog = catalog();
    let mut state = battle(&catalog);
    put(&mut state, &catalog, "castle", 0, 7, 7, Facing::North);
    give(&mut state, &catalog, 0, "kid-lion");
    let list = legal_placements(&catalog, &state, catalog.kind_of("kid-lion").unwrap());
    // 城（2x2）を囲む 12 マス × 4 方向
    assert_eq!(list.len(), 12 * 4);
}
