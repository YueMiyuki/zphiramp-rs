mod support;

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use support::ManualClock;
use zphira_common::gson::JsonElement;
use zphira_ledger::{
    Admins, ImportError, ImportMode, ImportResult, PointAward, Points, RoundRecord, RoundRecords,
};

const T0: i64 = 1_700_000_000_000;

fn points(dir: &Path) -> Points {
    Points::load_with_clock(dir, ManualClock::new(T0).clock()).unwrap()
}

fn award(id: i32, name: &str, gained: i32) -> PointAward {
    PointAward {
        player_id: id,
        name: Some(name.to_owned()),
        gained,
    }
}

fn simple(id: &str, finished: i64) -> RoundRecord {
    RoundRecord {
        id: id.to_owned(),
        room_id: "room".to_owned(),
        finished_at: finished,
        ..Default::default()
    }
}

fn row(id: &str, finished: i64) -> String {
    format!(
        "{{\"id\":\"{id}\",\"room_id\":\"r\",\"chart_id\":1,\"started_at\":{},\"finished_at\":{finished},\"results\":[]}}",
        finished - 1
    )
}

fn file(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

fn temp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

fn ids(records: &RoundRecords) -> String {
    records
        .query(100_000, 0, None, None)
        .iter()
        .map(|r| r.id.clone())
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone, Default)]
struct Recorder {
    states: Arc<Mutex<Vec<String>>>,
}

impl Recorder {
    fn callback(&self, name: &'static str) -> impl FnOnce(&str) -> io::Result<String> + use<> {
        let states = Arc::clone(&self.states);
        move |state| {
            states.lock().unwrap().push(state.to_owned());
            Ok(name.to_owned())
        }
    }

    fn calls(&self) -> Vec<String> {
        self.states.lock().unwrap().clone()
    }
}

fn counts(result: &ImportResult) -> (usize, usize, &str) {
    (result.imported, result.total, result.backup_file.as_str())
}

#[test]
fn points_backup_sees_the_state_the_import_replaces() {
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let dir = tempfile::tempdir().unwrap();
        let p = points(dir.path());
        p.add_points_batch(&[award(1, "Ann", 100), award(2, "Bo", 50)]);
        let before = p.export().unwrap();
        assert!(file(dir.path(), "player-points.json").is_none());

        let recorder = Recorder::default();
        let live_file_during_backup = Arc::new(Mutex::new(Some(String::from("unset"))));
        let seen = Arc::clone(&live_file_during_backup);
        let path = dir.path().join("player-points.json");
        let callback = recorder.callback("backups/points-1.json");
        let result = p
            .import_json(r#"{"3":{"name":"Cy","points":7}}"#, mode, move |state| {
                *seen.lock().unwrap() = std::fs::read_to_string(&path).ok();
                callback(state)
            })
            .unwrap();

        assert_eq!(
            recorder.calls(),
            vec![before],
            "{mode:?}: the pre-import state"
        );
        assert_eq!(
            *live_file_during_backup.lock().unwrap(),
            None,
            "the backup runs before the file is written"
        );
        let expected_rows = if mode == ImportMode::Merge { 3 } else { 1 };
        assert_eq!(
            counts(&result),
            (expected_rows, expected_rows, "backups/points-1.json")
        );
        assert_eq!(p.len(), expected_rows);
        assert_eq!(
            file(dir.path(), "player-points.json").unwrap(),
            p.export().unwrap()
        );
        assert!(temp_files(dir.path()).is_empty());
    }
}

#[test]
fn points_backup_is_what_export_returns_not_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "Ann", 100);
    p.flush().unwrap();
    p.add_points(2, "Bo", 50);
    let file_before = file(dir.path(), "player-points.json").unwrap();
    assert!(!file_before.contains("Bo"));
    let recorder = Recorder::default();
    p.import_json(
        r#"{"9":{"points":1}}"#,
        ImportMode::Replace,
        recorder.callback("b"),
    )
    .unwrap();
    let state = recorder.calls().remove(0);
    assert!(
        state.contains("\"Bo\""),
        "the backup has the unflushed row: {state}"
    );
    assert_ne!(state, file_before);
}

