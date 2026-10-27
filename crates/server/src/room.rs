//! ルーム: 待機・対局の進行・時間切れ・切断の処理。仕様は docs/specs/online.md の 3〜5 章。
//!
//! 通信にも時計にも依存しない。時刻（`Millis`）と乱数は、呼び出し側が渡し、送るべきメッセージは
//! `Outbound` として返す。

use std::sync::Arc;
use std::time::{Duration, Instant};

use ai::{Level, choose_action, choose_pick};
use protocol::{
    NoticeKind, RejectCode, RoomRules, RoomStatus, RoomView, SeatKind, SeatView, ServerMessage,
    StateView,
};
use rules::action::{Action, ActionError, Event, apply};
use rules::finish::resign;
use rules::pieces::Catalog;
use rules::rng::{Rng, SplitMix64};
use rules::state::{GameState, Phase, PlayerId, Stage};
use rules::turn::{draw_cards, time_out};

use crate::Millis;
use crate::outbound::Outbound;

/// 人間の持ち時間（R-30）。
pub const HUMAN_TURN_MS: Millis = 60_000;
/// AI の持ち時間（R-30）。
pub const AI_TURN_MS: Millis = 20_000;
/// AI が考える時間の上限。持ち時間より短くして、通信と演出の余裕を残す（online.md の 7 章）。
pub const AI_THINK_MS: u64 = 15_000;
/// 切断から、AI が代行するまで（N-25）。
pub const TAKEOVER_AFTER_MS: Millis = 30_000;
/// 全員が切断してから、ルームを閉じるまで。
pub const ALL_DISCONNECTED_MS: Millis = 60_000;
/// 待機中のルームを、破棄するまで（N-27）。
pub const WAITING_EXPIRY_MS: Millis = 600_000;

/// 操作を受け付けられない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomError {
    RoomFull,
    NotWaiting,
    NotPlaying,
    NotInRoom,
    NotYourTurn,
    StaleTurn,
    Illegal(ActionError),
}

impl RoomError {
    /// クライアントへの返事（`rejected`）。
    pub fn to_message(self) -> ServerMessage {
        match self {
            Self::Illegal(reason) => ServerMessage::illegal_action(reason),
            Self::RoomFull => ServerMessage::rejected(RejectCode::RoomFull, "ルームが満員です"),
            Self::NotWaiting => ServerMessage::rejected(
                RejectCode::NotWaiting,
                "対戦がすでに始まっている（または終わった）ルームです",
            ),
            Self::NotPlaying => {
                ServerMessage::rejected(RejectCode::NotPlaying, "対戦中ではありません")
            }
            Self::NotInRoom => {
                ServerMessage::rejected(RejectCode::NotInRoom, "ルームに入っていません")
            }
            Self::NotYourTurn => {
                ServerMessage::rejected(RejectCode::NotYourTurn, "あなたの手番ではありません")
            }
            Self::StaleTurn => ServerMessage::rejected(
                RejectCode::StaleTurn,
                "手番番号が古いか、すでに送信済みです",
            ),
        }
    }
}

#[derive(Debug, Clone)]
struct Human {
    player_id: String,
    name: String,
    connected: bool,
    disconnected_at: Option<Millis>,
    /// 切断が続いて、AI が代行している。
    takeover: bool,
}

#[derive(Debug, Clone)]
enum Occupant {
    Human(Human),
    Ai { name: String },
}

/// 対局の履歴の 1 件。シードと合わせて、対局の再現と記録に使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// 行動の手番番号（`state.turn`）。
    pub turn: u32,
    pub seat: PlayerId,
    /// 行動。ドローと、時間切れの強制終了は、`None`。
    pub action: Option<Action>,
    pub events: Vec<Event>,
    pub timed_out: bool,
}

/// 結果を反映するときの、行動の情報。
struct Step {
    /// 行動の手番番号（`state.turn`）。
    turn: u32,
    seat: PlayerId,
    /// 行動。ドローと、時間切れの強制終了、投了は、`None`。
    action: Option<Action>,
    timed_out: bool,
    /// 次の手番を始めるか（選択と、手番でない人の投了は、始めない）。
    new_turn: bool,
}

