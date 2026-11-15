mod support;

use std::path::Path;
use std::sync::{Arc, Barrier};

use support::{ManualClock, Script, capture_logs, read_fixture};
use tracing::Level;
use zphira_ledger::{Admins, ImportMode, RemoveOutcome};

const T0: i64 = 1_700_000_000_000;

fn load(dir: &Path, clock: &ManualClock) -> Admins {
    Admins::load_with_clock(dir, clock.clock()).unwrap()
}

fn file(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("admins.json")).unwrap()
}

fn edit(dir: &Path, text: &str) {
    let temp = dir.join("admins.json.editor");
    std::fs::write(&temp, text).unwrap();
    std::fs::rename(&temp, dir.join("admins.json")).unwrap();
}

fn show(ids: &[i32]) -> String {
    format!(
        "[{}]",
        ids.iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[test]
fn a_missing_file_is_created_as_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let clock = ManualClock::new(T0);
    let admins = load(&data, &clock);
    assert!(admins.list().is_empty());
    assert_eq!(file(&data), "[]");
    assert!(!admins.is_admin(1));
}

#[test]
fn replays_the_java_scenario() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    let mut java = Script::new("ledger_admins.tsv");

    assert_eq!(show(&admins.list()), java.expect("list_initial")[0]);
    assert_eq!(file(dir.path()), java.expect("file_created")[0]);

    let added: Vec<String> = [5, 3, 9, 3]
        .iter()
        .map(|id| admins.add(*id).unwrap().to_string())
        .collect();
    assert_eq!(added, java.expect("add"));
    assert_eq!(show(&admins.list()), java.expect("list_after_add")[0]);
    let text = file(dir.path());
    assert_eq!(text, java.expect("file_after_add")[0]);
    assert_eq!(text, read_fixture("admins_java.json"));

    let removed: Vec<String> = [5, 5, 77]
        .iter()
        .map(|id| admins.remove(*id).unwrap().to_string())
        .collect();
    assert_eq!(removed, java.expect("remove"));
    assert_eq!(show(&admins.list()), java.expect("list_after_remove")[0]);
    let flags = java.expect("is_admin");
    assert_eq!(
        vec![
            admins.is_admin(3).to_string(),
            admins.is_admin(5).to_string()
        ],
        flags
    );
    for id in [-2, 0, i32::MAX] {
        admins.add(id).unwrap();
    }
    assert_eq!(show(&admins.list()), java.expect("list_odd_ids")[0]);
    assert_eq!(file(dir.path()), java.expect("file_odd_ids")[0]);

    replay_imports(&admins, &mut java);
}

const REFUSED: [&str; 3] = ["replace_dups", "merge_dups", "replace_empty"];

fn replay_imports(admins: &Admins, java: &mut Script) {
    let mut last_case = String::new();
    while let Some(row) = java.next_row() {
        let label = row[0].as_str();
        if label == "import_base" {
            let report = admins.import(&row[2], ImportMode::Replace).unwrap();
            assert_eq!(report.imported.to_string(), row[4]);
            last_case.clear();
        } else if let Some(case) = label.strip_prefix("import_") {
            last_case = case.to_owned();
            let mode = if row[1].ends_with("merge") {
                ImportMode::Merge
            } else {
                ImportMode::Replace
            };
            let outcome = admins.import(&row[2], mode);
            match case {
                c if REFUSED.contains(&c) => {
                    let err = outcome.unwrap_err();
                    assert!(err.is_invalid(), "{case}: {err}");
                    if case == "replace_empty" {
                        assert_eq!(err.to_string(), "至少保留一名管理员");
                    }
                }
                _ => {
                    let report = outcome.unwrap();
                    assert_eq!(row[3], "OK", "{case}");
                    assert_eq!(report.imported.to_string(), row[4], "{case}: imported");
                    assert_eq!(report.total.to_string(), row[5], "{case}: total");
                }
            }
        } else if let Some(case) = label.strip_prefix("state_") {
            if case == "base" {
                assert_eq!(show(&admins.list()), row[1]);
            } else if REFUSED.contains(&case) {
                assert_eq!(show(&admins.list()), "[1, 2, 3]", "{case}");
            } else {
                assert_eq!(case, last_case);
                assert_eq!(show(&admins.list()), row[1], "{case}");
            }
        } else {
            panic!("unexpected row {label}");
        }
    }
}

#[test]
fn a_hand_edit_takes_effect_within_a_second_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    admins.add(2).unwrap();
    assert!(admins.is_admin(2));

    edit(dir.path(), "[1, 3]");
    clock.advance(999);
    assert!(admins.is_admin(2), "not looked at yet");
    clock.advance(1);
    assert!(!admins.is_admin(2), "revoked by editing the file");
    assert!(admins.is_admin(3));
    assert_eq!(admins.list(), vec![1, 3]);

    edit(dir.path(), "[9]");
    clock.advance(1000);
    assert_eq!(admins.list(), vec![9]);
}

