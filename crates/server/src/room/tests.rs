use ai::{Level, choose_action};
use protocol::{NoticeKind, RejectCode, RoomRules, RoomStatus, ServerMessage};
use rules::action::{Action, ActionError, Event, legal_actions};
use rules::rng::SplitMix64;
use rules::state::{Phase, PlayerId, Stage};

use super::*;

const T0: Millis = 1_000_000;

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog::builtin().unwrap())
}

fn pid(n: u32) -> String {
    format!("player-{n:012}")
}

fn rules(players: u8) -> RoomRules {
    RoomRules {
        players,
        ai_level: 1,
    }
}

fn room(players: u8) -> Room {
    Room::new("room-1", rules(players), catalog(), T0)
}

fn human(n: u32) -> Human {
    Human {
        player_id: pid(n),
        name: format!("ゲスト{n}"),
        connected: true,
        disconnected_at: None,
        takeover: false,
    }
}

fn kinds(seats: &[Occupant]) -> String {
    seats
        .iter()
        .map(|s| match s {
            Occupant::Human(_) => 'H',
            Occupant::Ai { .. } => 'A',
        })
        .collect()
}

/// 1 人の人間が入って、Go で始めた、4 人対戦（AI 3 人）。
fn started_with_one_human(seed: u64) -> (Room, SplitMix64) {
    let mut rng = SplitMix64(seed);
    let mut room = room(4);
    room.join(&pid(1), "ゲスト1", T0, &mut rng).unwrap();
    room.start(&pid(1), T0, &mut rng).unwrap();
    (room, rng)
}

fn messages_for<'a>(out: &'a [Outbound], id: &str) -> Vec<&'a ServerMessage> {
    out.iter()
        .filter(|o| o.to == id)
        .map(|o| &o.message)
        .collect()
}

fn has(out: &[Outbound], id: &str, f: impl Fn(&ServerMessage) -> bool) -> bool {
    messages_for(out, id).into_iter().any(f)
}

fn is_notice(kind: NoticeKind) -> impl Fn(&ServerMessage) -> bool {
    move |m| matches!(m, ServerMessage::Notice { kind: k, .. } if *k == kind)
}

fn is_rejected(code: RejectCode) -> impl Fn(&ServerMessage) -> bool {
    move |m| matches!(m, ServerMessage::Rejected { code: c, .. } if *c == code)
}

/// AI の手番を、人間の手番（または終局）まで進める。
fn advance_ai(room: &mut Room, now: Millis) -> Vec<Outbound> {
    let mut out = Vec::new();
    for _ in 0..1000 {
        if !room.needs_ai() {
            return out;
        }
        out.extend(room.run_ai(now));
    }
    panic!("AI の手番が終わらない");
}

/// 手番の人間が、強さ 1 の AI と同じやり方で、選択と行動をする。
fn human_turn(room: &mut Room, id: &str, now: Millis, rng: &mut SplitMix64) -> Vec<Outbound> {
    let catalog = catalog();
    let mut out = Vec::new();
    let state = room.state().unwrap().clone();
    if let Stage::Pick(_) = state.stage {
        out.extend(
            room.act(id, state.turn, Action::Pick { index: 0 }, now)
                .unwrap(),
        );
    }
    let state = room.state().unwrap().clone();
    if state.phase == Phase::Finished {
        return out;
    }
    let far = Instant::now() + Duration::from_secs(5);
    let action = choose_action(&catalog, &state, Level::One, far, rng).expect("行動がある");
    out.extend(room.act(id, state.turn, action, now).unwrap());
    out
}

/// 終局まで、AI と人間の手番を進める。
fn play_to_the_end(room: &mut Room, ids: &[String], rng: &mut SplitMix64) -> Vec<Outbound> {
    let mut out = Vec::new();
    for _ in 0..5000 {
        if room.status() != RoomStatus::Playing {
            return out;
        }
        out.extend(advance_ai(room, T0));
        if room.status() != RoomStatus::Playing {
            return out;
        }
        let Some(id) = room.current_human_id().map(str::to_string) else {
            continue;
        };
        assert!(ids.contains(&id));
        out.extend(human_turn(room, &id, T0, rng));
    }
    panic!("終局しない");
}

