//! ゲームの状態の型と、不変条件（R-04）の検証。仕様は docs/specs/game-rules.md。

use std::collections::BTreeMap;
use std::fmt;

use crate::BOARD_SIZE;
use crate::finish::Outcome;
use crate::geometry::{Facing, Pos, Rect};
use crate::pieces::{Catalog, KindId};

/// 席の番号（手番の順。0 始まり）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u8);

/// 盤上の駒を区別する番号。駒が消えても、再利用しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PieceId(pub u32);

/// 盤上の駒。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub id: PieceId,
    pub kind: KindId,
    pub owner: PlayerId,
    pub anchor: Pos,
    pub facing: Facing,
    pub hp: u32,
}

/// ゲームの段階（game-rules.md の 3 章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// 城を 1 つずつ置く（R-12）。
    Placement,
    /// ドロー・選択・行動を繰り返す。
    Battle,
    Finished,
}

/// プレイヤーごとの状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerState {
    pub eliminated: bool,
    /// 持ち駒（種類ごとの個数）。
    pub hand: BTreeMap<KindId, u32>,
}

/// 不変条件（R-04）の違反。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    UnknownKind(PieceId),
    UnknownOwner(PieceId),
    OutsideBoard(PieceId),
    Overlap(PieceId, PieceId),
    TooManyCastles(PlayerId),
    HpOutOfRange(PieceId),
    DuplicatePieceId(PieceId),
    /// 発行していない番号の駒がある（`issue_piece_id` が、同じ番号を再び発行してしまう）。
    PieceIdNotIssued(PieceId),
    /// 持ち駒に、不明な種類がある。
    UnknownHandKind(PlayerId),
    /// 手番のプレイヤーが、存在しない、または脱落している。
    InvalidCurrentPlayer(PlayerId),
    /// 脱落したプレイヤーが、盤上の駒か持ち駒を持っている（R-91）。
    EliminatedHasAssets(PlayerId),
    /// 脱落の記録と、`eliminated` の印が合わない。
    EliminationRecordMismatch,
    /// 終局の状態と、結果の有無が合わない。
    OutcomeMismatch,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind(id) => write!(f, "駒 {} の種類が不明です", id.0),
            Self::UnknownOwner(id) => write!(f, "駒 {} の持ち主が不明です", id.0),
            Self::OutsideBoard(id) => write!(f, "駒 {} が盤の外にはみ出しています", id.0),
            Self::Overlap(a, b) => write!(f, "駒 {} と駒 {} が重なっています", a.0, b.0),
            Self::TooManyCastles(p) => write!(f, "プレイヤー {} の城が 2 つ以上あります", p.0),
            Self::HpOutOfRange(id) => write!(f, "駒 {} の HP が範囲外です", id.0),
            Self::DuplicatePieceId(id) => write!(f, "駒の番号 {} が重複しています", id.0),
            Self::PieceIdNotIssued(id) => write!(f, "駒の番号 {} は、まだ発行されていません", id.0),
            Self::UnknownHandKind(p) => {
                write!(f, "プレイヤー {} の持ち駒に、不明な種類があります", p.0)
            }
            Self::InvalidCurrentPlayer(p) => write!(f, "手番のプレイヤー {} が不正です", p.0),
            Self::EliminatedHasAssets(p) => {
                write!(f, "脱落したプレイヤー {} が、駒か持ち駒を持っています", p.0)
            }
            Self::EliminationRecordMismatch => {
                write!(f, "脱落の記録が、プレイヤーの状態と合いません")
            }
            Self::OutcomeMismatch => write!(f, "終局の状態と、結果の有無が合いません"),
        }
    }
}

impl std::error::Error for Violation {}

/// 人数の下限。上限は、設定で決まる（定数にしない）。
pub const MIN_PLAYERS: usize = 2;

