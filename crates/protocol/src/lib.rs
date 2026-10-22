//! クライアント・サーバー間のメッセージの型。TypeScript の型は、ここから生成する。
//! 仕様は docs/specs/online.md の 6 章。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// プロトコルの版。合わないクライアントは接続時に拒否する。
pub const PROTOCOL_VERSION: u32 = 1;

/// クライアントが接続直後に送るメッセージ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Hello {
    pub v: u32,
    pub player_id: String,
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips_as_json() {
        let hello = Hello {
            v: PROTOCOL_VERSION,
            player_id: "p1".into(),
            display_name: "ゲスト1234".into(),
        };
        let json = serde_json::to_string(&hello).unwrap();
        assert_eq!(serde_json::from_str::<Hello>(&json).unwrap(), hello);
    }
}
