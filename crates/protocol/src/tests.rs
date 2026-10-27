use rules::action::{Action, ActionError, Event};
use rules::finish::{EndReason, Outcome};
use rules::geometry::{Facing, Pos};
use rules::pieces::{Catalog, KindId};
use rules::state::{GameState, Phase, PieceId, PlayerId, Stage};
use serde_json::{Value, json};

use super::*;

fn state() -> GameState {
    let catalog = Catalog::builtin().unwrap();
    GameState::new(&catalog, 4, PlayerId(1)).unwrap()
}

fn sample_id() -> String {
    "a1b2c3d4e5f6a7b8c9d0".to_string()
}

fn decode(value: Value) -> Result<ClientFrame, DecodeError> {
    decode_client(&value.to_string())
}

fn hello(name: &str) -> Value {
    json!({ "v": 1, "type": "hello", "player_id": sample_id(), "display_name": name })
}

// ---- JSON の形（online.md の 6 章） ----

#[test]
fn client_messages_have_the_documented_shape() {
    let frame = decode(hello("ゲスト1234")).unwrap();
    assert_eq!(frame.v, PROTOCOL_VERSION);
    assert_eq!(
        frame.message,
        ClientMessage::Hello {
            player_id: sample_id(),
            display_name: "ゲスト1234".into()
        }
    );

    let find =
        decode(json!({ "v": 1, "type": "find_room", "rules": { "players": 4, "ai_level": 2 } }))
            .unwrap();
    assert_eq!(
        find.message,
        ClientMessage::FindRoom {
            rules: RoomRules::DEFAULT
        }
    );

    for (kind, expected) in [
        ("leave_room", ClientMessage::LeaveRoom),
        ("start", ClientMessage::Start),
        ("resign", ClientMessage::Resign),
    ] {
        let frame = decode(json!({ "v": 1, "type": kind })).unwrap();
        assert_eq!(frame.message, expected);
    }
}

#[test]
fn act_carries_rules_actions_as_they_are() {
    let place = decode(json!({
        "v": 1, "type": "act", "turn": 3,
        "action": { "type": "place", "kind": 2, "anchor": { "x": 4, "y": 5 }, "facing": "west" }
    }))
    .unwrap();
    assert_eq!(
        place.message,
        ClientMessage::Act {
            turn: 3,
            action: Action::Place {
                kind: KindId(2),
                anchor: Pos::new(4, 5),
                facing: Facing::West
            }
        }
    );
    let moved = decode(json!({
        "v": 1, "type": "act", "turn": 4,
        "action": { "type": "move", "piece": 7, "to": { "x": 1, "y": 2 }, "promote": true }
    }))
    .unwrap();
    assert_eq!(
        moved.message,
        ClientMessage::Act {
            turn: 4,
            action: Action::Move {
                piece: PieceId(7),
                to: Pos::new(1, 2),
                promote: true
            }
        }
    );
    let rotate = decode(json!({
        "v": 1, "type": "act", "turn": 5,
        "action": { "type": "rotate", "piece": 1, "facing": "south" }
    }));
    assert!(rotate.is_ok());
    // 選択も、行動として送る
    let pick = decode(
        json!({ "v": 1, "type": "act", "turn": 6, "action": { "type": "pick", "index": 2 } }),
    )
    .unwrap();
    assert_eq!(
        pick.message,
        ClientMessage::Act {
            turn: 6,
            action: Action::Pick { index: 2 }
        }
    );
    let pass =
        decode(json!({ "v": 1, "type": "act", "turn": 6, "action": { "type": "pass" } })).unwrap();
    assert_eq!(
        pass.message,
        ClientMessage::Act {
            turn: 6,
            action: Action::Pass
        }
    );
}

