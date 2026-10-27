use protocol::{NoticeKind, RejectCode, RoomRules, RoomStatus, ServerMessage};
use rules::rng::SplitMix64;

use std::time::{Duration, Instant};

use super::*;
use crate::room::{HUMAN_TURN_MS, WAITING_EXPIRY_MS};

const T0: Millis = 1_000_000;

fn lobby() -> Lobby {
    Lobby::new(Arc::new(Catalog::builtin().unwrap()))
}

fn pid(n: u32) -> String {
    format!("player-{n:012}")
}

fn rules(players: u8, ai_level: u8) -> RoomRules {
    RoomRules { players, ai_level }
}

fn find(lobby: &mut Lobby, n: u32, rules: RoomRules, rng: &mut SplitMix64) -> Vec<Outbound> {
    lobby
        .find_room(&pid(n), &format!("ゲスト{n}"), rules, T0, rng)
        .unwrap()
}

fn room_id_of(lobby: &Lobby, n: u32) -> String {
    lobby.room_of(&pid(n)).unwrap().id().to_string()
}

#[test]
fn a_first_player_creates_a_room_and_waits() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    let out = find(&mut lobby, 1, RoomRules::DEFAULT, &mut rng);
    assert_eq!(lobby.room_count(), 1);
    assert!(
        out.iter()
            .any(|o| matches!(o.message, ServerMessage::RoomState { .. }))
    );
    let room = lobby.room_of(&pid(1)).unwrap();
    assert_eq!(room.status(), RoomStatus::Waiting);
    assert_eq!(room.rules(), RoomRules::DEFAULT);
}

#[test]
fn players_with_the_same_rules_share_a_room() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, rules(4, 2), &mut rng);
    find(&mut lobby, 2, rules(4, 2), &mut rng);
    assert_eq!(lobby.room_count(), 1);
    assert_eq!(room_id_of(&lobby, 1), room_id_of(&lobby, 2));
}

#[test]
fn players_with_different_rules_get_different_rooms() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, rules(4, 2), &mut rng);
    find(&mut lobby, 2, rules(4, 3), &mut rng); // AI の強さが違う
    find(&mut lobby, 3, rules(3, 2), &mut rng); // 人数が違う
    assert_eq!(lobby.room_count(), 3);
    let ids = [
        room_id_of(&lobby, 1),
        room_id_of(&lobby, 2),
        room_id_of(&lobby, 3),
    ];
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3
    );
}

#[test]
fn the_oldest_room_with_a_free_seat_is_chosen() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    // 同じルールのルームを 2 つ作る（1 つ目を、いったん始めて、埋める）
    find(&mut lobby, 1, rules(2, 2), &mut rng);
    find(&mut lobby, 2, rules(2, 2), &mut rng); // 1 つ目が満員になり、始まる
    find(&mut lobby, 3, rules(2, 2), &mut rng); // 新しいルーム
    find(&mut lobby, 4, rules(2, 2), &mut rng); // 3 のルームが満員
    find(&mut lobby, 5, rules(2, 2), &mut rng); // さらに新しいルーム
    assert_eq!(lobby.room_count(), 3);
    assert_eq!(room_id_of(&lobby, 1), room_id_of(&lobby, 2));
    assert_eq!(room_id_of(&lobby, 3), room_id_of(&lobby, 4));
    assert_ne!(room_id_of(&lobby, 4), room_id_of(&lobby, 5));
}

#[test]
fn a_full_room_starts_and_is_no_longer_matched() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, rules(2, 1), &mut rng);
    let out = find(&mut lobby, 2, rules(2, 1), &mut rng);
    assert!(
        out.iter()
            .any(|o| matches!(o.message, ServerMessage::GameStarted { .. }))
    );
    assert_eq!(
        lobby.room_of(&pid(1)).unwrap().status(),
        RoomStatus::Playing
    );
    // 3 人目は、新しいルーム
    find(&mut lobby, 3, rules(2, 1), &mut rng);
    assert_eq!(lobby.room_count(), 2);
    assert_ne!(room_id_of(&lobby, 3), room_id_of(&lobby, 1));
}

