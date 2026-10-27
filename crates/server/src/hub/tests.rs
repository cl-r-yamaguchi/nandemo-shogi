use std::time::Duration;

use protocol::{NoticeKind, RejectCode, RoomRules, SeatKind, ServerFrame, ServerMessage};
use rules::action::{Action, ActionError};
use rules::geometry::{Facing, Pos};
use rules::pieces::Catalog;
use rules::state::PlayerId;
use tokio::sync::Notify;
use tokio::sync::mpsc::Receiver;

use super::*;

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog::builtin().unwrap())
}

fn hub_with(max_connections: usize) -> Arc<Hub> {
    Hub::new(
        catalog(),
        HubConfig {
            ai_min_delay: Duration::ZERO,
            max_connections,
            log: false,
            ..HubConfig::default()
        },
        7,
    )
}

fn hub() -> Arc<Hub> {
    hub_with(100)
}

fn pid(n: u32) -> String {
    format!("player-{n:012}")
}

struct Client {
    id: String,
    conn: ConnId,
    rx: Receiver<ConnMsg>,
    closer: Arc<Notify>,
}

fn connect(hub: &Arc<Hub>, n: u32) -> Client {
    let (tx, rx) = mpsc::channel(OUTBOX_CAPACITY);
    let id = pid(n);
    let closer = Arc::new(Notify::new());
    let conn = hub
        .connect(&id, &format!("ゲスト{n}"), tx, Arc::clone(&closer))
        .unwrap();
    Client {
        id,
        conn,
        rx,
        closer,
    }
}

impl Client {
    /// いま届いているメッセージを、すべて取り出す。
    fn drain(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            if let ConnMsg::Text(body) = message {
                out.push(serde_json::from_str::<ServerFrame>(&body).unwrap().message);
            }
        }
        out
    }

    fn closed(&mut self) -> bool {
        let mut closed = false;
        while let Ok(message) = self.rx.try_recv() {
            if message == ConnMsg::Close {
                closed = true;
            }
        }
        closed
    }

    /// 条件に合うメッセージが届くまで待つ。届いたもの（それまでの分を含む）を返す。
    async fn wait_for(
        &mut self,
        what: &str,
        f: impl Fn(&ServerMessage) -> bool,
    ) -> Vec<ServerMessage> {
        let mut seen = Vec::new();
        for _ in 0..1000 {
            seen.extend(self.drain());
            if seen.iter().any(&f) {
                return seen;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{what} が届きません。届いたもの: {seen:?}");
    }
}

fn send(hub: &Arc<Hub>, client: &Client, message: ClientMessage) {
    hub.handle(&client.id, client.conn, message);
}

fn find_room(hub: &Arc<Hub>, client: &Client, players: u8) {
    send(
        hub,
        client,
        ClientMessage::FindRoom {
            rules: RoomRules {
                players,
                ai_level: 1,
            },
        },
    );
}

fn is_rejected(code: RejectCode) -> impl Fn(&ServerMessage) -> bool {
    move |m| matches!(m, ServerMessage::Rejected { code: c, .. } if *c == code)
}

// ---- 接続 ----

#[tokio::test]
async fn connecting_sends_a_welcome() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    let messages = client.drain();
    assert!(matches!(
        &messages[..],
        [ServerMessage::Welcome { reconnected: false, display_name, .. }] if display_name == "ゲスト1"
    ));
    assert_eq!(hub.connection_count(), 1);
}

#[tokio::test]
async fn a_second_hello_is_rejected() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    client.drain();
    send(
        &hub,
        &client,
        ClientMessage::Hello {
            player_id: pid(1),
            display_name: "x".into(),
        },
    );
    assert!(
        client
            .drain()
            .iter()
            .any(is_rejected(RejectCode::BadMessage))
    );
}

#[tokio::test]
async fn the_connection_limit_is_enforced_but_a_replacement_is_always_allowed() {
    let hub = hub_with(2);
    let _a = connect(&hub, 1);
    let _b = connect(&hub, 2);
    let (tx, _rx) = mpsc::channel(8);
    assert_eq!(
        hub.connect(&pid(3), "c", tx, Arc::new(Notify::new())),
        Err(ConnectError::TooMany)
    );
    // すでにいるプレイヤーの、新しい接続（再接続）は、上限に数えない
    let (tx, _rx) = mpsc::channel(8);
    assert!(
        hub.connect(&pid(1), "a", tx, Arc::new(Notify::new()))
            .is_ok()
    );
    assert_eq!(hub.connection_count(), 2);
}

