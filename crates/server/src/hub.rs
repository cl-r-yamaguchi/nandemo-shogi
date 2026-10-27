//! ハブ: 接続の管理、ロビーへの振り分け、結果の配送、時計と AI の駆動。
//! 仕様は docs/specs/online.md の 5・6 章。WebSocket そのものは、`ws.rs` が担う（ここは、通信の形に依存しない）。

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use protocol::{ClientMessage, RejectCode, ServerFrame, ServerMessage, encode_server};
use rules::pieces::Catalog;
use rules::rng::SplitMix64;
use tokio::sync::{Notify, mpsc};

use crate::Millis;
use crate::lobby::Lobby;
use crate::outbound::Outbound;
use crate::room::AI_THINK_MS;

/// 接続へ送るもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnMsg {
    /// 送る文字列（JSON）。
    Text(String),
    /// 生きているかの確認（ping）。応答がなければ、無通信として、閉じる。
    Ping,
    /// 接続を閉じる。
    Close,
}

/// 接続の送信口の大きさ。これを超えて詰まった（遅い）接続は、切る。
pub const OUTBOX_CAPACITY: usize = 256;

/// 時計を進める間隔。
pub const TICK_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct HubConfig {
    /// AI の手を反映するまでの、最低の間。
    pub ai_min_delay: Duration,
    /// 同時に受け付ける接続の上限。
    pub max_connections: usize,
    /// ログを標準エラーに出すか。
    pub log: bool,
    /// ping を送る間隔。
    pub ping_interval: Duration,
    /// 何も届かない（ping への応答も含めて）まま、これだけたったら、接続を閉じる。
    pub idle_timeout: Duration,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            ai_min_delay: Duration::from_millis(crate::config::DEFAULT_AI_DELAY_MS),
            max_connections: crate::config::DEFAULT_MAX_CONNECTIONS,
            log: false,
            ping_interval: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(90),
        }
    }
}

/// 接続できなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    /// 同時接続数の上限。
    TooMany,
}

/// 接続の番号。同じプレイヤー ID の、新旧の接続を区別する。
pub type ConnId = u64;

struct Conn {
    id: ConnId,
    name: String,
    tx: mpsc::Sender<ConnMsg>,
    /// ハブが接続を切るときの合図（詰まった・置き換えられた・終了）。送信口を手放すだけでは、
    /// 読み込みの側が、閉じないため。
    closer: Arc<Notify>,
}

struct Inner {
    lobby: Lobby,
    rng: SplitMix64,
    conns: HashMap<String, Conn>,
    next_conn: ConnId,
    /// AI を駆動しているルームの番号（同じルームで、駆動が重ならないように）。
    ai_running: HashSet<u64>,
}

pub struct Hub {
    inner: Mutex<Inner>,
    config: HubConfig,
    start: Instant,
    /// 開いている通信路の数（`hello` の前のものも含む）。
    open_sockets: AtomicUsize,
}

