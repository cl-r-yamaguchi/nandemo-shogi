//! 手番での行動（配置・移動・回転・パス）の適用。仕様は docs/specs/game-rules.md の 3 章。
//!
//! `apply` は、元の状態を変えずに、新しい状態と出来事の列を返す。

use std::fmt;

use crate::finish::{self, Outcome};
use crate::geometry::{Facing, Pos, Rect};
use crate::pieces::{Catalog, KindId, PieceDef};
use crate::state::{GameState, Phase, Piece, PieceId, PlayerId, Stage};

/// R-13: 城どうしの間に、あけるすきまの最小値（マス数）。
pub const CASTLE_MIN_GAP: i32 = 3;

/// 手番での行動。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 持ち駒を、`anchor`（占有範囲の左上）に、`facing` の向きで置く。
    Place {
        kind: KindId,
        anchor: Pos,
        facing: Facing,
    },
    /// 盤上の駒を、移動先のアンカー `to` へ動かす。`promote` は、捕食したときに成るか（R-63）。
    Move {
        piece: PieceId,
        to: Pos,
        promote: bool,
    },
    /// 盤上の駒の向きを変える。
    Rotate { piece: PieceId, facing: Facing },
    /// 行える行動がないとき、何もせずに手番を終える（R-80）。
    Pass,
    /// 引いた 4 枚のうち、`index` 番目（0 始まり）を持ち駒にする（R-06）。
    Pick { index: usize },
}

/// 行動の結果として起きたこと。画面のアニメーションと記録に使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Placed {
        piece: PieceId,
        kind: KindId,
        owner: PlayerId,
        anchor: Pos,
        facing: Facing,
    },
    Moved {
        piece: PieceId,
        from: Pos,
        to: Pos,
    },
    Rotated {
        piece: PieceId,
        facing: Facing,
    },
    /// ダメージを受けた。`hp` は、受けたあとの HP。
    Damaged {
        piece: PieceId,
        damage: u32,
        hp: u32,
    },
    /// 捕食された。`taker` の持ち駒になる（城は持ち駒にならず、`to_hand` が `false`）。
    Captured {
        piece: PieceId,
        kind: KindId,
        taker: PlayerId,
        to_hand: bool,
    },
    Promoted {
        piece: PieceId,
        from: KindId,
        to: KindId,
    },
    Passed {
        player: PlayerId,
    },
    /// 脱落した（R-90）。盤上の駒と持ち駒は、すべて消える（R-91）。
    Eliminated {
        player: PlayerId,
    },
    /// 終局した。結果は、順位と終了理由。
    Finished {
        outcome: Outcome,
    },
    /// 駒を引いた（R-05）。全員に公開される。
    Drawn {
        player: PlayerId,
        cards: Vec<KindId>,
    },
    /// 引いた駒から 1 枚を選び、持ち駒に加えた（R-06）。
    Picked {
        player: PlayerId,
        kind: KindId,
        index: usize,
    },
    /// 持ち時間を過ぎて、手番を強制的に終えた（R-31）。行動は、パスと同じ扱い（`Passed` は重ねない）。
    TimedOut {
        player: PlayerId,
    },
}

