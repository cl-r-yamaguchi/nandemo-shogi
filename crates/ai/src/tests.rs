use std::time::{Duration, Instant};

use super::*;
use rules::geometry::{Facing, Pos};
use rules::pieces::Catalog;
use rules::rng::SplitMix64;
use rules::state::{GameState, Piece, PieceId, PlayerId};
use rules::turn::draw_cards;

fn catalog() -> Catalog {
    Catalog::builtin().unwrap()
}

fn far() -> Instant {
    Instant::now() + Duration::from_secs(60)
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

fn battle(catalog: &Catalog, players: usize) -> GameState {
    let mut state = GameState::new(catalog, players, PlayerId(0)).unwrap();
    state.phase = Phase::Battle;
    state.stage = Stage::Act;
    state.players.iter_mut().for_each(|p| p.hand.clear());
    let spots = [(0, 0), (13, 13), (13, 0), (0, 13)];
    for (seat, (x, y)) in spots.into_iter().take(players).enumerate() {
        put(&mut state, catalog, "castle", seat as u8, x, y);
    }
    state
}

/// 席ごとの強さで、終局まで対局する。毎回の手が合法であることも確かめる。終局後の状態を返す。
fn play(catalog: &Catalog, levels: &[Level], seed: u64) -> GameState {
    play_with_deadline(catalog, levels, seed, Duration::from_secs(60))
}

/// 1 手あたりの持ち時間を指定して、対局する。
fn play_with_deadline(
    catalog: &Catalog,
    levels: &[Level],
    seed: u64,
    per_move: Duration,
) -> GameState {
    let mut rng = SplitMix64(seed);
    let first = PlayerId(rng.below(levels.len() as u64) as u8);
    let mut state = GameState::new(catalog, levels.len(), first).unwrap();
    for _ in 0..10_000 {
        if state.phase == Phase::Finished {
            return state;
        }
        let level = levels[usize::from(state.current.0)];
        let step = match (&state.stage, state.phase) {
            (Stage::Draw, Phase::Battle) => draw_cards(catalog, &state, &mut rng).unwrap().0,
            (Stage::Pick(_), _) => {
                let index = choose_pick(catalog, &state, level, &mut rng).expect("選べるはず");
                apply(catalog, &state, &Action::Pick { index })
                    .expect("選択は受理される")
                    .0
            }
            _ => {
                let action =
                    choose_action(catalog, &state, level, Instant::now() + per_move, &mut rng)
                        .expect("行動があるはず");
                apply(catalog, &state, &action)
                    .unwrap_or_else(|e| panic!("AI が不正な行動を選びました: {action:?}: {e}"))
                    .0
            }
        };
        state = step;
    }
    panic!("終局しませんでした");
}

// ---- 強さ 1 ----

#[test]
fn level1_always_picks_a_legal_action_and_a_valid_card() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    put(&mut state, &catalog, "lion", 0, 5, 5);
    put(&mut state, &catalog, "kid-elephant", 1, 5, 4);
    let legal = legal_actions(&catalog, &state);
    for seed in 0..50 {
        let action =
            choose_action(&catalog, &state, Level::One, far(), &mut SplitMix64(seed)).unwrap();
        assert!(legal.contains(&action));
    }
    let (picking, _) = draw_cards(
        &catalog,
        &{
            let mut s = state.clone();
            s.stage = Stage::Draw;
            s
        },
        &mut SplitMix64(1),
    )
    .unwrap();
    for seed in 0..50 {
        let index = choose_pick(&catalog, &picking, Level::One, &mut SplitMix64(seed)).unwrap();
        assert!(index < 4);
    }
}

#[test]
fn level1_prefers_a_capture_when_one_exists() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let lion = put(&mut state, &catalog, "lion", 0, 5, 5);
    let victim = put(&mut state, &catalog, "kid-elephant", 1, 5, 4);
    for seed in 0..50 {
        let action =
            choose_action(&catalog, &state, Level::One, far(), &mut SplitMix64(seed)).unwrap();
        let (next, _) = apply(&catalog, &state, &action).unwrap();
        assert!(
            next.piece(victim).is_none(),
            "seed {seed}: 捕食できるのに、{action:?} を選んだ"
        );
        assert!(matches!(action, Action::Move { piece, .. } if piece == lion));
    }
}