#[test]
fn the_file_is_looked_at_once_per_second_at_most() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    edit(dir.path(), "[5]");
    for _ in 0..10 {
        clock.advance(99);
        assert_eq!(admins.list(), vec![1], "still inside the second");
    }
    clock.advance(10);
    assert_eq!(admins.list(), vec![5]);
    edit(dir.path(), "[6]");
    clock.set(T0 - 3_600_000);
    assert_eq!(admins.list(), vec![6]);
}

#[test]
fn an_edit_that_changes_only_the_modification_time_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    let path = dir.path().join("admins.json");
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "[\n  2\n]").unwrap();
    // The modification time is moved by hand so a coarse file system clock cannot make two writes
    // alike.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(before + std::time::Duration::from_secs(5))
        .unwrap();
    clock.advance(1000);
    assert_eq!(admins.list(), vec![2]);
}

#[test]
fn a_malformed_file_keeps_the_list_and_logs_the_java_line_once() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    admins.add(2).unwrap();

    for broken in [
        "[1, 2",
        "{\"a\":1}",
        "[true]",
        "[1.5]",
        "[\"5\"]",
        "[1,]",
        "[1] x",
        "",
        "   ",
        "null",
        "[null]",
    ] {
        edit(dir.path(), broken);
        clock.advance(1000);
        let (list, logs) = capture_logs(|| admins.list());
        assert_eq!(list, vec![1, 2], "{broken:?}");
        let errors: Vec<_> = logs.iter().filter(|l| l.level == Level::ERROR).collect();
        assert_eq!(errors.len(), 1, "{broken:?}: {logs:?}");
        assert_eq!(
            errors[0].message,
            "admins.json is unreadable, keeping the current admin list"
        );
        assert!(errors[0].error.is_some(), "the cause is logged too");
        clock.advance(5000);
        let (_, again) = capture_logs(|| admins.list());
        assert!(again.is_empty(), "{broken:?}: {again:?}");
    }
    edit(dir.path(), "[4]");
    clock.advance(1000);
    assert_eq!(admins.list(), vec![4]);

    std::fs::write(dir.path().join("admins.json"), [0xff, 0xfe, 0x5b, 0x5d]).unwrap();
    clock.advance(1000);
    let (list, logs) = capture_logs(|| admins.list());
    assert_eq!(list, vec![4]);
    assert_eq!(logs.len(), 1);
}

#[test]
fn a_deleted_file_is_created_again_from_memory() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(8).unwrap();
    std::fs::remove_file(dir.path().join("admins.json")).unwrap();
    clock.advance(1000);
    assert_eq!(admins.list(), vec![8]);
    assert_eq!(file(dir.path()), "[\n  8\n]");
}

#[test]
fn a_change_made_just_before_add_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    edit(dir.path(), "[1, 2, 3]");
    assert!(admins.add(4).unwrap());
    assert_eq!(file(dir.path()), "[\n  1,\n  2,\n  3,\n  4\n]");
    edit(dir.path(), "[1, 2, 3, 4, 5]");
    assert!(admins.remove(2).unwrap());
    assert_eq!(admins.list(), vec![1, 3, 4, 5]);
}

#[test]
fn start_up_with_a_bad_file_logs_and_starts_empty_and_an_unreadable_one_fails() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    std::fs::write(dir.path().join("admins.json"), "[1,").unwrap();
    let (admins, logs) = capture_logs(|| load(dir.path(), &clock));
    assert!(admins.list().is_empty());
    assert_eq!(
        logs[0].message,
        "admins.json is unreadable, keeping the current admin list"
    );
    assert_eq!(
        file(dir.path()),
        "[1,",
        "the bad file is not touched until a change"
    );

    std::fs::write(dir.path().join("admins.json"), "[3, 1, 2]").unwrap();
    assert_eq!(load(dir.path(), &clock).list(), vec![1, 2, 3]);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("admins.json")).unwrap();
    let err = Admins::load(dir.path()).unwrap_err();
    assert_eq!(err.to_string(), "Failed to load admin list");
}

#[test]
fn add_and_remove_report_what_happened_and_keep_the_file_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    assert!(admins.add(30).unwrap());
    assert!(admins.add(-1).unwrap());
    assert!(admins.add(7).unwrap());
    assert!(!admins.add(7).unwrap());
    assert_eq!(file(dir.path()), "[\n  -1,\n  7,\n  30\n]");
    assert!(admins.remove(7).unwrap());
    assert!(!admins.remove(7).unwrap());
    assert_eq!(admins.list(), vec![-1, 30]);
    assert_eq!(admins.export().unwrap(), file(dir.path()));
}

#[test]
fn the_console_remove_has_no_last_admin_rule() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    assert!(
        admins.remove(1).unwrap(),
        "the console may remove the last admin"
    );
    assert!(admins.list().is_empty());
    assert_eq!(file(dir.path()), "[]");
}