/// 行動を拒否する理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// この段階では、その行動はできない（例: 配置フェーズに移動、対戦フェーズに城の配置）。
    WrongPhase,
    UnknownKind,
    NotInHand,
    /// 城の向きは北に固定（R-13）。
    CastleMustFaceNorth,
    OutsideBoard,
    Overlaps,
    /// 自分の駒に隣接していない（R-42）。
    NotAdjacentToOwn,
    /// 他のプレイヤーの城に近すぎる（R-13）。
    TooCloseToCastle,
    UnknownPiece,
    NotOwner,
    /// 移動先が、移動範囲に入っていない（または動けない駒）。
    IllegalTarget,
    /// 城は回転できない（R-70）。
    CannotRotate,
    /// いまと同じ向きには回転できない（R-70）。
    SameFacing,
    /// 行える行動があるので、パスできない（R-80）。
    PassNotAllowed,
    /// 存在しない、または脱落済みのプレイヤー。
    UnknownPlayer,
    /// 手番の段階に合わない操作（ドロー前の選択、選択前の行動、再選択など）。
    WrongStage,
    /// 引いた 4 枚の番号が範囲外。
    InvalidPick,
    /// 城を置ける場所がない（配置フェーズの時間切れ）。呼び出し側が、そのプレイヤーを投了させる。
    NoLegalPlacement,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::WrongPhase => "いまの段階では、その行動はできません",
            Self::UnknownKind => "不明な駒の種類です",
            Self::NotInHand => "その駒は持ち駒にありません",
            Self::CastleMustFaceNorth => "城の向きは北に固定です",
            Self::OutsideBoard => "盤の外にはみ出します",
            Self::Overlaps => "ほかの駒と重なります",
            Self::NotAdjacentToOwn => "自分の駒に隣り合う場所にしか置けません",
            Self::TooCloseToCastle => "ほかのプレイヤーの城に近すぎます",
            Self::UnknownPiece => "その駒は盤上にありません",
            Self::NotOwner => "自分の駒ではありません",
            Self::IllegalTarget => "そこへは動けません",
            Self::CannotRotate => "城は回転できません",
            Self::SameFacing => "いまと同じ向きです",
            Self::PassNotAllowed => "行える行動があるので、パスできません",
            Self::UnknownPlayer => "存在しない、または脱落済みのプレイヤーです",
            Self::WrongStage => "いまの手番の段階では、その操作はできません",
            Self::InvalidPick => "引いた駒の番号が範囲外です",
            Self::NoLegalPlacement => "城を置ける場所がありません",
        };
        f.write_str(message)
    }
}

impl std::error::Error for ActionError {}

/// 行動を適用する。元の状態は変えず、新しい状態と出来事の列を返す。
/// 手番のプレイヤーは `state.current`。
///
/// 行動のあとに、脱落・終局・手番数の上限（R-90〜R-95）も判定する（`finish` モジュール）。
/// 投了（R-96）は、手番でなくてもできるので、`finish::resign` で行う。
pub fn apply(
    catalog: &Catalog,
    state: &GameState,
    action: &Action,
) -> Result<(GameState, Vec<Event>), ActionError> {
    if state.phase == Phase::Finished {
        return Err(ActionError::WrongPhase);
    }
    let mut next = state.clone();
    if let Action::Pick { index } = action {
        let events = pick(&mut next, *index)?;
        debug_assert_eq!(next.validate(catalog), Ok(()));
        return Ok((next, events));
    }
    // 対戦フェーズでは、選択を終えた「行動待ち」の段階でだけ、行動できる（R-06）
    if next.phase == Phase::Battle && next.stage != Stage::Act {
        return Err(ActionError::WrongStage);
    }
    let mut events = match action {
        Action::Place {
            kind,
            anchor,
            facing,
        } => place(catalog, &mut next, *kind, *anchor, *facing)?,
        Action::Move { piece, to, promote } => mov(catalog, &mut next, *piece, *to, *promote)?,
        Action::Rotate { piece, facing } => rotate(catalog, &mut next, *piece, *facing)?,
        Action::Pass => pass(catalog, &mut next)?,
        Action::Pick { .. } => unreachable!("選択は、上で処理済み"),
    };
    finish::settle_after_action(catalog, &mut next, &mut events);
    debug_assert_eq!(next.validate(catalog), Ok(()));
    Ok((next, events))
}

/// 手番のプレイヤーが、配置・移動・回転のいずれかを行えるか（R-80）。
pub fn has_legal_action(catalog: &Catalog, state: &GameState) -> bool {
    if state.phase != Phase::Battle {
        return true;
    }
    let me = state.current;
    let hand = &state.players[usize::from(me.0)].hand;
    let placeable = hand
        .iter()
        .filter(|(kind, count)| **count > 0 && !is_castle(catalog, **kind))
        .any(|(kind, _)| placement_candidates(catalog, state, *kind).next().is_some());
    if placeable {
        return true;
    }
    state.pieces.iter().filter(|p| p.owner == me).any(|p| {
        !legal_targets(catalog, state, p.id).is_empty()
            || Facing::ALL
                .iter()
                .any(|f| check_rotation(catalog, state, p, *f).is_ok())
    })
}

