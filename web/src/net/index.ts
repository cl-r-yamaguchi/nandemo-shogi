// サーバーとの通信（WebSocket）。受信したメッセージは store へ渡す。
// メッセージの型は、Rust の `protocol` から生成したもの（generated/）。手で書かない。
export type { Action } from "./generated/Action";
export type { ActionError } from "./generated/ActionError";
export type { ClientFrame } from "./generated/ClientFrame";
export type { ClientMessage } from "./generated/ClientMessage";
export type { Event } from "./generated/Event";
export type { NoticeKind } from "./generated/NoticeKind";
export type { Outcome } from "./generated/Outcome";
export type { PieceView } from "./generated/PieceView";
export type { RejectCode } from "./generated/RejectCode";
export type { RoomRules } from "./generated/RoomRules";
export type { RoomStatus } from "./generated/RoomStatus";
export type { RoomView } from "./generated/RoomView";
export type { SeatKind } from "./generated/SeatKind";
export type { SeatView } from "./generated/SeatView";
export type { ServerFrame } from "./generated/ServerFrame";
export type { ServerMessage } from "./generated/ServerMessage";
export type { StateView } from "./generated/StateView";

/** プロトコルの版。Rust の `protocol::PROTOCOL_VERSION` と同じ値にする（合わないと、接続時に拒否される）。 */
export const PROTOCOL_VERSION = 1;