#[test]
fn every_client_message_round_trips() {
    let messages = vec![
        ClientMessage::Hello {
            player_id: sample_id(),
            display_name: "あ".into(),
        },
        ClientMessage::FindRoom {
            rules: RoomRules {
                players: 2,
                ai_level: 3,
            },
        },
        ClientMessage::LeaveRoom,
        ClientMessage::Start,
        ClientMessage::Act {
            turn: 0,
            action: Action::Pass,
        },
        ClientMessage::Act {
            turn: 9,
            action: Action::Pick { index: 3 },
        },
        ClientMessage::Resign,
    ];
    for message in messages {
        let frame = ClientFrame {
            v: PROTOCOL_VERSION,
            message,
        };
        let text = serde_json::to_string(&frame).unwrap();
        assert_eq!(decode_client(&text).unwrap(), frame);
    }
}

fn view() -> StateView {
    StateView::from_state(&state())
}

fn room() -> RoomView {
    RoomView {
        room_id: "r1".into(),
        status: RoomStatus::Waiting,
        rules: RoomRules::DEFAULT,
        seats: vec![SeatView {
            seat: PlayerId(0),
            kind: SeatKind::Human,
            name: "ゲスト1".into(),
            ai_level: None,
            connected: true,
        }],
    }
}

#[test]
fn every_server_message_round_trips_and_has_a_type_and_version() {
    let outcome = Outcome {
        reason: EndReason::LastStanding,
        ranking: vec![vec![PlayerId(0)], vec![PlayerId(1)]],
    };
    let messages = vec![
        ServerMessage::Welcome {
            player_id: sample_id(),
            display_name: "あ".into(),
            reconnected: false,
        },
        ServerMessage::RoomState { room: room() },
        ServerMessage::GameStarted {
            room: room(),
            you: PlayerId(2),
            state: view(),
        },
        ServerMessage::TurnStarted {
            player: PlayerId(1),
            turn: 7,
            remaining_ms: 59_000,
        },
        ServerMessage::State { state: view() },
        ServerMessage::Applied {
            turn: 7,
            events: vec![
                Event::Passed {
                    player: PlayerId(1),
                },
                Event::Drawn {
                    player: PlayerId(2),
                    cards: vec![KindId(1), KindId(2)],
                },
            ],
            state: view(),
        },
        ServerMessage::illegal_action(ActionError::NotAdjacentToOwn),
        ServerMessage::GameOver {
            outcome,
            state: view(),
        },
        ServerMessage::Notice {
            kind: NoticeKind::AiTakeover,
            message: "x".into(),
        },
    ];
    let types = [
        "welcome",
        "room_state",
        "game_started",
        "turn_started",
        "state",
        "applied",
        "rejected",
        "game_over",
        "notice",
    ];
    for (message, expected_type) in messages.into_iter().zip(types) {
        let frame = ServerFrame::new(message);
        let text = encode_server(&frame);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["v"], PROTOCOL_VERSION);
        assert_eq!(value["type"], expected_type);
        assert_eq!(serde_json::from_str::<ServerFrame>(&text).unwrap(), frame);
    }
}

#[test]
fn events_use_a_snake_case_type_tag() {
    let value = serde_json::to_value(Event::Captured {
        piece: PieceId(3),
        kind: KindId(4),
        taker: PlayerId(0),
        to_hand: true,
    })
    .unwrap();
    assert_eq!(
        value,
        json!({ "type": "captured", "piece": 3, "kind": 4, "taker": 0, "to_hand": true })
    );
}

#[test]
fn rejections_carry_a_plain_code_and_an_optional_reason() {
    let text = encode_server(&ServerFrame::new(ServerMessage::illegal_action(
        ActionError::WrongStage,
    )));
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["type"], "rejected");
    assert_eq!(value["code"], "illegal_action");
    assert_eq!(value["reason"], "wrong_stage");
    assert!(value["message"].as_str().is_some_and(|m| !m.is_empty()));
    let plain =
        serde_json::to_value(ServerMessage::rejected(RejectCode::StaleTurn, "古い")).unwrap();
    assert_eq!(
        plain,
        json!({ "type": "rejected", "code": "stale_turn", "reason": null, "message": "古い" })
    );
}

// ---- 盤面の写し ----