// ---- 席順（online.md の 4.3） ----

#[test]
fn seats_alternate_between_humans_and_ai_as_much_as_possible() {
    for (humans, ais, expected) in [
        (2, 2, "HAHA"),
        (1, 3, "AHAA"),
        (3, 1, "HAHH"),
        (4, 0, "HHHH"),
        (1, 1, "HA"),
        (2, 1, "HAH"),
        (1, 2, "AHA"),
    ] {
        let mut rng = SplitMix64(7);
        let seats = arrange((1..=humans).map(human).collect(), ais, true, &mut rng);
        assert_eq!(kinds(&seats), expected, "人間 {humans}・AI {ais}");
    }
}

#[test]
fn every_human_gets_exactly_one_seat_and_the_order_is_random() {
    let mut orders = std::collections::BTreeSet::new();
    for seed in 0..30 {
        let seats = arrange((1..=3).map(human).collect(), 1, true, &mut SplitMix64(seed));
        let ids: Vec<String> = seats
            .iter()
            .filter_map(|s| match s {
                Occupant::Human(h) => Some(h.player_id.clone()),
                Occupant::Ai { .. } => None,
            })
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(sorted, [pid(1), pid(2), pid(3)]);
        orders.insert(ids);
    }
    assert!(orders.len() > 1, "席順がランダムでない");
}

#[test]
fn ai_seats_are_numbered() {
    let seats = arrange(vec![human(1)], 3, true, &mut SplitMix64(1));
    let names: Vec<String> = seats
        .iter()
        .filter_map(|s| match s {
            Occupant::Ai { name } => Some(name.clone()),
            Occupant::Human(_) => None,
        })
        .collect();
    assert_eq!(names, ["AI 1", "AI 2", "AI 3"]);
}

// ---- 待機・開始 ----

#[test]
fn joining_fills_the_seats_and_a_full_room_starts_by_itself() {
    let mut rng = SplitMix64(1);
    let mut room = room(2);
    assert!(room.has_free_seat());
    let out = room.join(&pid(1), "ゲスト1", T0, &mut rng).unwrap();
    assert_eq!(room.status(), RoomStatus::Waiting);
    assert!(has(&out, &pid(1), |m| matches!(
        m,
        ServerMessage::RoomState { .. }
    )));
    assert!(has(&out, &pid(1), is_notice(NoticeKind::PlayerJoined)));

    let out = room.join(&pid(2), "ゲスト2", T0, &mut rng).unwrap();
    assert_eq!(room.status(), RoomStatus::Playing);
    for id in [pid(1), pid(2)] {
        assert!(
            has(&out, &id, |m| matches!(
                m,
                ServerMessage::GameStarted { .. }
            )),
            "{id}"
        );
    }
    assert!(!room.has_free_seat());
    assert_eq!(
        room.join(&pid(3), "ゲスト3", T0, &mut rng).unwrap_err(),
        RoomError::NotWaiting
    );
}

#[test]
fn the_go_button_starts_the_game_with_ai_filling_the_seats() {
    let (room, _) = started_with_one_human(5);
    let view = room.view();
    assert_eq!(view.status, RoomStatus::Playing);
    assert_eq!(view.seats.len(), 4);
    let humans = view
        .seats
        .iter()
        .filter(|s| s.kind == protocol::SeatKind::Human)
        .count();
    assert_eq!(humans, 1);
    for seat in view
        .seats
        .iter()
        .filter(|s| s.kind == protocol::SeatKind::Ai)
    {
        assert_eq!(seat.ai_level, Some(1));
        assert!(seat.connected);
    }
    assert!(room.seed().is_some());
}

#[test]
fn only_a_player_in_the_room_can_press_go() {
    let mut rng = SplitMix64(1);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    assert_eq!(
        room.start(&pid(9), T0, &mut rng).unwrap_err(),
        RoomError::NotInRoom
    );
    room.start(&pid(1), T0, &mut rng).unwrap();
    assert_eq!(
        room.start(&pid(1), T0, &mut rng).unwrap_err(),
        RoomError::NotWaiting
    );
}

