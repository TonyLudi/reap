//! Owner-thread allocation accounting for deterministic local benchmarks.
//!
//! The unsafe surface is confined to transparent delegation to
//! [`std::alloc::System`]. Consumers receive only a safe, take-once
//! measurement-window API.

#![forbid(unsafe_op_in_unsafe_fn)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

const INACTIVE: u64 = 0;
const INITIALIZING: u64 = 1;
const FIRST_TOKEN: u64 = 2;

static ACTIVE_WINDOW: AtomicU64 = AtomicU64::new(INACTIVE);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(FIRST_TOKEN);
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static DEALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
static PEAK_LIVE_BYTES: AtomicI64 = AtomicI64::new(0);

std::thread_local! {
    static OWNER_WINDOW: Cell<u64> = const { Cell::new(INACTIVE) };
}

/// Transparent system allocator with owner-thread measurement hooks.
///
/// Installing this type as `#[global_allocator]` does not start accounting;
/// callers must own a [`MeasurementWindow`].
pub struct TrackingAllocator;

// SAFETY: every operation delegates the original pointer/layout contract to
// `System`. Accounting observes only successful operations and never changes
// the pointer, layout, or allocation lifetime.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let token = active_token();
        // SAFETY: `layout` is forwarded unchanged from the caller.
        let pointer = unsafe { System.alloc(layout) };
        record_allocation(token, pointer, layout.size());
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let token = active_token();
        // SAFETY: `layout` is forwarded unchanged from the caller.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        record_allocation(token, pointer, layout.size());
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let token = active_token();
        // SAFETY: the caller promises that `pointer` and `layout` identify a
        // live allocation from this allocator.
        unsafe { System.dealloc(pointer, layout) };
        record_deallocation(token, pointer, layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let token = active_token();
        // SAFETY: the caller promises the original allocation contract and
        // `System` receives the values unchanged.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        record_reallocation(token, replacement, layout.size(), new_size);
        replacement
    }
}

/// Starts one exclusive owner-thread accounting window.
///
/// The process-wide claim prevents overlapping measurements, while the
/// thread-local token ensures unrelated runtime or test-harness threads do
/// not contaminate the owner-loop result.
pub fn start_measurement() -> Result<MeasurementWindow, MeasurementError> {
    ACTIVE_WINDOW
        .compare_exchange(INACTIVE, INITIALIZING, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| MeasurementError::AlreadyActive)?;
    reset_counters();
    let token = next_token();
    set_owner_window(token);
    ACTIVE_WINDOW.store(token, Ordering::SeqCst);
    Ok(MeasurementWindow {
        token,
        paused: false,
        stopped: false,
        not_send: PhantomData,
    })
}

/// Exclusive take-once window token. Dropping an unstopped window disables
/// accounting so a failed benchmark cannot poison the next invocation. The
/// token is deliberately `!Send`: measurement ownership cannot move away from
/// the thread whose allocations it records.
pub struct MeasurementWindow {
    token: u64,
    paused: bool,
    stopped: bool,
    not_send: PhantomData<Rc<()>>,
}

impl MeasurementWindow {
    /// Returns a point-in-time snapshot without closing the owned window.
    ///
    /// This is useful for comparing repeated terminal baselines inside one
    /// exclusive measurement. A paused window may also be inspected; its
    /// counters remain frozen until [`Self::resume`] succeeds.
    pub fn checkpoint(&self) -> Result<AllocationSnapshot, MeasurementError> {
        let expected = if self.paused {
            INITIALIZING
        } else {
            self.token
        };
        if ACTIVE_WINDOW.load(Ordering::SeqCst) != expected || owner_window() != expected {
            return Err(MeasurementError::WindowLost);
        }
        Ok(snapshot())
    }

    /// Temporarily excludes fixture construction or parsing while retaining
    /// exclusive ownership and all counters accumulated so far.
    pub fn pause(&mut self) -> Result<(), MeasurementError> {
        if self.paused {
            return Err(MeasurementError::AlreadyPaused);
        }
        ACTIVE_WINDOW
            .compare_exchange(self.token, INITIALIZING, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| MeasurementError::WindowLost)?;
        set_owner_window(INITIALIZING);
        self.paused = true;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), MeasurementError> {
        if !self.paused {
            return Err(MeasurementError::NotPaused);
        }
        set_owner_window(self.token);
        if ACTIVE_WINDOW
            .compare_exchange(INITIALIZING, self.token, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            set_owner_window(INITIALIZING);
            return Err(MeasurementError::WindowLost);
        }
        self.paused = false;
        Ok(())
    }

    pub fn stop(mut self) -> Result<AllocationSnapshot, MeasurementError> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> Result<AllocationSnapshot, MeasurementError> {
        let expected = if self.paused {
            INITIALIZING
        } else {
            self.token
        };
        ACTIVE_WINDOW
            .compare_exchange(expected, INACTIVE, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| MeasurementError::WindowLost)?;
        set_owner_window(INACTIVE);
        self.stopped = true;
        Ok(snapshot())
    }
}