/// 手番のプレイヤーの、合法な行動の一覧。どの行動も、`apply` に受理される。
/// - 配置フェーズ: 城の配置。
/// - 選択待ち: 引いた 4 枚の選択（`Pick`）。
/// - 行動待ち: 配置・移動（捕食して成れるときは、成る・成らないの両方）・回転。
///   どれも行えないときは、`Pass` だけ。
/// - ドロー待ちと終局後: 空（ドローは `turn::draw_cards` で行う）。
pub fn legal_actions(catalog: &Catalog, state: &GameState) -> Vec<Action> {
    match (state.phase, &state.stage) {
        (Phase::Finished, _) => return Vec::new(),
        (Phase::Battle, Stage::Draw) => return Vec::new(),
        (Phase::Battle, Stage::Pick(cards)) => {
            return (0..cards.len())
                .map(|index| Action::Pick { index })
                .collect();
        }
        _ => {}
    }
    let me = state.current;
    let mut actions = Vec::new();
    if state.phase == Phase::Placement {
        let castle = catalog.castle();
        actions.extend(
            placement_candidates(catalog, state, castle).map(|(anchor, facing)| Action::Place {
                kind: castle,
                anchor,
                facing,
            }),
        );
        return actions;
    }
    // 対戦フェーズの、行動待ち
    for (kind, count) in &state.players[usize::from(me.0)].hand {
        if *count == 0 || is_castle(catalog, *kind) {
            continue;
        }
        actions.extend(
            placement_candidates(catalog, state, *kind).map(|(anchor, facing)| Action::Place {
                kind: *kind,
                anchor,
                facing,
            }),
        );
    }
    for piece in state.pieces.iter().filter(|p| p.owner == me) {
        let can_promote = catalog.promotion(piece.kind).is_some();
        for to in legal_targets(catalog, state, piece.id) {
            actions.push(Action::Move {
                piece: piece.id,
                to,
                promote: false,
            });
            if can_promote {
                let promote = Action::Move {
                    piece: piece.id,
                    to,
                    promote: true,
                };
                // 成れるのは、捕食が起きて、成り先の範囲が空いているときだけ。成れないなら、重複なので入れない
                let promotes = apply(catalog, state, &promote).is_ok_and(|(_, events)| {
                    events.iter().any(|e| matches!(e, Event::Promoted { .. }))
                });
                if promotes {
                    actions.push(promote);
                }
            }
        }
        for facing in Facing::ALL {
            if check_rotation(catalog, state, piece, facing).is_ok() {
                actions.push(Action::Rotate {
                    piece: piece.id,
                    facing,
                });
            }
        }
    }
    if actions.is_empty() {
        actions.push(Action::Pass);
    }
    actions
}

/// 手番のプレイヤーが `kind` を置ける（アンカー, 向き）の一覧。
pub fn legal_placements(catalog: &Catalog, state: &GameState, kind: KindId) -> Vec<(Pos, Facing)> {
    placement_candidates(catalog, state, kind).collect()
}

/// 置ける（アンカー, 向き）を、1 つずつ返す。1 つ見つければよいときは、最初の 1 つで止められる。
fn placement_candidates<'a>(
    catalog: &'a Catalog,
    state: &'a GameState,
    kind: KindId,
) -> impl Iterator<Item = (Pos, Facing)> + 'a {
    let size = state.board_size as i32;
    (0..size)
        .flat_map(move |y| (0..size).map(move |x| Pos::new(x, y)))
        .flat_map(|anchor| Facing::ALL.into_iter().map(move |facing| (anchor, facing)))
        .filter(move |(anchor, facing)| {
            check_placement(catalog, state, kind, *anchor, *facing).is_ok()
        })
}