#[test]
fn game_started_tells_each_human_their_own_seat() {
    let mut rng = SplitMix64(3);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    room.join(&pid(2), "b", T0, &mut rng).unwrap();
    let out = room.start(&pid(1), T0, &mut rng).unwrap();
    for id in [pid(1), pid(2)] {
        let seat = room.seat_of(&id).unwrap();
        let you = messages_for(&out, &id)
            .into_iter()
            .find_map(|m| match m {
                ServerMessage::GameStarted { you, .. } => Some(*you),
                _ => None,
            })
            .unwrap();
        assert_eq!(you, PlayerId(seat as u8));
    }
}

#[test]
fn the_first_player_is_random() {
    let firsts: std::collections::BTreeSet<u8> = (0..30)
        .map(|seed| started_with_one_human(seed).0.state().unwrap().current.0)
        .collect();
    assert!(firsts.len() > 1);
}

#[test]
fn the_start_announces_the_turn_with_the_human_time_limit_for_humans() {
    let mut rng = SplitMix64(3);
    let mut room = room(2);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    let out = room.join(&pid(2), "b", T0, &mut rng).unwrap();
    let turn_started = messages_for(&out, &pid(1))
        .into_iter()
        .find_map(|m| match m {
            ServerMessage::TurnStarted {
                player,
                turn,
                remaining_ms,
            } => Some((*player, *turn, *remaining_ms)),
            _ => None,
        })
        .unwrap();
    assert_eq!(turn_started.0, room.state().unwrap().current);
    assert_eq!(turn_started.1, 0);
    assert_eq!(u64::from(turn_started.2), HUMAN_TURN_MS);
}

// ---- 対局の進行 ----

fn two_humans(seed: u64) -> (Room, SplitMix64) {
    let mut rng = SplitMix64(seed);
    let mut room = room(2);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    room.join(&pid(2), "b", T0, &mut rng).unwrap();
    (room, rng)
}

fn current_and_other(room: &Room) -> (String, String) {
    let current = room.current_human_id().unwrap().to_string();
    let other = if current == pid(1) { pid(2) } else { pid(1) };
    (current, other)
}

#[test]
fn a_castle_placement_is_applied_and_published_to_everyone() {
    let (mut room, _) = two_humans(11);
    let (current, other) = current_and_other(&room);
    let state = room.state().unwrap().clone();
    let action = legal_actions(&catalog(), &state).remove(0);
    let out = room.act(&current, state.turn, action.clone(), T0).unwrap();
    for id in [&current, &other] {
        assert!(has(
            &out,
            id,
            |m| matches!(m, ServerMessage::Applied { events, .. } if matches!(events[..], [Event::Placed { .. }]))
        ));
        assert!(has(&out, id, |m| matches!(
            m,
            ServerMessage::TurnStarted { .. }
        )));
    }
    assert_eq!(room.state().unwrap().pieces.len(), 1);
    assert_eq!(room.history().len(), 1);
    assert_eq!(room.history()[0].action, Some(action));
}

#[test]
fn acting_out_of_turn_with_a_stale_number_or_illegally_is_rejected_without_changing_anything() {
    let (mut room, _) = two_humans(11);
    let (current, other) = current_and_other(&room);
    let state = room.state().unwrap().clone();
    let good = legal_actions(&catalog(), &state).remove(0);
    let before = room.state().unwrap().clone();

    assert_eq!(
        room.act(&other, state.turn, good.clone(), T0).unwrap_err(),
        RoomError::NotYourTurn
    );
    assert_eq!(
        room.act(&current, state.turn + 5, good.clone(), T0)
            .unwrap_err(),
        RoomError::StaleTurn
    );
    assert_eq!(
        room.act(&pid(9), state.turn, good, T0).unwrap_err(),
        RoomError::NotInRoom
    );
    let bad = Action::Place {
        kind: catalog().castle(),
        anchor: rules::geometry::Pos::new(15, 15),
        facing: rules::geometry::Facing::North,
    };
    assert_eq!(
        room.act(&current, state.turn, bad, T0).unwrap_err(),
        RoomError::Illegal(ActionError::OutsideBoard)
    );
    assert_eq!(room.state().unwrap(), &before);
    assert!(room.history().is_empty());
}

