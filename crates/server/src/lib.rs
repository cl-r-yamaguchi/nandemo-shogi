//! サーバーの、通信に依存しない部分（ルーム・マッチング・対局の進行）。
//! 時計とメッセージは、外から渡す（`Millis` と `Outbound`）。通信と記録は、別のモジュールが担う。
//! 構成は docs/specs/architecture.md の 4.4。

pub mod config;
pub mod hub;
pub mod lobby;
pub mod outbound;
pub mod rate;
pub mod record;
pub mod room;
pub mod store;
pub mod ws;

/// 単調に増える時刻（ミリ秒）。起点は、呼び出し側が決める（サーバーの起動時など）。
pub type Millis = u64;
