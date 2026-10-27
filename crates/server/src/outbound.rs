//! ルームが、通信の層に渡す、送るべきメッセージ。

use protocol::ServerMessage;

/// 1 人のプレイヤーに送るメッセージ。ルーム全体への送信は、接続中の人間ごとに展開される。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    /// 宛先のプレイヤー ID。
    pub to: String,
    pub message: ServerMessage,
}

impl Outbound {
    pub fn new(to: impl Into<String>, message: ServerMessage) -> Self {
        Self {
            to: to.into(),
            message,
        }
    }
}