#[test]
fn a_double_sent_action_is_refused() {
    let (mut room, _) = two_humans(11);
    let (current, _) = current_and_other(&room);
    let state = room.state().unwrap().clone();
    let action = legal_actions(&catalog(), &state).remove(0);
    room.act(&current, state.turn, action.clone(), T0).unwrap();
    let again = room.act(&current, state.turn, action, T0).unwrap_err();
    assert!(matches!(
        again,
        RoomError::NotYourTurn | RoomError::StaleTurn
    ));
}

#[test]
fn acting_in_a_waiting_room_is_refused() {
    let mut rng = SplitMix64(1);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    assert_eq!(
        room.act(&pid(1), 0, Action::Pass, T0).unwrap_err(),
        RoomError::NotPlaying
    );
}

#[test]
fn rejection_messages_carry_the_matching_codes() {
    for (error, code) in [
        (RoomError::NotYourTurn, RejectCode::NotYourTurn),
        (RoomError::StaleTurn, RejectCode::StaleTurn),
        (RoomError::NotInRoom, RejectCode::NotInRoom),
        (RoomError::NotPlaying, RejectCode::NotPlaying),
        (RoomError::NotWaiting, RejectCode::NotWaiting),
        (RoomError::RoomFull, RejectCode::RoomFull),
    ] {
        assert!(is_rejected(code)(&error.to_message()), "{error:?}");
    }
    assert!(matches!(
        RoomError::Illegal(ActionError::WrongStage).to_message(),
        ServerMessage::Rejected {
            code: RejectCode::IllegalAction,
            reason: Some(ActionError::WrongStage),
            ..
        }
    ));
}

#[test]
fn the_battle_starts_with_a_public_draw_and_the_pick_does_not_start_a_new_turn() {
    let (mut room, mut rng) = started_with_one_human(21);
    let me = pid(1);
    // 配置フェーズを、人間と AI で終わらせる
    for _ in 0..50 {
        advance_ai(&mut room, T0);
        if room.state().unwrap().phase != Phase::Placement {
            break;
        }
        human_turn(&mut room, &me, T0, &mut rng);
    }
    advance_ai(&mut room, T0);
    let state = room.state().unwrap().clone();
    assert_eq!(state.phase, Phase::Battle);
    assert_eq!(room.current_human_id(), Some(me.as_str()));
    assert!(
        matches!(state.stage, Stage::Pick(_)),
        "ドロー済みで、選択待ち"
    );
    // 選択: 次の手番は始まらない（TurnStarted は来ない）
    let out = room
        .act(&me, state.turn, Action::Pick { index: 1 }, T0)
        .unwrap();
    assert!(has(
        &out,
        &me,
        |m| matches!(m, ServerMessage::Applied { events, .. } if matches!(events[..], [Event::Picked { .. }]))
    ));
    assert!(!has(&out, &me, |m| matches!(
        m,
        ServerMessage::TurnStarted { .. }
    )));
    assert_eq!(room.state().unwrap().stage, Stage::Act);
    assert_eq!(room.state().unwrap().current, state.current);
    // 行動の確定: 次の手番が始まる
    let state = room.state().unwrap().clone();
    let action = legal_actions(&catalog(), &state).remove(0);
    let out = room.act(&me, state.turn, action, T0).unwrap();
    assert!(has(&out, &me, |m| matches!(
        m,
        ServerMessage::TurnStarted { .. }
    )));
}

#[test]
fn a_whole_game_with_one_human_and_ai_seats_reaches_the_end() {
    let (mut room, mut rng) = started_with_one_human(33);
    let ids = vec![pid(1)];
    let out = play_to_the_end(&mut room, &ids, &mut rng);
    assert_eq!(room.status(), RoomStatus::Finished);
    assert!(has(&out, &pid(1), |m| matches!(
        m,
        ServerMessage::GameOver { .. }
    )));
    let state = room.state().unwrap();
    assert_eq!(state.phase, Phase::Finished);
    assert!(state.outcome.is_some());
    assert!(!room.history().is_empty());
}

#[test]
fn the_same_seed_gives_the_same_game() {
    let play = |seed: u64| {
        let (mut room, mut rng) = started_with_one_human(seed);
        play_to_the_end(&mut room, &[pid(1)], &mut rng);
        (
            room.state().unwrap().clone(),
            room.history().to_vec(),
            room.seed(),
        )
    };
    assert_eq!(play(77), play(77));
    assert_ne!(play(77).2, play(78).2);
}