struct Game {
    state: GameState,
    /// ドローと時間切れの乱数。
    draw_rng: SplitMix64,
    /// AI の乱数。ドローと分けて、AI の考え方が、ドローの結果に影響しないようにする。
    ai_rng: SplitMix64,
    seed: u64,
    turn_deadline: Millis,
    history: Vec<HistoryEntry>,
}

/// AI の手番の計画。ルームの状態の写しなので、ルームの外で、時間をかけて考えられる。
pub struct AiPlan {
    catalog: Arc<Catalog>,
    state: GameState,
    level: Level,
    rng: SplitMix64,
}

/// AI が考えた結果（選択と行動）。`Room::apply_ai` で反映する。
#[derive(Debug, Clone)]
pub struct AiDecision {
    seat: PlayerId,
    turn: u32,
    /// 考え始めたときの、手番の段階（反映するとき、状態が変わっていないかの確認に使う）。
    stage: Stage,
    pick: Option<usize>,
    action: Option<Action>,
    rng: SplitMix64,
}

impl AiPlan {
    /// 考えられなかったとき（考える処理が異常終了した、など）の結果。行動なしなので、反映すると、
    /// 手番が強制的に終わる（時間切れと同じ扱い）。同じ失敗を、繰り返し続けない。
    pub fn abstain(&self) -> AiDecision {
        AiDecision {
            seat: self.state.current,
            turn: self.state.turn,
            stage: self.state.stage.clone(),
            pick: None,
            action: None,
            rng: self.rng.clone(),
        }
    }

    /// 締め切りまで考えて、選択と行動を決める。ルームには触れない。
    pub fn think(mut self, deadline: Instant) -> AiDecision {
        let seat = self.state.current;
        let turn = self.state.turn;
        let stage = self.state.stage.clone();
        let mut pick = None;
        if let Stage::Pick(_) = self.state.stage {
            pick = choose_pick(&self.catalog, &self.state, self.level, &mut self.rng);
            let Some(index) = pick else {
                return AiDecision {
                    seat,
                    turn,
                    stage,
                    pick,
                    action: None,
                    rng: self.rng,
                };
            };
            match apply(&self.catalog, &self.state, &Action::Pick { index }) {
                Ok((next, _)) => self.state = next,
                Err(_) => {
                    return AiDecision {
                        seat,
                        turn,
                        stage,
                        pick: None,
                        action: None,
                        rng: self.rng,
                    };
                }
            }
        }
        let action = choose_action(
            &self.catalog,
            &self.state,
            self.level,
            deadline,
            &mut self.rng,
        );
        AiDecision {
            seat,
            turn,
            stage,
            pick,
            action,
            rng: self.rng,
        }
    }
}

pub struct Room {
    id: String,
    rules: RoomRules,
    status: RoomStatus,
    /// 待機中は、入室した人間だけ（入室順）。対戦中は、全席（席順）。
    seats: Vec<Occupant>,
    game: Option<Game>,
    created_at: Millis,
    all_disconnected_since: Option<Millis>,
    catalog: Arc<Catalog>,
}

/// 人間と AI を、なるべく交互に並べる席順（online.md の 4.3）。人間の並びは、ランダム。
/// 多いほうの種類を先頭にして交互に並べ、少ないほうを使い切ったら、残りは続ける。
fn arrange(
    mut humans: Vec<Human>,
    ai_count: usize,
    ai_names: bool,
    rng: &mut impl Rng,
) -> Vec<Occupant> {
    // フィッシャー–イェーツのシャッフル
    for i in (1..humans.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        humans.swap(i, j);
    }
    let human_majority = humans.len() >= ai_count;
    let total = humans.len() + ai_count;
    let mut humans = humans.into_iter();
    let mut ai_left = ai_count;
    let mut ai_number = 0;
    let mut seats = Vec::with_capacity(total);
    let mut humans_left = total - ai_count;
    for i in 0..total {
        // 偶数番目は、多いほうの種類。奇数番目は、少ないほうの種類（なければ、残りのほう）
        let want_human = if i % 2 == 0 {
            human_majority
        } else {
            !human_majority
        };
        let take_human = if want_human {
            humans_left > 0
        } else {
            ai_left == 0
        };
        if take_human {
            seats.push(Occupant::Human(
                humans.next().expect("人間が残っているはず"),
            ));
            humans_left -= 1;
        } else {
            ai_number += 1;
            ai_left -= 1;
            let name = if ai_names {
                format!("AI {ai_number}")
            } else {
                String::from("AI")
            };
            seats.push(Occupant::Ai { name });
        }
    }
    seats
}

