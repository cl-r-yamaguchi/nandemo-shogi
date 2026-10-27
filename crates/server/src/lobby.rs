//! ロビー: マッチング（同じルールの、空きのある待機中のルームを探す。なければ作る）と、ルームの管理。
//! 仕様は docs/specs/online.md の 4 章。

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use protocol::{RejectCode, RoomRules, RoomStatus, ServerMessage};
use rules::action::Action;
use rules::pieces::Catalog;
use rules::rng::Rng;

use crate::Millis;
use crate::outbound::Outbound;
use crate::record::GameRecord;
use crate::room::{AiDecision, AiPlan, Room, RoomError};

/// ルームを探せなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyError {
    InvalidRules,
    /// すでに、進行中（または待機中）のルームに入っている。
    AlreadyInRoom,
    Room(RoomError),
}

impl LobbyError {
    /// クライアントへの返事（`rejected`）。
    pub fn to_message(self) -> ServerMessage {
        match self {
            Self::InvalidRules => {
                ServerMessage::rejected(RejectCode::InvalidRules, "ルールの値が範囲外です")
            }
            Self::AlreadyInRoom => ServerMessage::rejected(
                RejectCode::AlreadyInRoom,
                "すでにルームに入っています（対戦中のときは、投了するか、対戦が終わるまで待ってください）",
            ),
            Self::Room(error) => error.to_message(),
        }
    }
}

pub struct Lobby {
    catalog: Arc<Catalog>,
    /// ルーム ID の昇順（作った順）。
    rooms: BTreeMap<u64, Room>,
    /// プレイヤー ID → 入っているルームの番号。
    player_room: HashMap<String, u64>,
    next_id: u64,
    /// 終わったルームの記録の渡し先（保存）。
    record_sink: Option<RecordSink>,
}

/// 終わった対局の記録の渡し先。
pub type RecordSink = Box<dyn Fn(GameRecord) + Send>;