/// ゲームの状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameState {
    pub board_size: u32,
    pub players: Vec<PlayerState>,
    pub pieces: Vec<Piece>,
    pub phase: Phase,
    /// 手番のプレイヤー。
    pub current: PlayerId,
    /// 対戦フェーズの手番番号（0 始まり）。
    pub turn: u32,
    /// 脱落した順の記録。同じ行動で同時に脱落した人は、同じ組に入る（R-94・R-97）。
    pub out_batches: Vec<Vec<PlayerId>>,
    /// 終局したときの結果（`phase` が `Finished` のときだけ）。
    pub outcome: Option<Outcome>,
    next_piece_id: u32,
}

impl GameState {
    /// R-10: 盤上に駒はなく、各プレイヤーの持ち駒は城 1 個だけ。
    /// `first` は最初の手番のプレイヤー（R-11。乱数で決めるのは呼び出し側）。
    pub fn new(catalog: &Catalog, player_count: usize, first: PlayerId) -> Option<Self> {
        Self::with_board_size(catalog, BOARD_SIZE, player_count, first)
    }

    /// 盤の大きさを指定して作る（人数・盤の大きさは、定数ではなく設定値）。
    /// 人数は、`MIN_PLAYERS` 以上、席の番号（`u8`）で表せる範囲まで。
    pub fn with_board_size(
        catalog: &Catalog,
        board_size: u32,
        player_count: usize,
        first: PlayerId,
    ) -> Option<Self> {
        if player_count < MIN_PLAYERS
            || player_count > usize::from(u8::MAX)
            || usize::from(first.0) >= player_count
            || board_size == 0
        {
            return None;
        }
        let hand = BTreeMap::from([(catalog.castle(), 1)]);
        let players = (0..player_count)
            .map(|_| PlayerState {
                eliminated: false,
                hand: hand.clone(),
            })
            .collect();
        Some(Self {
            board_size,
            players,
            pieces: Vec::new(),
            phase: Phase::Placement,
            current: first,
            turn: 0,
            out_batches: Vec::new(),
            outcome: None,
            next_piece_id: 0,
        })
    }