impl Drop for MeasurementWindow {
    fn drop(&mut self) {
        if !self.stopped {
            let expected = if self.paused {
                INITIALIZING
            } else {
                self.token
            };
            let _ = ACTIVE_WINDOW.compare_exchange(
                expected,
                INACTIVE,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            set_owner_window(INACTIVE);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocationSnapshot {
    pub allocation_calls: u64,
    pub allocated_bytes: u64,
    pub deallocation_calls: u64,
    pub deallocated_bytes: u64,
    pub live_bytes_delta: i64,
    pub peak_live_bytes_delta: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementError {
    AlreadyActive,
    AlreadyPaused,
    NotPaused,
    WindowLost,
}

impl fmt::Display for MeasurementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive => formatter.write_str("an allocation measurement is active"),
            Self::AlreadyPaused => formatter.write_str("allocation measurement is already paused"),
            Self::NotPaused => formatter.write_str("allocation measurement is not paused"),
            Self::WindowLost => formatter.write_str("allocation measurement ownership was lost"),
        }
    }
}

impl std::error::Error for MeasurementError {}

fn active_token() -> Option<u64> {
    let token = owner_window();
    (token >= FIRST_TOKEN && ACTIVE_WINDOW.load(Ordering::Relaxed) == token).then_some(token)
}

fn window_still_active(token: Option<u64>) -> bool {
    token.is_some_and(|token| ACTIVE_WINDOW.load(Ordering::Relaxed) == token)
}

fn record_allocation(token: Option<u64>, pointer: *mut u8, bytes: usize) {
    if pointer.is_null() || !window_still_active(token) {
        return;
    }
    ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    add_live(bytes as i64);
}

fn record_deallocation(token: Option<u64>, pointer: *mut u8, bytes: usize) {
    if pointer.is_null() || !window_still_active(token) {
        return;
    }
    DEALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
    DEALLOCATED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    add_live(-(bytes as i64));
}

fn record_reallocation(
    token: Option<u64>,
    replacement: *mut u8,
    old_bytes: usize,
    new_bytes: usize,
) {
    if replacement.is_null() || !window_still_active(token) {
        return;
    }
    ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED_BYTES.fetch_add(new_bytes as u64, Ordering::Relaxed);
    DEALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
    DEALLOCATED_BYTES.fetch_add(old_bytes as u64, Ordering::Relaxed);
    add_live((new_bytes as i64).saturating_sub(old_bytes as i64));
}

fn add_live(delta: i64) {
    let live = LIVE_BYTES.fetch_add(delta, Ordering::Relaxed) + delta;
    let mut peak = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    while live > peak {
        match PEAK_LIVE_BYTES.compare_exchange_weak(
            peak,
            live,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

fn reset_counters() {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    DEALLOCATION_CALLS.store(0, Ordering::Relaxed);
    DEALLOCATED_BYTES.store(0, Ordering::Relaxed);
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(0, Ordering::Relaxed);
}

fn snapshot() -> AllocationSnapshot {
    AllocationSnapshot {
        allocation_calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        deallocation_calls: DEALLOCATION_CALLS.load(Ordering::Relaxed),
        deallocated_bytes: DEALLOCATED_BYTES.load(Ordering::Relaxed),
        live_bytes_delta: LIVE_BYTES.load(Ordering::Relaxed),
        peak_live_bytes_delta: PEAK_LIVE_BYTES.load(Ordering::Relaxed).max(0) as u64,
    }
}

fn next_token() -> u64 {
    loop {
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        if token >= FIRST_TOKEN {
            return token;
        }
    }
}

fn owner_window() -> u64 {
    OWNER_WINDOW.with(Cell::get)
}

fn set_owner_window(token: u64) {
    OWNER_WINDOW.with(|window| window.set(token));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;
    use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn counts_successful_alloc_zeroed_dealloc_and_realloc_exactly() {
        let _guard = TEST_LOCK.lock().expect("allocator test lock");
        let allocator = TrackingAllocator;
        let original = Layout::from_size_align(32, 8).expect("layout");
        let window = start_measurement().expect("exclusive window");
        // SAFETY: each successful allocation is paired with its exact current
        // layout and is released once.
        unsafe {
            let first = allocator.alloc(original);
            assert!(!first.is_null());
            let replacement = allocator.realloc(first, original, 64);
            assert!(!replacement.is_null());
            allocator.dealloc(
                replacement,
                Layout::from_size_align(64, 8).expect("replacement layout"),
            );
            let zeroed = allocator.alloc_zeroed(original);
            assert!(!zeroed.is_null());
            allocator.dealloc(zeroed, original);
        }
        let observed = window.stop().expect("owned window");
        assert_eq!(observed.allocation_calls, 3);
        assert_eq!(observed.allocated_bytes, 32 + 64 + 32);
        assert_eq!(observed.deallocation_calls, 3);
        assert_eq!(observed.deallocated_bytes, 32 + 64 + 32);
        assert_eq!(observed.live_bytes_delta, 0);
        assert!(observed.peak_live_bytes_delta >= 64);
    }

    #[test]
    fn rejects_overlapping_windows_and_drop_releases_ownership() {
        let _guard = TEST_LOCK.lock().expect("allocator test lock");
        let first = start_measurement().expect("first window");
        assert!(matches!(
            start_measurement(),
            Err(MeasurementError::AlreadyActive)
        ));
        drop(first);
        let second = start_measurement().expect("drop released window");
        second.stop().expect("second window");
    }

    #[test]
    fn null_results_are_not_counted() {
        let _guard = TEST_LOCK.lock().expect("allocator test lock");
        let window = start_measurement().expect("exclusive window");
        let token = active_token();
        record_allocation(token, ptr::null_mut(), 64);
        record_reallocation(token, ptr::null_mut(), 64, 128);
        record_deallocation(token, ptr::null_mut(), 64);
        assert_eq!(
            window.stop().expect("owned window"),
            AllocationSnapshot::default()
        );
    }

    #[test]
    fn pause_excludes_work_without_releasing_exclusive_window() {
        let _guard = TEST_LOCK.lock().expect("allocator test lock");
        let allocator = TrackingAllocator;
        let layout = Layout::from_size_align(24, 8).expect("layout");
        let mut window = start_measurement().expect("exclusive window");
        window.pause().expect("pause");
        assert!(matches!(
            start_measurement(),
            Err(MeasurementError::AlreadyActive)
        ));
        // SAFETY: this excluded allocation is checked and released once.
        unsafe {
            let pointer = allocator.alloc(layout);
            assert!(!pointer.is_null());
            allocator.dealloc(pointer, layout);
        }
        window.resume().expect("resume");
        assert_eq!(
            window.stop().expect("owned window"),
            AllocationSnapshot::default()
        );
    }

    #[test]
    fn checkpoint_preserves_window_and_observes_cumulative_state() {
        let _guard = TEST_LOCK.lock().expect("allocator test lock");
        let allocator = TrackingAllocator;
        let layout = Layout::from_size_align(40, 8).expect("layout");
        let mut window = start_measurement().expect("exclusive window");
        // SAFETY: the allocation is checked and released exactly once.
        unsafe {
            let pointer = allocator.alloc(layout);
            assert!(!pointer.is_null());
            let first = window.checkpoint().expect("active checkpoint");
            assert_eq!(first.allocation_calls, 1);
            assert_eq!(first.live_bytes_delta, 40);
            window.pause().expect("pause");
            assert_eq!(
                window.checkpoint().expect("paused checkpoint"),
                first,
                "paused accounting must preserve the cumulative snapshot"
            );
            window.resume().expect("resume");
            allocator.dealloc(pointer, layout);
        }
        let terminal = window.checkpoint().expect("terminal checkpoint");
        assert_eq!(terminal.allocation_calls, 1);
        assert_eq!(terminal.live_bytes_delta, 0);
        assert_eq!(window.stop().expect("stop"), terminal);
    }

    #[test]
    fn deallocation_of_pre_window_memory_reports_signed_live_delta() {
        let _guard = TEST_LOCK.lock().expect("allocator test lock");
        let allocator = TrackingAllocator;
        let layout = Layout::from_size_align(16, 8).expect("layout");
        // SAFETY: the allocation is checked and released exactly once.
        unsafe {
            let pointer = allocator.alloc(layout);
            assert!(!pointer.is_null());
            let window = start_measurement().expect("exclusive window");
            allocator.dealloc(pointer, layout);
            let observed = window.stop().expect("owned window");
            assert_eq!(observed.allocation_calls, 0);
            assert_eq!(observed.deallocation_calls, 1);
            assert_eq!(observed.live_bytes_delta, -16);
            assert_eq!(observed.peak_live_bytes_delta, 0);
        }
    }

    #[test]
    fn background_thread_allocations_do_not_contaminate_owner_measurement() {
        let _guard = TEST_LOCK.lock().expect("allocator test lock");
        let state = Arc::new(AtomicU8::new(0));
        let worker_state = Arc::clone(&state);
        let worker = std::thread::spawn(move || {
            while worker_state.load(AtomicOrdering::Acquire) != 1 {
                std::hint::spin_loop();
            }
            let allocator = TrackingAllocator;
            let layout = Layout::from_size_align(72, 8).expect("layout");
            // SAFETY: the worker releases its successful allocation exactly
            // once with the original layout.
            unsafe {
                let pointer = allocator.alloc(layout);
                assert!(!pointer.is_null());
                allocator.dealloc(pointer, layout);
            }
            worker_state.store(2, AtomicOrdering::Release);
        });

        let window = start_measurement().expect("exclusive owner-thread window");
        state.store(1, AtomicOrdering::Release);
        while state.load(AtomicOrdering::Acquire) != 2 {
            std::hint::spin_loop();
        }
        assert_eq!(
            window.stop().expect("owner-thread window"),
            AllocationSnapshot::default()
        );
        worker.join().expect("background allocator worker");
    }
}