/// 盤上の駒 `id` が、いまの向きで動ける移動先のアンカーの一覧（R-50〜R-53）。
/// 持ち主や手番は見ない。移動先に他の駒がいてもよい（攻撃になる）。
pub fn legal_targets(catalog: &Catalog, state: &GameState, id: PieceId) -> Vec<Pos> {
    let Some(piece) = state.piece(id) else {
        return Vec::new();
    };
    let Some(def) = catalog.get(piece.kind) else {
        return Vec::new();
    };
    let rect = def.footprint(piece.anchor, piece.facing);
    let board = state.board_size;

    if let Some((dx, dy)) = def.charge_delta(piece.facing) {
        // R-53: 前方へ 1 マスずつ進め、最初に他の駒と重なる位置か、盤の端の手前で止まる。
        let others = other_rects(catalog, state, id);
        let mut stop = None;
        // 盤の大きさより先へは進めないので、その手数で打ち切る
        for step in 1..=state.board_size as i32 {
            let moved = rect.translated(dx * step, dy * step);
            if !moved.is_inside(board) {
                break;
            }
            stop = Some(step);
            if others.iter().any(|o| o.overlaps(&moved)) {
                break;
            }
        }
        return stop
            .map(|step| {
                vec![Pos::new(
                    piece.anchor.x + dx * step,
                    piece.anchor.y + dy * step,
                )]
            })
            .unwrap_or_default();
    }

    def.displacements(piece.facing)
        .into_iter()
        .filter(|(dx, dy)| rect.translated(*dx, *dy).is_inside(board))
        .map(|(dx, dy)| Pos::new(piece.anchor.x + dx, piece.anchor.y + dy))
        .collect()
}

/// R-54: `piece` が、アンカーを `to` へ動かしたときに、重なった駒のそれぞれに与えるダメージ。
/// ダメージは、（重なったマス数）×（攻撃力）。駒の番号の順に返す。
/// 実際の移動（`apply`）と、危険な駒の判定（`threats`）が、同じ計算を使う。
pub fn strike_damage(
    catalog: &Catalog,
    state: &GameState,
    piece: &Piece,
    to: Pos,
) -> Vec<(PieceId, u32)> {
    strike_damage_in(catalog, &rects(catalog, state), piece, to)
}

/// `strike_damage` の、全駒の占有範囲（`rects`）を、呼び出し側が持っている版。
/// 何度も呼ぶとき（危険な駒の判定）に、占有範囲の計算を使い回すためのもの。
pub(crate) fn strike_damage_in(
    catalog: &Catalog,
    all: &[(PieceId, Rect)],
    piece: &Piece,
    to: Pos,
) -> Vec<(PieceId, u32)> {
    let Some(def) = catalog.get(piece.kind) else {
        return Vec::new();
    };
    let target = def
        .footprint(piece.anchor, piece.facing)
        .translated(to.x - piece.anchor.x, to.y - piece.anchor.y);
    let mut victims: Vec<(PieceId, u32)> = all
        .iter()
        .filter(|(other, _)| *other != piece.id)
        .filter_map(|(other, rect)| {
            let area = target.overlap_area(rect);
            (area > 0).then(|| (*other, area as u32 * def.attack))
        })
        .collect();
    victims.sort_by_key(|(other, _)| other.0);
    victims
}

fn is_castle(catalog: &Catalog, kind: KindId) -> bool {
    catalog.get(kind).is_some_and(PieceDef::is_castle)
}

/// 全駒の占有範囲（駒の番号つき）。
pub(crate) fn rects(catalog: &Catalog, state: &GameState) -> Vec<(PieceId, Rect)> {
    state
        .pieces
        .iter()
        .filter_map(|p| state.footprint(catalog, p).map(|r| (p.id, r)))
        .collect()
}

fn other_rects(catalog: &Catalog, state: &GameState, except: PieceId) -> Vec<Rect> {
    rects(catalog, state)
        .into_iter()
        .filter(|(id, _)| *id != except)
        .map(|(_, r)| r)
        .collect()
}