// ---- 投了 ----

#[test]
fn resigning_in_a_two_player_game_ends_it_and_names_the_winner() {
    let (mut room, _) = two_humans(11);
    let out = room.resign(&pid(1), T0).unwrap();
    assert_eq!(room.status(), RoomStatus::Finished);
    for id in [pid(1), pid(2)] {
        assert!(has(&out, &id, |m| matches!(
            m,
            ServerMessage::GameOver { .. }
        )));
    }
    let outcome = room.state().unwrap().outcome.clone().unwrap();
    assert_eq!(
        outcome.winners(),
        [PlayerId(room.seat_of(&pid(2)).unwrap() as u8)]
    );
}

#[test]
fn a_player_who_is_not_on_turn_can_resign_without_a_new_turn_starting() {
    let mut rng = SplitMix64(4);
    let mut room = room(3);
    for n in 1..=3 {
        room.join(&pid(n), "x", T0, &mut rng).unwrap();
    }
    let (current, _) = (room.current_human_id().unwrap().to_string(), 0);
    let resigner = (1..=3).map(pid).find(|id| *id != current).unwrap();
    let turn_before = room.state().unwrap().turn;
    let out = room.resign(&resigner, T0).unwrap();
    assert!(has(
        &out,
        &current,
        |m| matches!(m, ServerMessage::Applied { events, .. } if events.iter().any(|e| matches!(e, Event::Eliminated { .. })))
    ));
    assert!(!has(&out, &current, |m| matches!(
        m,
        ServerMessage::TurnStarted { .. }
    )));
    assert_eq!(room.state().unwrap().turn, turn_before);
    assert_eq!(room.status(), RoomStatus::Playing);
    // 手番は、そのまま
    assert_eq!(room.current_human_id(), Some(current.as_str()));
}

#[test]
fn resigning_in_a_waiting_room_leaves_it() {
    let mut rng = SplitMix64(1);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    room.join(&pid(2), "b", T0, &mut rng).unwrap();
    room.resign(&pid(1), T0).unwrap();
    assert_eq!(room.human_ids(), [pid(2).as_str()]);
}

// ---- 時間切れ ----

#[test]
fn a_human_turn_times_out_at_the_limit_and_the_turn_moves_on() {
    let (mut room, _) = two_humans(11);
    let (current, other) = current_and_other(&room);
    assert!(
        room.tick(T0 + HUMAN_TURN_MS - 1).is_empty(),
        "期限の前は、何も起きない"
    );
    assert_eq!(room.next_wakeup(), Some(T0 + HUMAN_TURN_MS));
    let out = room.tick(T0 + HUMAN_TURN_MS);
    for id in [&current, &other] {
        assert!(has(&out, id, is_notice(NoticeKind::TimedOut)));
    }
    // 配置フェーズの時間切れは、城を置く（R-14）
    assert_eq!(room.state().unwrap().pieces.len(), 1);
    assert!(room.history()[0].timed_out);
    assert_eq!(room.current_human_id(), Some(other.as_str()));
    // 新しい手番の持ち時間は、そこから数え直す
    assert_eq!(room.next_wakeup(), Some(T0 + 2 * HUMAN_TURN_MS));
}

#[test]
fn a_battle_turn_that_times_out_picks_a_card_and_passes() {
    let (mut room, mut rng) = started_with_one_human(21);
    let me = pid(1);
    for _ in 0..50 {
        advance_ai(&mut room, T0);
        if room.state().unwrap().phase != Phase::Placement {
            break;
        }
        human_turn(&mut room, &me, T0, &mut rng);
    }
    advance_ai(&mut room, T0);
    let turn = room.state().unwrap().turn;
    let hand_before: u32 = room.state().unwrap().players[room.seat_of(&me).unwrap()]
        .hand
        .values()
        .sum();
    let deadline = room.next_wakeup().unwrap();
    let out = room.tick(deadline);
    assert!(has(&out, &me, is_notice(NoticeKind::TimedOut)));
    let state = room.state().unwrap();
    assert_eq!(
        state.players[room.seat_of(&me).unwrap()]
            .hand
            .values()
            .sum::<u32>(),
        hand_before + 1
    );
    assert!(state.turn > turn);
}