#[test]
fn state_view_exposes_the_game_without_internals() {
    let catalog = Catalog::builtin().unwrap();
    let state = state();
    let view = StateView::from_state(&state);
    assert_eq!(view.board_size, 16);
    assert_eq!(view.phase, Phase::Placement);
    assert_eq!(view.current, PlayerId(1));
    assert_eq!(view.players.len(), 4);
    assert_eq!(
        view.players[0].hand,
        vec![HandEntry {
            kind: catalog.castle(),
            count: 1
        }]
    );
    assert!(view.pieces.is_empty());
    assert!(view.outcome.is_none());
    let value = serde_json::to_value(&view).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        !keys
            .iter()
            .any(|k| k.contains("next_piece_id") || k.contains("out_batches"))
    );
}

#[test]
fn state_view_carries_the_drawn_cards_in_the_stage() {
    let mut state = state();
    state.phase = Phase::Battle;
    state.stage = Stage::Pick(vec![KindId(1), KindId(2), KindId(3), KindId(4)]);
    let value = serde_json::to_value(StateView::from_state(&state)).unwrap();
    assert_eq!(
        value["stage"],
        json!({ "type": "pick", "cards": [1, 2, 3, 4] })
    );
    assert_eq!(value["phase"], "battle");
}

// ---- 受信の検証（online.md の 6.4） ----

#[test]
fn oversized_messages_are_rejected_before_parsing() {
    let text = format!(
        "{{\"v\":1,\"type\":\"hello\",\"pad\":\"{}\"}}",
        "x".repeat(MAX_MESSAGE_BYTES)
    );
    let error = decode_client(&text).unwrap_err();
    assert!(matches!(error, DecodeError::TooLarge { .. }));
    assert_eq!(error.reject_code(), RejectCode::TooLarge);
    // 上限ちょうどは、サイズの検査を通る
    let exact = "x".repeat(MAX_MESSAGE_BYTES);
    assert!(!matches!(
        decode_client(&exact),
        Err(DecodeError::TooLarge { .. })
    ));
}

#[test]
fn malformed_messages_are_rejected_with_a_reason() {
    for text in [
        "",
        "not json",
        "[]",
        "{}",
        "{\"v\":1}",
        "{\"v\":1,\"type\":\"unknown\"}",
        "{\"v\":1,\"type\":\"act\"}",
        "{\"v\":\"1\",\"type\":\"start\"}",
    ] {
        let error = decode_client(text).unwrap_err();
        assert!(
            matches!(error, DecodeError::Malformed(_)),
            "{text}: {error:?}"
        );
        assert_eq!(error.reject_code(), RejectCode::BadMessage);
        assert!(!error.message().is_empty());
    }
}

#[test]
fn a_different_version_is_rejected_even_if_the_rest_is_unknown() {
    for v in [0, 2, 99] {
        let error =
            decode_client(&json!({ "v": v, "type": "from_the_future", "x": 1 }).to_string())
                .unwrap_err();
        assert_eq!(error, DecodeError::VersionMismatch { got: v.to_string() });
        assert_eq!(error.reject_code(), RejectCode::VersionMismatch);
    }
}

#[test]
fn rules_must_be_in_range() {
    for (players, ai_level) in [(1, 2), (5, 2), (0, 0), (4, 0), (4, 4), (255, 255)] {
        let error = decode(json!({ "v": 1, "type": "find_room", "rules": { "players": players, "ai_level": ai_level } })).unwrap_err();
        assert_eq!(
            error,
            DecodeError::Field(FieldError::Rules),
            "{players}, {ai_level}"
        );
        assert_eq!(error.reject_code(), RejectCode::InvalidRules);
    }
    for (players, ai_level) in [(2, 1), (4, 3), (3, 2)] {
        assert!(decode(json!({ "v": 1, "type": "find_room", "rules": { "players": players, "ai_level": ai_level } })).is_ok());
    }
    // 型が違う値（負数・文字列）は、形のエラー
    let error =
        decode(json!({ "v": 1, "type": "find_room", "rules": { "players": -1, "ai_level": "2" } }))
            .unwrap_err();
    assert!(matches!(error, DecodeError::Malformed(_)));
}

