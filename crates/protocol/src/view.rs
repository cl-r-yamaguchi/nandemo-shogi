//! 盤面の全体の写し。`GameState` の内部を漏らさず、クライアントに必要なものだけを持つ。

use rules::finish::Outcome;
use rules::geometry::{Facing, Pos};
use rules::pieces::KindId;
use rules::state::{GameState, Phase, Piece, PieceId, PlayerId, Stage};
use serde::{Deserialize, Serialize};

/// 持ち駒の 1 種類ぶん。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct HandEntry {
    pub kind: KindId,
    pub count: u32,
}

/// 盤上の駒。`rules` の `Piece` の内部が、通信の形に漏れないように、別の型にしている。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PieceView {
    pub id: PieceId,
    pub kind: KindId,
    pub owner: PlayerId,
    /// 占有範囲の左上のマス。
    pub anchor: Pos,
    pub facing: Facing,
    pub hp: u32,
}

impl From<&Piece> for PieceView {
    fn from(piece: &Piece) -> Self {
        Self {
            id: piece.id,
            kind: piece.kind,
            owner: piece.owner,
            anchor: piece.anchor,
            facing: piece.facing,
            hp: piece.hp,
        }
    }
}

/// プレイヤーごとの状態。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PlayerView {
    pub eliminated: bool,
    /// 持ち駒。種類の番号の順。
    pub hand: Vec<HandEntry>,
}

/// 盤面の全体。ドローで引いた駒・持ち駒を含め、すべてが全員に公開される。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StateView {
    pub board_size: u32,
    pub phase: Phase,
    pub stage: Stage,
    /// 手番のプレイヤー（席の番号）。
    pub current: PlayerId,
    /// 対戦フェーズの手番番号。
    pub turn: u32,
    pub players: Vec<PlayerView>,
    pub pieces: Vec<PieceView>,
    pub outcome: Option<Outcome>,
}

impl StateView {
    pub fn from_state(state: &GameState) -> Self {
        Self {
            board_size: state.board_size,
            phase: state.phase,
            stage: state.stage.clone(),
            current: state.current,
            turn: state.turn,
            players: state
                .players
                .iter()
                .map(|p| PlayerView {
                    eliminated: p.eliminated,
                    hand: p
                        .hand
                        .iter()
                        .map(|(kind, count)| HandEntry {
                            kind: *kind,
                            count: *count,
                        })
                        .collect(),
                })
                .collect(),
            pieces: state.pieces.iter().map(PieceView::from).collect(),
            outcome: state.outcome.clone(),
        }
    }
}