#[test]
fn ai_turns_do_not_time_out() {
    // 手番が AI の席で始まる開始を探す。時間が過ぎても、tick は何も強制しない
    for seed in 0..30 {
        let mut rng = SplitMix64(seed);
        let mut candidate = room(4);
        candidate.join(&pid(1), "a", T0, &mut rng).unwrap();
        candidate.start(&pid(1), T0, &mut rng).unwrap();
        if candidate.needs_ai() {
            let state = candidate.state().unwrap().clone();
            let out = candidate.tick(T0 + 10 * HUMAN_TURN_MS);
            assert!(!has(&out, &pid(1), is_notice(NoticeKind::TimedOut)));
            assert_eq!(candidate.state().unwrap(), &state);
            return;
        }
    }
    panic!("AI が最初の手番になる開始が見つからない");
}

// ---- 切断・再接続 ----

#[test]
fn a_disconnect_is_announced_and_ai_takes_over_after_thirty_seconds() {
    let (mut room, _) = two_humans(11);
    let (_, other) = current_and_other(&room);
    let out = room.disconnect(&other, T0 + 1_000);
    assert!(
        out.iter().all(|o| o.to != other),
        "切断した人には、送らない"
    );
    assert!(
        out.iter()
            .any(|o| is_notice(NoticeKind::PlayerDisconnected)(&o.message))
    );
    // 30 秒の前は、代行しない
    let early = room.tick(T0 + 1_000 + TAKEOVER_AFTER_MS - 1);
    assert!(
        !early
            .iter()
            .any(|o| is_notice(NoticeKind::AiTakeover)(&o.message))
    );
    let late = room.tick(T0 + 1_000 + TAKEOVER_AFTER_MS);
    assert!(
        late.iter()
            .any(|o| is_notice(NoticeKind::AiTakeover)(&o.message))
    );
    // 二重には、通知しない
    assert!(
        room.tick(T0 + 1_000 + TAKEOVER_AFTER_MS + 5)
            .iter()
            .all(|o| !is_notice(NoticeKind::AiTakeover)(&o.message))
    );
}

#[test]
fn a_taken_over_seat_is_played_by_the_ai() {
    let (mut room, _) = started_with_one_human(41);
    let me = pid(1);
    room.disconnect(&me, T0);
    room.tick(T0 + TAKEOVER_AFTER_MS);
    // 切断した人間の席も、AI が指して、終局まで進む（代行中は、持ち時間の強制もない）
    let mut guard = 0;
    while room.status() == RoomStatus::Playing && guard < 5000 {
        assert!(room.needs_ai(), "全席が AI なので、常に AI の手番");
        room.run_ai(T0 + TAKEOVER_AFTER_MS);
        guard += 1;
    }
    assert_eq!(room.status(), RoomStatus::Finished);
}

#[test]
fn reconnecting_restores_the_seat_and_sends_the_full_state() {
    let (mut room, _) = two_humans(11);
    let (_, other) = current_and_other(&room);
    room.disconnect(&other, T0);
    room.tick(T0 + TAKEOVER_AFTER_MS);
    let out = room
        .reconnect(&other, T0 + TAKEOVER_AFTER_MS + 1_000)
        .unwrap();
    assert!(has(&out, &other, |m| matches!(
        m,
        ServerMessage::State { .. }
    )));
    let turn_started = messages_for(&out, &other)
        .into_iter()
        .find_map(|m| match m {
            ServerMessage::TurnStarted { remaining_ms, .. } => Some(*remaining_ms),
            _ => None,
        })
        .unwrap();
    assert!(u64::from(turn_started) <= HUMAN_TURN_MS);
    assert!(
        out.iter()
            .any(|o| is_notice(NoticeKind::PlayerReconnected)(&o.message))
    );
    // 本人に戻ったので、AI の代行ではない
    assert!(room.view().seats.iter().all(|s| s.connected));
    assert!(room.next_wakeup().is_some());
}

