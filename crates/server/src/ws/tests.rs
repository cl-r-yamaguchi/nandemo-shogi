use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{ServerFrame, ServerMessage};
use rules::action::Action;
use rules::geometry::{Facing, Pos};
use rules::pieces::Catalog;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::*;
use crate::hub::{Hub, HubConfig};
use crate::rate::{BURST, MAX_STRIKES};

const ORIGIN: &str = "http://localhost:5173";

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct TestServer {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    /// `serve` の終了を待つためのもの。
    done: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn config() -> Config {
    Config::from_lookup_for_tests(ORIGIN)
}

fn hub_config(max_connections: usize) -> HubConfig {
    HubConfig {
        ai_min_delay: Duration::ZERO,
        max_connections,
        log: false,
        ..HubConfig::default()
    }
}

async fn start_with_config(hub_config: HubConfig) -> TestServer {
    let config = config();
    let hub = Hub::new(Arc::new(Catalog::builtin().unwrap()), hub_config, 11);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    hub.spawn_ticker();
    let (stop, stopped) = oneshot::channel::<()>();
    let hub_for_shutdown = Arc::clone(&hub);
    let done = tokio::spawn(async move {
        let _ = serve(listener, hub, &config, async move {
            let _ = stopped.await;
            hub_for_shutdown.shutdown();
        })
        .await;
    });
    TestServer {
        addr,
        stop: Some(stop),
        done: Some(done),
    }
}

async fn start_with(max_connections: usize) -> TestServer {
    start_with_config(hub_config(max_connections)).await
}

async fn start() -> TestServer {
    start_with(100).await
}

async fn open(server: &TestServer, origin: Option<&str>) -> Result<Client, WsError> {
    let mut request = format!("ws://{}/ws", server.addr)
        .into_client_request()
        .unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
    }
    connect_async(request).await.map(|(client, _)| client)
}

async fn connect(server: &TestServer) -> Client {
    open(server, Some(ORIGIN)).await.expect("接続できる")
}

fn pid(n: u32) -> String {
    format!("player-{n:012}")
}