impl Room {
    pub fn new(
        id: impl Into<String>,
        rules: RoomRules,
        catalog: Arc<Catalog>,
        now: Millis,
    ) -> Self {
        Self {
            id: id.into(),
            rules,
            status: RoomStatus::Waiting,
            seats: Vec::new(),
            game: None,
            created_at: now,
            all_disconnected_since: None,
            catalog,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn rules(&self) -> RoomRules {
        self.rules
    }

    pub fn status(&self) -> RoomStatus {
        self.status
    }

    /// 待機中で、空き席がある。
    pub fn has_free_seat(&self) -> bool {
        self.status == RoomStatus::Waiting && self.seats.len() < usize::from(self.rules.players)
    }

    pub fn created_at(&self) -> Millis {
        self.created_at
    }

    /// 対局の状態（対戦が始まってから）。
    pub fn state(&self) -> Option<&GameState> {
        self.game.as_ref().map(|g| &g.state)
    }

    /// 対局のシード。
    pub fn seed(&self) -> Option<u64> {
        self.game.as_ref().map(|g| g.seed)
    }

    /// 手番が、人間の席のときの、そのプレイヤー ID（AI の席・代行中の席は `None`）。
    pub fn current_human_id(&self) -> Option<&str> {
        let game = self.game.as_ref()?;
        match &self.seats[usize::from(game.state.current.0)] {
            Occupant::Human(h) if !h.takeover => Some(h.player_id.as_str()),
            _ => None,
        }
    }

    pub fn history(&self) -> &[HistoryEntry] {
        self.game.as_ref().map_or(&[], |g| g.history.as_slice())
    }

    /// 入室している人間のプレイヤー ID。
    pub fn human_ids(&self) -> Vec<&str> {
        self.seats
            .iter()
            .filter_map(|s| match s {
                Occupant::Human(h) => Some(h.player_id.as_str()),
                Occupant::Ai { .. } => None,
            })
            .collect()
    }

    pub fn contains(&self, player_id: &str) -> bool {
        self.seat_of(player_id).is_some()
    }

    /// 席の番号（対戦中は席順。待機中は入室順）。
    pub fn seat_of(&self, player_id: &str) -> Option<usize> {
        self.seats.iter().position(|s| match s {
            Occupant::Human(h) => h.player_id == player_id,
            Occupant::Ai { .. } => false,
        })
    }

    pub fn view(&self) -> RoomView {
        RoomView {
            room_id: self.id.clone(),
            status: self.status,
            rules: self.rules,
            seats: self
                .seats
                .iter()
                .enumerate()
                .map(|(i, s)| match s {
                    Occupant::Human(h) => SeatView {
                        seat: PlayerId(i as u8),
                        kind: SeatKind::Human,
                        name: h.name.clone(),
                        ai_level: None,
                        connected: h.connected,
                    },
                    Occupant::Ai { name } => SeatView {
                        seat: PlayerId(i as u8),
                        kind: SeatKind::Ai,
                        name: name.clone(),
                        ai_level: Some(self.rules.ai_level),
                        connected: true,
                    },
                })
                .collect(),
        }
    }

    /// 接続中の人間全員に送る。
    fn broadcast(&self, message: ServerMessage) -> Vec<Outbound> {
        self.seats
            .iter()
            .filter_map(|s| match s {
                Occupant::Human(h) if h.connected => {
                    Some(Outbound::new(h.player_id.clone(), message.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn notice(&self, kind: NoticeKind, message: impl Into<String>) -> Vec<Outbound> {
        self.broadcast(ServerMessage::Notice {
            kind,
            message: message.into(),
        })
    }

    fn name_of(&self, seat: usize) -> String {
        match &self.seats[seat] {
            Occupant::Human(h) => h.name.clone(),
            Occupant::Ai { name } => name.clone(),
        }
    }

    // ---- 待機 ----

    /// 入室する。満員になったら、自動で対戦を始める。待機中の部屋に、すでにいるときは、接続し直しとして扱う。
    pub fn join(
        &mut self,
        player_id: &str,
        name: &str,
        now: Millis,
        rng: &mut impl Rng,
    ) -> Result<Vec<Outbound>, RoomError> {
        if self.status != RoomStatus::Waiting {
            return Err(RoomError::NotWaiting);
        }
        if let Some(seat) = self.seat_of(player_id) {
            if let Occupant::Human(h) = &mut self.seats[seat] {
                h.connected = true;
                h.disconnected_at = None;
            }
            return Ok(self.broadcast(ServerMessage::RoomState { room: self.view() }));
        }
        if !self.has_free_seat() {
            return Err(RoomError::RoomFull);
        }
        self.seats.push(Occupant::Human(Human {
            player_id: player_id.to_string(),
            name: name.to_string(),
            connected: true,
            disconnected_at: None,
            takeover: false,
        }));
        let mut out = self.notice(NoticeKind::PlayerJoined, format!("{name} が入室しました"));
        out.extend(self.broadcast(ServerMessage::RoomState { room: self.view() }));
        if self.seats.len() == usize::from(self.rules.players) {
            out.extend(self.start_game(now, rng));
        }
        Ok(out)
    }

    /// Go ボタン。いる人間で、対戦を始める。空き席は AI で埋める。
    pub fn start(
        &mut self,
        player_id: &str,
        now: Millis,
        rng: &mut impl Rng,
    ) -> Result<Vec<Outbound>, RoomError> {
        if self.status != RoomStatus::Waiting {
            return Err(RoomError::NotWaiting);
        }
        if self.seat_of(player_id).is_none() {
            return Err(RoomError::NotInRoom);
        }
        Ok(self.start_game(now, rng))
    }

    fn start_game(&mut self, now: Millis, rng: &mut impl Rng) -> Vec<Outbound> {
        let humans: Vec<Human> = std::mem::take(&mut self.seats)
            .into_iter()
            .filter_map(|s| match s {
                Occupant::Human(h) => Some(h),
                Occupant::Ai { .. } => None,
            })
            .collect();
        let total = usize::from(self.rules.players);
        let ai_count = total.saturating_sub(humans.len());
        self.seats = arrange(humans, ai_count, true, rng);

        let seed = rng.next_u64();
        let mut draw_rng = SplitMix64(seed);
        let first = PlayerId(draw_rng.below(total as u64) as u8);
        let state = GameState::new(&self.catalog, total, first)
            .expect("人数と最初の手番は、検証済みのルールから決まる");
        self.game = Some(Game {
            state,
            draw_rng,
            ai_rng: SplitMix64(seed ^ 0xA1A1_A1A1_A1A1_A1A1),
            seed,
            turn_deadline: now,
            history: Vec::new(),
        });
        self.status = RoomStatus::Playing;

        let room = self.view();
        let view = StateView::from_state(&self.game.as_ref().expect("作ったばかり").state);
        let mut out: Vec<Outbound> = self
            .seats
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                Occupant::Human(h) => Some(Outbound::new(
                    h.player_id.clone(),
                    ServerMessage::GameStarted {
                        room: room.clone(),
                        you: PlayerId(i as u8),
                        state: view.clone(),
                    },
                )),
                Occupant::Ai { .. } => None,
            })
            .collect();
        self.begin_turn(now, &mut out);
        out
    }

    // ---- 対局の進行 ----

    /// その席を、AI が動かすか（AI の席、または、切断して AI が代行している人間の席）。
    fn controlled_by_ai(&self, seat: usize) -> bool {
        match &self.seats[seat] {
            Occupant::Ai { .. } => true,
            Occupant::Human(h) => h.takeover,
        }
    }

    /// 手番の持ち時間（R-30）。
    fn turn_limit(&self, seat: usize) -> Millis {
        if self.controlled_by_ai(seat) {
            AI_TURN_MS
        } else {
            HUMAN_TURN_MS
        }
    }

    /// 手番を始める: 対戦フェーズならドローし、持ち時間を決めて、手番の開始を知らせる。
    fn begin_turn(&mut self, now: Millis, out: &mut Vec<Outbound>) {
        let catalog = Arc::clone(&self.catalog);
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if game.state.phase == Phase::Finished {
            return;
        }
        let mut drawn = None;
        if game.state.phase == Phase::Battle && game.state.stage == Stage::Draw {
            let (next, events) = draw_cards(&catalog, &game.state, &mut game.draw_rng)
                .expect("ドロー待ちの段階で、ドローできる");
            game.history.push(HistoryEntry {
                turn: game.state.turn,
                seat: game.state.current,
                action: None,
                events: events.clone(),
                timed_out: false,
            });
            game.state = next;
            drawn = Some((events, StateView::from_state(&game.state)));
        }
        let seat = usize::from(game.state.current.0);
        let turn = game.state.turn;
        let player = game.state.current;
        let limit = self.turn_limit(seat);
        if let Some(game) = self.game.as_mut() {
            game.turn_deadline = now + limit;
        }
        if let Some((events, state)) = drawn {
            out.extend(self.broadcast(ServerMessage::Applied {
                turn,
                events,
                state,
            }));
        }
        out.extend(self.broadcast(ServerMessage::TurnStarted {
            player,
            turn,
            remaining_ms: u32::try_from(limit).unwrap_or(u32::MAX),
        }));
    }

    /// 行動の結果を、状態と履歴に反映し、全員に知らせる。終局したら、結果を送る。
    /// `step.new_turn` が `false` のときは、次の手番を始めない（選択、手番でない人の投了）。
    fn publish(
        &mut self,
        now: Millis,
        step: Step,
        result: (GameState, Vec<Event>),
        out: &mut Vec<Outbound>,
    ) {
        let (state, events) = result;
        let Some(game) = self.game.as_mut() else {
            return;
        };
        game.history.push(HistoryEntry {
            turn: step.turn,
            seat: step.seat,
            action: step.action,
            events: events.clone(),
            timed_out: step.timed_out,
        });
        game.state = state;
        let view = StateView::from_state(&game.state);
        let finished = game.state.phase == Phase::Finished;
        let outcome = game.state.outcome.clone();
        out.extend(self.broadcast(ServerMessage::Applied {
            turn: step.turn,
            events,
            state: view.clone(),
        }));
        if finished {
            self.status = RoomStatus::Finished;
            if let Some(outcome) = outcome {
                out.extend(self.broadcast(ServerMessage::GameOver {
                    outcome,
                    state: view,
                }));
            }
        } else if step.new_turn {
            self.begin_turn(now, out);
        }
    }

    /// 人間の、選択・行動の確定。
    pub fn act(
        &mut self,
        player_id: &str,
        turn: u32,
        action: Action,
        now: Millis,
    ) -> Result<Vec<Outbound>, RoomError> {
        if self.status != RoomStatus::Playing {
            return Err(RoomError::NotPlaying);
        }
        let seat = self.seat_of(player_id).ok_or(RoomError::NotInRoom)?;
        let game = self.game.as_ref().ok_or(RoomError::NotPlaying)?;
        if usize::from(game.state.current.0) != seat {
            return Err(RoomError::NotYourTurn);
        }
        if turn != game.state.turn {
            return Err(RoomError::StaleTurn);
        }
        let result = apply(&self.catalog, &game.state, &action).map_err(RoomError::Illegal)?;
        let mut out = Vec::new();
        let new_turn = !matches!(action, Action::Pick { .. });
        let step = Step {
            turn,
            seat: PlayerId(seat as u8),
            action: Some(action),
            timed_out: false,
            new_turn,
        };
        self.publish(now, step, result, &mut out);
        Ok(out)
    }

    /// 投了。手番でなくても、いつでもできる。待機中は、退室になる。
    pub fn resign(&mut self, player_id: &str, now: Millis) -> Result<Vec<Outbound>, RoomError> {
        match self.status {
            RoomStatus::Waiting => Ok(self.leave(player_id, now)),
            RoomStatus::Finished => Err(RoomError::NotPlaying),
            RoomStatus::Playing => {
                let seat = self.seat_of(player_id).ok_or(RoomError::NotInRoom)?;
                self.resign_seat(seat, now)
            }
        }
    }

    fn resign_seat(&mut self, seat: usize, now: Millis) -> Result<Vec<Outbound>, RoomError> {
        let game = self.game.as_ref().ok_or(RoomError::NotPlaying)?;
        let player = PlayerId(seat as u8);
        let result = resign(&self.catalog, &game.state, player).map_err(RoomError::Illegal)?;
        let was_current = game.state.current == player;
        let turn = game.state.turn;
        let mut out = Vec::new();
        // 手番のプレイヤーの投了だけが、次の手番を始める。手番でない人の投了では、始め直さない
        let step = Step {
            turn,
            seat: player,
            action: None,
            timed_out: false,
            new_turn: was_current,
        };
        self.publish(now, step, result, &mut out);
        Ok(out)
    }

    // ---- AI ----

    /// 手番が、AI（または、AI の代行）の席か。
    pub fn needs_ai(&self) -> bool {
        self.status == RoomStatus::Playing
            && self
                .game
                .as_ref()
                .is_some_and(|g| self.controlled_by_ai(usize::from(g.state.current.0)))
    }

    /// AI の手番の計画を作る（状態の写し）。AI の席でなければ `None`。
    /// 考える（`AiPlan::think`）のは、ルームの外（ロビーの外）で行えるので、考えている間、
    /// ほかのルームや、このルームの操作を止めない。
    pub fn plan_ai(&self) -> Option<AiPlan> {
        if !self.needs_ai() {
            return None;
        }
        let game = self.game.as_ref()?;
        Some(AiPlan {
            catalog: Arc::clone(&self.catalog),
            state: game.state.clone(),
            level: Level::from_number(self.rules.ai_level).unwrap_or(Level::Two),
            rng: game.ai_rng.clone(),
        })
    }

    /// 考えた結果を、ルームに反映する。`now` は、考え終わった時点の時刻（持ち時間の数え始めに使う）。
    /// 考えている間に、状態が変わっていた（手番が進んだ、投了した、など）ときは、何もしない。
    pub fn apply_ai(&mut self, now: Millis, decision: AiDecision) -> Vec<Outbound> {
        let mut out = Vec::new();
        let unchanged = self.needs_ai()
            && self.game.as_ref().is_some_and(|g| {
                g.state.current == decision.seat
                    && g.state.turn == decision.turn
                    && g.state.stage == decision.stage
            });
        if !unchanged {
            return out;
        }
        if let Some(game) = self.game.as_mut() {
            game.ai_rng = decision.rng;
        }
        if let Some(index) = decision.pick {
            let action = Action::Pick { index };
            let Some(result) = self
                .game
                .as_ref()
                .and_then(|g| apply(&self.catalog, &g.state, &action).ok())
            else {
                return self.force_end_turn(now);
            };
            let step = Step {
                turn: decision.turn,
                seat: decision.seat,
                action: Some(action),
                timed_out: false,
                new_turn: false,
            };
            self.publish(now, step, result, &mut out);
        }
        // 行動が決まらなかった、または、受理されなかったときは、手番を強制的に終える
        let applied = decision.action.and_then(|action| {
            let game = self.game.as_ref()?;
            let result = apply(&self.catalog, &game.state, &action).ok()?;
            Some((action, result))
        });
        match applied {
            Some((action, result)) => {
                let step = Step {
                    turn: decision.turn,
                    seat: decision.seat,
                    action: Some(action),
                    timed_out: false,
                    new_turn: true,
                };
                self.publish(now, step, result, &mut out);
            }
            None => out.extend(self.force_end_turn(now)),
        }
        out
    }

    /// AI の手番を 1 つ進める（計画 → 考える → 反映）。AI の席でなければ、何もしない。
    /// 考える時間が長いので、実際のサーバーは、`plan_ai`・`AiPlan::think`・`apply_ai` を分けて呼ぶ。
    pub fn run_ai(&mut self, now: Millis) -> Vec<Outbound> {
        let Some(plan) = self.plan_ai() else {
            return Vec::new();
        };
        let decision = plan.think(Instant::now() + Duration::from_millis(AI_THINK_MS));
        self.apply_ai(now, decision)
    }

    // ---- 時間切れ・切断 ----

    /// 持ち時間を過ぎた手番を、強制的に終える（R-31）。城を置けないときは、投了扱い。
    fn force_end_turn(&mut self, now: Millis) -> Vec<Outbound> {
        let Some(game) = self.game.as_mut() else {
            return Vec::new();
        };
        let seat = game.state.current;
        let turn = game.state.turn;
        match time_out(&self.catalog, &game.state, &mut game.draw_rng) {
            Ok(result) => {
                let mut out = self.notice(
                    NoticeKind::TimedOut,
                    format!(
                        "{} の持ち時間が切れました",
                        self.name_of(usize::from(seat.0))
                    ),
                );
                let step = Step {
                    turn,
                    seat,
                    action: None,
                    timed_out: true,
                    new_turn: true,
                };
                self.publish(now, step, result, &mut out);
                out
            }
            // 手番のプレイヤーは、必ず生きているので、投了できる
            Err(_) => self
                .resign_seat(usize::from(seat.0), now)
                .unwrap_or_default(),
        }
    }

    /// 退室する。待機中は席を空ける（誰もいなくなったら、ルームを閉じる）。対戦中は、切断として扱う。
    pub fn leave(&mut self, player_id: &str, now: Millis) -> Vec<Outbound> {
        let Some(seat) = self.seat_of(player_id) else {
            return Vec::new();
        };
        match self.status {
            RoomStatus::Waiting => {
                let name = self.name_of(seat);
                self.seats.remove(seat);
                let mut out = Vec::new();
                if self.seats.is_empty() {
                    self.status = RoomStatus::Finished;
                } else {
                    out.extend(
                        self.notice(NoticeKind::PlayerLeft, format!("{name} が退室しました")),
                    );
                    out.extend(self.broadcast(ServerMessage::RoomState { room: self.view() }));
                }
                out
            }
            RoomStatus::Playing => self.disconnect(player_id, now),
            RoomStatus::Finished => Vec::new(),
        }
    }

    /// 接続が切れた。待機中は退室、対戦中は、切断の記録（30 秒で AI が代行）。
    pub fn disconnect(&mut self, player_id: &str, now: Millis) -> Vec<Outbound> {
        let Some(seat) = self.seat_of(player_id) else {
            return Vec::new();
        };
        match self.status {
            RoomStatus::Waiting => self.leave(player_id, now),
            RoomStatus::Finished => Vec::new(),
            RoomStatus::Playing => {
                let name = self.name_of(seat);
                if let Occupant::Human(h) = &mut self.seats[seat] {
                    if !h.connected {
                        return Vec::new();
                    }
                    h.connected = false;
                    h.disconnected_at = Some(now);
                }
                // 全員が切断したら、この時刻から、閉じるまでを数える
                let anyone_connected = self
                    .seats
                    .iter()
                    .any(|s| matches!(s, Occupant::Human(h) if h.connected));
                if !anyone_connected && self.all_disconnected_since.is_none() {
                    self.all_disconnected_since = Some(now);
                }
                let mut out = self.notice(
                    NoticeKind::PlayerDisconnected,
                    format!("{name} の接続が切れました"),
                );
                out.extend(self.broadcast(ServerMessage::RoomState { room: self.view() }));
                out
            }
        }
    }

    /// 同じプレイヤーが、接続し直した。対戦中なら、全状態を送り、本人の席に戻す。
    pub fn reconnect(&mut self, player_id: &str, now: Millis) -> Result<Vec<Outbound>, RoomError> {
        let seat = self.seat_of(player_id).ok_or(RoomError::NotInRoom)?;
        match self.status {
            RoomStatus::Finished => Err(RoomError::NotPlaying),
            RoomStatus::Waiting => {
                if let Occupant::Human(h) = &mut self.seats[seat] {
                    h.connected = true;
                    h.disconnected_at = None;
                }
                Ok(self.broadcast(ServerMessage::RoomState { room: self.view() }))
            }
            RoomStatus::Playing => {
                let name = self.name_of(seat);
                if let Occupant::Human(h) = &mut self.seats[seat] {
                    h.connected = true;
                    h.disconnected_at = None;
                    h.takeover = false;
                }
                self.all_disconnected_since = None;
                let game = self.game.as_ref().ok_or(RoomError::NotPlaying)?;
                let remaining = game.turn_deadline.saturating_sub(now);
                let mut out = vec![
                    Outbound::new(
                        player_id,
                        ServerMessage::State {
                            state: StateView::from_state(&game.state),
                        },
                    ),
                    Outbound::new(
                        player_id,
                        ServerMessage::TurnStarted {
                            player: game.state.current,
                            turn: game.state.turn,
                            remaining_ms: u32::try_from(remaining).unwrap_or(u32::MAX),
                        },
                    ),
                ];
                out.extend(self.notice(
                    NoticeKind::PlayerReconnected,
                    format!("{name} が戻りました"),
                ));
                out.extend(self.broadcast(ServerMessage::RoomState { room: self.view() }));
                Ok(out)
            }
        }
    }

    /// ルームを閉じる（待機の期限切れ、全員の切断）。
    fn close(&mut self, message: &str) -> Vec<Outbound> {
        let out = self.notice(NoticeKind::RoomClosed, message);
        self.status = RoomStatus::Finished;
        out
    }

    /// 時計が進んだときの処理: 待機の期限、AI の代行、全員の切断、持ち時間。
    pub fn tick(&mut self, now: Millis) -> Vec<Outbound> {
        match self.status {
            RoomStatus::Finished => Vec::new(),
            RoomStatus::Waiting => {
                if now >= self.created_at + WAITING_EXPIRY_MS {
                    self.close("開始されないまま、時間が過ぎたので、ルームを閉じました")
                } else {
                    Vec::new()
                }
            }
            RoomStatus::Playing => self.tick_playing(now),
        }
    }

    fn tick_playing(&mut self, now: Millis) -> Vec<Outbound> {
        let mut out = Vec::new();
        // 切断が続いた人間の席を、AI が代行する
        let mut takeovers = Vec::new();
        for (seat, occupant) in self.seats.iter_mut().enumerate() {
            if let Occupant::Human(h) = occupant
                && !h.connected
                && !h.takeover
                && h.disconnected_at
                    .is_some_and(|t| now >= t + TAKEOVER_AFTER_MS)
            {
                h.takeover = true;
                takeovers.push(seat);
            }
        }
        for seat in takeovers {
            let name = self.name_of(seat);
            out.extend(self.notice(
                NoticeKind::AiTakeover,
                format!("{name} の席を、AI が代行します"),
            ));
        }
        // 全員が切断したまま、一定時間が過ぎたら、ルームを閉じる
        let humans_connected = self
            .seats
            .iter()
            .any(|s| matches!(s, Occupant::Human(h) if h.connected));
        if humans_connected {
            self.all_disconnected_since = None;
        } else {
            let since = *self.all_disconnected_since.get_or_insert(now);
            if now >= since + ALL_DISCONNECTED_MS {
                out.extend(self.close("全員の接続が切れたので、ルームを閉じました"));
                return out;
            }
        }
        // 人間の持ち時間（AI の席と、代行中の席は、AI がすぐに指すので、見ない）
        let expired = self.game.as_ref().is_some_and(|g| {
            !self.controlled_by_ai(usize::from(g.state.current.0)) && now >= g.turn_deadline
        });
        if expired {
            out.extend(self.force_end_turn(now));
        }
        out
    }

    /// 次に `tick` を呼ぶべき時刻。なければ `None`。AI の手番は、`needs_ai` で見る。
    pub fn next_wakeup(&self) -> Option<Millis> {
        match self.status {
            RoomStatus::Finished => None,
            RoomStatus::Waiting => Some(self.created_at + WAITING_EXPIRY_MS),
            RoomStatus::Playing => {
                let mut times = Vec::new();
                for occupant in &self.seats {
                    if let Occupant::Human(h) = occupant
                        && !h.connected
                        && !h.takeover
                        && let Some(t) = h.disconnected_at
                    {
                        times.push(t + TAKEOVER_AFTER_MS);
                    }
                }
                if let Some(since) = self.all_disconnected_since {
                    times.push(since + ALL_DISCONNECTED_MS);
                }
                if let Some(game) = &self.game
                    && !self.controlled_by_ai(usize::from(game.state.current.0))
                {
                    times.push(game.turn_deadline);
                }
                times.into_iter().min()
            }
        }
    }
}

#[cfg(test)]
mod tests;
