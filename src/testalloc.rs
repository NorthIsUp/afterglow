//! The one counting global allocator, and the helper that reads it.
//!
//! A crate may declare exactly ONE `#[global_allocator]`. Every saver wants a
//! `render_never_allocates` test — the frame loop is the product, and a `Vec`
//! that grows inside `render` is a malloc per frame on a 500m budget — so the
//! allocator cannot live in whichever saver module happened to need it first.
//! It lives here and every saver's test calls `allocs_during`.
//!
//! Test-only: `#[cfg(test)]` on the `mod` in `main.rs` keeps it out of the
//! shipped binary entirely, so the release build allocates through `System`
//! with no counter in the path.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static N: Cell<usize> = const { Cell::new(0) };
}

pub struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        bump();
        unsafe { System.realloc(p, l, new) }
    }
}

/// `try_with`, because TLS is already gone during thread teardown and a panic
/// inside the allocator there aborts the process rather than failing a test.
fn bump() {
    let _ = N.try_with(|n| n.set(n.get() + 1));
}

/// Allocations counted on this thread so far. Use `allocs_during` unless you
/// need to bracket something a closure cannot wrap.
pub fn count() -> usize {
    N.with(|n| n.get())
}

/// Allocations made by `f` on this thread.
///
/// The count is per-thread, so `cargo test`'s parallel harness cannot make one
/// test see another's allocations — which is the whole reason this is a
/// `thread_local` and not an atomic.
pub fn allocs_during(f: impl FnOnce()) -> usize {
    let before = N.with(|n| n.get());
    f();
    N.with(|n| n.get()) - before
}

/// Declared here rather than in `main.rs` so the whole mechanism is one file:
/// the `mod` is `#[cfg(test)]`, so this static exists only in the test binary.
#[global_allocator]
static ALLOC: Counting = Counting;

#[cfg(test)]
mod tests {
    use super::*;

    /// A counter that never counts would make every `render_never_allocates`
    /// test in the crate pass vacuously — which is the failure this file is
    /// most likely to have and the one nothing else would catch.
    #[test]
    fn the_counter_counts_allocations_and_only_allocations() {
        assert_eq!(allocs_during(|| {}), 0);

        let n = allocs_during(|| {
            let v: Vec<u8> = Vec::with_capacity(64);
            std::hint::black_box(&v);
        });
        assert_eq!(n, 1, "one Vec allocation should count exactly once");

        // `vec![0; n]` takes `alloc_zeroed`, NOT `alloc`. Leaving that hook
        // unimplemented is the quiet way this counter goes blind — the default
        // impl forwards to `alloc` on the same allocator, so it would still
        // count here, but an allocator that overrides only `alloc` would not.
        let z = allocs_during(|| {
            let v: Vec<u32> = vec![0; 256];
            std::hint::black_box(&v);
        });
        assert_eq!(z, 1, "a zeroed allocation should count exactly once");

        // Writing into already-reserved capacity must NOT count: that is
        // precisely what a saver's render does, and a counter that flagged it
        // would make the tests unusable rather than merely weak.
        let mut v: Vec<u8> = Vec::with_capacity(1024);
        assert_eq!(
            allocs_during(|| {
                for i in 0..1024u32 {
                    v.push(i as u8);
                }
            }),
            0
        );
    }
}