#[test]
fn invalid_rules_are_refused() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    for bad in [rules(1, 2), rules(5, 2), rules(4, 0), rules(4, 4)] {
        let error = lobby
            .find_room(&pid(1), "a", bad, T0, &mut rng)
            .unwrap_err();
        assert_eq!(error, LobbyError::InvalidRules);
    }
    assert_eq!(lobby.room_count(), 0);
}

#[test]
fn a_player_already_in_a_room_cannot_search_again() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, RoomRules::DEFAULT, &mut rng);
    let error = lobby
        .find_room(&pid(1), "a", RoomRules::DEFAULT, T0, &mut rng)
        .unwrap_err();
    assert_eq!(error, LobbyError::AlreadyInRoom);
    assert!(matches!(
        error.to_message(),
        ServerMessage::Rejected {
            code: RejectCode::AlreadyInRoom,
            ..
        }
    ));
    assert_eq!(lobby.room_count(), 1);
}

#[test]
fn leaving_a_waiting_room_lets_the_player_search_again_and_empties_the_room() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, RoomRules::DEFAULT, &mut rng);
    lobby.leave(&pid(1), T0);
    assert!(lobby.room_of(&pid(1)).is_none());
    lobby.tick(T0); // 空になったルームは、片付く
    assert_eq!(lobby.room_count(), 0);
    find(&mut lobby, 1, RoomRules::DEFAULT, &mut rng);
    assert_eq!(lobby.room_count(), 1);
}

#[test]
fn the_go_button_works_through_the_lobby() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, RoomRules::DEFAULT, &mut rng);
    assert_eq!(
        lobby.start(&pid(9), T0, &mut rng).unwrap_err(),
        RoomError::NotInRoom
    );
    let out = lobby.start(&pid(1), T0, &mut rng).unwrap();
    assert!(
        out.iter()
            .any(|o| matches!(o.message, ServerMessage::GameStarted { .. }))
    );
    assert_eq!(lobby.room_of(&pid(1)).unwrap().view().seats.len(), 4);
}

#[test]
fn a_finished_room_is_swept_and_the_players_can_play_again() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, rules(2, 1), &mut rng);
    find(&mut lobby, 2, rules(2, 1), &mut rng);
    let out = lobby.resign(&pid(1), T0).unwrap();
    assert!(
        out.iter()
            .any(|o| matches!(o.message, ServerMessage::GameOver { .. }))
    );
    assert_eq!(
        lobby.room_of(&pid(1)).unwrap().status(),
        RoomStatus::Finished
    );
    // 終わったルームの人は、そのまま、探し直せる（古い対応は、片付く）
    find(&mut lobby, 1, rules(2, 1), &mut rng);
    assert_eq!(
        lobby.room_of(&pid(1)).unwrap().status(),
        RoomStatus::Waiting
    );
    lobby.sweep();
    assert_eq!(lobby.room_count(), 1);
    assert!(lobby.room_of(&pid(2)).is_none());
}

#[test]
fn routing_requires_a_room() {
    let mut lobby = lobby();
    assert_eq!(lobby.resign(&pid(1), T0).unwrap_err(), RoomError::NotInRoom);
    assert_eq!(
        lobby.act(&pid(1), 0, Action::Pass, T0).unwrap_err(),
        RoomError::NotInRoom
    );
    assert_eq!(
        lobby.reconnect(&pid(1), T0).unwrap_err(),
        RoomError::NotInRoom
    );
    assert!(lobby.leave(&pid(1), T0).is_empty());
    assert!(lobby.disconnect(&pid(1), T0).is_empty());
}

#[test]
fn a_disconnect_during_a_game_keeps_the_seat_and_reconnect_returns() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, rules(2, 1), &mut rng);
    find(&mut lobby, 2, rules(2, 1), &mut rng);
    let out = lobby.disconnect(&pid(1), T0 + 10);
    assert!(out.iter().any(|o| matches!(
        o.message,
        ServerMessage::Notice {
            kind: NoticeKind::PlayerDisconnected,
            ..
        }
    )));
    assert!(lobby.room_of(&pid(1)).is_some(), "対戦中は、席を保つ");
    let out = lobby.reconnect(&pid(1), T0 + 20).unwrap();
    assert!(
        out.iter()
            .any(|o| o.to == pid(1) && matches!(o.message, ServerMessage::State { .. }))
    );
}