    /// 脱落していないプレイヤー（席の順）。
    pub fn alive_players(&self) -> Vec<PlayerId> {
        self.players
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.eliminated)
            .map(|(seat, _)| PlayerId(seat as u8))
            .collect()
    }

    /// 手番を、次の席（脱落した席は飛ばす）へ回す（R-22）。生き残りがいなければ、そのまま。
    pub fn advance_current(&mut self) {
        let seats = self.players.len();
        let from = usize::from(self.current.0);
        for offset in 1..=seats {
            let seat = (from + offset) % seats;
            if !self.players[seat].eliminated {
                self.current = PlayerId(seat as u8);
                return;
            }
        }
    }

    /// 新しい駒の番号を発行する。
    pub fn issue_piece_id(&mut self) -> PieceId {
        let id = PieceId(self.next_piece_id);
        self.next_piece_id += 1;
        id
    }

    pub fn piece(&self, id: PieceId) -> Option<&Piece> {
        self.pieces.iter().find(|p| p.id == id)
    }

    /// 駒の占有範囲（R-03）。種類が不明なら `None`。
    pub fn footprint(&self, catalog: &Catalog, piece: &Piece) -> Option<Rect> {
        let def = catalog.get(piece.kind)?;
        Some(def.footprint(piece.anchor, piece.facing))
    }

    /// 不変条件（R-04）を確かめる。最初に見つかった違反を返す。
    pub fn validate(&self, catalog: &Catalog) -> Result<(), Violation> {
        let mut rects: Vec<(PieceId, Rect)> = Vec::with_capacity(self.pieces.len());
        let mut castles: BTreeMap<PlayerId, u32> = BTreeMap::new();

        // 終局したあとは、手番のプレイヤーが脱落していてもよい
        if self.phase != Phase::Finished {
            match self.players.get(usize::from(self.current.0)) {
                Some(player) if !player.eliminated => {}
                _ => return Err(Violation::InvalidCurrentPlayer(self.current)),
            }
        } else if usize::from(self.current.0) >= self.players.len() {
            return Err(Violation::InvalidCurrentPlayer(self.current));
        }
        if (self.phase == Phase::Finished) != self.outcome.is_some() {
            return Err(Violation::OutcomeMismatch);
        }
        if let Some(outcome) = &self.outcome {
            // 順位は、全員を、過不足なく 1 回ずつ含む
            let mut ranked: Vec<u8> = outcome.ranking.iter().flatten().map(|p| p.0).collect();
            ranked.sort_unstable();
            let all: Vec<u8> = (0..self.players.len() as u8).collect();
            if ranked != all || outcome.ranking.iter().any(Vec::is_empty) {
                return Err(Violation::OutcomeMismatch);
            }
        }
        let recorded: Vec<PlayerId> = self.out_batches.iter().flatten().copied().collect();
        let flagged = self.players.iter().filter(|p| p.eliminated).count();
        let unique: std::collections::BTreeSet<_> = recorded.iter().collect();
        let consistent = recorded.len() == flagged
            && unique.len() == recorded.len()
            && recorded.iter().all(|p| {
                self.players
                    .get(usize::from(p.0))
                    .is_some_and(|state| state.eliminated)
            });
        if !consistent {
            return Err(Violation::EliminationRecordMismatch);
        }
        for (seat, player) in self.players.iter().enumerate() {
            if player.hand.keys().any(|kind| catalog.get(*kind).is_none()) {
                return Err(Violation::UnknownHandKind(PlayerId(seat as u8)));
            }
            if player.eliminated && !player.hand.is_empty() {
                return Err(Violation::EliminatedHasAssets(PlayerId(seat as u8)));
            }
        }

        for piece in &self.pieces {
            let def = catalog
                .get(piece.kind)
                .ok_or(Violation::UnknownKind(piece.id))?;
            if usize::from(piece.owner.0) >= self.players.len() {
                return Err(Violation::UnknownOwner(piece.id));
            }
            if self.players[usize::from(piece.owner.0)].eliminated {
                return Err(Violation::EliminatedHasAssets(piece.owner));
            }
            if piece.id.0 >= self.next_piece_id {
                return Err(Violation::PieceIdNotIssued(piece.id));
            }
            if rects.iter().any(|(id, _)| *id == piece.id) {
                return Err(Violation::DuplicatePieceId(piece.id));
            }
            // R-04 の 4
            if piece.hp == 0 || piece.hp > def.hp {
                return Err(Violation::HpOutOfRange(piece.id));
            }
            // R-04 の 1
            let rect = def.footprint(piece.anchor, piece.facing);
            if !rect.is_inside(self.board_size) {
                return Err(Violation::OutsideBoard(piece.id));
            }
            // R-04 の 2
            if let Some((other, _)) = rects.iter().find(|(_, r)| r.overlaps(&rect)) {
                return Err(Violation::Overlap(*other, piece.id));
            }
            // R-04 の 3
            if def.is_castle() {
                let count = castles.entry(piece.owner).or_insert(0);
                *count += 1;
                if *count > 1 {
                    return Err(Violation::TooManyCastles(piece.owner));
                }
            }
            rects.push((piece.id, rect));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::builtin().unwrap()
    }

    fn state(catalog: &Catalog) -> GameState {
        GameState::new(catalog, 4, PlayerId(0)).unwrap()
    }

    fn put(
        state: &mut GameState,
        catalog: &Catalog,
        id: &str,
        owner: u8,
        x: i32,
        y: i32,
    ) -> PieceId {
        let kind = catalog.kind_of(id).unwrap();
        let hp = catalog.get(kind).unwrap().hp;
        let piece_id = state.issue_piece_id();
        state.pieces.push(Piece {
            id: piece_id,
            kind,
            owner: PlayerId(owner),
            anchor: Pos::new(x, y),
            facing: Facing::North,
            hp,
        });
        piece_id
    }

    #[test]
    fn r10_new_game_has_an_empty_board_and_only_castles_in_hand() {
        let catalog = catalog();
        let state = state(&catalog);
        assert!(state.pieces.is_empty());
        assert_eq!(state.phase, Phase::Placement);
        assert_eq!(state.players.len(), 4);
        for player in &state.players {
            assert_eq!(player.hand.len(), 1);
            assert_eq!(player.hand[&catalog.castle()], 1);
            assert!(!player.eliminated);
        }
        assert_eq!(state.board_size, 16);
        assert_eq!(state.validate(&catalog), Ok(()));
    }

    #[test]
    fn new_game_rejects_too_few_players_and_bad_first_player() {
        let catalog = catalog();
        assert!(GameState::new(&catalog, 1, PlayerId(0)).is_none());
        assert!(GameState::new(&catalog, 3, PlayerId(3)).is_none());
        assert!(GameState::new(&catalog, 2, PlayerId(1)).is_some());
    }

    #[test]
    fn issued_piece_ids_are_unique() {
        let catalog = catalog();
        let mut state = state(&catalog);
        let a = state.issue_piece_id();
        let b = state.issue_piece_id();
        assert_ne!(a, b);
    }

    #[test]
    fn r04_valid_board_passes() {
        let catalog = catalog();
        let mut state = state(&catalog);
        put(&mut state, &catalog, "castle", 0, 0, 0);
        put(&mut state, &catalog, "castle", 1, 14, 14);
        put(&mut state, &catalog, "lion", 0, 2, 0);
        assert_eq!(state.validate(&catalog), Ok(()));
    }

    #[test]
    fn r04_1_piece_outside_board_is_rejected() {
        let catalog = catalog();
        let mut state = state(&catalog);
        let id = put(&mut state, &catalog, "castle", 0, 15, 15);
        assert_eq!(state.validate(&catalog), Err(Violation::OutsideBoard(id)));
    }

    #[test]
    fn r04_1_rotation_can_push_a_piece_outside() {
        let catalog = catalog();
        let mut state = state(&catalog);
        // シャチ（1x2）は、北向きなら (15, 14) に収まるが、東向きでは 2x1 になりはみ出す
        let id = put(&mut state, &catalog, "orca", 0, 15, 14);
        assert_eq!(state.validate(&catalog), Ok(()));
        state.pieces[0].facing = Facing::East;
        assert_eq!(state.validate(&catalog), Err(Violation::OutsideBoard(id)));
    }

    #[test]
    fn r04_2_overlapping_pieces_are_rejected() {
        let catalog = catalog();
        let mut state = state(&catalog);
        let a = put(&mut state, &catalog, "castle", 0, 4, 4);
        let b = put(&mut state, &catalog, "lion", 1, 5, 5);
        assert_eq!(state.validate(&catalog), Err(Violation::Overlap(a, b)));
    }

    #[test]
    fn r04_3_a_player_may_have_at_most_one_castle() {
        let catalog = catalog();
        let mut state = state(&catalog);
        put(&mut state, &catalog, "castle", 0, 0, 0);
        let second = put(&mut state, &catalog, "castle", 0, 8, 8);
        assert_eq!(
            state.validate(&catalog),
            Err(Violation::TooManyCastles(PlayerId(0)))
        );
        assert_eq!(state.piece(second).map(|p| p.owner), Some(PlayerId(0)));
        // 別のプレイヤーなら、城は 1 つずつ置ける
        state.pieces[1].owner = PlayerId(1);
        assert_eq!(state.validate(&catalog), Ok(()));
    }

    #[test]
    fn r04_4_hp_must_be_between_one_and_the_maximum() {
        let catalog = catalog();
        let mut state = state(&catalog);
        let id = put(&mut state, &catalog, "lion", 0, 0, 0);
        state.pieces[0].hp = 0;
        assert_eq!(state.validate(&catalog), Err(Violation::HpOutOfRange(id)));
        state.pieces[0].hp = 3; // ライオンの最大 HP は 2
        assert_eq!(state.validate(&catalog), Err(Violation::HpOutOfRange(id)));
        state.pieces[0].hp = 2;
        assert_eq!(state.validate(&catalog), Ok(()));
    }

    #[test]
    fn unknown_owner_and_kind_are_rejected() {
        let catalog = catalog();
        let mut state = state(&catalog);
        let id = put(&mut state, &catalog, "lion", 0, 0, 0);
        state.pieces[0].owner = PlayerId(9);
        assert_eq!(state.validate(&catalog), Err(Violation::UnknownOwner(id)));
        state.pieces[0].owner = PlayerId(0);
        state.pieces[0].kind = KindId(999);
        assert_eq!(state.validate(&catalog), Err(Violation::UnknownKind(id)));
    }

    #[test]
    fn duplicate_piece_ids_are_rejected() {
        let catalog = catalog();
        let mut state = state(&catalog);
        let id = put(&mut state, &catalog, "lion", 0, 0, 0);
        put(&mut state, &catalog, "lion", 0, 3, 3);
        state.pieces[1].id = id;
        assert_eq!(
            state.validate(&catalog),
            Err(Violation::DuplicatePieceId(id))
        );
    }

    #[test]
    fn footprint_uses_size_and_facing() {
        let catalog = catalog();
        let mut state = state(&catalog);
        put(&mut state, &catalog, "orca", 0, 3, 3);
        state.pieces[0].facing = Facing::West;
        let rect = state.footprint(&catalog, &state.pieces[0]).unwrap();
        assert_eq!(
            rect,
            Rect {
                x: 3,
                y: 3,
                w: 2,
                h: 1
            }
        );
    }

    #[test]
    fn current_player_must_exist_and_be_alive() {
        let catalog = catalog();
        let mut state = state(&catalog);
        state.current = PlayerId(7);
        assert_eq!(
            state.validate(&catalog),
            Err(Violation::InvalidCurrentPlayer(PlayerId(7)))
        );
        state.current = PlayerId(1);
        state.players[1].eliminated = true;
        assert_eq!(
            state.validate(&catalog),
            Err(Violation::InvalidCurrentPlayer(PlayerId(1)))
        );
    }

    #[test]
    fn hand_kinds_must_be_known() {
        let catalog = catalog();
        let mut state = state(&catalog);
        state.players[2].hand.insert(KindId(999), 1);
        assert_eq!(
            state.validate(&catalog),
            Err(Violation::UnknownHandKind(PlayerId(2)))
        );
    }

    #[test]
    fn piece_ids_must_have_been_issued() {
        let catalog = catalog();
        let mut state = state(&catalog);
        let id = put(&mut state, &catalog, "lion", 0, 0, 0);
        state.pieces[0].id = PieceId(50);
        assert_eq!(
            state.validate(&catalog),
            Err(Violation::PieceIdNotIssued(PieceId(50)))
        );
        state.pieces[0].id = id;
        assert_eq!(state.validate(&catalog), Ok(()));
    }

    #[test]
    fn board_size_and_player_count_are_settings() {
        let catalog = catalog();
        let small = GameState::with_board_size(&catalog, 8, 3, PlayerId(0)).unwrap();
        assert_eq!(small.board_size, 8);
        assert!(GameState::with_board_size(&catalog, 0, 3, PlayerId(0)).is_none());
        assert!(GameState::with_board_size(&catalog, 16, 256, PlayerId(0)).is_none());
        let many = GameState::with_board_size(&catalog, 64, 255, PlayerId(254)).unwrap();
        assert_eq!(many.players.len(), 255);
        assert_eq!(many.validate(&catalog), Ok(()));
    }
}
