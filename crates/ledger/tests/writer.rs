mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use support::{ManualClock, capture_logs};
use tracing::Level;
use zphira_ledger::{PointAward, Points, RoundRecord, RoundRecords};

const T0: i64 = 1_700_000_000_000;

fn points(dir: &Path) -> Points {
    Points::load_with_clock(dir, ManualClock::new(T0).clock()).unwrap()
}

fn award(id: i32) -> PointAward {
    PointAward {
        player_id: id,
        name: Some(format!("p{id}")),
        gained: 10,
    }
}

fn simple(id: &str) -> RoundRecord {
    RoundRecord {
        id: id.to_owned(),
        room_id: "room".to_owned(),
        ..Default::default()
    }
}

async fn wait_for(path: &Path, what: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
        assert!(Instant::now() < deadline, "{what} was never written");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn points_are_written_by_the_periodic_writer_until_it_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("player-points.json");
    let p = points(dir.path());
    let writer = p.spawn_writer_every(Duration::from_millis(30));

    p.add_points_batch(&[award(1)]);
    let text = wait_for(&path, "points").await;
    assert!(text.contains("\"p1\""), "{text}");

    p.add_points_batch(&[award(2)]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(&path).unwrap().contains("\"p2\"") {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    writer.stop().await;
    p.add_points_batch(&[award(3)]);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !std::fs::read_to_string(&path).unwrap().contains("\"p3\""),
        "a stopped writer writes nothing"
    );
    p.flush().unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("\"p3\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round_records_are_written_by_the_periodic_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("round-records.json");
    let records = RoundRecords::load(dir.path()).unwrap();
    let writer = records.spawn_writer_every(Duration::from_millis(30));
    records.append(simple("one"));
    let text = wait_for(&path, "round records").await;
    assert!(text.contains("\"one\""));
    writer.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_write_comes_one_interval_after_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("player-points.json");
    let p = points(dir.path());
    let writer = p.spawn_writer_every(Duration::from_millis(600));
    p.add_points_batch(&[award(1)]);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !path.exists(),
        "scheduleWithFixedDelay: the initial delay is a whole interval"
    );
    wait_for(&path, "points").await;
    writer.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_and_flush_saves_what_the_timer_had_not_yet() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    let records = RoundRecords::load(dir.path()).unwrap();
    let points_writer = p.spawn_writer_every(Duration::from_secs(3600));
    let records_writer = records.spawn_writer_every(Duration::from_secs(3600));
    p.add_points_batch(&[award(1)]);
    records.append(simple("last"));

    points_writer.stop_and_flush().await.unwrap();
    records_writer.stop_and_flush().await.unwrap();
    assert!(
        std::fs::read_to_string(dir.path().join("player-points.json"))
            .unwrap()
            .contains("\"p1\"")
    );
    assert!(
        std::fs::read_to_string(dir.path().join("round-records.json"))
            .unwrap()
            .contains("\"last\"")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_handle_stops_the_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("player-points.json");
    let p = points(dir.path());
    drop(p.spawn_writer_every(Duration::from_millis(30)));
    tokio::time::sleep(Duration::from_millis(100)).await;
    p.add_points_batch(&[award(1)]);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!path.exists());
}

#[test]
fn a_failed_save_is_logged_keeps_the_dirty_flag_and_is_retried() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("player-points.json");
    let p = points(dir.path());
    p.add_points_batch(&[award(1)]);

    std::fs::create_dir(&path).unwrap();
    let (result, logs) = capture_logs(|| p.flush());
    let err = result.unwrap_err();
    assert_eq!(err.to_string(), "Failed to save player points");
    assert_eq!(err.path(), path);
    assert!(std::error::Error::source(&err).is_some());
    let errors: Vec<_> = logs.iter().filter(|l| l.level == Level::ERROR).collect();
    assert_eq!(errors.len(), 1, "{logs:?}");
    assert_eq!(errors[0].message, "Failed to save player points");
    assert!(errors[0].error.is_some());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    std::fs::remove_dir(&path).unwrap();
    p.flush().unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("\"p1\""));

    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(simple("r"));
    let rpath = dir.path().join("round-records.json");
    std::fs::create_dir(&rpath).unwrap();
    let (result, logs) = capture_logs(|| records.flush());
    assert_eq!(
        result.unwrap_err().to_string(),
        "Failed to save round records"
    );
    assert_eq!(logs[0].message, "Failed to save round records");
    std::fs::remove_dir(&rpath).unwrap();
    records.flush().unwrap();
    assert!(std::fs::read_to_string(&rpath).unwrap().contains("\"r\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_periodic_writer_retries_after_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("player-points.json");
    let p = points(dir.path());
    std::fs::create_dir(&path).unwrap();
    let writer = p.spawn_writer_every(Duration::from_millis(30));
    p.add_points_batch(&[award(1)]);
    tokio::time::sleep(Duration::from_millis(150)).await;
    std::fs::remove_dir(&path).unwrap();
    let text = wait_for(&path, "points").await;
    assert!(text.contains("\"p1\""));
    writer.stop().await;
}

#[test]
fn flush_without_changes_writes_nothing_and_a_change_marks_dirty_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("player-points.json");
    let p = points(dir.path());
    p.flush().unwrap();
    assert!(!path.exists());
    p.add_points(1, "a", 1);
    p.flush().unwrap();
    let first = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    p.flush().unwrap();
    assert!(!path.exists(), "clean after a successful write");
    p.add_points(1, "a", 1);
    p.flush().unwrap();
    assert_ne!(std::fs::read_to_string(&path).unwrap(), first);

    std::fs::remove_file(&path).unwrap();
    let _ = p.summary(77, "visitor");
    p.flush().unwrap();
    assert!(!path.exists());
}

#[test]
fn a_reader_never_sees_a_torn_points_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("player-points.json");
    let p = points(dir.path());
    p.add_points_batch(&(0..200).map(award).collect::<Vec<_>>());
    p.flush().unwrap();

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let stop = std::sync::Arc::clone(&stop);
        let path = path.clone();
        std::thread::spawn(move || {
            let mut reads = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let text = std::fs::read_to_string(&path).unwrap();
                let parsed: Result<zphira_common::gson::JsonElement, _> =
                    zphira_common::gson::JsonElement::parse(&text);
                assert!(parsed.is_ok(), "torn file: {} bytes", text.len());
                reads += 1;
            }
            reads
        })
    };
    for i in 0..60 {
        p.add_points(i % 200, "x", 1);
        p.flush().unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.join().unwrap() > 0);
}
