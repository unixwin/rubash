//! Feature-gated global allocation counter (`--features alloc-counter`).
//!
//! Wraps `std::alloc::System` and records allocation counts and byte totals
//! using relaxed atomics without performing any heap allocations during
//! counting.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

/// Snapshot of process-wide heap allocation counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocStats {
    pub alloc_count: u64,
    pub dealloc_count: u64,
    pub realloc_count: u64,
    pub alloc_bytes: u64,
    pub live_bytes: u64,
    pub peak_bytes: u64,
}

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static DEALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static REALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_BYTES: AtomicU64 = AtomicU64::new(0);

struct CountingAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

#[inline]
fn record_alloc(size: u64) {
    ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
    ALLOC_BYTES.fetch_add(size, Ordering::Relaxed);
    let prev_live = LIVE_BYTES.fetch_add(size, Ordering::Relaxed);
    let new_live = prev_live.saturating_add(size);
    PEAK_BYTES.fetch_max(new_live, Ordering::Relaxed);
}

#[inline]
fn sub_live_bytes(size: u64) {
    let mut cur = LIVE_BYTES.load(Ordering::Relaxed);
    loop {
        let next = cur.saturating_sub(size);
        match LIVE_BYTES.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size() as u64);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size() as u64);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        sub_live_bytes(layout.size() as u64);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let old_size = layout.size() as u64;
            let new_size_u64 = new_size as u64;
            REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size_u64, Ordering::Relaxed);
            if new_size_u64 >= old_size {
                let diff = new_size_u64 - old_size;
                let prev_live = LIVE_BYTES.fetch_add(diff, Ordering::Relaxed);
                PEAK_BYTES.fetch_max(prev_live.saturating_add(diff), Ordering::Relaxed);
            } else {
                sub_live_bytes(old_size - new_size_u64);
            }
        }
        new_ptr
    }
}

/// Return a snapshot of the current process-wide allocation counters.
pub fn alloc_stats() -> AllocStats {
    AllocStats {
        alloc_count: ALLOC_COUNT.load(Ordering::Relaxed),
        dealloc_count: DEALLOC_COUNT.load(Ordering::Relaxed),
        realloc_count: REALLOC_COUNT.load(Ordering::Relaxed),
        alloc_bytes: ALLOC_BYTES.load(Ordering::Relaxed),
        live_bytes: LIVE_BYTES.load(Ordering::Relaxed),
        peak_bytes: PEAK_BYTES.load(Ordering::Relaxed),
    }
}

/// Reset all allocation counters to zero.
pub fn reset_alloc_stats() {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    DEALLOC_COUNT.store(0, Ordering::Relaxed);
    REALLOC_COUNT.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_BYTES.store(0, Ordering::Relaxed);
}
