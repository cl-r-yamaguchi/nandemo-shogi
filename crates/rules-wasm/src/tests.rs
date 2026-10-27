use protocol::StateView;
use rules::action::{Action, apply, legal_actions, legal_placements, legal_targets};
use rules::pieces::{Catalog, KindId};
use rules::rng::SplitMix64;
use rules::state::{GameState, PieceId, PlayerId, Stage};
use rules::threats::threatened_pieces;
use rules::turn::draw_cards;

use super::*;

fn catalog() -> Catalog {
    Catalog::builtin().unwrap()
}

/// 配置を終えて、対戦フェーズを数手進めた 3 人の状態（最初に見つかる行動を指す）。
fn midgame() -> GameState {
    let catalog = catalog();
    let mut rng = SplitMix64(3);
    let mut state = GameState::new(&catalog, 3, PlayerId(0)).unwrap();
    for _ in 0..80 {
        if state.outcome.is_some() {
            break;
        }
        if state.stage == Stage::Draw {
            state = draw_cards(&catalog, &state, &mut rng).unwrap().0;
        }
        if let Stage::Pick(_) = state.stage {
            state = apply(&catalog, &state, &Action::Pick { index: 0 })
                .unwrap()
                .0;
        }
        let actions = legal_actions(&catalog, &state);
        let action = actions
            .iter()
            .find(|a| !matches!(a, Action::Pass))
            .unwrap_or(&Action::Pass);
        state = apply(&catalog, &state, action).unwrap().0;
    }
    // 行動を選べる段階（ドローと選択を終えた状態）で返す
    if state.outcome.is_none() && state.stage == Stage::Draw {
        state = draw_cards(&catalog, &state, &mut rng).unwrap().0;
    }
    if let Stage::Pick(_) = state.stage {
        state = apply(&catalog, &state, &Action::Pick { index: 0 })
            .unwrap()
            .0;
    }
    state
}

fn json_of(state: &GameState) -> String {
    serde_json::to_string(&StateView::from_state(state)).unwrap()
}

#[test]
fn piece_kinds_describe_every_kind_in_catalog_order() {
    let catalog = catalog();
    let kinds: Vec<serde_json::Value> =
        serde_json::from_str(&queries::piece_kinds().unwrap()).unwrap();
    assert_eq!(kinds.len(), catalog.len());
    let castle = catalog.castle().0;
    assert_eq!(kinds[castle]["castle"], true);
    assert_eq!(kinds[castle]["immobile"], true);
    for (kind, def) in catalog.iter() {
        assert_eq!(kinds[kind.0]["id"], def.id.as_str());
        assert_eq!(
            kinds[kind.0]["promotes_to"].as_u64().map(|k| k as usize),
            catalog.promotion(kind).map(|k| k.0)
        );
    }
}

#[test]
fn queries_match_calling_rules_directly() {
    let catalog = catalog();
    let state = midgame();
    assert!(state.pieces.len() > 3, "盤上に、動ける駒がいる");
    let json = json_of(&state);

    assert_eq!(
        queries::legal_actions(&json).unwrap(),
        serde_json::to_string(&legal_actions(&catalog, &state)).unwrap()
    );
    for piece in &state.pieces {
        assert_eq!(
            queries::legal_targets(&json, piece.id.0).unwrap(),
            serde_json::to_string(&legal_targets(&catalog, &state, piece.id)).unwrap()
        );
    }
    for (kind, _) in catalog.iter() {
        assert_eq!(
            queries::legal_placements(&json, kind.0 as u32).unwrap(),
            serde_json::to_string(&legal_placements(&catalog, &state, kind)).unwrap()
        );
    }
    assert_eq!(
        queries::threatened(&json).unwrap(),
        serde_json::to_string(&threatened_pieces(&catalog, &state)).unwrap()
    );
}

#[test]
fn preview_matches_apply() {
    let catalog = catalog();
    let state = midgame();
    let action = legal_actions(&catalog, &state)
        .into_iter()
        .next()
        .expect("行動がある");
    let (next, events) = apply(&catalog, &state, &action).unwrap();
    let out: serde_json::Value = serde_json::from_str(
        &queries::preview(&json_of(&state), &serde_json::to_string(&action).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(out["events"], serde_json::to_value(&events).unwrap());
    assert_eq!(
        out["state"],
        serde_json::to_value(StateView::from_state(&next)).unwrap()
    );
}

#[test]
fn a_restored_state_after_elimination_still_validates() {
    // 脱落した人がいる写しも、復元できる（脱落の順は近似）
    let catalog = catalog();
    let mut state = midgame();
    let victim = PlayerId(2);
    state = rules::finish::resign(&catalog, &state, victim).unwrap().0;
    assert!(state.players[2].eliminated);
    assert!(queries::legal_actions(&json_of(&state)).is_ok());
}

#[test]
fn bad_input_comes_back_as_errors() {
    let good = json_of(&midgame());
    assert!(queries::legal_actions("{").is_err(), "JSON ではない");
    assert!(queries::legal_actions("{}").is_err(), "形が違う");

    let mut view: serde_json::Value = serde_json::from_str(&good).unwrap();
    view["board_size"] = 0.into();
    assert!(queries::threatened(&view.to_string()).is_err(), "盤が 0");
    view["board_size"] = 4_000_000_000_u64.into();
    assert!(
        queries::threatened(&view.to_string()).is_err(),
        "盤が大きすぎる"
    );

    let mut view: serde_json::Value = serde_json::from_str(&good).unwrap();
    view["pieces"][0]["owner"] = 9.into();
    assert!(
        queries::threatened(&view.to_string()).is_err(),
        "持ち主が、席にいない"
    );
    let mut view: serde_json::Value = serde_json::from_str(&good).unwrap();
    view["pieces"][0]["kind"] = 999.into();
    assert!(
        queries::threatened(&view.to_string()).is_err(),
        "不明な種類"
    );

    assert!(queries::legal_placements(&good, 999).is_err(), "不明な種類");
    assert!(queries::preview(&good, "{}").is_err(), "行動の形が違う");
    let illegal = serde_json::to_string(&Action::Move {
        piece: PieceId(9999),
        to: rules::geometry::Pos::new(0, 0),
        promote: false,
    })
    .unwrap();
    assert!(queries::preview(&good, &illegal).is_err(), "規則に合わない");
    // 存在しない駒の移動先は、空
    assert_eq!(queries::legal_targets(&good, 9999).unwrap(), "[]");
    let _ = KindId(0);
}
