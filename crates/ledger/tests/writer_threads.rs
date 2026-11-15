mod support;

use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing::{Event, Metadata, Subscriber, span};
use zphira_ledger::{PointAward, Points};

static EVENTS: OnceLock<Mutex<Vec<(ThreadId, String)>>> = OnceLock::new();

struct Recorder;

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Subscriber for Recorder {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        EVENTS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .push((std::thread::current().id(), message.0));
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

#[tokio::test(flavor = "current_thread")]
async fn the_periodic_write_runs_off_the_async_thread() {
    tracing::subscriber::set_global_default(Recorder).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let points = Points::load(dir.path()).unwrap();
    // A directory where the file goes makes every write fail and log from the thread it ran on.
    std::fs::create_dir(dir.path().join("player-points.json")).unwrap();
    points.add_points_batch(&[PointAward {
        player_id: 1,
        name: None,
        gained: 1,
    }]);
    let async_thread = std::thread::current().id();

    let writer = points.spawn_writer_every(Duration::from_millis(20));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let writer_thread = loop {
        let found = EVENTS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .iter()
            .find(|(_, m)| m == "Failed to save player points")
            .map(|(t, _)| *t);
        if let Some(thread) = found {
            break thread;
        }
        assert!(std::time::Instant::now() < deadline, "the writer never ran");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_ne!(writer_thread, async_thread);
    writer.stop().await;
}