#[test]
fn default_rules_are_four_players_and_level_two() {
    assert_eq!(
        RoomRules::DEFAULT,
        RoomRules {
            players: 4,
            ai_level: 2
        }
    );
    assert!(RoomRules::DEFAULT.is_valid());
}

#[test]
fn display_names_are_trimmed_and_limited() {
    assert_eq!(
        validate_display_name("  ゲスト1234  ").unwrap(),
        "ゲスト1234"
    );
    assert_eq!(
        validate_display_name("あ".repeat(12).as_str())
            .unwrap()
            .chars()
            .count(),
        12
    );
    assert_eq!(
        validate_display_name(&"あ".repeat(13)),
        Err(FieldError::DisplayName)
    );
    for bad in ["", "   ", "a\nb", "a\u{0}b", "\u{7f}", "tab\there"] {
        assert_eq!(
            validate_display_name(bad),
            Err(FieldError::DisplayName),
            "{bad:?}"
        );
    }
    // 文字数で数える（バイト数ではない）
    assert!(validate_display_name("日本語の名前です").is_ok());
}

#[test]
fn decode_normalizes_the_display_name() {
    let frame = decode(hello("  名前  ")).unwrap();
    assert_eq!(
        frame.message,
        ClientMessage::Hello {
            player_id: sample_id(),
            display_name: "名前".into()
        }
    );
    let error = decode(hello("")).unwrap_err();
    assert_eq!(error.reject_code(), RejectCode::InvalidName);
}

#[test]
fn player_ids_are_16_to_64_safe_characters() {
    assert!(validate_player_id(&"a".repeat(16)).is_ok());
    assert!(validate_player_id(&"A-b_9".repeat(12)).is_ok()); // 60 文字
    assert!(validate_player_id(&"a".repeat(64)).is_ok());
    for bad in [
        "".to_string(),
        "a".repeat(15),
        "a".repeat(65),
        format!("{}!", "a".repeat(16)),
        format!("{} {}", "a".repeat(8), "a".repeat(8)),
        format!("{}あ", "a".repeat(16)),
        format!("{}/../", "a".repeat(16)),
    ] {
        assert_eq!(
            validate_player_id(&bad),
            Err(FieldError::PlayerId),
            "{bad:?}"
        );
    }
    let error =
        decode(json!({ "v": 1, "type": "hello", "player_id": "short", "display_name": "x" }))
            .unwrap_err();
    assert_eq!(error.reject_code(), RejectCode::InvalidPlayerId);
}

#[test]
fn a_rejection_can_be_built_from_a_decode_error() {
    let error = decode_client("nope").unwrap_err();
    let message = ServerMessage::rejected(error.reject_code(), error.message());
    let text = encode_server(&ServerFrame::new(message));
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["type"], "rejected");
    assert_eq!(value["code"], "bad_message");
}

#[test]
fn an_out_of_range_or_fractional_version_is_still_a_version_mismatch() {
    for text in [
        r#"{"v":1.5,"type":"start"}"#,
        r#"{"v":1e30,"type":"start"}"#,
    ] {
        let error = decode_client(text).unwrap_err();
        assert!(
            matches!(error, DecodeError::VersionMismatch { .. }),
            "{text}: {error:?}"
        );
    }
    // 数値でない版は、形のエラー
    for text in [
        r#"{"v":"1","type":"start"}"#,
        r#"{"v":null,"type":"start"}"#,
        r#"{"type":"start"}"#,
    ] {
        assert!(
            matches!(decode_client(text), Err(DecodeError::Malformed(_))),
            "{text}"
        );
    }
}

#[test]
fn invisible_and_direction_changing_characters_are_not_allowed_in_names() {
    for bad in [
        "\u{200B}",
        "\u{202E}abc",
        "a\u{200D}b",
        "\u{FEFF}x",
        "x\u{2066}",
        "\u{00AD}",
    ] {
        assert_eq!(
            validate_display_name(bad),
            Err(FieldError::DisplayName),
            "{bad:?}"
        );
    }
    // 日本語・絵文字・記号は使える
    for ok in ["ゲスト", "たろう🐻", "Bob_2", "ｱｲｳ"] {
        assert!(validate_display_name(ok).is_ok(), "{ok}");
    }
}