#[test]
fn level1_actually_varies_its_choices() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "lion", 0, 5, 5);
    let chosen: std::collections::BTreeSet<String> = (0..60)
        .map(|seed| {
            let a =
                choose_action(&catalog, &state, Level::One, far(), &mut SplitMix64(seed)).unwrap();
            format!("{a:?}")
        })
        .collect();
    assert!(chosen.len() > 3);
}

// ---- 強さ 2 ----

#[test]
fn level2_takes_a_capture_that_wins_the_game() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let lion = put(&mut state, &catalog, "lion", 0, 12, 13);
    state
        .pieces
        .iter_mut()
        .find(|p| p.owner == PlayerId(1))
        .unwrap()
        .hp = 4; // 相手の城。1 回で捕食できる
    for seed in 0..20 {
        let action =
            choose_action(&catalog, &state, Level::Two, far(), &mut SplitMix64(seed)).unwrap();
        assert!(matches!(action, Action::Move { piece, .. } if piece == lion));
        let (next, _) = apply(&catalog, &state, &action).unwrap();
        assert_eq!(next.phase, Phase::Finished);
    }
}

#[test]
fn level2_does_not_leave_a_valuable_piece_hanging_if_it_can_avoid_it() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    // ライオンは、敵のライオンの隣。動かさなければ捕食される位置。動かせば、逃げられる
    let mine = put(&mut state, &catalog, "lion", 0, 5, 5);
    put(&mut state, &catalog, "lion", 1, 5, 4);
    for seed in 0..10 {
        let action =
            choose_action(&catalog, &state, Level::Two, far(), &mut SplitMix64(seed)).unwrap();
        let (next, _) = apply(&catalog, &state, &action).unwrap();
        let still_there = next.piece(mine).is_some();
        let threatened = rules::threats::threatened_pieces(&catalog, &next).contains(&mine);
        assert!(
            !still_there || !threatened || matches!(action, Action::Move { .. }),
            "seed {seed}: {action:?}"
        );
    }
}

#[test]
fn level2_picks_the_most_valuable_card() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let kinds =
        ["kid-lion", "elephant", "foal", "kid-elephant"].map(|id| catalog.kind_of(id).unwrap());
    state.stage = Stage::Pick(kinds.to_vec());
    for seed in 0..10 {
        let index = choose_pick(&catalog, &state, Level::Two, &mut SplitMix64(seed)).unwrap();
        assert_eq!(kinds[index], catalog.kind_of("elephant").unwrap());
    }
}

#[test]
fn level2_places_its_castle_far_from_the_others() {
    let catalog = catalog();
    let mut state = GameState::new(&catalog, 2, PlayerId(1)).unwrap();
    put(&mut state, &catalog, "castle", 0, 0, 0);
    state.players[0].hand.clear();
    let action = choose_action(&catalog, &state, Level::Two, far(), &mut SplitMix64(3)).unwrap();
    let Action::Place { anchor, .. } = action else {
        panic!("城の配置のはず: {action:?}")
    };
    // すきまが最大（盤の反対側の端。縦横の大きいほうで測るので、同じ値の位置が複数ある）
    let def = catalog.get(catalog.castle()).unwrap();
    let chosen = def.footprint(anchor, Facing::North);
    let existing = def.footprint(Pos::new(0, 0), Facing::North);
    assert_eq!(chosen.gap(&existing), 12);
}

#[test]
fn level2_returns_a_legal_action_even_after_the_deadline() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "lion", 0, 5, 5);
    put(&mut state, &catalog, "elephant", 0, 8, 8);
    let legal = legal_actions(&catalog, &state);
    let passed = Instant::now() - Duration::from_secs(1);
    let action = choose_action(&catalog, &state, Level::Two, passed, &mut SplitMix64(1)).unwrap();
    assert!(legal.contains(&action));
}