/// R-12・R-13・R-40〜R-42: 手番のプレイヤーが、その位置・向きで駒を置けるか。
fn check_placement(
    catalog: &Catalog,
    state: &GameState,
    kind: KindId,
    anchor: Pos,
    facing: Facing,
) -> Result<(), ActionError> {
    let def = catalog.get(kind).ok_or(ActionError::UnknownKind)?;
    match (state.phase, def.is_castle()) {
        (Phase::Placement, true) | (Phase::Battle, false) => {}
        _ => return Err(ActionError::WrongPhase),
    }
    let in_hand = state.players[usize::from(state.current.0)]
        .hand
        .get(&kind)
        .copied()
        .unwrap_or(0);
    if in_hand == 0 {
        return Err(ActionError::NotInHand);
    }
    if def.is_castle() && facing != Facing::North {
        return Err(ActionError::CastleMustFaceNorth);
    }
    let rect = def.footprint(anchor, facing);
    if !rect.is_inside(state.board_size) {
        return Err(ActionError::OutsideBoard);
    }
    let all = rects(catalog, state);
    if all.iter().any(|(_, r)| r.overlaps(&rect)) {
        return Err(ActionError::Overlaps);
    }
    if def.is_castle() {
        let too_close = state
            .pieces
            .iter()
            .filter(|p| p.owner != state.current && is_castle(catalog, p.kind))
            .filter_map(|p| state.footprint(catalog, p))
            .any(|castle| castle.gap(&rect) < CASTLE_MIN_GAP);
        if too_close {
            return Err(ActionError::TooCloseToCastle);
        }
    } else {
        let adjacent = state
            .pieces
            .iter()
            .filter(|p| p.owner == state.current)
            .filter_map(|p| state.footprint(catalog, p))
            .any(|own| own.gap(&rect) == 0);
        if !adjacent {
            return Err(ActionError::NotAdjacentToOwn);
        }
    }
    Ok(())
}

fn place(
    catalog: &Catalog,
    state: &mut GameState,
    kind: KindId,
    anchor: Pos,
    facing: Facing,
) -> Result<Vec<Event>, ActionError> {
    check_placement(catalog, state, kind, anchor, facing)?;
    let def = catalog.get(kind).ok_or(ActionError::UnknownKind)?;
    let owner = state.current;
    let hand = &mut state.players[usize::from(owner.0)].hand;
    if let Some(count) = hand.get_mut(&kind) {
        *count -= 1;
        if *count == 0 {
            hand.remove(&kind);
        }
    }
    let id = state.issue_piece_id();
    state.pieces.push(Piece {
        id,
        kind,
        owner,
        anchor,
        facing,
        hp: def.hp,
    });
    Ok(vec![Event::Placed {
        piece: id,
        kind,
        owner,
        anchor,
        facing,
    }])
}

/// 手番のプレイヤーの駒を探す。
fn own_piece(state: &GameState, id: PieceId) -> Result<&Piece, ActionError> {
    let piece = state.piece(id).ok_or(ActionError::UnknownPiece)?;
    if piece.owner != state.current {
        return Err(ActionError::NotOwner);
    }
    Ok(piece)
}

fn mov(
    catalog: &Catalog,
    state: &mut GameState,
    id: PieceId,
    to: Pos,
    promote: bool,
) -> Result<Vec<Event>, ActionError> {
    if state.phase != Phase::Battle {
        return Err(ActionError::WrongPhase);
    }
    let piece = own_piece(state, id)?.clone();
    if !legal_targets(catalog, state, id).contains(&to) {
        return Err(ActionError::IllegalTarget);
    }
    // R-54: 重なった駒のそれぞれに、同時にダメージを与える。
    let victims = strike_damage(catalog, state, &piece, to);

    let mut events = Vec::new();
    let mut captured_any = false;
    let mut survivors = 0;
    for (victim_id, damage) in victims {
        let Some(index) = state.pieces.iter().position(|p| p.id == victim_id) else {
            continue;
        };
        let hp = state.pieces[index].hp.saturating_sub(damage);
        events.push(Event::Damaged {
            piece: victim_id,
            damage,
            hp,
        });
        if hp > 0 {
            state.pieces[index].hp = hp;
            survivors += 1;
            continue;
        }
        // R-55・R-58: 捕食された駒は盤から消え、攻撃した人の持ち駒になる。城は消えるだけ（R-59）。
        let victim = state.pieces.remove(index);
        let to_hand = !is_castle(catalog, victim.kind);
        if to_hand {
            *state.players[usize::from(piece.owner.0)]
                .hand
                .entry(victim.kind)
                .or_insert(0) += 1;
        }
        captured_any = true;
        events.push(Event::Captured {
            piece: victim_id,
            kind: victim.kind,
            taker: piece.owner,
            to_hand,
        });
    }

    // R-56: 移動先の駒がすべて消えたときだけ、移動する。
    if survivors == 0 {
        if let Some(p) = state.pieces.iter_mut().find(|p| p.id == id) {
            p.anchor = to;
        }
        events.push(Event::Moved {
            piece: id,
            from: piece.anchor,
            to,
        });
    }

    // R-60〜R-63: 捕食したときだけ、選んだ場合に、成り先の範囲が空いていれば成る。
    if captured_any && promote {
        promote_piece(catalog, state, id, &mut events);
    }
    Ok(events)
}