async fn send_json(client: &mut Client, value: Value) {
    client
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn send_text(client: &mut Client, text: &str) {
    client.send(Message::Text(text.into())).await.unwrap();
}

/// 次のサーバーのメッセージ。閉じられたら `None`。
async fn next_frame(client: &mut Client) -> Option<ServerMessage> {
    loop {
        let received = tokio::time::timeout(Duration::from_secs(10), client.next())
            .await
            .expect("10 秒以内に、何かが届く");
        match received {
            Some(Ok(Message::Text(body))) => {
                return Some(
                    serde_json::from_str::<ServerFrame>(body.as_str())
                        .unwrap()
                        .message,
                );
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
            Some(Ok(_)) => continue,
        }
    }
}

async fn hello(client: &mut Client, n: u32) -> ServerMessage {
    send_json(
        client,
        json!({ "v": 1, "type": "hello", "player_id": pid(n), "display_name": format!("ゲスト{n}") }),
    )
    .await;
    next_frame(client).await.expect("welcome が届く")
}

async fn expect_closed(client: &mut Client) {
    for _ in 0..100 {
        if next_frame(client).await.is_none() {
            return;
        }
    }
    panic!("接続が閉じられませんでした");
}

fn is_rejected(message: &ServerMessage, code: RejectCode) -> bool {
    matches!(message, ServerMessage::Rejected { code: c, .. } if *c == code)
}

// ---- オリジン ----

#[test]
fn origins_are_matched_exactly_and_a_missing_origin_is_refused() {
    let allowed = vec![
        "http://localhost:5173".to_string(),
        "http://127.0.0.1:5173".to_string(),
    ];
    assert!(origin_allowed(&allowed, Some("http://localhost:5173")));
    assert!(origin_allowed(&allowed, Some("http://127.0.0.1:5173")));
    assert!(origin_allowed(&allowed, Some("HTTP://LOCALHOST:5173")));
    for bad in [
        "http://localhost:5174",
        "http://localhost",
        "https://localhost:5173",
        "http://localhost:5173.evil.example",
        "http://evil.example",
        "null",
        "",
    ] {
        assert!(!origin_allowed(&allowed, Some(bad)), "{bad}");
    }
    assert!(!origin_allowed(&allowed, None));
    assert!(!origin_allowed(&[], Some("http://localhost:5173")));
}

#[tokio::test]
async fn only_an_allowed_origin_can_connect() {
    let server = start().await;
    assert!(open(&server, Some(ORIGIN)).await.is_ok());
    for origin in [Some("http://evil.example"), Some("null"), None] {
        let error = open(&server, origin).await.expect_err("拒否される");
        match error {
            WsError::Http(response) => assert_eq!(response.status(), 403, "{origin:?}"),
            other => panic!("403 のはずが、{other:?}"),
        }
    }
}

#[tokio::test]
async fn the_connection_limit_returns_503() {
    let server = start_with(1).await;
    let mut first = connect(&server).await;
    hello(&mut first, 1).await;
    let error = open(&server, Some(ORIGIN)).await.expect_err("拒否される");
    match error {
        WsError::Http(response) => assert_eq!(response.status(), 503),
        other => panic!("503 のはずが、{other:?}"),
    }
}

// ---- hello ----

#[tokio::test]
async fn hello_is_answered_with_welcome() {
    let server = start().await;
    let mut client = connect(&server).await;
    let welcome = hello(&mut client, 1).await;
    assert!(matches!(
        welcome,
        ServerMessage::Welcome { reconnected: false, ref display_name, .. } if display_name == "ゲスト1"
    ));
}

#[tokio::test]
async fn the_first_message_must_be_a_valid_hello() {
    let server = start().await;
    for first in [
        json!({ "v": 1, "type": "start" }),
        json!({ "v": 2, "type": "hello", "player_id": pid(1), "display_name": "a" }),
        json!({ "v": 1, "type": "hello", "player_id": "short", "display_name": "a" }),
        json!({ "v": 1, "type": "hello", "player_id": pid(1), "display_name": "\u{200B}" }),
    ] {
        let mut client = connect(&server).await;
        send_json(&mut client, first.clone()).await;
        let reply = next_frame(&mut client).await.expect("rejected が届く");
        assert!(matches!(reply, ServerMessage::Rejected { .. }), "{first}");
        expect_closed(&mut client).await;
    }
    let mut client = connect(&server).await;
    send_text(&mut client, "not json").await;
    assert!(matches!(
        next_frame(&mut client).await,
        Some(ServerMessage::Rejected { .. })
    ));
    expect_closed(&mut client).await;
}

// ---- 不正な入力 ----

#[tokio::test]
async fn an_oversized_message_closes_the_connection() {
    let server = start().await;
    let mut client = connect(&server).await;
    hello(&mut client, 1).await;
    send_text(&mut client, &"x".repeat(5000)).await;
    expect_closed(&mut client).await;
}

#[tokio::test]
async fn binary_messages_are_rejected() {
    let server = start().await;
    let mut client = connect(&server).await;
    hello(&mut client, 1).await;
    client
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    let reply = next_frame(&mut client).await.unwrap();
    assert!(is_rejected(&reply, RejectCode::BadMessage));
}

#[tokio::test]
async fn bad_messages_get_a_reason_and_a_run_of_them_cuts_the_connection() {
    let server = start().await;
    let mut client = connect(&server).await;
    hello(&mut client, 1).await;
    send_text(&mut client, "{\"v\":1,\"type\":\"nope\"}").await;
    let reply = next_frame(&mut client).await.unwrap();
    assert!(is_rejected(&reply, RejectCode::BadMessage));
    send_json(&mut client, json!({ "v": 9, "type": "start" })).await;
    let reply = next_frame(&mut client).await.unwrap();
    assert!(is_rejected(&reply, RejectCode::VersionMismatch));
    // 正しいメッセージが続けば、切られない。不正が続けば、切られる
    for _ in 0..MAX_STRIKES {
        send_text(&mut client, "garbage").await;
    }
    expect_closed(&mut client).await;
}

#[tokio::test]
async fn a_good_message_resets_the_run_of_bad_ones() {
    let server = start().await;
    let mut client = connect(&server).await;
    hello(&mut client, 1).await;
    for _ in 0..3 {
        for _ in 0..MAX_STRIKES - 1 {
            send_text(&mut client, "garbage").await;
            assert!(matches!(
                next_frame(&mut client).await,
                Some(ServerMessage::Rejected { .. })
            ));
        }
        // 正しいメッセージ（ルームに入っていないので、何も返らない）で、数え直す
        send_json(&mut client, json!({ "v": 1, "type": "leave_room" })).await;
    }
    // 不正が累計では何十件あっても、連続していないので、接続は生きている
    send_text(&mut client, "garbage").await;
    assert!(matches!(
        next_frame(&mut client).await,
        Some(ServerMessage::Rejected { .. })
    ));
}

#[tokio::test]
async fn flooding_is_rate_limited_and_then_cut() {
    let server = start().await;
    let mut client = connect(&server).await;
    hello(&mut client, 1).await;
    for _ in 0..(BURST + MAX_STRIKES + 20) {
        if client
            .send(Message::Text(
                json!({ "v": 1, "type": "leave_room" }).to_string().into(),
            ))
            .await
            .is_err()
        {
            break;
        }
    }
    let mut limited = false;
    for _ in 0..200 {
        match next_frame(&mut client).await {
            Some(message) => limited |= is_rejected(&message, RejectCode::RateLimited),
            None => break,
        }
    }
    assert!(limited, "頻度超過の拒否が返る");
    expect_closed(&mut client).await;
}

// ---- 通し ----

async fn start_game(client: &mut Client) -> (protocol::StateView, u8, u32) {
    send_json(
        client,
        json!({ "v": 1, "type": "find_room", "rules": { "players": 4, "ai_level": 1 } }),
    )
    .await;
    send_json(client, json!({ "v": 1, "type": "start" })).await;
    let mut you = None;
    let mut state = None;
    loop {
        match next_frame(client).await.expect("対局が始まる") {
            ServerMessage::GameStarted {
                you: seat,
                state: s,
                ..
            } => {
                you = Some(seat.0);
                state = Some(s);
            }
            ServerMessage::TurnStarted { player, turn, .. } if Some(player.0) == you => {
                return (state.unwrap(), player.0, turn);
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn a_game_starts_over_the_socket_and_the_human_can_place_a_castle() {
    let server = start().await;
    let mut client = connect(&server).await;
    hello(&mut client, 1).await;
    let (state, you, turn) = start_game(&mut client).await;
    assert_eq!(state.players.len(), 4);
    assert!(you < 4);
    let castle = Catalog::builtin().unwrap().castle().0;
    let mut applied = false;
    'search: for y in (0..15).step_by(5) {
        for x in (0..15).step_by(5) {
            send_json(
                &mut client,
                json!({ "v": 1, "type": "act", "turn": turn, "action": { "type": "place", "kind": castle, "anchor": { "x": x, "y": y }, "facing": "north" } }),
            )
            .await;
            loop {
                match next_frame(&mut client).await.expect("返事が届く") {
                    ServerMessage::Applied { .. } => {
                        applied = true;
                        break 'search;
                    }
                    ServerMessage::Rejected { .. } => break,
                    _ => {}
                }
            }
        }
    }
    assert!(applied);
}

#[tokio::test]
async fn reconnecting_with_the_same_id_returns_to_the_seat() {
    let server = start().await;
    let mut first = connect(&server).await;
    hello(&mut first, 1).await;
    start_game(&mut first).await;
    drop(first);
    // サーバーが、切断に気づくまで、少し待つ
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut second = connect(&server).await;
    let welcome = hello(&mut second, 1).await;
    assert!(matches!(
        welcome,
        ServerMessage::Welcome {
            reconnected: true,
            ..
        }
    ));
    let mut got_state = false;
    for _ in 0..10 {
        if let Some(ServerMessage::State { .. }) = next_frame(&mut second).await {
            got_state = true;
            break;
        }
    }
    assert!(got_state);
}

#[tokio::test]
async fn a_second_connection_with_the_same_id_replaces_the_first() {
    let server = start().await;
    let mut first = connect(&server).await;
    hello(&mut first, 1).await;
    let mut second = connect(&server).await;
    hello(&mut second, 1).await;
    expect_closed(&mut first).await;
    // 新しい接続は、使える
    send_json(
        &mut second,
        json!({ "v": 1, "type": "find_room", "rules": { "players": 4, "ai_level": 2 } }),
    )
    .await;
    assert!(matches!(
        next_frame(&mut second).await,
        Some(ServerMessage::Notice { .. })
    ));
}

#[tokio::test]
async fn the_action_type_matches_the_protocol() {
    // テストが送る JSON が、`rules` の `Action` と一致していることの確認
    let castle = Catalog::builtin().unwrap().castle();
    let value = json!({ "type": "place", "kind": castle.0, "anchor": { "x": 1, "y": 2 }, "facing": "north" });
    let action: Action = serde_json::from_value(value).unwrap();
    assert_eq!(
        action,
        Action::Place {
            kind: castle,
            anchor: Pos::new(1, 2),
            facing: Facing::North
        }
    );
}

// ---- 無通信・終了 ----

#[tokio::test]
async fn a_silent_client_is_closed_after_the_idle_timeout_but_a_responsive_one_is_kept() {
    let server = start_with_config(HubConfig {
        ping_interval: Duration::from_millis(100),
        idle_timeout: Duration::from_millis(400),
        ..hub_config(100)
    })
    .await;
    // 応答する（読み続ける）クライアントは、ping に、自動で応答するので、切られない
    let mut alive = connect(&server).await;
    hello(&mut alive, 1).await;
    // 読まないクライアントは、ping に応答できず、切られる
    let mut silent = connect(&server).await;
    hello(&mut silent, 2).await;
    let reading = tokio::time::timeout(Duration::from_millis(1200), async {
        loop {
            if alive.next().await.is_none() {
                return false;
            }
        }
    })
    .await;
    assert!(
        reading.is_err(),
        "応答するクライアントは、1.2 秒たっても、切られない"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    // 読まなかったほうは、閉じられている（読み始めると、すぐに終わりが見える）
    let mut closed = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_millis(500), silent.next()).await {
            Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => {
                closed = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            Err(_) => break,
        }
    }
    assert!(closed, "無通信の接続は、閉じられる");
}

#[tokio::test]
async fn sockets_that_never_say_hello_count_against_the_limit_and_are_freed() {
    let server = start_with(2).await;
    let _a = connect(&server).await; // hello を送らない
    let _b = connect(&server).await;
    let error = open(&server, Some(ORIGIN)).await.expect_err("上限");
    assert!(matches!(error, WsError::Http(ref r) if r.status() == 503));
}

#[tokio::test]
async fn the_server_shuts_down_even_while_clients_are_connected() {
    let mut server = start().await;
    let mut client = connect(&server).await;
    hello(&mut client, 1).await;
    let done = server.done.take().unwrap();
    if let Some(stop) = server.stop.take() {
        stop.send(()).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), done)
        .await
        .expect("接続が残っていても、終了できる")
        .unwrap();
    expect_closed(&mut client).await;
}