#[test]
fn reconnecting_before_the_takeover_cancels_it() {
    let (mut room, _) = two_humans(11);
    let (_, other) = current_and_other(&room);
    room.disconnect(&other, T0);
    room.reconnect(&other, T0 + 5_000).unwrap();
    let out = room.tick(T0 + TAKEOVER_AFTER_MS + 10);
    assert!(
        !out.iter()
            .any(|o| is_notice(NoticeKind::AiTakeover)(&o.message))
    );
}

#[test]
fn unknown_players_cannot_reconnect_and_finished_rooms_refuse() {
    let (mut room, _) = two_humans(11);
    assert_eq!(
        room.reconnect(&pid(9), T0).unwrap_err(),
        RoomError::NotInRoom
    );
    room.resign(&pid(1), T0).unwrap();
    assert_eq!(
        room.reconnect(&pid(2), T0).unwrap_err(),
        RoomError::NotPlaying
    );
}

#[test]
fn the_room_closes_when_everyone_stays_disconnected() {
    let (mut room, _) = two_humans(11);
    room.disconnect(&pid(1), T0);
    room.disconnect(&pid(2), T0 + 100);
    room.tick(T0 + 200);
    assert_eq!(room.status(), RoomStatus::Playing);
    room.tick(T0 + 100 + ALL_DISCONNECTED_MS - 1);
    assert_eq!(room.status(), RoomStatus::Playing);
    room.tick(T0 + 100 + ALL_DISCONNECTED_MS);
    assert_eq!(room.status(), RoomStatus::Finished);
}

#[test]
fn someone_coming_back_resets_the_all_disconnected_timer() {
    let (mut room, _) = two_humans(11);
    room.disconnect(&pid(1), T0);
    room.disconnect(&pid(2), T0);
    room.tick(T0 + 10);
    room.reconnect(&pid(1), T0 + 20_000).unwrap();
    room.tick(T0 + ALL_DISCONNECTED_MS + 5_000);
    assert_eq!(room.status(), RoomStatus::Playing);
}

#[test]
fn disconnecting_twice_is_ignored() {
    let (mut room, _) = two_humans(11);
    let first = room.disconnect(&pid(1), T0);
    assert!(!first.is_empty());
    assert!(room.disconnect(&pid(1), T0 + 5).is_empty());
    assert!(room.disconnect(&pid(9), T0).is_empty());
}

// ---- 待機中の退室・期限 ----

#[test]
fn leaving_a_waiting_room_frees_the_seat_and_the_last_one_closes_it() {
    let mut rng = SplitMix64(1);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    room.join(&pid(2), "b", T0, &mut rng).unwrap();
    let out = room.leave(&pid(2), T0);
    assert!(has(&out, &pid(1), is_notice(NoticeKind::PlayerLeft)));
    assert_eq!(room.human_ids(), [pid(1).as_str()]);
    assert!(room.has_free_seat());
    room.leave(&pid(1), T0);
    assert_eq!(room.status(), RoomStatus::Finished);
    assert!(room.leave(&pid(1), T0).is_empty());
}

#[test]
fn a_disconnect_while_waiting_is_a_leave() {
    let mut rng = SplitMix64(1);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    room.join(&pid(2), "b", T0, &mut rng).unwrap();
    room.disconnect(&pid(2), T0);
    assert_eq!(room.human_ids(), [pid(1).as_str()]);
}

#[test]
fn a_waiting_room_expires_after_ten_minutes() {
    let mut rng = SplitMix64(1);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    assert_eq!(room.next_wakeup(), Some(T0 + WAITING_EXPIRY_MS));
    assert!(room.tick(T0 + WAITING_EXPIRY_MS - 1).is_empty());
    let out = room.tick(T0 + WAITING_EXPIRY_MS);
    assert!(has(&out, &pid(1), is_notice(NoticeKind::RoomClosed)));
    assert_eq!(room.status(), RoomStatus::Finished);
    assert_eq!(room.next_wakeup(), None);
}

#[test]
fn joining_again_while_waiting_is_a_reconnect() {
    let mut rng = SplitMix64(1);
    let mut room = room(4);
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    room.join(&pid(1), "a", T0, &mut rng).unwrap();
    assert_eq!(room.human_ids().len(), 1);
}