#[test]
fn tick_runs_every_rooms_timers_and_sweeps_expired_rooms() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, rules(4, 2), &mut rng); // 待機
    find(&mut lobby, 2, rules(2, 1), &mut rng);
    find(&mut lobby, 3, rules(2, 1), &mut rng); // 対局
    assert_eq!(lobby.room_count(), 2);
    let first = lobby.next_wakeup().unwrap();
    assert_eq!(first, T0 + HUMAN_TURN_MS.min(WAITING_EXPIRY_MS));
    // 対局の持ち時間が切れる
    let out = lobby.tick(T0 + HUMAN_TURN_MS);
    assert!(out.iter().any(|o| matches!(
        o.message,
        ServerMessage::Notice {
            kind: NoticeKind::TimedOut,
            ..
        }
    )));
    // 待機のルームは、10 分で破棄され、片付く
    let out = lobby.tick(T0 + WAITING_EXPIRY_MS);
    assert!(out.iter().any(|o| o.to == pid(1)
        && matches!(
            o.message,
            ServerMessage::Notice {
                kind: NoticeKind::RoomClosed,
                ..
            }
        )));
    assert!(lobby.room_of(&pid(1)).is_none());
}

#[test]
fn ai_turns_are_driven_through_the_lobby() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(5);
    find(&mut lobby, 1, rules(4, 1), &mut rng);
    lobby.start(&pid(1), T0, &mut rng).unwrap();
    let mut guard = 0;
    loop {
        let needing = lobby.rooms_needing_ai();
        if needing.is_empty() {
            break;
        }
        for room in needing {
            let out = lobby.run_ai(room, T0);
            assert!(out.iter().all(|o| o.to == pid(1)));
        }
        guard += 1;
        assert!(guard < 1000);
    }
    // 人間の手番か、終局のどちらかで止まる
    let room = lobby.room_of(&pid(1)).unwrap();
    assert!(room.current_human_id().is_some() || room.status() == RoomStatus::Finished);
}

#[test]
fn run_ai_on_an_unknown_room_does_nothing() {
    let mut lobby = lobby();
    assert!(lobby.run_ai(99, T0).is_empty());
    assert_eq!(lobby.next_wakeup(), None);
}

#[test]
fn resigning_in_a_waiting_room_frees_the_player_to_search_again() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(1);
    find(&mut lobby, 1, RoomRules::DEFAULT, &mut rng);
    find(&mut lobby, 2, RoomRules::DEFAULT, &mut rng);
    lobby.resign(&pid(1), T0).unwrap();
    assert!(lobby.room_of(&pid(1)).is_none());
    let out = lobby.find_room(&pid(1), "a", RoomRules::DEFAULT, T0, &mut rng);
    assert!(out.is_ok(), "探し直せる: {out:?}");
}

#[test]
fn the_ai_plan_is_taken_outside_the_lobby_and_applied_afterwards() {
    let mut lobby = lobby();
    let mut rng = SplitMix64(5);
    find(&mut lobby, 1, rules(4, 1), &mut rng);
    lobby.start(&pid(1), T0, &mut rng).unwrap();
    let mut guard = 0;
    while let Some(&room) = lobby.rooms_needing_ai().first() {
        let plan = lobby.plan_ai(room).expect("AI の手番");
        // ここで、ロビーを手放して、別のスレッドで考えられる
        let decision = plan.think(Instant::now() + Duration::from_secs(5));
        let out = lobby.apply_ai(room, T0 + 3_000, decision);
        assert!(out.iter().all(|o| o.to == pid(1)));
        guard += 1;
        assert!(guard < 1000);
    }
    let room = lobby.room_of(&pid(1)).unwrap();
    assert!(room.current_human_id().is_some() || room.status() == RoomStatus::Finished);
    assert!(lobby.plan_ai(99).is_none());
}