#[test]
fn records_backup_sees_the_state_the_import_replaces() {
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let dir = tempfile::tempdir().unwrap();
        let records = RoundRecords::load(dir.path()).unwrap();
        records.append(simple("held1", 10));
        records.append(simple("held2", 20));
        let before = records.export().unwrap();

        let recorder = Recorder::default();
        let result = records
            .import_json(
                &format!("[{}]", row("doc1", 15)),
                mode,
                recorder.callback("backups/records-1.json"),
            )
            .unwrap();
        assert_eq!(recorder.calls(), vec![before], "{mode:?}");
        let expected = if mode == ImportMode::Merge { 3 } else { 1 };
        assert_eq!(
            counts(&result),
            (expected, expected, "backups/records-1.json")
        );
        let expected_ids = if mode == ImportMode::Merge {
            "held2,doc1,held1"
        } else {
            "doc1"
        };
        assert_eq!(ids(&records), expected_ids);
        assert_eq!(
            file(dir.path(), "round-records.json").unwrap(),
            records.export().unwrap()
        );
    }
}

#[test]
fn admins_backup_sees_the_state_the_import_replaces() {
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let dir = tempfile::tempdir().unwrap();
        let admins = Admins::load(dir.path()).unwrap();
        admins.add(1).unwrap();
        admins.add(2).unwrap();
        let before = admins.export().unwrap();

        let recorder = Recorder::default();
        let result = admins
            .import_json("[2,3]", mode, recorder.callback("backups/admins-1.json"))
            .unwrap();
        assert_eq!(recorder.calls(), vec![before], "{mode:?}");
        let (expected, list) = if mode == ImportMode::Merge {
            (3, vec![1, 2, 3])
        } else {
            (2, vec![2, 3])
        };
        assert_eq!(
            counts(&result),
            (expected, expected, "backups/admins-1.json")
        );
        assert_eq!(admins.list(), list);
        assert_eq!(
            file(dir.path(), "admins.json").unwrap(),
            admins.export().unwrap()
        );
    }
}

#[test]
fn the_backup_of_an_empty_store_is_the_empty_document() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Recorder::default();
    points(dir.path())
        .import_json(
            r#"{"1":{"points":1}}"#,
            ImportMode::Replace,
            recorder.callback("x"),
        )
        .unwrap();
    RoundRecords::load(dir.path())
        .unwrap()
        .import_json(
            &format!("[{}]", row("a", 1)),
            ImportMode::Replace,
            recorder.callback("x"),
        )
        .unwrap();
    assert_eq!(recorder.calls(), vec!["{}".to_owned(), "[]".to_owned()]);
}

#[test]
fn a_refused_document_never_calls_the_backup() {
    let dir = tempfile::tempdir().unwrap();
    let called = Arc::new(AtomicUsize::new(0));
    let callback = || {
        let called = Arc::clone(&called);
        move |_: &str| -> io::Result<String> {
            called.fetch_add(1, Ordering::SeqCst);
            Ok(String::new())
        }
    };

    let p = points(dir.path());
    p.add_points(1, "Ann", 10);
    let before = p.export().unwrap();
    for bad in [
        "",
        "{",
        "null",
        "[]",
        r#"{"abc":{"points":1}}"#,
        r#"{"1":{}}"#,
        r#"{"1":{"points":-1}}"#,
        r#"{"kind":"points","content":"{}"}"#,
    ] {
        for mode in [ImportMode::Replace, ImportMode::Merge] {
            let err = p.import_json(bad, mode, callback()).unwrap_err();
            assert!(err.is_invalid(), "{bad}: {err:?}");
        }
    }
    assert_eq!(p.export().unwrap(), before);

    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(simple("keep", 1));
    let before = records.export().unwrap();
    let dup = format!("[{},{}]", row("same", 1), row("same", 2));
    for bad in ["", "{}", "[1,2]", "[{\"id\":\"a\"}]", dup.as_str()] {
        for mode in [ImportMode::Replace, ImportMode::Merge] {
            let err = records.import_json(bad, mode, callback()).unwrap_err();
            assert!(err.is_invalid(), "{bad}: {err:?}");
        }
    }
    assert_eq!(records.export().unwrap(), before);

    let admins = Admins::load(dir.path()).unwrap();
    admins.add(1).unwrap();
    let before = admins.export().unwrap();
    for bad in ["", "{}", "[null]", "[1.5]", "[2,2]", r#"[{"id":"r"}]"#] {
        for mode in [ImportMode::Replace, ImportMode::Merge] {
            let err = admins.import_json(bad, mode, callback()).unwrap_err();
            assert!(err.is_invalid(), "{bad}: {err:?}");
        }
    }
    let err = admins
        .import_json("[]", ImportMode::Replace, callback())
        .unwrap_err();
    assert_eq!(err.to_string(), "至少保留一名管理员");
    assert_eq!(admins.export().unwrap(), before);

    assert_eq!(
        called.load(Ordering::SeqCst),
        0,
        "no refused document reached the backup"
    );
}

fn disk_full(_: &str) -> io::Result<String> {
    Err(io::Error::other("disk full"))
}

