//! Value-entry indirection for the read/dirty containers (Go `sync.Map`-style
//! shared entries).
//!
//! Every slot in a container is an `Arc<Entry<V>>` shared between the `read`
//! snapshot and the `dirty` map. `Entry` holds an atomic pointer to the
//! current value, so updates replace the pointer in place (O(1), no snapshot
//! rebuild) and readers always observe the latest value without taking the
//! lock. Replaced values are *retired* (kept alive) so in-flight `&V`
//! references stay valid until the owner is dropped.

use std::cell::UnsafeCell;
use std::fmt::{Debug, Display, Formatter};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

/// A heap-allocated, atomically replaceable value slot.
pub(crate) struct Entry<V> {
    value: AtomicPtr<V>,
}

// Safety: the pointee is immutable once published and is only swapped out
// atomically; retired pointees are kept alive by the owner's `Retired` list.
unsafe impl<V: Send> Send for Entry<V> {}
unsafe impl<V: Sync> Sync for Entry<V> {}

impl<V> Entry<V> {
    pub fn new(value: V) -> Self {
        Self {
            value: AtomicPtr::new(Box::into_raw(Box::new(value))),
        }
    }

    /// Lock-free load of the current value.
    #[inline]
    pub fn load(&self) -> &V {
        // Acquire pairs with the release store in `swap`, so the caller sees
        // the fully-initialised value.
        unsafe { &*self.value.load(Ordering::Acquire) }
    }

    /// Mutable access to the value. Only valid while this entry is uniquely
    /// owned (no snapshot shares it).
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        unsafe { &mut **self.value.get_mut() }
    }

    /// Replace the value, returning the pointer to the previous one. The
    /// caller must keep the old pointer alive (see [`Retired`]).
    pub fn swap(&self, value: V) -> *mut V {
        self.value.swap(Box::into_raw(Box::new(value)), Ordering::AcqRel)
    }

    /// Extract the owned value (for consuming APIs). Leaves a null pointer.
    pub fn take(&self) -> V {
        let p = self.value.swap(ptr::null_mut(), Ordering::AcqRel);
        assert!(!p.is_null(), "entry already taken");
        unsafe { *Box::from_raw(p) }
    }
}

impl<V> Drop for Entry<V> {
    fn drop(&mut self) {
        let p = self.value.load(Ordering::Relaxed);
        if !p.is_null() {
            drop(unsafe { Box::from_raw(p) });
        }
    }
}

impl<V: Debug> Debug for Entry<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.load().fmt(f)
    }
}

impl<V: Display> Display for Entry<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.load().fmt(f)
    }
}

/// Value boxes that were swapped out of entries. They must stay alive until
/// the owner is dropped, because in-flight `&V` references may still point
/// into them.
pub(crate) struct Retired<V> {
    inner: UnsafeCell<Vec<*mut V>>,
}

// Safety: `push` is only ever called while holding the owner's write lock, and
// the contents are only freed when the owner is dropped.
unsafe impl<V: Send> Send for Retired<V> {}
unsafe impl<V: Sync> Sync for Retired<V> {}

impl<V> Retired<V> {
    pub fn new() -> Self {
        Self {
            inner: UnsafeCell::new(Vec::new()),
        }
    }

    pub fn push(&self, p: *mut V) {
        unsafe {
            (*self.inner.get()).push(p);
        }
    }
}

impl<V> Default for Retired<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> Drop for Retired<V> {
    fn drop(&mut self) {
        unsafe {
            let retired = std::mem::take(&mut *self.inner.get());
            for p in retired {
                drop(Box::from_raw(p));
            }
        }
    }
}
