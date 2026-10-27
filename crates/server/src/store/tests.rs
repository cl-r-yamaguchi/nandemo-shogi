use std::sync::Arc;
use std::time::{Duration, Instant};

use ai::{Level, choose_action};
use protocol::{RoomRules, RoomStatus};
use rules::action::Action;
use rules::pieces::Catalog;
use rules::rng::SplitMix64;
use rules::state::{Phase, Stage};

use super::*;
use crate::record::replay;
use crate::room::Room;

const T0: u64 = 1_000_000;

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog::builtin().unwrap())
}

fn pid() -> String {
    format!("player-{:012}", 1)
}

/// 人間 1 人と AI 3 人の対局を、終局まで進めた記録。
fn finished_record(seed: u64) -> GameRecord {
    let catalog = catalog();
    let mut rng = SplitMix64(seed);
    let mut room = Room::new(
        "room-1",
        RoomRules {
            players: 4,
            ai_level: 1,
        },
        Arc::clone(&catalog),
        T0,
    );
    room.join(&pid(), "ゲスト1", T0, &mut rng).unwrap();
    room.start(&pid(), T0, &mut rng).unwrap();
    for _ in 0..5000 {
        if room.status() != RoomStatus::Playing {
            break;
        }
        while room.needs_ai() {
            room.run_ai(T0);
        }
        let Some(id) = room.current_human_id().map(str::to_string) else {
            continue;
        };
        let state = room.state().unwrap().clone();
        if let Stage::Pick(_) = state.stage {
            room.act(&id, state.turn, Action::Pick { index: 0 }, T0)
                .unwrap();
        }
        let state = room.state().unwrap().clone();
        if state.phase == Phase::Finished {
            break;
        }
        let far = Instant::now() + Duration::from_secs(5);
        let action = choose_action(&catalog, &state, Level::One, far, &mut rng).unwrap();
        room.act(&id, state.turn, action, T0).unwrap();
    }
    room.record().unwrap()
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nandemo-store-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("games.sqlite")
}

#[test]
fn a_saved_record_loads_back_equal_and_replays() {
    let record = finished_record(3);
    let mut store = Store::open(":memory:").unwrap();
    let id = store.save(&record).unwrap();
    let loaded = store.load(id).unwrap().expect("保存した記録がある");
    assert_eq!(loaded, record);
    assert!(
        replay(&catalog(), &loaded).is_ok(),
        "読み出した記録から再現できる"
    );
    assert_eq!(store.count().unwrap(), 1);
    assert_eq!(store.load(id + 1).unwrap(), None);
}

#[test]
fn a_seed_beyond_i64_is_kept_exactly() {
    let mut record = finished_record(3);
    record.seed = u64::MAX - 3;
    record.history.clear();
    record.outcome = None;
    let mut store = Store::open(":memory:").unwrap();
    let id = store.save(&record).unwrap();
    assert_eq!(store.load(id).unwrap().unwrap().seed, u64::MAX - 3);
}

#[test]
fn an_aborted_record_is_stored_with_its_reason() {
    let mut record = finished_record(3);
    record.outcome = None;
    let mut store = Store::open(":memory:").unwrap();
    let id = store.save(&record).unwrap();
    let reason: String = store
        .conn
        .query_row("SELECT end_reason FROM games WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(reason, "aborted");
    assert_eq!(store.load(id).unwrap().unwrap().outcome, None);
}

#[test]
fn the_database_holds_no_player_ids() {
    let record = finished_record(3);
    let mut store = Store::open(":memory:").unwrap();
    store.save(&record).unwrap();
    let tables = ["games", "seats", "moves"];
    for table in tables {
        let mut stmt = store
            .conn
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap();
        let columns = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..columns {
                if let Ok(text) = row.get::<_, String>(i) {
                    assert!(!text.contains(&pid()), "{table} の {i} 列目");
                }
            }
        }
    }
}

#[test]
fn a_failed_save_leaves_nothing_behind() {
    let record = finished_record(3);
    let mut store = Store::open(":memory:").unwrap();
    store.save(&record).unwrap();
    // moves への挿入が失敗するようにして、保存を途中で失敗させる
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER fail_moves BEFORE INSERT ON moves BEGIN SELECT RAISE(ABORT, 'x'); END;",
        )
        .unwrap();
    assert!(store.save(&record).is_err());
    assert_eq!(store.count().unwrap(), 1, "games にも seats にも残らない");
    let seats: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM seats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(seats, 4);
}

#[test]
fn a_file_database_persists_and_reopens() {
    let path = temp_path("reopen");
    let record = finished_record(4);
    let id = {
        let mut store = Store::open(path.to_str().unwrap()).unwrap();
        store.save(&record).unwrap()
    };
    let store = Store::open(path.to_str().unwrap()).unwrap();
    assert_eq!(store.load(id).unwrap().unwrap(), record);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_database_from_a_newer_version_is_refused() {
    let path = temp_path("newer");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
            .unwrap();
    }
    assert!(Store::open(path.to_str().unwrap()).is_err());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn an_unopenable_path_is_an_error_not_a_panic() {
    assert!(Store::open("/nonexistent-dir-for-test/games.sqlite").is_err());
    assert!(StoreWriter::spawn("/nonexistent-dir-for-test/games.sqlite").is_err());
}

#[test]
fn the_writer_saves_in_order_and_finishes_what_it_was_asked() {
    let path = temp_path("writer");
    let writer = StoreWriter::spawn(path.to_str().unwrap()).unwrap();
    let sink = writer.sink();
    sink(finished_record(5));
    writer.save(finished_record(6));
    writer.finish();
    let store = Store::open(path.to_str().unwrap()).unwrap();
    assert_eq!(store.count().unwrap(), 2);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