#[test]
fn a_failing_backup_aborts_the_points_import_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "Ann", 100);
    p.flush().unwrap();
    p.add_points(2, "Bo", 50);
    let file_before = file(dir.path(), "player-points.json").unwrap();
    let memory_before = p.export().unwrap();

    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let err = p
            .import_json(r#"{"7":{"points":7}}"#, mode, disk_full)
            .unwrap_err();
        assert_eq!(err, ImportError::Write("disk full".to_owned()));
        assert!(!err.is_invalid());
        assert_eq!(file(dir.path(), "player-points.json").unwrap(), file_before);
        assert_eq!(p.export().unwrap(), memory_before);
    }
    assert!(temp_files(dir.path()).is_empty());
    p.flush().unwrap();
    assert_eq!(
        file(dir.path(), "player-points.json").unwrap(),
        memory_before
    );
}

#[test]
fn a_failing_backup_aborts_the_records_and_admins_imports_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();

    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(simple("held", 1));
    records.flush().unwrap();
    let file_before = file(dir.path(), "round-records.json").unwrap();
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let err = records
            .import_json(&format!("[{}]", row("doc", 5)), mode, disk_full)
            .unwrap_err();
        assert_eq!(err, ImportError::Write("disk full".to_owned()));
        assert_eq!(ids(&records), "held");
        assert_eq!(file(dir.path(), "round-records.json").unwrap(), file_before);
    }

    let admins = Admins::load(dir.path()).unwrap();
    admins.add(1).unwrap();
    let file_before = file(dir.path(), "admins.json").unwrap();
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let err = admins.import_json("[9]", mode, disk_full).unwrap_err();
        assert_eq!(err, ImportError::Write("disk full".to_owned()));
        assert_eq!(admins.list(), vec![1]);
        assert_eq!(file(dir.path(), "admins.json").unwrap(), file_before);
    }
    assert!(temp_files(dir.path()).is_empty());

    records
        .import_json(&format!("[{}]", row("doc", 5)), ImportMode::Merge, |_| {
            Ok(String::new())
        })
        .unwrap();
    assert_eq!(
        ids(&records),
        "held,doc".split(',').rev().collect::<Vec<_>>().join(",")
    );
}

#[test]
fn a_panicking_backup_changes_nothing_and_leaves_the_store_usable() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "Ann", 100);
    let before = p.export().unwrap();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = p.import_json(r#"{"7":{"points":7}}"#, ImportMode::Replace, |_| {
            panic!("backup bug")
        });
    }));
    assert!(panicked.is_err());
    assert_eq!(p.export().unwrap(), before);
    assert_eq!(p.add_points(1, "Ann", 1), 101, "the store still answers");
    p.import_json(r#"{"7":{"points":7}}"#, ImportMode::Replace, |_| {
        Ok(String::new())
    })
    .unwrap();
}

struct Intruder {
    handle: thread::JoinHandle<()>,
    finished_during_backup: bool,
}

impl Intruder {
    fn start(work: impl FnOnce() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            work();
            let _ = tx.send(());
        });
        // Let the thread reach the store lock before looking.
        thread::sleep(Duration::from_millis(150));
        Self {
            finished_during_backup: rx.try_recv().is_ok(),
            handle,
        }
    }

    fn finish(self) {
        assert!(
            !self.finished_during_backup,
            "the mutation completed while the import held the store"
        );
        self.handle.join().unwrap();
    }
}

#[test]
fn a_points_award_during_the_backup_waits_and_lands_after_the_import() {
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let dir = tempfile::tempdir().unwrap();
        let p = points(dir.path());
        p.add_points(1, "Ann", 10);
        let slot: Arc<Mutex<Option<Intruder>>> = Arc::default();
        let recorder = Recorder::default();
        let callback = recorder.callback("b");
        let (p2, slot2) = (p.clone(), Arc::clone(&slot));
        p.import_json(r#"{"2":{"name":"Bo","points":5}}"#, mode, move |state| {
            let p3 = p2.clone();
            *slot2.lock().unwrap() = Some(Intruder::start(move || {
                p3.add_points(99, "late", 3);
            }));
            callback(state)
        })
        .unwrap();
        slot.lock().unwrap().take().unwrap().finish();

        assert!(
            !recorder.calls()[0].contains("late"),
            "the backup predates the award"
        );
        let snapshot = p.ranking_snapshot(10);
        let late = snapshot.iter().find(|e| e.player_id == 99).unwrap();
        assert_eq!(late.points, 3);
        assert!(snapshot.iter().any(|e| e.player_id == 2));
        assert_eq!(
            snapshot.iter().any(|e| e.player_id == 1),
            mode == ImportMode::Merge
        );
        assert!(
            !file(dir.path(), "player-points.json")
                .unwrap()
                .contains("late")
        );
        p.flush().unwrap();
        assert!(
            file(dir.path(), "player-points.json")
                .unwrap()
                .contains("late")
        );
    }
}

