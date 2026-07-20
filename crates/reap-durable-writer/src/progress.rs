use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterProgressSnapshot {
    pub records_enqueued: u64,
    pub records_written: u64,
    pub durable_sync_completions: u64,
    pub write_failures: u64,
    pub sync_failures: u64,
    pub dropped_records: u64,
    pub records_outstanding: usize,
    pub queue_capacity: usize,
    pub queue_depth: usize,
    pub queue_high_water: usize,
    pub last_writer_progress_ns: u64,
    pub last_writer_progress_age_ns: u64,
}

pub(crate) struct WriterProgress {
    records_enqueued: AtomicU64,
    records_written: AtomicU64,
    durable_sync_completions: AtomicU64,
    write_failures: AtomicU64,
    sync_failures: AtomicU64,
    dropped_records: AtomicU64,
    records_outstanding: AtomicUsize,
    queue_capacity: usize,
    queue_depth: AtomicUsize,
    queue_high_water: AtomicUsize,
    last_writer_progress_ns: AtomicU64,
}

impl WriterProgress {
    pub(crate) fn new(queue_capacity: usize) -> Self {
        let queue_capacity = queue_capacity.max(1);
        let _ = process_monotonic_ns();
        Self {
            records_enqueued: AtomicU64::new(0),
            records_written: AtomicU64::new(0),
            durable_sync_completions: AtomicU64::new(0),
            write_failures: AtomicU64::new(0),
            sync_failures: AtomicU64::new(0),
            dropped_records: AtomicU64::new(0),
            records_outstanding: AtomicUsize::new(0),
            queue_capacity,
            queue_depth: AtomicUsize::new(0),
            queue_high_water: AtomicUsize::new(0),
            last_writer_progress_ns: AtomicU64::new(0),
        }
    }

    pub(crate) fn record_enqueued(&self) {
        increment(&self.records_enqueued);
        self.records_outstanding.fetch_add(1, Ordering::Relaxed);
        let depth = self.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        // Tokio releases channel capacity immediately before `recv` returns.
        // Clamping this accounting handoff preserves the historical metric
        // without introducing a second capacity gate.
        self.queue_high_water
            .fetch_max(depth.min(self.queue_capacity), Ordering::Relaxed);
    }

    pub(crate) fn record_received(&self) {
        let previous = self.queue_depth.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0);
    }

    pub(crate) fn record_completed(&self) {
        let previous = self.records_outstanding.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0);
    }

    pub(crate) fn record_dropped(&self) {
        increment(&self.dropped_records);
    }

    pub(crate) fn record_written(&self) {
        increment(&self.records_written);
        self.record_writer_progress();
    }

    pub(crate) fn record_durable_sync_completion(&self) {
        increment(&self.durable_sync_completions);
        self.record_writer_progress();
    }

    pub(crate) fn record_write_failure(&self) {
        increment(&self.write_failures);
    }

    pub(crate) fn record_sync_failure(&self) {
        increment(&self.sync_failures);
    }

    pub(crate) fn record_writer_progress(&self) {
        self.last_writer_progress_ns
            .store(process_monotonic_ns(), Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> WriterProgressSnapshot {
        let last_writer_progress_ns = self.last_writer_progress_ns.load(Ordering::Relaxed);
        let last_writer_progress_age_ns = if last_writer_progress_ns == 0 {
            0
        } else {
            process_monotonic_ns().saturating_sub(last_writer_progress_ns)
        };
        WriterProgressSnapshot {
            records_enqueued: self.records_enqueued.load(Ordering::Relaxed),
            records_written: self.records_written.load(Ordering::Relaxed),
            durable_sync_completions: self.durable_sync_completions.load(Ordering::Relaxed),
            write_failures: self.write_failures.load(Ordering::Relaxed),
            sync_failures: self.sync_failures.load(Ordering::Relaxed),
            dropped_records: self.dropped_records.load(Ordering::Relaxed),
            records_outstanding: self.records_outstanding.load(Ordering::Relaxed),
            queue_capacity: self.queue_capacity,
            queue_depth: self
                .queue_depth
                .load(Ordering::Relaxed)
                .min(self.queue_capacity),
            queue_high_water: self
                .queue_high_water
                .load(Ordering::Relaxed)
                .min(self.queue_capacity),
            last_writer_progress_ns,
            last_writer_progress_age_ns,
        }
    }
}

fn increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

fn process_monotonic_ns() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let elapsed = ORIGIN.get_or_init(Instant::now).elapsed().as_nanos();
    elapsed.min(u64::MAX.saturating_sub(1) as u128) as u64 + 1
}

#[cfg(test)]
mod tests {
    fn rust_item<'a>(source: &'a str, marker: &str) -> &'a str {
        let start = source
            .find(marker)
            .expect("item marker must remain present");
        let source = &source[start..];
        let open = source.find('{').expect("item must have a body");
        let mut depth = 0_usize;
        for (offset, byte) in source.as_bytes()[open..].iter().copied().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[..=open + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("item body must be balanced");
    }

    #[test]
    fn progress_updates_are_atomic_numeric_only() {
        let source = include_str!("progress.rs");
        let source = source.split_once("#[cfg(test)]").unwrap().0;
        for forbidden in [
            "Mutex",
            "RwLock",
            "String",
            "Vec<",
            "HashMap",
            "Box<",
            "format!",
            ".to_string(",
            "serde_json",
        ] {
            assert!(
                !source.contains(forbidden),
                "writer progress path must not contain {forbidden}"
            );
        }

        let snapshot = rust_item(
            source,
            "pub(crate) fn snapshot(&self) -> WriterProgressSnapshot",
        );
        for forbidden in [
            ".store(",
            ".swap(",
            ".fetch_",
            ".send(",
            ".reserve(",
            "shutdown",
            "spawn",
            ".await",
            "unsafe",
        ] {
            assert!(
                !snapshot.contains(forbidden),
                "writer progress reader must not contain mutation/control operation {forbidden}"
            );
        }
    }
}