#[tokio::test]
async fn a_new_connection_with_the_same_id_closes_the_old_one_and_the_old_one_is_ignored() {
    let hub = hub();
    let mut old = connect(&hub, 1);
    old.drain();
    let mut new = connect(&hub, 1);
    assert!(old.closed(), "古い接続には、閉じる指示が届く");
    assert_eq!(hub.connection_count(), 1);
    new.drain();
    // 古い接続からのメッセージは、無視される
    find_room(&hub, &old, 4);
    assert!(new.drain().is_empty());
    assert_eq!(hub.connection_count(), 1);
    // 古い接続が閉じても、新しい接続は、残る
    hub.disconnect(&old.id, old.conn);
    assert_eq!(hub.connection_count(), 1);
}

#[tokio::test]
async fn a_disconnect_removes_the_connection() {
    let hub = hub();
    let client = connect(&hub, 1);
    hub.disconnect(&client.id, client.conn);
    assert_eq!(hub.connection_count(), 0);
}

#[tokio::test]
async fn open_sockets_are_counted_before_hello_and_released_when_dropped() {
    let hub = hub_with(2);
    let first = hub.try_open_socket().expect("1 つ目");
    let second = hub.try_open_socket().expect("2 つ目");
    assert_eq!(hub.open_socket_count(), 2);
    assert!(hub.try_open_socket().is_none(), "上限に達した");
    drop(first);
    assert_eq!(hub.open_socket_count(), 1);
    let third = hub.try_open_socket();
    assert!(third.is_some(), "空きができた");
    drop((second, third));
    assert_eq!(hub.open_socket_count(), 0);
}

#[tokio::test]
async fn the_hub_signals_a_connection_to_close_when_it_replaces_or_shuts_down() {
    let hub = hub();
    let old = connect(&hub, 1);
    let new = connect(&hub, 1);
    // 置き換えられた古い接続は、閉じる合図を受ける（送信口が詰まっていても、届く）
    tokio::time::timeout(Duration::from_millis(200), old.closer.notified())
        .await
        .expect("古い接続に、閉じる合図が届く");
    // サーバーの終了では、全接続に合図が届く
    hub.shutdown();
    tokio::time::timeout(Duration::from_millis(200), new.closer.notified())
        .await
        .expect("終了の合図が届く");
}

// ---- 振り分け ----

#[tokio::test]
async fn find_room_puts_the_player_in_a_waiting_room() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    client.drain();
    find_room(&hub, &client, 4);
    let messages = client.drain();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, ServerMessage::RoomState { .. }))
    );
}

#[tokio::test]
async fn invalid_operations_are_rejected_with_codes() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    client.drain();
    // ルームに入る前の、行動・開始・投了
    send(
        &hub,
        &client,
        ClientMessage::Act {
            turn: 0,
            action: Action::Pass,
        },
    );
    send(&hub, &client, ClientMessage::Start);
    send(&hub, &client, ClientMessage::Resign);
    let messages = client.drain();
    assert_eq!(
        messages
            .iter()
            .filter(|m| is_rejected(RejectCode::NotInRoom)(m))
            .count(),
        3
    );
    // すでに入っているのに、もう一度探す
    find_room(&hub, &client, 4);
    client.drain();
    find_room(&hub, &client, 4);
    assert!(
        client
            .drain()
            .iter()
            .any(is_rejected(RejectCode::AlreadyInRoom))
    );
    // 待機中の行動
    send(
        &hub,
        &client,
        ClientMessage::Act {
            turn: 0,
            action: Action::Pass,
        },
    );
    assert!(
        client
            .drain()
            .iter()
            .any(is_rejected(RejectCode::NotPlaying))
    );
}

#[tokio::test]
async fn leaving_a_waiting_room_lets_the_player_search_again() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    client.drain();
    find_room(&hub, &client, 4);
    send(&hub, &client, ClientMessage::LeaveRoom);
    client.drain();
    find_room(&hub, &client, 4);
    let messages = client.drain();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, ServerMessage::RoomState { .. }))
    );
    assert!(
        !messages
            .iter()
            .any(|m| matches!(m, ServerMessage::Rejected { .. }))
    );
}