/// 開いている通信路の 1 つ分の記録。手放すと、数が減る。
pub struct SocketGuard {
    hub: Arc<Hub>,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        self.hub.open_sockets.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 実行時の乱数の種（ルームの席順・ドローなどに使う）。外部の乱数の crate を使わず、標準の、
/// 起動ごとに変わるハッシュの種から作る。
pub fn random_seed() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

impl Hub {
    pub fn new(catalog: Arc<Catalog>, config: HubConfig, seed: u64) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                lobby: Lobby::new(catalog),
                rng: SplitMix64(seed),
                conns: HashMap::new(),
                next_conn: 1,
                ai_running: HashSet::new(),
            }),
            config,
            start: Instant::now(),
            open_sockets: AtomicUsize::new(0),
        })
    }

    pub fn config(&self) -> &HubConfig {
        &self.config
    }

    /// 通信路を 1 つ開く。上限に達していたら `None`。`hello` の前の通信路も数えるので、
    /// `hello` を送らずに通信路だけを大量に開いても、上限を超えられない。
    pub fn try_open_socket(self: &Arc<Self>) -> Option<SocketGuard> {
        let max = self.config.max_connections;
        self.open_sockets
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < max).then_some(n + 1)
            })
            .ok()?;
        Some(SocketGuard {
            hub: Arc::clone(self),
        })
    }

    /// 開いている通信路の数。
    pub fn open_socket_count(&self) -> usize {
        self.open_sockets.load(Ordering::SeqCst)
    }

    /// すべての接続に、閉じる合図を送る（サーバーの終了のとき）。
    pub fn shutdown(&self) {
        let inner = self.lock();
        for conn in inner.conns.values() {
            conn.closer.notify_one();
        }
    }

    /// 起動からの経過（ミリ秒）。ルームの時計として使う。
    pub fn now(&self) -> Millis {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // 別のスレッドが落ちて、ロックが汚れても、状態は、メッセージごとに完結しているので、続ける
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn connection_count(&self) -> usize {
        self.lock().conns.len()
    }

    fn log(&self, message: &str) {
        if self.config.log {
            eprintln!("[{:>8} ms] {message}", self.now());
        }
    }

    // ---- 接続 ----

    /// `hello` を受けた接続を登録する。同じプレイヤー ID の古い接続は、閉じる。
    /// 対戦中（または待機中）のルームに、同じ ID がいれば、席に戻す。
    pub fn connect(
        self: &Arc<Self>,
        player_id: &str,
        display_name: &str,
        tx: mpsc::Sender<ConnMsg>,
        closer: Arc<Notify>,
    ) -> Result<ConnId, ConnectError> {
        let now = self.now();
        let id;
        {
            let mut inner = self.lock();
            let replacing = inner.conns.contains_key(player_id);
            if !replacing && inner.conns.len() >= self.config.max_connections {
                return Err(ConnectError::TooMany);
            }
            id = inner.next_conn;
            inner.next_conn += 1;
            if let Some(old) = inner.conns.insert(
                player_id.to_string(),
                Conn {
                    id,
                    name: display_name.to_string(),
                    tx,
                    closer,
                },
            ) {
                // 古い接続には、閉じるよう伝える。送信口が詰まっていても、合図で、確実に閉じる
                old.closer.notify_one();
                let _ = old.tx.try_send(ConnMsg::Close);
            }
            let reconnect = inner.lobby.reconnect(player_id, now);
            let reconnected = reconnect.is_ok();
            let mut outs = vec![Outbound::new(
                player_id,
                ServerMessage::Welcome {
                    player_id: player_id.to_string(),
                    display_name: display_name.to_string(),
                    reconnected,
                },
            )];
            if let Ok(rest) = reconnect {
                outs.extend(rest);
            }
            Self::deliver(&mut inner, now, outs);
            self.log(&format!("接続 conn={id} 再接続={reconnected}"));
        }
        self.kick_ai();
        Ok(id)
    }

    /// 接続が切れた。登録されている接続と同じなら、ロビーに切断を知らせる。
    pub fn disconnect(self: &Arc<Self>, player_id: &str, conn: ConnId) {
        let now = self.now();
        {
            let mut inner = self.lock();
            if inner.conns.get(player_id).map(|c| c.id) != Some(conn) {
                return; // すでに、新しい接続に置き換わっている
            }
            inner.conns.remove(player_id);
            let outs = inner.lobby.disconnect(player_id, now);
            Self::deliver(&mut inner, now, outs);
            self.log(&format!("切断 conn={conn}"));
        }
        self.kick_ai();
    }

    // ---- メッセージ ----

    /// 検証済みのメッセージを、ロビーへ振り分ける。結果は、宛先の接続へ送る。
    pub fn handle(self: &Arc<Self>, player_id: &str, conn: ConnId, message: ClientMessage) {
        let now = self.now();
        {
            let mut inner = self.lock();
            if inner.conns.get(player_id).map(|c| c.id) != Some(conn) {
                return; // 古い接続からのメッセージは、無視する
            }
            let outs = Self::dispatch(&mut inner, player_id, message, now);
            self.log_events(&outs);
            Self::deliver(&mut inner, now, outs);
        }
        self.kick_ai();
    }

    /// 検証に通らなかったメッセージへの返事（`rejected`）を送る。
    pub fn reject(&self, player_id: &str, conn: ConnId, code: RejectCode, message: &str) {
        let now = self.now();
        let mut inner = self.lock();
        if inner.conns.get(player_id).map(|c| c.id) != Some(conn) {
            return;
        }
        let outs = vec![Outbound::new(
            player_id,
            ServerMessage::rejected(code, message),
        )];
        Self::deliver(&mut inner, now, outs);
    }

    fn dispatch(
        inner: &mut Inner,
        player_id: &str,
        message: ClientMessage,
        now: Millis,
    ) -> Vec<Outbound> {
        let name = inner
            .conns
            .get(player_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let reject = |m: ServerMessage| vec![Outbound::new(player_id, m)];
        match message {
            // 2 回目の `hello` は、受け付けない
            ClientMessage::Hello { .. } => reject(ServerMessage::rejected(
                RejectCode::BadMessage,
                "hello は、接続の最初に 1 回だけ送れます",
            )),
            ClientMessage::FindRoom { rules } => {
                let Inner { lobby, rng, .. } = inner;
                lobby
                    .find_room(player_id, &name, rules, now, rng)
                    .unwrap_or_else(|e| reject(e.to_message()))
            }
            ClientMessage::LeaveRoom => inner.lobby.leave(player_id, now),
            ClientMessage::Start => {
                let Inner { lobby, rng, .. } = inner;
                lobby
                    .start(player_id, now, rng)
                    .unwrap_or_else(|e| reject(e.to_message()))
            }
            ClientMessage::Act { turn, action } => inner
                .lobby
                .act(player_id, turn, action, now)
                .unwrap_or_else(|e| reject(e.to_message())),
            ClientMessage::Resign => inner
                .lobby
                .resign(player_id, now)
                .unwrap_or_else(|e| reject(e.to_message())),
        }
    }

    // ---- 配送 ----

    /// 結果を、宛先の接続へ送る。接続がない宛先は、捨てる。詰まった（遅い）接続は、切る。
    fn deliver(inner: &mut Inner, now: Millis, outs: Vec<Outbound>) {
        let mut pending = outs;
        // 詰まった接続を切ると、ロビーへの切断の通知で、新しいメッセージが生まれるので、繰り返す。
        // 1 周ごとに、接続が 1 つ以上減るか、メッセージが尽きるので、必ず終わる
        while !pending.is_empty() {
            let mut slow: Vec<String> = Vec::new();
            for out in pending.drain(..) {
                let Some(conn) = inner.conns.get(&out.to) else {
                    continue;
                };
                let text = encode_server(&ServerFrame::new(out.message));
                if conn.tx.try_send(ConnMsg::Text(text)).is_err() && !slow.contains(&out.to) {
                    slow.push(out.to);
                }
            }
            for player in slow {
                if let Some(conn) = inner.conns.remove(&player) {
                    conn.closer.notify_one(); // 詰まっていて、閉じる指示が送れないので、合図で閉じる
                }
                pending.extend(inner.lobby.disconnect(&player, now));
            }
        }
    }

    fn log_events(&self, outs: &[Outbound]) {
        if !self.config.log {
            return;
        }
        let mut seen = HashSet::new();
        for out in outs {
            match &out.message {
                ServerMessage::GameStarted { room, .. }
                    if seen.insert(("start", room.room_id.clone())) =>
                {
                    self.log(&format!(
                        "対局開始 {}（{} 席）",
                        room.room_id,
                        room.seats.len()
                    ));
                }
                ServerMessage::GameOver { outcome, .. }
                    if seen.insert(("over", format!("{:?}", outcome.reason))) =>
                {
                    self.log(&format!("対局終了 理由={:?}", outcome.reason));
                }
                ServerMessage::Rejected { code, .. }
                    if seen.insert(("rej", format!("{code:?}"))) =>
                {
                    self.log(&format!("拒否 {code:?}"));
                }
                _ => {}
            }
        }
    }

    // ---- 時計 ----

    /// 時計を進める（時間切れ・切断・期限）。
    pub fn tick(self: &Arc<Self>) {
        let now = self.now();
        {
            let mut inner = self.lock();
            let outs = inner.lobby.tick(now);
            self.log_events(&outs);
            Self::deliver(&mut inner, now, outs);
        }
        self.kick_ai();
    }

    /// 一定間隔で `tick` を呼ぶ、背景のタスクを始める。
    pub fn spawn_ticker(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TICK_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                hub.tick();
            }
        })
    }

    // ---- AI ----

    /// AI の手番を待っているルームに、駆動のタスクを始める（すでに動いているルームは、飛ばす）。
    pub fn kick_ai(self: &Arc<Self>) {
        let rooms: Vec<u64> = {
            let mut inner = self.lock();
            let needing = inner.lobby.rooms_needing_ai();
            needing
                .into_iter()
                .filter(|room| inner.ai_running.insert(*room))
                .collect()
        };
        for room in rooms {
            let hub = Arc::clone(self);
            tokio::spawn(async move { hub.drive_ai(room).await });
        }
    }

    /// 1 つのルームの AI の手番を、続けて進める。考える間は、ロックを手放す。
    async fn drive_ai(self: Arc<Self>, room: u64) {
        loop {
            let plan = {
                let mut inner = self.lock();
                match inner.lobby.plan_ai(room) {
                    Some(plan) => plan,
                    None => {
                        // 計画がない（AI の手番ではない、またはルームがない）: 駆動を終える
                        inner.ai_running.remove(&room);
                        return;
                    }
                }
            };
            let started = Instant::now();
            let think = Duration::from_millis(AI_THINK_MS);
            let fallback = plan.abstain();
            let decision =
                match tokio::task::spawn_blocking(move || plan.think(started + think)).await {
                    Ok(decision) => decision,
                    Err(_) => {
                        // 考える処理が異常終了した: 手番を強制的に終える（同じ局面で、失敗を繰り返さない）
                        self.log("AI の思考が異常終了したので、手番を強制的に終えます");
                        fallback
                    }
                };
            // 何が起きたか分かるように、最低の間を置く
            let elapsed = started.elapsed();
            if elapsed < self.config.ai_min_delay {
                tokio::time::sleep(self.config.ai_min_delay - elapsed).await;
            }
            let now = self.now();
            let mut inner = self.lock();
            let outs = inner.lobby.apply_ai(room, now, decision);
            self.log_events(&outs);
            Self::deliver(&mut inner, now, outs);
        }
    }
}

#[cfg(test)]
mod tests;
