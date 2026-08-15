pub mod map_btree;
pub mod map_hash;
pub mod map_index;
pub mod vec;
pub mod wg;

pub mod duration;

use parking_lot::{Mutex, MutexGuard};
use std::boxed::Box;
use std::cell::{Cell, RefCell};
use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Per-thread reader slots.
///
/// Every thread that reads a container remembers the location of its private
/// reader counter in that container's registry (a `Box<AtomicUsize>` that is
/// owned by the container and never moves). The counter is only written by its
/// owning thread, so concurrent readers touch their own cache line and never
/// contend with each other; writers scan the registry to know when all readers
/// are gone. Entries for dropped containers are simply never used again (each
/// container has a unique `id`, so address reuse cannot alias a stale entry).
struct ReaderSlots {
    // Fast path: most threads read only one container, so cache the last
    // (container id -> counter) pair to avoid scanning the vector every read.
    last: Cell<(usize, *const AtomicUsize)>,
    all: RefCell<Vec<(usize, *const AtomicUsize)>>,
}

thread_local! {
    static SLOTS: ReaderSlots = ReaderSlots {
        last: Cell::new((usize::MAX, std::ptr::null())),
        all: RefCell::new(Vec::new()),
    };
}

/// Returns (and lazily registers) the current thread's reader counter for the
/// container identified by `id`, adding it to `registry` on first use. The
/// returned reference is valid for as long as the registry (i.e. the
/// container) lives.
pub(crate) fn reader_count_for<'a>(
    id: usize,
    registry: &'a Mutex<Vec<Box<AtomicUsize>>>,
) -> &'a AtomicUsize {
    SLOTS.with(|slots| {
        let (last_id, last_ptr) = slots.last.get();
        if last_id == id && !last_ptr.is_null() {
            // SAFETY: the counter lives in `registry`, which is borrowed for
            // 'a and never removes entries, so the Box address stays valid.
            return unsafe { &*last_ptr };
        }
        let mut all = slots.all.borrow_mut();
        if let Some((_, c)) = all.iter().find(|(k, _)| *k == id) {
            slots.last.set((id, *c));
            // SAFETY: the counter lives in `registry`, borrowed for 'a, and
            // registry entries are never removed, so the address stays valid.
            return unsafe { &**c };
        }
        let mut reg = registry.lock();
        reg.push(Box::new(AtomicUsize::new(0)));
        let ptr: *const AtomicUsize = &**reg.last().unwrap();
        all.push((id, ptr));
        slots.last.set((id, ptr));
        // SAFETY: the newly pushed Box is in `registry` (borrowed for 'a) and
        // never moves, so this reference stays valid for 'a.
        unsafe { &*ptr }
    })
}

/// Unique id source for containers, so a thread-local slot can never alias a
/// different container that happens to reuse the same memory address.
pub(crate) static CONTAINER_ID: AtomicUsize = AtomicUsize::new(0);

/// An RAII read guard returned by the `get` methods of the synchronous
/// containers (`SyncHashMap`, `SyncBtreeMap`, `SyncVec`, `SyncIndexMap`).
///
/// Reading the value is lock-free and contention-free: the guard only holds a
/// reader slot in the calling thread's private counter (writers wait for all
/// threads' counters to drain before mutating), so the pointed-to value can
/// never be invalidated or raced while the guard is alive. It is not `Send`:
/// it must be dropped on the same thread that created it.
pub struct ReadGuard<'a, V> {
    count: &'a AtomicUsize,
    value: &'a V,
    _not_send: PhantomData<*const ()>,
}

impl<'a, V> ReadGuard<'a, V> {
    #[inline]
    pub(crate) fn new(count: &'a AtomicUsize, value: &'a V) -> Self {
        ReadGuard {
            count,
            value,
            _not_send: PhantomData,
        }
    }
}

impl<'a, V> Deref for ReadGuard<'a, V> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, V> Drop for ReadGuard<'a, V> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Release);
    }
}