#[tokio::test]
async fn two_players_with_the_same_rules_meet_in_one_room_and_start_by_themselves() {
    let hub = hub();
    let mut a = connect(&hub, 1);
    let mut b = connect(&hub, 2);
    a.drain();
    b.drain();
    find_room(&hub, &a, 2);
    find_room(&hub, &b, 2);
    for client in [&mut a, &mut b] {
        let messages = client
            .wait_for("GameStarted", |m| {
                matches!(m, ServerMessage::GameStarted { .. })
            })
            .await;
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, ServerMessage::TurnStarted { .. }))
        );
    }
}

// ---- 対局（AI の駆動を含む） ----

fn your_seat(messages: &[ServerMessage]) -> PlayerId {
    messages
        .iter()
        .find_map(|m| match m {
            ServerMessage::GameStarted { you, .. } => Some(*you),
            _ => None,
        })
        .expect("GameStarted が届いている")
}

/// 人間 1 人 + AI で始める。`hub` の AI の駆動で、人間の手番まで進む。
async fn start_with_ai(hub: &Arc<Hub>, client: &mut Client) -> (PlayerId, u32) {
    client.drain();
    find_room(hub, client, 4);
    send(hub, client, ClientMessage::Start);
    let mut seen = client
        .wait_for("GameStarted", |m| {
            matches!(m, ServerMessage::GameStarted { .. })
        })
        .await;
    let you = your_seat(&seen);
    // 自分の手番になるまで（AI が先に指す）
    let turn = loop {
        if let Some(turn) = seen.iter().rev().find_map(|m| match m {
            ServerMessage::TurnStarted { player, turn, .. } if *player == you => Some(*turn),
            _ => None,
        }) {
            break turn;
        }
        seen.extend(
            client
                .wait_for("自分の手番", |m| {
                    matches!(m, ServerMessage::TurnStarted { .. })
                })
                .await,
        );
    };
    (you, turn)
}

#[tokio::test]
async fn ai_seats_play_by_themselves_until_it_is_the_humans_turn() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    let (you, _turn) = start_with_ai(&hub, &mut client).await;
    assert!(you.0 < 4);
}

#[tokio::test]
async fn the_human_can_place_a_castle_on_their_turn() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    let (_, turn) = start_with_ai(&hub, &mut client).await;
    let castle = rules::pieces::Catalog::builtin().unwrap().castle();
    // 空いている場所を探す（ほかの城に近すぎるところは、拒否される）
    let mut placed = false;
    'search: for y in (0..15).step_by(5) {
        for x in (0..15).step_by(5) {
            send(
                &hub,
                &client,
                ClientMessage::Act {
                    turn,
                    action: Action::Place {
                        kind: castle,
                        anchor: Pos::new(x, y),
                        facing: Facing::North,
                    },
                },
            );
            let messages = client.drain();
            if messages
                .iter()
                .any(|m| matches!(m, ServerMessage::Applied { .. }))
            {
                placed = true;
                break 'search;
            }
            assert!(
                messages.iter().all(|m| matches!(
                    m,
                    ServerMessage::Rejected {
                        code: RejectCode::IllegalAction,
                        ..
                    } | ServerMessage::Rejected {
                        code: RejectCode::NotYourTurn,
                        ..
                    } | ServerMessage::Rejected {
                        code: RejectCode::StaleTurn,
                        ..
                    }
                )),
                "{messages:?}"
            );
        }
    }
    assert!(placed, "城を置ける場所が見つかるはず");
}

#[tokio::test]
async fn an_action_with_a_wrong_turn_number_is_rejected_and_changes_nothing() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    let (_, turn) = start_with_ai(&hub, &mut client).await;
    send(
        &hub,
        &client,
        ClientMessage::Act {
            turn: turn + 3,
            action: Action::Pass,
        },
    );
    let messages = client.drain();
    assert!(messages.iter().any(is_rejected(RejectCode::StaleTurn)));
    assert!(
        !messages
            .iter()
            .any(|m| matches!(m, ServerMessage::Applied { .. }))
    );
}

