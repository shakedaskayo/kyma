//! In-process "rows appended" notifications. Published by the write path
//! after a commit makes rows queryable; consumed by live Discover sessions.
//! Lossy by design (bounded broadcast) — consumers fall back to timer scans.

use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct RowsAppended {
    pub database: String,
    pub table: String,
    pub rows: u64,
}

#[derive(Debug, Clone)]
pub struct IngestEvents {
    tx: broadcast::Sender<RowsAppended>,
}

impl IngestEvents {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
    /// Fire-and-forget: no subscribers is fine; lagging subscribers drop.
    pub fn publish(&self, ev: RowsAppended) {
        let _ = self.tx.send(ev);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RowsAppended> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_is_lossy_nonblocking_and_received() {
        let bus = IngestEvents::new(8);
        let mut sub = bus.subscribe();
        bus.publish(RowsAppended {
            database: "db".into(),
            table: "t".into(),
            rows: 5,
        });
        let ev = sub.try_recv().expect("event delivered");
        assert_eq!(
            (ev.database.as_str(), ev.table.as_str(), ev.rows),
            ("db", "t", 5)
        );
    }

    #[tokio::test]
    async fn publish_without_subscribers_does_not_error() {
        let bus = IngestEvents::new(8);
        bus.publish(RowsAppended {
            database: "db".into(),
            table: "t".into(),
            rows: 1,
        }); // must not panic
    }
}