impl Lobby {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            rooms: BTreeMap::new(),
            player_room: HashMap::new(),
            next_id: 1,
            record_sink: None,
        }
    }

    /// 終わった対局の記録の渡し先を決める。
    pub fn set_record_sink(&mut self, sink: RecordSink) {
        self.record_sink = Some(sink);
    }

    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// プレイヤーが入っているルーム。
    pub fn room_of(&self, player_id: &str) -> Option<&Room> {
        self.player_room
            .get(player_id)
            .and_then(|id| self.rooms.get(id))
    }

    fn room_mut(&mut self, player_id: &str) -> Option<&mut Room> {
        let id = *self.player_room.get(player_id)?;
        self.rooms.get_mut(&id)
    }

    /// 全ルーム（作った順）。
    pub fn rooms(&self) -> impl Iterator<Item = &Room> {
        self.rooms.values()
    }

    /// ルールを決めて、ルームを探して入る。同じルールの、待機中で空きのある、最も古いルームに入れる。
    /// なければ、新しいルームを作る。
    pub fn find_room(
        &mut self,
        player_id: &str,
        name: &str,
        rules: RoomRules,
        now: Millis,
        rng: &mut impl Rng,
    ) -> Result<Vec<Outbound>, LobbyError> {
        if !rules.is_valid() {
            return Err(LobbyError::InvalidRules);
        }
        // 終わった（閉じた）ルームの対応は、片付ける。続いているルームには、入れ直せない
        if let Some(room) = self.room_of(player_id) {
            if room.status() == RoomStatus::Finished {
                self.player_room.remove(player_id);
            } else {
                return Err(LobbyError::AlreadyInRoom);
            }
        }
        let existing = self
            .rooms
            .iter()
            .find(|(_, r)| r.rules() == rules && r.has_free_seat())
            .map(|(id, _)| *id);
        let id = match existing {
            Some(id) => id,
            None => {
                let id = self.next_id;
                self.next_id += 1;
                let room = Room::new(format!("room-{id}"), rules, Arc::clone(&self.catalog), now);
                self.rooms.insert(id, room);
                id
            }
        };
        let room = self.rooms.get_mut(&id).expect("いま見つけた、または作った");
        let out = room
            .join(player_id, name, now, rng)
            .map_err(LobbyError::Room)?;
        self.player_room.insert(player_id.to_string(), id);
        Ok(out)
    }

    pub fn start(
        &mut self,
        player_id: &str,
        now: Millis,
        rng: &mut impl Rng,
    ) -> Result<Vec<Outbound>, RoomError> {
        self.room_mut(player_id)
            .ok_or(RoomError::NotInRoom)?
            .start(player_id, now, rng)
    }

    pub fn act(
        &mut self,
        player_id: &str,
        turn: u32,
        action: Action,
        now: Millis,
    ) -> Result<Vec<Outbound>, RoomError> {
        self.room_mut(player_id)
            .ok_or(RoomError::NotInRoom)?
            .act(player_id, turn, action, now)
    }

    /// 投了する。待機中は退室（対応も外す）、対戦中は脱落。
    pub fn resign(&mut self, player_id: &str, now: Millis) -> Result<Vec<Outbound>, RoomError> {
        let room = self.room_mut(player_id).ok_or(RoomError::NotInRoom)?;
        let waiting = room.status() == RoomStatus::Waiting;
        let out = room.resign(player_id, now)?;
        if waiting {
            self.player_room.remove(player_id);
        }
        Ok(out)
    }

    /// 退室する。待機中は席を空け、対戦中は切断として扱う。
    pub fn leave(&mut self, player_id: &str, now: Millis) -> Vec<Outbound> {
        let Some(room) = self.room_mut(player_id) else {
            return Vec::new();
        };
        let waiting = room.status() == RoomStatus::Waiting;
        let out = room.leave(player_id, now);
        if waiting {
            self.player_room.remove(player_id);
        }
        out
    }

    pub fn disconnect(&mut self, player_id: &str, now: Millis) -> Vec<Outbound> {
        let Some(room) = self.room_mut(player_id) else {
            return Vec::new();
        };
        let waiting = room.status() == RoomStatus::Waiting;
        let out = room.disconnect(player_id, now);
        if waiting {
            self.player_room.remove(player_id);
        }
        out
    }

    pub fn reconnect(&mut self, player_id: &str, now: Millis) -> Result<Vec<Outbound>, RoomError> {
        self.room_mut(player_id)
            .ok_or(RoomError::NotInRoom)?
            .reconnect(player_id, now)
    }

    /// 時計が進んだときの処理。すべてのルームの `tick` を呼び、終わったルームを片付ける。
    pub fn tick(&mut self, now: Millis) -> Vec<Outbound> {
        let mut out = Vec::new();
        for room in self.rooms.values_mut() {
            out.extend(room.tick(now));
        }
        self.sweep();
        out
    }

    /// 終わった（閉じた）ルームを取り除く。入っていた人の対応も外す。
    pub fn sweep(&mut self) {
        let finished: Vec<u64> = self
            .rooms
            .iter()
            .filter(|(_, r)| r.status() == RoomStatus::Finished)
            .map(|(id, _)| *id)
            .collect();
        for id in finished {
            if let (Some(room), Some(sink)) = (self.rooms.remove(&id), &self.record_sink)
                && let Some(record) = room.record()
            {
                sink(record);
            }
            self.player_room.retain(|_, room| *room != id);
        }
    }

    /// AI の手番を待っているルームの番号の一覧（作った順）。
    pub fn rooms_needing_ai(&self) -> Vec<u64> {
        self.rooms
            .iter()
            .filter(|(_, r)| r.needs_ai())
            .map(|(id, _)| *id)
            .collect()
    }

    /// 指定のルーム（`rooms_needing_ai` の番号）の、AI の手番の計画。計画は、状態の写しなので、
    /// ロビーを手放して、別のスレッドで考えられる（`AiPlan::think`）。
    pub fn plan_ai(&self, room: u64) -> Option<AiPlan> {
        self.rooms.get(&room).and_then(Room::plan_ai)
    }

    /// 考えた結果を反映する。`now` は、考え終わった時点の時刻。ルームの状態が変わっていたら、何もしない。
    pub fn apply_ai(&mut self, room: u64, now: Millis, decision: AiDecision) -> Vec<Outbound> {
        let out = self
            .rooms
            .get_mut(&room)
            .map(|r| r.apply_ai(now, decision))
            .unwrap_or_default();
        self.sweep();
        out
    }

    /// 指定のルーム（`rooms_needing_ai` の番号）で、AI の手番を 1 つ進める（考えるのを含む。テスト向け）。
    pub fn run_ai(&mut self, room: u64, now: Millis) -> Vec<Outbound> {
        let out = self
            .rooms
            .get_mut(&room)
            .map(|r| r.run_ai(now))
            .unwrap_or_default();
        self.sweep();
        out
    }

    /// 次に `tick` を呼ぶべき時刻（全ルームのうち、最も早いもの）。
    pub fn next_wakeup(&self) -> Option<Millis> {
        self.rooms.values().filter_map(Room::next_wakeup).min()
    }
}

#[cfg(test)]
mod tests;
