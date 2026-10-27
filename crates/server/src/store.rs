//! 対局の記録の保存（SQLite）。**DB に触れるのは、このモジュールだけ**。仕様は online.md の 8 章。
//!
//! 保存は、書き込み専用の履歴。対局中の状態は、メモリを正とする。
//! 保存は、専用のスレッドで行い、対局の進行を止めない。失敗しても、対局には影響させない。

use std::fmt;
use std::path::Path;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use protocol::{RoomRules, SeatKind};
use rules::finish::Outcome;
use rusqlite::{Connection, OptionalExtension, params};

use crate::record::{GameRecord, SeatRecord};
use crate::room::{HistoryEntry, Move};

/// このコードが扱う、スキーマの版（`PRAGMA user_version`）。
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS games (
    id INTEGER PRIMARY KEY,
    players INTEGER NOT NULL,
    ai_level INTEGER NOT NULL,
    -- u64 を欠けなく残すため、10 進の文字列で持つ
    seed TEXT NOT NULL,
    ended_at INTEGER NOT NULL,
    -- 'last_standing' | 'all_eliminated' | 'turn_limit' | 'aborted'（中断）
    end_reason TEXT NOT NULL,
    outcome TEXT
);
CREATE TABLE IF NOT EXISTS seats (
    game_id INTEGER NOT NULL REFERENCES games(id),
    seat INTEGER NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (game_id, seat)
);
CREATE TABLE IF NOT EXISTS moves (
    game_id INTEGER NOT NULL REFERENCES games(id),
    idx INTEGER NOT NULL,
    turn INTEGER NOT NULL,
    seat INTEGER NOT NULL,
    -- 'draw' | 'act' | 'time_out' | 'resign'
    kind TEXT NOT NULL,
    action TEXT,
    events TEXT NOT NULL,
    PRIMARY KEY (game_id, idx)
);
";