#[test]
fn error_messages_are_built_from_the_limits() {
    let rules = FieldError::Rules.to_string();
    assert!(rules.contains(&format!("{MIN_PLAYERS}〜{MAX_PLAYERS}")));
    assert!(rules.contains(&format!("{MIN_AI_LEVEL}〜{MAX_AI_LEVEL}")));
    assert!(
        FieldError::DisplayName
            .to_string()
            .contains(&MAX_DISPLAY_NAME_CHARS.to_string())
    );
}

#[test]
fn the_typescript_protocol_version_matches_the_rust_one() {
    let source = include_str!("../../../web/src/net/index.ts");
    let expected = format!("export const PROTOCOL_VERSION = {PROTOCOL_VERSION};");
    assert!(
        source.contains(&expected),
        "web/src/net/index.ts の版を {PROTOCOL_VERSION} に揃える"
    );
}

#[test]
fn state_view_pieces_expose_only_the_wire_fields() {
    let catalog = Catalog::builtin().unwrap();
    let mut state = state();
    state.phase = Phase::Battle;
    let id = state.issue_piece_id();
    state.pieces.push(rules::state::Piece {
        id,
        kind: catalog.castle(),
        owner: PlayerId(0),
        anchor: Pos::new(1, 2),
        facing: Facing::North,
        hp: 10,
    });
    let value = serde_json::to_value(StateView::from_state(&state)).unwrap();
    assert_eq!(
        value["pieces"][0],
        json!({ "id": 0, "kind": catalog.castle().0, "owner": 0, "anchor": { "x": 1, "y": 2 }, "facing": "north", "hp": 10 })
    );
}

#[test]
fn a_state_view_restores_the_same_game() {
    let catalog = Catalog::builtin().unwrap();
    let original = state();
    let restored = StateView::from_state(&original).to_state(&catalog).unwrap();
    assert_eq!(restored, original);
}

#[test]
fn a_state_view_that_breaks_an_invariant_is_not_restored() {
    let catalog = Catalog::builtin().unwrap();
    let mut view = StateView::from_state(&state());
    view.current = PlayerId(9);
    assert!(view.to_state(&catalog).is_err());
}

#[test]
fn a_state_view_with_a_malformed_hand_is_not_restored() {
    let catalog = Catalog::builtin().unwrap();
    let base = StateView::from_state(&state());
    let castle = catalog.castle();

    let mut duplicated = base.clone();
    duplicated.players[0].hand.push(HandEntry {
        kind: castle,
        count: 5,
    });
    assert!(duplicated.to_state(&catalog).is_err());

    let mut zero = base.clone();
    zero.players[0].hand[0].count = 0;
    assert!(zero.to_state(&catalog).is_err());

    let mut many = base;
    many.players = (0..300).map(|_| many.players[0].clone()).collect();
    assert!(many.to_state(&catalog).is_err());
}

#[test]
fn a_piece_id_at_the_maximum_does_not_overflow() {
    let catalog = Catalog::builtin().unwrap();
    let mut view = StateView::from_state(&state());
    view.pieces.push(PieceView {
        id: PieceId(u32::MAX),
        kind: catalog.castle(),
        owner: PlayerId(0),
        anchor: Pos::new(0, 0),
        facing: Facing::North,
        hp: 1,
    });
    assert!(view.to_state(&catalog).is_err());
}

#[test]
fn a_piece_anchored_at_the_edge_of_the_integer_range_is_rejected_without_overflow() {
    let catalog = Catalog::builtin().unwrap();
    for x in [i32::MAX, i32::MAX - 1, i32::MIN] {
        let mut view = StateView::from_state(&state());
        view.pieces.push(PieceView {
            id: PieceId(0),
            kind: catalog.castle(),
            owner: PlayerId(0),
            anchor: Pos::new(x, 0),
            facing: Facing::North,
            hp: 1,
        });
        assert!(view.to_state(&catalog).is_err(), "x = {x}");
    }
}