#[test]
fn level2_thins_out_a_huge_number_of_placements_but_keeps_every_move() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    put(&mut state, &catalog, "elephant", 0, 6, 6);
    put(&mut state, &catalog, "orca", 0, 9, 3);
    for id in ["lion", "bear", "deer", "horse", "foal", "kid-lion"] {
        *state.players[0]
            .hand
            .entry(catalog.kind_of(id).unwrap())
            .or_insert(0) += 1;
    }
    let all = legal_actions(&catalog, &state);
    let places = all
        .iter()
        .filter(|a| matches!(a, Action::Place { .. }))
        .count();
    assert!(places > MAX_PLACEMENTS, "間引きの対象になる数: {places}");
    let thinned = thin_out_placements(all.clone(), &mut SplitMix64(1));
    assert_eq!(
        thinned
            .iter()
            .filter(|a| matches!(a, Action::Place { .. }))
            .count(),
        MAX_PLACEMENTS
    );
    for action in all.iter().filter(|a| !matches!(a, Action::Place { .. })) {
        assert!(thinned.contains(action));
    }
    assert!(thinned.iter().all(|a| all.contains(a)));
    for level in [Level::Two, Level::Three] {
        let chosen = choose_action(&catalog, &state, level, far(), &mut SplitMix64(1)).unwrap();
        assert!(all.contains(&chosen));
    }
}

#[test]
fn pick_best_handles_infinite_values_and_ties() {
    let mut rng = SplitMix64(1);
    assert!(pick_best(&[f64::NEG_INFINITY, f64::NEG_INFINITY], &mut rng) < 2);
    assert_eq!(pick_best(&[1.0, f64::INFINITY, 3.0], &mut rng), 1);
    let tie_picks: std::collections::BTreeSet<usize> = (0..40)
        .map(|s| pick_best(&[2.0, 2.0, 1.0], &mut SplitMix64(s)))
        .collect();
    assert_eq!(tie_picks, std::collections::BTreeSet::from([0, 1]));
}

// ---- 強さ 3 ----

/// 次の局面で、`loser` が、相手の 1 手で負ける（城を失って終局する）手があるか。
fn can_lose_next_move(catalog: &Catalog, state: &GameState, loser: PlayerId) -> bool {
    let mut next = state.clone();
    if next.phase != Phase::Battle {
        return false;
    }
    next.stage = Stage::Act;
    legal_actions(catalog, &next)
        .into_iter()
        .filter(|a| matches!(a, Action::Move { .. }))
        .any(|a| {
            apply(catalog, &next, &a).is_ok_and(|(after, _)| {
                after.phase == Phase::Finished
                    && after
                        .outcome
                        .as_ref()
                        .is_some_and(|o| !o.winners().contains(&loser))
            })
        })
}

/// 自分の城（HP 4）の隣に、敵のライオンがいて、次の手で城を捕食される。
/// 防ぐには、自分のライオンで、敵のライオンを捕食する（手番は自分）。ほかの行動は、すべて、負けにつながる。
fn castle_under_attack(catalog: &Catalog) -> (GameState, PieceId) {
    let mut state = battle(catalog, 2);
    state
        .pieces
        .iter_mut()
        .find(|p| p.owner == PlayerId(0))
        .unwrap()
        .hp = 4;
    put(&mut state, catalog, "lion", 1, 2, 0); // 城（0..1 x 0..1）の隣。1,0 へ動くと、城を攻撃する
    let defender = put(&mut state, catalog, "lion", 0, 3, 1);
    // 囮: 価値の高い敵の駒を捕食できる位置にも、自分の駒を置く
    put(&mut state, catalog, "bear", 0, 9, 9);
    put(&mut state, catalog, "kid-elephant", 1, 9, 8);
    (state, defender)
}

#[test]
fn level3_avoids_a_move_that_loses_the_game_next_turn() {
    let catalog = catalog();
    let (state, _) = castle_under_attack(&catalog);
    // 何もしなければ（手番を相手に渡せば）、負ける局面
    let mut if_pass = state.clone();
    if_pass.current = PlayerId(1);
    assert!(can_lose_next_move(&catalog, &if_pass, PlayerId(0)));
    for seed in 0..5 {
        let action =
            choose_action(&catalog, &state, Level::Three, far(), &mut SplitMix64(seed)).unwrap();
        let (after, _) = apply(&catalog, &state, &action).unwrap();
        assert!(
            !can_lose_next_move(&catalog, &after, PlayerId(0)),
            "seed {seed}: {action:?} は、次の手で負ける"
        );
    }
}

