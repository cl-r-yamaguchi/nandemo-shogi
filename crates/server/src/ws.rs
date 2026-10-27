//! WebSocket の入口: オリジンの検証、接続の受け付け、メッセージの読み書き。
//! 仕様は docs/specs/online.md の 5・6 章、architecture.md の 6 章。

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use protocol::{
    ClientMessage, MAX_MESSAGE_BYTES, RejectCode, ServerFrame, ServerMessage, decode_client,
    encode_server,
};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};

use crate::config::Config;
use crate::hub::{ConnMsg, Hub, OUTBOX_CAPACITY, SocketGuard};
use crate::rate::{RateLimiter, Strikes};

/// 最初のメッセージ（`hello`）を待つ時間。
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct App {
    hub: Arc<Hub>,
    allowed_origins: Arc<Vec<String>>,
}

/// `Origin` ヘッダーが、許可するオリジンのどれかと一致するか。ヘッダーがないときは、許可しない
/// （ブラウザは、WebSocket の接続に、必ず `Origin` を付ける）。
pub fn origin_allowed(allowed: &[String], origin: Option<&str>) -> bool {
    match origin {
        Some(origin) => allowed
            .iter()
            .any(|a| a.eq_ignore_ascii_case(origin.trim())),
        None => false,
    }
}

pub fn router(hub: Arc<Hub>, config: &Config) -> Router {
    let app = App {
        hub,
        allowed_origins: Arc::new(config.allowed_origins.clone()),
    };
    Router::new().route("/ws", get(ws_handler)).with_state(app)
}

/// 待ち受けて、`shutdown` が完了するまで、接続を受け付ける。
pub async fn serve(
    listener: TcpListener,
    hub: Arc<Hub>,
    config: &Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(hub, config))
        .with_graceful_shutdown(shutdown)
        .await
}

async fn ws_handler(State(app): State<App>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !origin_allowed(&app.allowed_origins, origin) {
        return (StatusCode::FORBIDDEN, "許可されていないオリジンです").into_response();
    }
    // `hello` の前の通信路も数えて、上限を守る
    let Some(guard) = app.hub.try_open_socket() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "接続数の上限です").into_response();
    };
    let hub = Arc::clone(&app.hub);
    // 通信の層でも、メッセージの大きさの上限を守る
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, hub, guard))
}

fn text(message: ServerMessage) -> ConnMsg {
    ConnMsg::Text(encode_server(&ServerFrame::new(message)))
}

async fn handle_socket(socket: WebSocket, hub: Arc<Hub>, _guard: SocketGuard) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ConnMsg>(OUTBOX_CAPACITY);

    // 書き込み: 送信口に入ったものを、順に送る。閉じる指示、または、送信口が手放されたら、終わる。
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                ConnMsg::Text(body) => {
                    if sink.send(Message::Text(body.into())).await.is_err() {
                        break;
                    }
                }
                ConnMsg::Ping => {
                    if sink.send(Message::Ping(Default::default())).await.is_err() {
                        break;
                    }
                }
                ConnMsg::Close => break,
            }
        }
        let _ = sink.send(Message::Close(None)).await;
        let _ = sink.close().await;
    });

    // 読み込み: 最初は `hello`。
    let identity = match read_hello(&mut stream, &tx).await {
        Some(identity) => identity,
        None => {
            drop(tx);
            let _ = writer.await;
            return;
        }
    };
    let closer = Arc::new(Notify::new());
    let Ok(conn) = hub.connect(&identity.0, &identity.1, tx.clone(), Arc::clone(&closer)) else {
        let _ = tx
            .send(text(ServerMessage::rejected(
                RejectCode::BadMessage,
                "接続数の上限です",
            )))
            .await;
        drop(tx);
        let _ = writer.await;
        return;
    };

    let mut limiter = RateLimiter::new(hub.now());
    let mut strikes = Strikes::default();
    let ping_interval = hub.config().ping_interval;
    let idle_timeout = hub.config().idle_timeout;
    let mut ping = tokio::time::interval(ping_interval);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            // ハブが、切る（詰まった・置き換えられた・サーバーの終了）
            () = closer.notified() => break,
            // 定期的に、生きているか確かめる。何も届かないまま、期限が過ぎたら、閉じる
            _ = ping.tick() => {
                if last_heard.elapsed() >= idle_timeout {
                    break;
                }
                let _ = tx.try_send(ConnMsg::Ping);
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break };
                last_heard = tokio::time::Instant::now();
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(_) => {
                        // 大きすぎるフレームなど、通信の層で拒否されたもの
                        hub.reject(&identity.0, conn, RejectCode::TooLarge, "メッセージが大きすぎるか、形式が不正です");
                        break;
                    }
                };
                if !process_frame(&hub, &identity.0, conn, frame, &mut limiter, &mut strikes) {
                    break;
                }
            }
        }
    }

    hub.disconnect(&identity.0, conn);
    drop(tx);
    // 書き込みは、残りを送り終えて、閉じる。詰まっていて終わらないときは、待たずに止める
    let mut writer = writer;
    if tokio::time::timeout(Duration::from_secs(1), &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

/// 1 つのフレームを処理する。接続を続けるなら `true`、切るなら `false`。
fn process_frame(
    hub: &Arc<Hub>,
    player_id: &str,
    conn: crate::hub::ConnId,
    frame: Message,
    limiter: &mut RateLimiter,
    strikes: &mut Strikes,
) -> bool {
    let body = match frame {
        Message::Text(body) => body,
        Message::Close(_) => return false,
        Message::Ping(_) | Message::Pong(_) => return true,
        Message::Binary(_) => {
            // テキストだけを受け付ける
            hub.reject(
                player_id,
                conn,
                RejectCode::BadMessage,
                "テキストだけ送れます",
            );
            return !strikes.hit();
        }
    };
    if !limiter.allow(hub.now()) {
        hub.reject(
            player_id,
            conn,
            RejectCode::RateLimited,
            "メッセージの頻度が高すぎます",
        );
        return !strikes.hit();
    }
    match decode_client(body.as_str()) {
        Ok(frame) => {
            strikes.ok();
            hub.handle(player_id, conn, frame.message);
            true
        }
        Err(error) => {
            hub.reject(player_id, conn, error.reject_code(), &error.message());
            !strikes.hit()
        }
    }
}

/// 最初のメッセージ（`hello`）を読む。`(プレイヤー ID, 表示名)`。時間内に来ない、または、
/// `hello` でないときは、理由を返して、`None`。
async fn read_hello(
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    tx: &mpsc::Sender<ConnMsg>,
) -> Option<(String, String)> {
    let first = tokio::time::timeout(HELLO_TIMEOUT, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(body))) => return Some(body),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return None,
            }
        }
    })
    .await;
    let Ok(Some(body)) = first else {
        let _ = tx
            .send(text(ServerMessage::rejected(
                RejectCode::BadMessage,
                "最初のメッセージは、hello を、時間内に送ってください",
            )))
            .await;
        return None;
    };
    match decode_client(body.as_str()) {
        Ok(frame) => match frame.message {
            ClientMessage::Hello {
                player_id,
                display_name,
            } => Some((player_id, display_name)),
            _ => {
                let _ = tx
                    .send(text(ServerMessage::rejected(
                        RejectCode::BadMessage,
                        "最初のメッセージは、hello にしてください",
                    )))
                    .await;
                None
            }
        },
        Err(error) => {
            let _ = tx
                .send(text(ServerMessage::rejected(
                    error.reject_code(),
                    error.message(),
                )))
                .await;
            None
        }
    }
}

#[cfg(test)]
mod tests;
