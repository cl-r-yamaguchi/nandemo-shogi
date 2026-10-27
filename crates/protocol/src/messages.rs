//! メッセージの型。すべてのメッセージは `{ "v": 版, "type": 種別, ... }` の形（online.md の 6 章）。

use rules::action::{Action, ActionError, Event};
use rules::finish::Outcome;
use rules::state::PlayerId;
use serde::{Deserialize, Serialize};

use crate::view::StateView;
use crate::{MAX_AI_LEVEL, MAX_PLAYERS, MIN_AI_LEVEL, MIN_PLAYERS};

/// 対戦のルール（リクエスト）。現時点の項目は、人数と AI の強さ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RoomRules {
    /// AI を含む合計人数。
    pub players: u8,
    /// AI の強さ（1〜3）。
    pub ai_level: u8,
}

impl RoomRules {
    /// 既定のルール（人数 4、AI の強さ 2）。
    pub const DEFAULT: RoomRules = RoomRules {
        players: 4,
        ai_level: 2,
    };

    pub fn is_valid(&self) -> bool {
        (MIN_PLAYERS..=MAX_PLAYERS).contains(&self.players)
            && (MIN_AI_LEVEL..=MAX_AI_LEVEL).contains(&self.ai_level)
    }
}

/// クライアント → サーバー。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ClientMessage {
    /// 接続直後、または再接続のとき。
    Hello {
        player_id: String,
        display_name: String,
    },
    /// ルールを決めて、ルームを探す（なければ作られる）。
    FindRoom {
        rules: RoomRules,
    },
    LeaveRoom,
    /// 待機中のルームを、いま始める（空き席は AI で埋める）。
    Start,
    /// 行動の確定。選択（`pick`）も、ここで送る。`turn` は、手番番号。
    Act {
        turn: u32,
        action: Action,
    },
    Resign,
}

/// 版つきのクライアントのメッセージ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ClientFrame {
    pub v: u32,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub message: ClientMessage,
}

/// ルームの状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum RoomStatus {
    Waiting,
    Playing,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum SeatKind {
    Human,
    Ai,
}

/// 席の 1 つ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SeatView {
    pub seat: PlayerId,
    pub kind: SeatKind,
    pub name: String,
    /// AI の席のときだけ。
    pub ai_level: Option<u8>,
    /// 人間の席が、接続しているか。AI は常に `true`。
    pub connected: bool,
}

/// ルームの状況。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RoomView {
    pub room_id: String,
    pub status: RoomStatus,
    pub rules: RoomRules,
    /// 席の一覧。待機中は、入室済みの人間だけ。対戦が始まったら、全席（席順）。
    pub seats: Vec<SeatView>,
}

/// 操作を拒否した理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum RejectCode {
    /// 読み取れないメッセージ（壊れた JSON、未知の種別、欠けた項目）。
    BadMessage,
    TooLarge,
    VersionMismatch,
    InvalidName,
    InvalidPlayerId,
    InvalidRules,
    /// ルームに入っていない。
    NotInRoom,
    /// 対戦中でない（待機中に行動した、など）。
    NotPlaying,
    /// 待機中でない（対戦が始まっているルームの、入室や開始）。
    NotWaiting,
    /// ルームが満員。
    RoomFull,
    /// すでに、ほかのルームに入っている。
    AlreadyInRoom,
    NotYourTurn,
    /// 手番番号が、古い（二重送信、遅れた操作）。
    StaleTurn,
    /// 規則に反する行動。詳しい理由は、`rejected` の `reason`。
    IllegalAction,
}

/// 通知の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum NoticeKind {
    PlayerJoined,
    PlayerLeft,
    PlayerDisconnected,
    PlayerReconnected,
    /// 切断した席を、AI が代行する。
    AiTakeover,
    /// 持ち時間を過ぎて、手番が強制的に終わった。
    TimedOut,
    RoomClosed,
}

/// サーバー → クライアント。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ServerMessage {
    /// `hello` への返事。
    Welcome {
        player_id: String,
        display_name: String,
        /// 切断前の席に、戻れたか。
        reconnected: bool,
    },
    RoomState {
        room: RoomView,
    },
    /// 対戦が始まった。`you` は、あなたの席（観戦なし。常にある）。
    GameStarted {
        room: RoomView,
        you: PlayerId,
        state: StateView,
    },
    /// 手番が始まった。`remaining_ms` は、持ち時間の残り（クライアントの時計に依存しないため、残りで送る）。
    TurnStarted {
        player: PlayerId,
        turn: u32,
        remaining_ms: u32,
    },
    /// 全状態（再接続のときなど）。
    State {
        state: StateView,
    },
    /// 行動の結果。出来事を順に再生できる。
    Applied {
        turn: u32,
        events: Vec<Event>,
        state: StateView,
    },
    /// 操作の拒否。`reason` は、`code` が `illegal_action` のときだけ。
    Rejected {
        code: RejectCode,
        reason: Option<ActionError>,
        message: String,
    },
    GameOver {
        outcome: Outcome,
        state: StateView,
    },
    Notice {
        kind: NoticeKind,
        message: String,
    },
}

/// 版つきのサーバーのメッセージ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ServerFrame {
    pub v: u32,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub message: ServerMessage,
}

impl ServerFrame {
    pub fn new(message: ServerMessage) -> Self {
        Self {
            v: crate::PROTOCOL_VERSION,
            message,
        }
    }
}