#[test]
fn a_round_appended_during_the_backup_waits_and_lands_after_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(simple("held", 10));
    let slot: Arc<Mutex<Option<Intruder>>> = Arc::default();
    let recorder = Recorder::default();
    let callback = recorder.callback("b");
    let (r2, slot2) = (records.clone(), Arc::clone(&slot));
    records
        .import_json(
            &format!("[{}]", row("doc", 5)),
            ImportMode::Replace,
            move |state| {
                let r3 = r2.clone();
                *slot2.lock().unwrap() =
                    Some(Intruder::start(move || r3.append(simple("late", 20))));
                callback(state)
            },
        )
        .unwrap();
    slot.lock().unwrap().take().unwrap().finish();
    assert!(!recorder.calls()[0].contains("late"));
    assert_eq!(ids(&records), "late,doc");
    assert!(
        !file(dir.path(), "round-records.json")
            .unwrap()
            .contains("late")
    );
}

#[test]
fn an_admin_change_during_the_backup_waits_and_lands_after_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let admins = Admins::load(dir.path()).unwrap();
    admins.add(1).unwrap();
    let slot: Arc<Mutex<Option<Intruder>>> = Arc::default();
    let recorder = Recorder::default();
    let callback = recorder.callback("b");
    let (a2, slot2) = (admins.clone(), Arc::clone(&slot));
    admins
        .import_json("[5]", ImportMode::Replace, move |state| {
            let a3 = a2.clone();
            *slot2.lock().unwrap() = Some(Intruder::start(move || {
                a3.add(77).unwrap();
            }));
            callback(state)
        })
        .unwrap();
    slot.lock().unwrap().take().unwrap().finish();
    assert!(!recorder.calls()[0].contains("77"));
    assert_eq!(admins.list(), vec![5, 77]);
}

#[test]
fn no_award_is_lost_between_the_backup_and_the_swap() {
    const AWARDS: i32 = 3000;
    const IMPORTED: i32 = 1_000_000;
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "p", 0);

    let started = Arc::new(AtomicBool::new(false));
    let awarder = {
        let (p, started) = (p.clone(), Arc::clone(&started));
        thread::spawn(move || {
            for _ in 0..AWARDS {
                p.add_points(1, "p", 1);
                started.store(true, Ordering::SeqCst);
            }
        })
    };
    while !started.load(Ordering::SeqCst) {
        thread::yield_now();
    }
    let backed_up = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&backed_up);
    p.import_json(
        &format!("{{\"1\":{{\"name\":\"p\",\"points\":{IMPORTED}}}}}"),
        ImportMode::Replace,
        move |state| {
            *sink.lock().unwrap() = state.to_owned();
            thread::sleep(Duration::from_millis(100));
            Ok(String::new())
        },
    )
    .unwrap();
    awarder.join().unwrap();

    let state = JsonElement::parse(&backed_up.lock().unwrap()).unwrap();
    let seen = state
        .as_object()
        .and_then(|o| o.get("1"))
        .and_then(JsonElement::as_object)
        .and_then(|o| o.get("points"))
        .and_then(JsonElement::get_as_int)
        .unwrap();
    let final_points = p.ranking_snapshot(1)[0].points;
    assert_eq!(
        final_points,
        IMPORTED + (AWARDS - seen),
        "backup saw {seen} awards"
    );
}

#[test]
fn no_round_is_lost_between_the_backup_and_the_swap() {
    const ROUNDS: usize = 600;
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let appender = {
        let (records, started) = (records.clone(), Arc::clone(&started));
        thread::spawn(move || {
            for i in 0..ROUNDS {
                records.append(simple(&format!("live{i}"), i as i64));
                started.store(true, Ordering::SeqCst);
            }
        })
    };
    while !started.load(Ordering::SeqCst) {
        thread::yield_now();
    }
    let backed_up = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&backed_up);
    records
        .import_json(
            &format!("[{}]", row("doc", 1)),
            ImportMode::Replace,
            move |state| {
                *sink.lock().unwrap() = state.to_owned();
                thread::sleep(Duration::from_millis(100));
                Ok(String::new())
            },
        )
        .unwrap();
    appender.join().unwrap();

    let seen = backed_up.lock().unwrap().matches("\"live").count();
    assert_eq!(
        records.len(),
        1 + (ROUNDS - seen),
        "backup saw {seen} rounds"
    );
    assert!(records.find("doc").is_some());
}