#[test]
fn a_finished_room_has_no_wakeup_and_ignores_ticks() {
    let (mut room, _) = two_humans(11);
    room.resign(&pid(1), T0).unwrap();
    assert_eq!(room.next_wakeup(), None);
    assert!(room.tick(T0 + 10 * HUMAN_TURN_MS).is_empty());
    assert!(!room.needs_ai());
    assert!(room.run_ai(T0).is_empty());
}

// ---- AI の計画と反映（ロビーの外で考えるための分割） ----

fn first_ai_turn_room(seed_from: u64) -> (Room, SplitMix64) {
    for seed in seed_from..seed_from + 60 {
        let (room, rng) = started_with_one_human(seed);
        if room.needs_ai() {
            return (room, rng);
        }
    }
    panic!("AI が最初の手番になる開始が見つからない");
}

#[test]
fn planning_thinking_and_applying_gives_the_same_result_as_run_ai() {
    let (mut direct, _) = first_ai_turn_room(100);
    let (mut split, _) = first_ai_turn_room(100);
    direct.run_ai(T0);
    let plan = split.plan_ai().expect("AI の手番");
    let decision = plan.think(Instant::now() + Duration::from_secs(5));
    split.apply_ai(T0, decision);
    assert_eq!(direct.state(), split.state());
    assert_eq!(direct.history(), split.history());
}

#[test]
fn a_plan_needs_an_ai_turn_and_does_not_touch_the_room() {
    let (mut room, _) = two_humans(11);
    assert!(room.plan_ai().is_none(), "人間の手番では、計画しない");
    let (room2, _) = first_ai_turn_room(100);
    let before = room2.state().unwrap().clone();
    let plan = room2.plan_ai().unwrap();
    let _ = plan.think(Instant::now() + Duration::from_secs(5));
    assert_eq!(
        room2.state().unwrap(),
        &before,
        "考えても、ルームの状態は変わらない"
    );
    let _ = &mut room;
}

#[test]
fn a_stale_decision_is_ignored() {
    let (mut room, _) = first_ai_turn_room(100);
    let decision = room
        .plan_ai()
        .unwrap()
        .think(Instant::now() + Duration::from_secs(5));
    let first = room.apply_ai(T0, decision.clone());
    assert!(!first.is_empty());
    let after_first = room.state().unwrap().clone();
    // 同じ結果を、もう一度反映しても、状態が変わっていたので、何も起きない
    assert!(room.apply_ai(T0, decision).is_empty());
    assert_eq!(room.state().unwrap(), &after_first);
}

#[test]
fn the_next_humans_clock_starts_when_the_ai_finished_thinking() {
    for seed in 100..160 {
        let (mut room, _) = started_with_one_human(seed);
        if !room.needs_ai() {
            continue;
        }
        let decision = room
            .plan_ai()
            .unwrap()
            .think(Instant::now() + Duration::from_secs(5));
        let finished_at = T0 + 12_000;
        room.apply_ai(finished_at, decision);
        if room.current_human_id().is_some() {
            assert_eq!(room.next_wakeup(), Some(finished_at + HUMAN_TURN_MS));
            return;
        }
    }
    panic!("AI の次が、人間の手番になる開始が見つからない");
}

// ---- 補修の確認 ----

#[test]
fn resigning_a_seat_that_is_already_out_is_an_error() {
    let mut rng = SplitMix64(4);
    let mut room = room(3);
    for n in 1..=3 {
        room.join(&pid(n), "x", T0, &mut rng).unwrap();
    }
    let resigner = (1..=3)
        .map(pid)
        .find(|id| room.current_human_id() != Some(id.as_str()))
        .unwrap();
    room.resign(&resigner, T0).unwrap();
    assert_eq!(
        room.resign(&resigner, T0).unwrap_err(),
        RoomError::Illegal(ActionError::UnknownPlayer)
    );
}

#[test]
fn the_all_disconnected_timer_starts_at_the_last_disconnect_even_without_a_tick() {
    let (mut room, _) = two_humans(11);
    room.disconnect(&pid(1), T0);
    room.disconnect(&pid(2), T0 + 100);
    // 途中の tick がなくても、最後の切断から数える
    room.tick(T0 + 100 + ALL_DISCONNECTED_MS);
    assert_eq!(room.status(), RoomStatus::Finished);
}