#[test]
fn remove_keeping_one_reports_each_case() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);

    assert_eq!(
        admins.remove_keeping_one(1).unwrap(),
        RemoveOutcome::NotAnAdmin
    );
    admins.add(1).unwrap();
    assert_eq!(
        admins.remove_keeping_one(99).unwrap(),
        RemoveOutcome::NotAnAdmin
    );
    assert_eq!(
        admins.remove_keeping_one(1).unwrap(),
        RemoveOutcome::LastAdmin
    );
    assert_eq!(admins.list(), vec![1]);

    admins.add(2).unwrap();
    assert_eq!(
        admins.remove_keeping_one(99).unwrap(),
        RemoveOutcome::NotAnAdmin
    );
    assert_eq!(admins.list(), vec![1, 2]);
    assert_eq!(
        admins.remove_keeping_one(1).unwrap(),
        RemoveOutcome::Removed
    );
    assert_eq!(admins.list(), vec![2]);
    assert_eq!(file(dir.path()), "[\n  2\n]");
    assert_eq!(
        admins.remove_keeping_one(2).unwrap(),
        RemoveOutcome::LastAdmin
    );
    assert_eq!(
        admins.remove_keeping_one(1).unwrap(),
        RemoveOutcome::NotAnAdmin
    );
    assert_eq!(admins.list(), vec![2]);
}

#[test]
fn concurrent_removals_never_empty_the_list() {
    for round in 0..60 {
        let dir = tempfile::tempdir().unwrap();
        let admins = Admins::load(dir.path()).unwrap();
        admins.add(1).unwrap();
        admins.add(2).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = [1, 2]
            .into_iter()
            .map(|id| {
                let admins = admins.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    admins.remove_keeping_one(id).unwrap()
                })
            })
            .collect();
        let outcomes: Vec<RemoveOutcome> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        let removed = outcomes
            .iter()
            .filter(|o| **o == RemoveOutcome::Removed)
            .count();
        let refused = outcomes
            .iter()
            .filter(|o| **o == RemoveOutcome::LastAdmin)
            .count();
        assert_eq!((removed, refused), (1, 1), "round {round}: {outcomes:?}");
        assert_eq!(admins.list().len(), 1, "round {round}");
        assert_eq!(file(dir.path()).lines().count(), 3, "the file agrees");
    }
}

#[test]
fn many_admins_removed_concurrently_leave_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    let admins = Admins::load(dir.path()).unwrap();
    for id in 1..=8 {
        admins.add(id).unwrap();
    }
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (1..=8)
        .map(|id| {
            let admins = admins.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                admins.remove_keeping_one(id).unwrap()
            })
        })
        .collect();
    let outcomes: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == RemoveOutcome::Removed)
            .count(),
        7
    );
    assert_eq!(admins.list().len(), 1);
}

#[test]
fn a_failed_write_is_reported_changes_nothing_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    admins.add(2).unwrap();
    std::fs::remove_file(dir.path().join("admins.json")).unwrap();
    std::fs::create_dir(dir.path().join("admins.json")).unwrap();

    let err = admins.add(3).unwrap_err();
    assert_eq!(err.to_string(), "Failed to save admin list");
    assert!(!admins.is_admin(3), "the id was not added");
    assert_eq!(admins.list(), vec![1, 2]);

    let err = admins.remove(2).unwrap_err();
    assert_eq!(err.to_string(), "Failed to save admin list");
    assert!(admins.is_admin(2), "the admin was not removed");

    let err = admins.remove_keeping_one(2).unwrap_err();
    assert_eq!(err.to_string(), "Failed to save admin list");
    assert_eq!(admins.list(), vec![1, 2]);

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    std::fs::remove_dir(dir.path().join("admins.json")).unwrap();
    assert!(admins.add(3).unwrap());
    assert_eq!(file(dir.path()), "[\n  1,\n  2,\n  3\n]");
}

#[test]
fn the_import_checks_run_against_the_file_a_moment_ago_too() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = load(dir.path(), &clock);
    admins.add(1).unwrap();
    edit(dir.path(), "[1, 2]");
    let report = admins.import("[5]", ImportMode::Merge).unwrap();
    assert_eq!(report.total, 3, "the hand edit is part of the merge");
    assert_eq!(admins.list(), vec![1, 2, 5]);
}

#[test]
fn stale_temp_files_are_removed_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join(".admins.json.1-0.tmp");
    std::fs::write(&stale, "[").unwrap();
    let _ = Admins::load(dir.path()).unwrap();
    assert!(!stale.exists());
}

#[test]
fn the_documents_of_both_servers_load_in_the_other() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("admins.json"),
        read_fixture("admins_java.json"),
    )
    .unwrap();
    let admins = Admins::load(dir.path()).unwrap();
    assert_eq!(admins.list(), vec![3, 5, 9]);
    admins.remove(9).unwrap();
    admins.add(9).unwrap();
    assert_eq!(file(dir.path()), read_fixture("admins_java.json"));
}