#[tokio::test]
async fn an_illegal_action_carries_the_reason() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    let (_, turn) = start_with_ai(&hub, &mut client).await;
    send(
        &hub,
        &client,
        ClientMessage::Act {
            turn,
            action: Action::Pass,
        },
    );
    let messages = client.drain();
    assert!(messages.iter().any(|m| matches!(
        m,
        ServerMessage::Rejected {
            code: RejectCode::IllegalAction,
            reason: Some(ActionError::WrongPhase),
            ..
        }
    )));
}

#[tokio::test]
async fn resigning_ends_the_game_for_everyone() {
    let hub = hub();
    let mut client = connect(&hub, 1);
    start_with_ai(&hub, &mut client).await;
    send(&hub, &client, ClientMessage::Resign);
    client.wait_for("Eliminated を含む Applied", |m| {
        matches!(m, ServerMessage::Applied { events, .. } if events.iter().any(|e| matches!(e, rules::action::Event::Eliminated { .. })))
    })
    .await;
}

// ---- 切断・再接続 ----

#[tokio::test]
async fn a_player_who_reconnects_during_a_game_gets_the_seat_and_the_full_state() {
    let hub = hub();
    let mut first = connect(&hub, 1);
    start_with_ai(&hub, &mut first).await;
    hub.disconnect(&first.id, first.conn);
    let mut second = connect(&hub, 1);
    let messages = second.drain();
    assert!(matches!(
        &messages[0],
        ServerMessage::Welcome {
            reconnected: true,
            ..
        }
    ));
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, ServerMessage::State { .. }))
    );
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, ServerMessage::TurnStarted { .. }))
    );
}

#[tokio::test]
async fn others_are_told_about_a_disconnect() {
    let hub = hub();
    let mut a = connect(&hub, 1);
    let mut b = connect(&hub, 2);
    find_room(&hub, &a, 4);
    find_room(&hub, &b, 4);
    a.drain();
    b.drain();
    send(&hub, &a, ClientMessage::Start);
    a.wait_for("GameStarted", |m| {
        matches!(m, ServerMessage::GameStarted { .. })
    })
    .await;
    b.wait_for("GameStarted", |m| {
        matches!(m, ServerMessage::GameStarted { .. })
    })
    .await;
    hub.disconnect(&a.id, a.conn);
    let messages = b
        .wait_for("切断の通知", |m| {
            matches!(
                m,
                ServerMessage::Notice {
                    kind: NoticeKind::PlayerDisconnected,
                    ..
                }
            )
        })
        .await;
    assert!(messages.iter().any(|m| matches!(m, ServerMessage::RoomState { room } if room.seats.iter().any(|s| s.kind == SeatKind::Human && !s.connected))));
}

// ---- 配送 ----

#[tokio::test]
async fn a_slow_client_that_never_reads_is_cut_loose() {
    let hub = hub();
    // 送信口が 1 つしかない、読まない接続
    let (tx, _rx) = mpsc::channel(1);
    let id = pid(1);
    let closer = Arc::new(Notify::new());
    hub.connect(&id, "遅い", tx, Arc::clone(&closer)).unwrap(); // Welcome で、送信口が埋まる
    let conn = hub.lock().conns.get(&id).unwrap().id;
    hub.handle(
        &id,
        conn,
        ClientMessage::FindRoom {
            rules: RoomRules::DEFAULT,
        },
    );
    assert_eq!(hub.connection_count(), 0, "詰まった接続は、切られる");
    // 送信口が詰まっていて、閉じる指示を送れなくても、閉じる合図は、確実に届く
    tokio::time::timeout(Duration::from_millis(200), closer.notified())
        .await
        .expect("閉じる合図が届く");
}

#[tokio::test]
async fn messages_for_players_without_a_connection_are_dropped_quietly() {
    let hub = hub();
    let client = connect(&hub, 1);
    find_room(&hub, &client, 4);
    hub.disconnect(&client.id, client.conn);
    // 待機中の切断は、退室になる。ルームは、次の tick で片付く
    hub.tick();
    assert_eq!(hub.connection_count(), 0);
}

#[tokio::test]
async fn the_clock_advances_with_tick_without_a_runtime_panic() {
    let hub = hub();
    let _client = connect(&hub, 1);
    let ticker = hub.spawn_ticker();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(hub.now() >= 500);
    ticker.abort();
}