#[test]
fn level3_takes_a_capture_that_wins_the_game() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    let lion = put(&mut state, &catalog, "lion", 0, 12, 13);
    state
        .pieces
        .iter_mut()
        .find(|p| p.owner == PlayerId(1))
        .unwrap()
        .hp = 4;
    for seed in 0..5 {
        let action =
            choose_action(&catalog, &state, Level::Three, far(), &mut SplitMix64(seed)).unwrap();
        assert!(matches!(action, Action::Move { piece, .. } if piece == lion));
        let (next, _) = apply(&catalog, &state, &action).unwrap();
        assert_eq!(next.phase, Phase::Finished);
    }
}

#[test]
fn level3_returns_a_legal_action_even_after_the_deadline() {
    let catalog = catalog();
    let (state, _) = castle_under_attack(&catalog);
    let legal = legal_actions(&catalog, &state);
    let passed = Instant::now() - Duration::from_secs(1);
    let action = choose_action(&catalog, &state, Level::Three, passed, &mut SplitMix64(1)).unwrap();
    assert!(legal.contains(&action));
}

#[test]
fn level3_stops_near_the_deadline() {
    let catalog = catalog();
    let mut state = battle(&catalog, 4);
    for (id, owner, x, y) in [
        ("elephant", 0, 5, 5),
        ("orca", 0, 8, 2),
        ("lion", 1, 10, 10),
        ("horse", 1, 3, 9),
        ("deer", 2, 12, 5),
        ("bear", 2, 6, 12),
        ("jet-penguin", 3, 2, 3),
        ("foal", 3, 7, 7),
    ] {
        put(&mut state, &catalog, id, owner, x, y);
    }
    let limit = Duration::from_millis(200);
    let start = Instant::now();
    let action = choose_action(
        &catalog,
        &state,
        Level::Three,
        start + limit,
        &mut SplitMix64(1),
    )
    .unwrap();
    let elapsed = start.elapsed();
    assert!(legal_actions(&catalog, &state).contains(&action));
    assert!(
        elapsed < Duration::from_secs(3),
        "締め切り（{limit:?}）を大きく超えた: {elapsed:?}"
    );
}

#[test]
fn level3_is_reproducible_when_the_search_finishes() {
    let catalog = catalog();
    let (state, _) = castle_under_attack(&catalog);
    let a = choose_action(&catalog, &state, Level::Three, far(), &mut SplitMix64(5));
    let b = choose_action(&catalog, &state, Level::Three, far(), &mut SplitMix64(5));
    assert_eq!(a, b);
}

#[test]
fn level3_in_placement_chooses_like_level2() {
    let catalog = catalog();
    let mut state = GameState::new(&catalog, 2, PlayerId(1)).unwrap();
    put(&mut state, &catalog, "castle", 0, 0, 0);
    state.players[0].hand.clear();
    let two = choose_action(&catalog, &state, Level::Two, far(), &mut SplitMix64(3));
    let three = choose_action(&catalog, &state, Level::Three, far(), &mut SplitMix64(3));
    assert_eq!(two, three);
}

// ---- 共通 ----

#[test]
fn no_action_or_pick_outside_the_right_stage() {
    let catalog = catalog();
    let mut state = battle(&catalog, 2);
    state.stage = Stage::Draw;
    assert!(choose_action(&catalog, &state, Level::Two, far(), &mut SplitMix64(1)).is_none());
    assert!(choose_pick(&catalog, &state, Level::Two, &mut SplitMix64(1)).is_none());
    state.stage = Stage::Act;
    assert!(choose_pick(&catalog, &state, Level::One, &mut SplitMix64(1)).is_none());
}

#[test]
fn the_same_seed_and_state_give_the_same_move() {
    let catalog = catalog();
    let mut state = battle(&catalog, 3);
    put(&mut state, &catalog, "lion", 0, 5, 5);
    put(&mut state, &catalog, "deer", 0, 8, 8);
    for level in [Level::One, Level::Two] {
        let a = choose_action(&catalog, &state, level, far(), &mut SplitMix64(77));
        let b = choose_action(&catalog, &state, level, far(), &mut SplitMix64(77));
        assert_eq!(a, b);
    }
}

