//! Atomic immutable snapshot support for the read/dirty containers.
//!
//! The containers follow a Go `sync.Map`-style architecture:
//!
//! * `read` — an immutable snapshot of the data, atomically published via an
//!   atomic pointer. Readers load it lock-free and take references into it.
//! * `dirty` — the canonical, mutable data, guarded by a mutex. Writers mutate
//!   it and lazily publish a fresh immutable snapshot.
//!
//! Snapshots are immutable and *retired* (kept alive) until the owner is
//! dropped, so references handed out by readers stay valid even after newer
//! snapshots are published and the underlying data is mutated. This is the
//! price of a `get(&self) -> &V` API on a concurrently mutable container.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicPtr, Ordering};

/// An atomically published, immutable snapshot of `T`.
///
/// Old snapshots are retired as raw pointers, never freed, so any `&T` handed
/// out by [`AtomicSnapshot::load`] remains valid for as long as the owner of
/// this snapshot lives. Storage is reclaimed (with `Box::from_raw`, at a point
/// where no reader can possibly observe it any more) when the owner is
/// dropped.
pub(crate) struct AtomicSnapshot<T> {
    read: AtomicPtr<T>,
    retired: UnsafeCell<Vec<*mut T>>,
}

// Safety: every pointer stored in `read` points to a fully-initialised `Box`
// whose contents are never mutated after publication. `publish` swaps in a new
// `Box` and `retired` keeps the old pointers alive (and immutable) until the
// owner is dropped, so concurrent `load` callers only ever observe live
// snapshots. Retired snapshots are stored as raw pointers so that retiring
// never re-tags (reclaims) an allocation that readers may still be using.
unsafe impl<T: Send> Send for AtomicSnapshot<T> {}
unsafe impl<T: Sync> Sync for AtomicSnapshot<T> {}

impl<T> AtomicSnapshot<T> {
    /// Create a snapshot holder with `initial` as the current snapshot.
    pub fn new(initial: T) -> Self {
        Self {
            read: AtomicPtr::new(Box::into_raw(Box::new(initial))),
            retired: UnsafeCell::new(Vec::new()),
        }
    }

    /// Lock-free load of the current snapshot.
    #[inline]
    pub fn load(&self) -> &T {
        // Acquire pairs with the release store in `publish`, making the
        // fully-initialised snapshot visible to readers.
        unsafe { &*self.read.load(Ordering::Acquire) }
    }

    /// Publish `new_snapshot` as the current one, retiring the previous.
    ///
    /// The caller must hold the owner's write lock (or have exclusive access).
    /// The previous snapshot is retired as a raw pointer and only reclaimed in
    /// `Drop`, so references already handed out stay valid.
    pub fn publish(&self, new_snapshot: T) {
        let new_ptr = Box::into_raw(Box::new(new_snapshot));
        let old = self.read.swap(new_ptr, Ordering::AcqRel);
        unsafe {
            (*self.retired.get()).push(old);
        }
    }

    /// Free every snapshot. Only safe when no reader can observe this holder
    /// any more (the owner is being dropped).
    fn release(&self) {
        unsafe {
            let retired = std::mem::take(&mut *self.retired.get());
            for p in retired {
                drop(Box::from_raw(p));
            }
            let current = self.read.load(Ordering::Relaxed);
            drop(Box::from_raw(current));
        }
    }
}

impl<T> Drop for AtomicSnapshot<T> {
    fn drop(&mut self) {
        self.release();
    }
}