/// 保存・読み出しの失敗。
#[derive(Debug)]
pub struct StoreError(String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self(format!("データベースの失敗: {e}"))
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self(format!("記録の変換の失敗: {e}"))
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// ファイルを開く（なければ作る）。`":memory:"` なら、メモリ上のデータベース。
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(Path::new(path))?
        };
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError(format!(
                "新しい版のデータベース（版 {version}）です。このサーバーは、版 {SCHEMA_VERSION} まで扱えます"
            )));
        }
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        if version == 0 {
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        }
        Ok(Self { conn })
    }

    /// 記録を保存する（1 つの取引で。途中で失敗したら、何も残さない）。保存した記録の番号を返す。
    pub fn save(&mut self, record: &GameRecord) -> Result<i64, StoreError> {
        let tx = self.conn.transaction()?;
        let (reason, outcome) = match &record.outcome {
            Some(o) => (
                serde_json::to_value(o.reason)?
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string(),
                Some(serde_json::to_string(o)?),
            ),
            None => ("aborted".to_string(), None),
        };
        tx.execute(
            "INSERT INTO games (players, ai_level, seed, ended_at, end_reason, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.rules.players,
                record.rules.ai_level,
                record.seed.to_string(),
                now_unix_ms(),
                reason,
                outcome
            ],
        )?;
        let id = tx.last_insert_rowid();
        {
            let mut seat = tx
                .prepare("INSERT INTO seats (game_id, seat, kind, name) VALUES (?1, ?2, ?3, ?4)")?;
            for (i, s) in record.seats.iter().enumerate() {
                let kind = match s.kind {
                    SeatKind::Human => "human",
                    SeatKind::Ai => "ai",
                };
                seat.execute(params![id, i as i64, kind, s.name])?;
            }
            let mut mv = tx.prepare(
                "INSERT INTO moves (game_id, idx, turn, seat, kind, action, events)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (i, e) in record.history.iter().enumerate() {
                let (kind, action) = match &e.mv {
                    Move::Draw => ("draw", None),
                    Move::Act { action } => ("act", Some(serde_json::to_string(action)?)),
                    Move::TimeOut => ("time_out", None),
                    Move::Resign => ("resign", None),
                };
                mv.execute(params![
                    id,
                    i as i64,
                    e.turn,
                    e.seat.0,
                    kind,
                    action,
                    serde_json::to_string(&e.events)?
                ])?;
            }
        }
        tx.commit()?;
        Ok(id)
    }

    /// 記録を読み出す。
    pub fn load(&self, id: i64) -> Result<Option<GameRecord>, StoreError> {
        let head = self
            .conn
            .query_row(
                "SELECT players, ai_level, seed, outcome FROM games WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, u8>(0)?,
                        r.get::<_, u8>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((players, ai_level, seed, outcome)) = head else {
            return Ok(None);
        };
        let seed = seed
            .parse::<u64>()
            .map_err(|_| StoreError("シードが、数として読めません".into()))?;
        let outcome: Option<Outcome> = outcome.map(|o| serde_json::from_str(&o)).transpose()?;

        let mut stmt = self
            .conn
            .prepare("SELECT kind, name FROM seats WHERE game_id = ?1 ORDER BY seat")?;
        let seats = stmt
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (kind, name) = row?;
                let kind = match kind.as_str() {
                    "human" => SeatKind::Human,
                    "ai" => SeatKind::Ai,
                    other => return Err(StoreError(format!("不明な席の種類: {other}"))),
                };
                Ok(SeatRecord { kind, name })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT turn, seat, kind, action, events FROM moves WHERE game_id = ?1 ORDER BY idx",
        )?;
        let history = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u8>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .map(|row| {
                let (turn, seat, kind, action, events) = row?;
                let mv = match (kind.as_str(), action) {
                    ("draw", None) => Move::Draw,
                    ("act", Some(a)) => Move::Act {
                        action: serde_json::from_str(&a)?,
                    },
                    ("time_out", None) => Move::TimeOut,
                    ("resign", None) => Move::Resign,
                    (other, _) => return Err(StoreError(format!("不明な操作の種類: {other}"))),
                };
                Ok(HistoryEntry {
                    turn,
                    seat: rules::state::PlayerId(seat),
                    mv,
                    events: serde_json::from_str(&events)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;

        Ok(Some(GameRecord {
            rules: RoomRules { players, ai_level },
            seed,
            seats,
            history,
            outcome,
        }))
    }

    /// 保存してある対局の数。
    pub fn count(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM games", [], |r| r.get(0))?)
    }
}

enum Command {
    Save(Box<GameRecord>),
    Stop,
}

/// 専用のスレッドで保存する窓口。`save` は待たずに戻り、失敗は標準エラーに出す。
pub struct StoreWriter {
    tx: mpsc::Sender<Command>,
    handle: JoinHandle<()>,
}

impl StoreWriter {
    /// データベースを開いて（失敗したら、ここで返す）、保存のスレッドを始める。
    pub fn spawn(path: &str) -> Result<Self, StoreError> {
        let mut store = Store::open(path)?;
        let (tx, rx) = mpsc::channel::<Command>();
        let handle = std::thread::spawn(move || {
            while let Ok(Command::Save(record)) = rx.recv() {
                if let Err(error) = store.save(&record) {
                    eprintln!("対局の記録に失敗しました: {error}");
                }
            }
        });
        Ok(Self { tx, handle })
    }

    /// 保存を頼む（待たない）。
    pub fn save(&self, record: GameRecord) {
        let _ = self.tx.send(Command::Save(Box::new(record)));
    }

    /// 頼まれた分を保存し終えてから、スレッドを止める。
    pub fn finish(self) {
        let _ = self.tx.send(Command::Stop);
        let _ = self.handle.join();
    }

    /// `Lobby` などに渡す、保存の関数。
    pub fn sink(&self) -> impl Fn(GameRecord) + Send + Sync + use<> {
        let tx = self.tx.clone();
        move |record| {
            let _ = tx.send(Command::Save(Box::new(record)));
        }
    }
}

#[cfg(test)]
mod tests;
