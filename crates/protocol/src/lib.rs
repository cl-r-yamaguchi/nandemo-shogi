//! クライアント・サーバー間のメッセージの型と、受信の検証。仕様は docs/specs/online.md の 6 章。
//!
//! TypeScript の型は、ここから生成する（`cargo test -p protocol` で `web/src/net/generated/` に出力）。
//! 行動（`Action`）と出来事（`Event`）は、`rules` の型をそのまま使う。

mod messages;
mod validate;
mod view;

pub use messages::{
    ClientFrame, ClientMessage, NoticeKind, RejectCode, RoomRules, RoomStatus, RoomView, SeatKind,
    SeatView, ServerFrame, ServerMessage,
};
pub use validate::{
    DecodeError, FieldError, decode_client, encode_server, validate_display_name,
    validate_player_id,
};
pub use view::{HandEntry, PieceView, PlayerView, StateView};

/// プロトコルの版。合わないクライアントは、接続時に拒否する。
pub const PROTOCOL_VERSION: u32 = 1;

/// 1 つのメッセージの大きさの上限（バイト）。
pub const MAX_MESSAGE_BYTES: usize = 4096;

/// 人数の下限・上限（ルールの設定）。上限は、将来引き上げうる。
pub const MIN_PLAYERS: u8 = 2;
pub const MAX_PLAYERS: u8 = 4;

/// AI の強さの下限・上限。
pub const MIN_AI_LEVEL: u8 = 1;
pub const MAX_AI_LEVEL: u8 = 3;

/// 表示名の長さの上限（文字数）。
pub const MAX_DISPLAY_NAME_CHARS: usize = 12;

/// プレイヤー ID の長さの範囲（文字数）。
pub const PLAYER_ID_LEN: std::ops::RangeInclusive<usize> = 16..=64;

#[cfg(test)]
mod tests;
