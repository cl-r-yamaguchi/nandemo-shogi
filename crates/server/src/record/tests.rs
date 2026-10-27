use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ai::{Level, choose_action};
use protocol::{RoomRules, RoomStatus, SeatKind};
use rules::action::Action;
use rules::pieces::Catalog;
use rules::rng::SplitMix64;
use rules::state::{Phase, Stage};

use super::*;
use crate::Millis;
use crate::lobby::Lobby;
use crate::room::{ALL_DISCONNECTED_MS, HUMAN_TURN_MS, Room};

const T0: Millis = 1_000_000;

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog::builtin().unwrap())
}

fn pid(n: u32) -> String {
    format!("player-{n:012}")
}

/// 人間 1 人（強さ 1 の AI 3 人）の、4 人対戦を始める。
fn started(seed: u64) -> (Room, SplitMix64) {
    let mut rng = SplitMix64(seed);
    let mut room = Room::new(
        "room-1",
        RoomRules {
            players: 4,
            ai_level: 1,
        },
        catalog(),
        T0,
    );
    room.join(&pid(1), "ゲスト1", T0, &mut rng).unwrap();
    room.start(&pid(1), T0, &mut rng).unwrap();
    (room, rng)
}

fn advance_ai(room: &mut Room, now: Millis) {
    for _ in 0..2000 {
        if !room.needs_ai() {
            return;
        }
        room.run_ai(now);
    }
    panic!("AI の手番が終わらない");
}

/// 人間の席が、強さ 1 の AI と同じやり方で指して、終局まで進める。
fn play_to_the_end(room: &mut Room, rng: &mut SplitMix64) {
    let catalog = catalog();
    for _ in 0..5000 {
        if room.status() != RoomStatus::Playing {
            return;
        }
        advance_ai(room, T0);
        let Some(id) = room.current_human_id().map(str::to_string) else {
            continue;
        };
        let state = room.state().unwrap().clone();
        if let Stage::Pick(_) = state.stage {
            room.act(&id, state.turn, Action::Pick { index: 0 }, T0)
                .unwrap();
        }
        let state = room.state().unwrap().clone();
        if state.phase == Phase::Finished {
            return;
        }
        let far = Instant::now() + Duration::from_secs(5);
        let action = choose_action(&catalog, &state, Level::One, far, rng).unwrap();
        room.act(&id, state.turn, action, T0).unwrap();
    }
    panic!("終局しない");
}

#[test]
fn a_finished_game_replays_to_the_same_events_and_outcome() {
    for seed in [1, 2, 3] {
        let (mut room, mut rng) = started(seed);
        play_to_the_end(&mut room, &mut rng);
        let record = room.record().unwrap();
        assert!(record.outcome.is_some());
        assert_eq!(record.seats.len(), 4);
        assert_eq!(record.seed, room.seed().unwrap());
        let state = replay(&catalog(), &record).expect("再現できる");
        assert_eq!(&state, room.state().unwrap(), "seed {seed}");
    }
}

#[test]
fn timeouts_and_resignations_replay_too() {
    let (mut room, _) = started(5);
    advance_ai(&mut room, T0);
    // 人間の持ち時間切れ（城を置く）を、何度か
    let mut now = T0;
    for _ in 0..3 {
        now += HUMAN_TURN_MS;
        room.tick(now);
        advance_ai(&mut room, now);
    }
    room.resign(&pid(1), now).unwrap();
    advance_ai(&mut room, now);
    let record = room.record().unwrap();
    assert!(record.history.iter().any(|e| e.mv == Move::TimeOut));
    assert!(record.history.iter().any(|e| e.mv == Move::Resign));
    assert_eq!(&replay(&catalog(), &record).unwrap(), room.state().unwrap());
}

#[test]
fn an_aborted_game_has_no_outcome_and_still_replays() {
    // 2 人の人間が、満員で始まる。両方が切断したままなら、ルームが閉じる
    let mut rng = SplitMix64(5);
    let mut room = Room::new(
        "room-1",
        RoomRules {
            players: 2,
            ai_level: 1,
        },
        catalog(),
        T0,
    );
    room.join(&pid(1), "ゲスト1", T0, &mut rng).unwrap();
    room.join(&pid(2), "ゲスト2", T0, &mut rng).unwrap();
    room.disconnect(&pid(1), T0);
    room.disconnect(&pid(2), T0);
    room.tick(T0 + ALL_DISCONNECTED_MS);
    assert_eq!(room.status(), RoomStatus::Finished);
    let record = room.record().unwrap();
    assert!(record.outcome.is_none());
    assert!(replay(&catalog(), &record).is_ok());
}

#[test]
fn a_tampered_record_is_not_replayed() {
    let (mut room, mut rng) = started(2);
    play_to_the_end(&mut room, &mut rng);
    let record = room.record().unwrap();

    let mut other_seed = record.clone();
    other_seed.seed ^= 1;
    assert!(replay(&catalog(), &other_seed).is_err(), "シードが違う");

    let mut dropped = record.clone();
    dropped.history.remove(dropped.history.len() / 2);
    assert!(replay(&catalog(), &dropped).is_err(), "操作が欠けている");

    let mut wrong_outcome = record.clone();
    wrong_outcome.outcome = None;
    assert_eq!(
        replay(&catalog(), &wrong_outcome),
        Err(ReplayError::Diverged { index: None })
    );

    let mut wrong_seats = record;
    wrong_seats.seats.pop();
    assert_eq!(replay(&catalog(), &wrong_seats), Err(ReplayError::BadSeats));
}

#[test]
fn the_record_has_no_player_ids() {
    let (mut room, mut rng) = started(2);
    play_to_the_end(&mut room, &mut rng);
    let record = room.record().unwrap();
    let human: Vec<_> = record
        .seats
        .iter()
        .filter(|s| s.kind == SeatKind::Human)
        .collect();
    assert_eq!(human.len(), 1);
    assert_eq!(human[0].name, "ゲスト1");
    assert!(!format!("{record:?}").contains(&pid(1)));
}

#[test]
fn the_lobby_hands_over_a_record_when_a_started_room_is_swept() {
    let mut lobby = Lobby::new(catalog());
    let records: Arc<Mutex<Vec<GameRecord>>> = Arc::default();
    let sink = Arc::clone(&records);
    lobby.set_record_sink(Box::new(move |r| sink.lock().unwrap().push(r)));
    let mut rng = SplitMix64(9);
    let rules = RoomRules {
        players: 2,
        ai_level: 1,
    };
    // 待機のまま閉じるルームは、記録しない
    lobby
        .find_room(&pid(2), "ゲスト2", rules, T0, &mut rng)
        .unwrap();
    lobby.leave(&pid(2), T0);
    lobby.sweep();
    assert!(records.lock().unwrap().is_empty());

    lobby
        .find_room(&pid(1), "ゲスト1", rules, T0, &mut rng)
        .unwrap();
    lobby.start(&pid(1), T0, &mut rng).unwrap();
    lobby.disconnect(&pid(1), T0);
    lobby.tick(T0 + ALL_DISCONNECTED_MS);
    let records = records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(lobby.room_count(), 0);
}
