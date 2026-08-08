use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8},
    },
    time::Duration,
};

use reap_durable_writer::JournalCodec;
use thiserror::Error;

use super::schema::{
    MAX_PM_AUTHENTICATED_JOURNAL_BYTES, MAX_PM_AUTHENTICATED_JOURNAL_LINE_BYTES,
    PmAuthenticatedJournalLineV1,
};

#[derive(Debug)]
pub(super) struct PmAuthenticatedJournalCodec {
    encoded_bytes: AtomicU64,
    #[cfg(test)]
    control: Arc<PmAuthenticatedJournalTestControl>,
}

impl PmAuthenticatedJournalCodec {
    #[cfg(not(test))]
    pub(super) const fn new(existing_bytes: u64) -> Self {
        Self {
            encoded_bytes: AtomicU64::new(existing_bytes),
        }
    }

    #[cfg(test)]
    pub(super) fn new(
        existing_bytes: u64,
        control: Arc<PmAuthenticatedJournalTestControl>,
    ) -> Self {
        Self {
            encoded_bytes: AtomicU64::new(existing_bytes),
            control,
        }
    }
}

#[cfg(test)]
const TEST_WRITE_NORMAL: u8 = 0;
#[cfg(test)]
const TEST_WRITE_FAIL: u8 = 1;
#[cfg(test)]
const TEST_WRITE_CLOSE: u8 = 2;
#[cfg(test)]
const TEST_WRITE_DELAY: u8 = 3;
#[cfg(test)]
const MAX_TEST_WRITE_LATCHES: usize = 2;

/// Test-only handle for one exact writer entry. Dropping the handle always
/// releases the writer so a failed assertion cannot strand its owned thread.
#[cfg(test)]
pub(crate) struct PmAuthenticatedJournalWriteLatch {
    state: Arc<PmAuthenticatedJournalWriteLatchState>,
}

#[cfg(test)]
impl PmAuthenticatedJournalWriteLatch {
    pub(crate) fn entered(&self) -> bool {
        self.state.entered.load(Ordering::SeqCst)
    }

    pub(crate) fn release(&self) {
        self.state.release();
    }
}

#[cfg(test)]
impl Drop for PmAuthenticatedJournalWriteLatch {
    fn drop(&mut self) {
        self.state.release();
    }
}

#[cfg(test)]
#[derive(Debug)]
struct PmAuthenticatedJournalWriteLatchState {
    entered: AtomicBool,
    released: Mutex<bool>,
    release_signal: Condvar,
}

#[cfg(test)]
impl PmAuthenticatedJournalWriteLatchState {
    const fn new() -> Self {
        Self {
            entered: AtomicBool::new(false),
            released: Mutex::new(false),
            release_signal: Condvar::new(),
        }
    }

    fn enter_and_wait(&self) {
        self.entered.store(true, Ordering::SeqCst);
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*released {
            released = self
                .release_signal
                .wait(released)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn release(&self) {
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *released = true;
        self.release_signal.notify_all();
    }
}

/// Per-journal deterministic fault control used only by the in-crate
/// durability-boundary tests. It cannot alter a non-test build.
#[cfg(test)]
#[derive(Debug)]
pub(super) struct PmAuthenticatedJournalTestControl {
    next: AtomicU8,
    write_latches: Mutex<VecDeque<Arc<PmAuthenticatedJournalWriteLatchState>>>,
}

#[cfg(test)]
impl PmAuthenticatedJournalTestControl {
    pub(super) const fn new() -> Self {
        Self {
            next: AtomicU8::new(TEST_WRITE_NORMAL),
            write_latches: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) fn latch_next_write(&self) -> PmAuthenticatedJournalWriteLatch {
        let state = Arc::new(PmAuthenticatedJournalWriteLatchState::new());
        let mut latches = self.write_latches.lock().expect("test write latch queue");
        assert!(
            latches.len() < MAX_TEST_WRITE_LATCHES,
            "authenticated journal tests may queue at most two write latches"
        );
        latches.push_back(Arc::clone(&state));
        PmAuthenticatedJournalWriteLatch { state }
    }

    pub(super) fn fail_next(&self) {
        self.next.store(TEST_WRITE_FAIL, Ordering::SeqCst);
    }

    pub(super) fn close_next(&self) {
        self.next.store(TEST_WRITE_CLOSE, Ordering::SeqCst);
    }

    pub(super) fn delay_next(&self) {
        self.next.store(TEST_WRITE_DELAY, Ordering::SeqCst);
    }

    fn take(&self) -> u8 {
        self.next.swap(TEST_WRITE_NORMAL, Ordering::SeqCst)
    }

    fn take_write_latch(&self) -> Option<Arc<PmAuthenticatedJournalWriteLatchState>> {
        self.write_latches
            .lock()
            .expect("test write latch queue")
            .pop_front()
    }
}

impl JournalCodec<PmAuthenticatedJournalLineV1> for PmAuthenticatedJournalCodec {
    type Error = PmAuthenticatedJournalCodecError;

    fn encode(
        &self,
        record: &PmAuthenticatedJournalLineV1,
        output: &mut Vec<u8>,
    ) -> Result<(), Self::Error> {
        #[cfg(test)]
        {
            if let Some(latch) = self.control.take_write_latch() {
                latch.enter_and_wait();
            }
            match self.control.take() {
                TEST_WRITE_NORMAL => {}
                TEST_WRITE_FAIL => return Err(PmAuthenticatedJournalCodecError::InjectedFailure),
                TEST_WRITE_CLOSE => {
                    panic!("injected authenticated-journal writer disappearance")
                }
                TEST_WRITE_DELAY => std::thread::sleep(Duration::from_millis(250)),
                _ => unreachable!("test writer control accepts only fixed fault modes"),
            }
        }
        let start = output.len();
        if let Err(error) = serde_json::to_writer(&mut *output, record) {
            output.truncate(start);
            return Err(error.into());
        }
        let encoded = output.len() - start;
        if encoded > MAX_PM_AUTHENTICATED_JOURNAL_LINE_BYTES {
            output.truncate(start);
            return Err(PmAuthenticatedJournalCodecError::LineTooLarge);
        }
        let bytes_with_newline = u64::try_from(encoded.saturating_add(1))
            .map_err(|_| PmAuthenticatedJournalCodecError::FileTooLarge)?;
        let update =
            self.encoded_bytes
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current
                        .checked_add(bytes_with_newline)
                        .filter(|next| *next <= MAX_PM_AUTHENTICATED_JOURNAL_BYTES)
                });
        if update.is_err() {
            output.truncate(start);
            return Err(PmAuthenticatedJournalCodecError::FileTooLarge);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub(super) enum PmAuthenticatedJournalCodecError {
    #[error("PM authenticated journal JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("PM authenticated journal encoded line exceeds its bounded size")]
    LineTooLarge,
    #[error("PM authenticated journal encoded generation exceeds its bounded size")]
    FileTooLarge,
    #[cfg(test)]
    #[error("injected authenticated journal codec failure")]
    InjectedFailure,
}