fn promote_piece(catalog: &Catalog, state: &mut GameState, id: PieceId, events: &mut Vec<Event>) {
    let Some(piece) = state.piece(id).cloned() else {
        return;
    };
    let Some(to_kind) = catalog.promotion(piece.kind) else {
        return;
    };
    let Some(to_def) = catalog.get(to_kind) else {
        return;
    };
    let rect = to_def.footprint(piece.anchor, piece.facing);
    let free = rect.is_inside(state.board_size)
        && other_rects(catalog, state, id)
            .iter()
            .all(|other| !other.overlaps(&rect));
    if !free {
        return;
    }
    if let Some(p) = state.pieces.iter_mut().find(|p| p.id == id) {
        p.kind = to_kind;
        p.hp = to_def.hp;
    }
    events.push(Event::Promoted {
        piece: id,
        from: piece.kind,
        to: to_kind,
    });
}

/// R-71: 向きを変えたあとの占有範囲が、盤の内側で、ほかの駒と重ならないか。
fn check_rotation(
    catalog: &Catalog,
    state: &GameState,
    piece: &Piece,
    facing: Facing,
) -> Result<(), ActionError> {
    let def = catalog.get(piece.kind).ok_or(ActionError::UnknownKind)?;
    if def.is_castle() {
        return Err(ActionError::CannotRotate);
    }
    if facing == piece.facing {
        return Err(ActionError::SameFacing);
    }
    let rect = def.footprint(piece.anchor, facing);
    if !rect.is_inside(state.board_size) {
        return Err(ActionError::OutsideBoard);
    }
    if other_rects(catalog, state, piece.id)
        .iter()
        .any(|other| other.overlaps(&rect))
    {
        return Err(ActionError::Overlaps);
    }
    Ok(())
}

fn rotate(
    catalog: &Catalog,
    state: &mut GameState,
    id: PieceId,
    facing: Facing,
) -> Result<Vec<Event>, ActionError> {
    if state.phase != Phase::Battle {
        return Err(ActionError::WrongPhase);
    }
    let piece = own_piece(state, id)?.clone();
    check_rotation(catalog, state, &piece, facing)?;
    if let Some(p) = state.pieces.iter_mut().find(|p| p.id == id) {
        p.facing = facing;
    }
    Ok(vec![Event::Rotated { piece: id, facing }])
}

/// R-06・R-20・R-21: 引いた 4 枚から 1 枚を持ち駒に加え、残りは捨てる。選び直しはできない。
fn pick(state: &mut GameState, index: usize) -> Result<Vec<Event>, ActionError> {
    if state.phase != Phase::Battle {
        return Err(ActionError::WrongPhase);
    }
    let Stage::Pick(cards) = &state.stage else {
        return Err(ActionError::WrongStage);
    };
    let kind = *cards.get(index).ok_or(ActionError::InvalidPick)?;
    let player = state.current;
    *state.players[usize::from(player.0)]
        .hand
        .entry(kind)
        .or_insert(0) += 1;
    state.stage = Stage::Act;
    Ok(vec![Event::Picked {
        player,
        kind,
        index,
    }])
}

fn pass(catalog: &Catalog, state: &mut GameState) -> Result<Vec<Event>, ActionError> {
    if state.phase != Phase::Battle {
        return Err(ActionError::WrongPhase);
    }
    if has_legal_action(catalog, state) {
        return Err(ActionError::PassNotAllowed);
    }
    Ok(vec![Event::Passed {
        player: state.current,
    }])
}

#[cfg(test)]
mod tests;