impl<'a, V: Debug> Debug for ReadGuard<'a, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self.value, f)
    }
}

impl<'a, V: Display> Display for ReadGuard<'a, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.value, f)
    }
}

impl<'a, V: PartialEq> PartialEq for ReadGuard<'a, V> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<'a, V: Eq> Eq for ReadGuard<'a, V> {}

impl<'a, V: PartialEq> PartialEq<V> for ReadGuard<'a, V> {
    fn eq(&self, other: &V) -> bool {
        **self == *other
    }
}

impl<'a, V: PartialEq> PartialEq<&V> for ReadGuard<'a, V> {
    fn eq(&self, other: &&V) -> bool {
        **self == **other
    }
}

/// A read guard for whole-container access (`iter`, `dirty_ref`, ...).
///
/// Reading is lock-free and contention-free; the guard only pins a reader slot
/// in the calling thread's private counter. It is not `Send`: it must be
/// dropped on the same thread that created it.
pub struct ReadMapGuard<'a, C> {
    count: &'a AtomicUsize,
    value: &'a C,
    _not_send: PhantomData<*const ()>,
}

impl<'a, C> ReadMapGuard<'a, C> {
    #[inline]
    pub(crate) fn new(count: &'a AtomicUsize, value: &'a C) -> Self {
        ReadMapGuard {
            count,
            value,
            _not_send: PhantomData,
        }
    }
}

impl<'a, C> Deref for ReadMapGuard<'a, C> {
    type Target = C;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, C> Drop for ReadMapGuard<'a, C> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Release);
    }
}

impl<'a, C: Debug> Debug for ReadMapGuard<'a, C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self.value, f)
    }
}

impl<'a, C: Display> Display for ReadMapGuard<'a, C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.value, f)
    }
}

/// Internal RAII token for the write path: holds the writer mutex and keeps
/// the `writing` flag set until dropped, so readers know a writer is active.
pub(crate) struct WriteLock<'a> {
    _lock: MutexGuard<'a, ()>,
    writing: &'a AtomicBool,
}

impl<'a> WriteLock<'a> {
    #[inline]
    pub(crate) fn new(_lock: MutexGuard<'a, ()>, writing: &'a AtomicBool) -> Self {
        WriteLock { _lock, writing }
    }
}

impl<'a> Drop for WriteLock<'a> {
    fn drop(&mut self) {
        self.writing.store(false, Ordering::SeqCst);
    }
}

/// An RAII write guard returned by the `get_mut` methods of the synchronous
/// containers (`SyncHashMap`, `SyncBtreeMap`, `SyncVec`, `SyncIndexMap`).
///
/// It holds the writer lock (and the `writing` flag) until dropped, so no
/// reader or writer can touch the value while the guard is alive.
pub struct WriteGuard<'a, V> {
    _w: WriteLock<'a>,
    value: &'a mut V,
}

impl<'a, V> WriteGuard<'a, V> {
    #[inline]
    pub(crate) fn new(_w: WriteLock<'a>, value: &'a mut V) -> Self {
        WriteGuard { _w, value }
    }
}

impl<'a, V> Deref for WriteGuard<'a, V> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &*self.value
    }
}

impl<'a, V> DerefMut for WriteGuard<'a, V> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.value
    }
}

impl<'a, V: Debug> Debug for WriteGuard<'a, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&*self.value, f)
    }
}

impl<'a, V: Display> Display for WriteGuard<'a, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&*self.value, f)
    }
}

impl<'a, V: PartialEq> PartialEq for WriteGuard<'a, V> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<'a, V: Eq> Eq for WriteGuard<'a, V> {}

pub use duration::*;
pub use map_btree::SyncBtreeMap;
pub use map_hash::SyncHashMap;
pub use map_index::SyncIndexMap;
pub use vec::*;
pub use wg::*;