#[test]
fn level_numbers_map_to_levels() {
    assert_eq!(Level::from_number(1), Some(Level::One));
    assert_eq!(Level::from_number(2), Some(Level::Two));
    assert_eq!(Level::from_number(3), Some(Level::Three));
    assert_eq!(Level::from_number(0), None);
    assert_eq!(Level::from_number(4), None);
}

// ---- 対局 ----

#[test]
fn level1_players_finish_a_four_player_game_with_only_legal_moves() {
    let catalog = catalog();
    let done = play(&catalog, &[Level::One; 4], 1);
    assert_eq!(done.phase, Phase::Finished);
    assert_eq!(done.validate(&catalog), Ok(()));
    assert!(done.outcome.is_some());
}

#[test]
fn level2_players_finish_a_two_player_game_with_only_legal_moves() {
    let catalog = catalog();
    let done = play(&catalog, &[Level::Two, Level::One], 2);
    assert_eq!(done.phase, Phase::Finished);
    assert_eq!(done.validate(&catalog), Ok(()));
}

#[test]
fn level3_players_finish_a_two_player_game_with_only_legal_moves() {
    let catalog = catalog();
    let done = play(&catalog, &[Level::Three, Level::Two], 4);
    assert_eq!(done.phase, Phase::Finished);
    assert_eq!(done.validate(&catalog), Ok(()));
}

#[test]
fn mixed_levels_finish_a_three_player_game() {
    let catalog = catalog();
    let done = play(&catalog, &[Level::Two, Level::One, Level::Three], 3);
    assert_eq!(done.phase, Phase::Finished);
}

/// 強さ 2 が、強さ 1 より強いことの確認。時間がかかるので、通常のテストからは外している。
/// 実行: `cargo test -p ai --release -- --ignored level2_beats_level1 --nocapture`
#[test]
#[ignore = "時間がかかる。強さの確認用"]
fn level2_beats_level1_over_many_games() {
    let catalog = catalog();
    let games = 20u64;
    let mut level2_wins = 0;
    let mut level1_wins = 0;
    for game in 0..games {
        // 席を入れ替えて、先手・後手の差を打ち消す
        let levels = if game % 2 == 0 {
            [Level::Two, Level::One]
        } else {
            [Level::One, Level::Two]
        };
        let done = play(&catalog, &levels, 100 + game);
        let outcome = done.outcome.expect("終局しているはず");
        if let [winner] = outcome.winners() {
            if levels[usize::from(winner.0)] == Level::Two {
                level2_wins += 1;
            } else {
                level1_wins += 1;
            }
        }
    }
    println!("強さ 2 の勝ち: {level2_wins}、強さ 1 の勝ち: {level1_wins}（{games} 局）");
    assert!(level2_wins > level1_wins);
}

/// 強さ 3 が、強さ 2 より強いことの確認。時間がかかるので、通常のテストからは外している。
/// 実行: `cargo test -p ai --release -- --ignored level3_beats_level2 --nocapture`
#[test]
#[ignore = "時間がかかる。強さの確認用"]
fn level3_beats_level2_over_many_games() {
    let catalog = catalog();
    let games = 20u64;
    let mut level3_wins = 0;
    let mut level2_wins = 0;
    for game in 0..games {
        let levels = if game % 2 == 0 {
            [Level::Three, Level::Two]
        } else {
            [Level::Two, Level::Three]
        };
        let done = play_with_deadline(&catalog, &levels, 200 + game, Duration::from_millis(500));
        let outcome = done.outcome.expect("終局しているはず");
        if let [winner] = outcome.winners() {
            if levels[usize::from(winner.0)] == Level::Three {
                level3_wins += 1;
            } else {
                level2_wins += 1;
            }
        }
    }
    println!("強さ 3 の勝ち: {level3_wins}、強さ 2 の勝ち: {level2_wins}（{games} 局）");
    assert!(level3_wins > level2_wins);
}
