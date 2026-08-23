use parking_lot::Mutex;
use serde::{Deserializer, Serialize, Serializer};
use std::cell::UnsafeCell;
use std::fmt::{Debug, Display, Formatter};
use std::slice::{Iter as SliceIter, IterMut as SliceIterMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::vec::IntoIter;

use super::{ReadGuard, ReadMapGuard, WriteGuard, WriteLock};

/// Read guard returned by [`SyncVec::get`].
pub type VecGet<'a, V> = ReadGuard<'a, V>;

/// Write guard returned by [`SyncVec::get_mut`].
pub type VecRefMut<'a, V> = WriteGuard<'a, V>;

/// Read iterator returned by [`SyncVec::iter`].
///
/// The iterator is `Send` (when `V: Sync`) and may be moved between threads:
/// the reader counter is a shared atomic owned by the container, so releasing
/// it from another thread (on drop) is safe.
pub struct VecIter<'a, V> {
    count: &'a AtomicUsize,
    inner: SliceIter<'a, V>,
}

impl<'a, V> Drop for VecIter<'a, V> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Release);
    }
}

impl<'a, V> Iterator for VecIter<'a, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// Write iterator returned by [`SyncVec::iter_mut`].
pub struct VecIterMut<'a, V> {
    _w: WriteLock<'a>,
    inner: SliceIterMut<'a, V>,
}

impl<'a, V> Iterator for VecIterMut<'a, V> {
    type Item = &'a mut V;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// An asynchronous vector that can be safely shared between threads.
///
/// Reads are lock-free: `get`/`iter`/`dirty_ref`/`len`/`contains` only
/// register a reader slot with an atomic counter and then read the vector
/// without any lock (readers never block each other and never touch a lock
/// word). Writes take a mutex, raise a `writing` flag and wait until all
/// in-flight readers are gone before mutating the vector in place (amortised
/// O(1) push, no whole-container copy).
///
/// # Deadlock note
/// A read guard makes writers wait until it is dropped. Do not call a write
/// method while a read/write guard is alive in the same scope: drop the guard
/// first (e.g. `drop(g)` before `push`/`remove`/`get_mut`), otherwise the
/// writer waits for its own guard and deadlocks.
pub struct SyncVec<V> {
    dirty: UnsafeCell<Vec<V>>,
    write: Mutex<()>,
    id: usize,
    writing: AtomicBool,
    registry: Mutex<Vec<std::boxed::Box<AtomicUsize>>>,
}

// SAFETY: all writers hold `write` and wait for `readers` to drain before
// touching `dirty`; readers either see a consistent snapshot or retry while a
// writer is active, so concurrent access to `dirty` is race-free.
unsafe impl<V: Send> Send for SyncVec<V> {}
unsafe impl<V: Sync> Sync for SyncVec<V> {}

impl<V> SyncVec<V> {
    #[inline]
    fn begin_read(&self) -> &AtomicUsize {
        // The counter lives in thread-local storage: concurrent readers only
        // touch their own cache line and never contend with each other. SeqCst
        // closes the store-buffering window with the writer's all-zero scan.
        let count = super::reader_count_for(self.id, &self.registry);
        loop {
            count.fetch_add(1, Ordering::SeqCst);
            if !self.writing.load(Ordering::SeqCst) {
                return count;
            }
            count.fetch_sub(1, Ordering::SeqCst);
            std::thread::yield_now();
        }
    }

    #[inline]
    fn begin_write(&self) -> WriteLock<'_> {
        let lock = self.write.lock();
        self.writing.store(true, Ordering::SeqCst);
        loop {
            let registry = self.registry.lock();
            let all_zero = registry.iter().all(|c| c.load(Ordering::SeqCst) == 0);
            if all_zero {
                break;
            }
            drop(registry);
            std::thread::yield_now();
        }
        WriteLock::new(lock, &self.writing)
    }

    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            dirty: UnsafeCell::new(Vec::new()),
            write: Mutex::new(()),
            id: super::CONTAINER_ID.fetch_add(1, Ordering::Relaxed),
            writing: AtomicBool::new(false),
            registry: Mutex::new(Vec::new()),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            dirty: UnsafeCell::new(Vec::with_capacity(capacity)),
            write: Mutex::new(()),
            id: super::CONTAINER_ID.fetch_add(1, Ordering::Relaxed),
            writing: AtomicBool::new(false),
            registry: Mutex::new(Vec::new()),
        }
    }

    pub fn with_vec(vec: Vec<V>) -> Self {
        Self {
            dirty: UnsafeCell::new(vec),
            write: Mutex::new(()),
            id: super::CONTAINER_ID.fetch_add(1, Ordering::Relaxed),
            writing: AtomicBool::new(false),
            registry: Mutex::new(Vec::new()),
        }
    }

    pub fn insert(&self, index: usize, v: V) -> Option<V> {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.insert(index, v);
        None
    }

    pub fn set(&self, index: usize, v: V) -> Option<V> {
        let _w = self.begin_write();
        let m = unsafe { &mut *self.dirty.get() };
        m[index] = v;
        None
    }

    pub fn push(&self, v: V) -> Option<V> {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.push(v);
        None
    }

    pub fn pushes(&self, arr: Vec<V>) -> Option<V> {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.extend(arr);
        None
    }

    pub fn push_mut(&mut self, v: V) -> Option<V> {
        unsafe { &mut *self.dirty.get() }.push(v);
        None
    }

    pub fn pop(&self) -> Option<V> {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.pop()
    }

    pub fn pop_mut(&mut self) -> Option<V> {
        unsafe { &mut *self.dirty.get() }.pop()
    }

    pub fn remove(&self, index: usize) -> Option<V> {
        let _w = self.begin_write();
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            Some(m.remove(index))
        } else {
            None
        }
    }

    pub fn remove_mut(&mut self, index: usize) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            Some(m.remove(index))
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        let count = self.begin_read();
        let n = unsafe { &*self.dirty.get() }.len();
        count.fetch_sub(1, Ordering::Release);
        n
    }

    pub fn is_empty(&self) -> bool {
        let count = self.begin_read();
        let b = unsafe { &*self.dirty.get() }.is_empty();
        count.fetch_sub(1, Ordering::Release);
        b
    }

    pub fn clear(&self) {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.clear();
    }

    pub fn shrink_to_fit(&self) {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.shrink_to_fit();
    }

    pub fn from(vec: Vec<V>) -> Self {
        Self::with_vec(vec)
    }

    /// Returns a read-guarded reference to the value at `index`.
    ///
    /// The read is lock-free: it only registers a reader slot, so concurrent
    /// reads never block each other and never take a lock. Writers wait for
    /// the returned guard to be dropped before mutating the vector.
    #[inline]
    pub fn get(&self, index: usize) -> Option<VecGet<'_, V>> {
        let count = self.begin_read();
        let m = unsafe { &*self.dirty.get() };
        match m.get(index) {
            Some(v) => Some(ReadGuard::new(count, v)),
            None => {
                count.fetch_sub(1, Ordering::Release);
                None
            }
        }
    }

    /// Returns a write-guarded mutable reference to the value at `index`.
    ///
    /// The guard holds the writer lock (writers are mutually exclusive and
    /// wait for in-flight readers) until it is dropped, so the mutable
    /// reference can never race with concurrent readers or writers. Drop it
    /// before calling another method from the same scope.
    #[inline]
    pub fn get_mut(&self, index: usize) -> Option<VecRefMut<'_, V>> {
        let w = self.begin_write();
        let m = unsafe { &mut *self.dirty.get() };
        match m.get_mut(index) {
            Some(v) => Some(WriteGuard::new(w, v)),
            None => None,
        }
    }

    #[inline]
    pub fn contains(&self, x: &V) -> bool
    where
        V: PartialEq,
    {
        let count = self.begin_read();
        let b = unsafe { &*self.dirty.get() }.contains(x);
        count.fetch_sub(1, Ordering::Release);
        b
    }

    pub fn iter(&self) -> VecIter<'_, V> {
        let count = self.begin_read();
        let m = unsafe { &*self.dirty.get() };
        VecIter {
            count,
            inner: m.iter(),
        }
    }

    pub fn iter_mut(&self) -> VecIterMut<'_, V> {
        let w = self.begin_write();
        let m = unsafe { &mut *self.dirty.get() };
        VecIterMut {
            _w: w,
            inner: m.iter_mut(),
        }
    }

    pub fn into_iter(self) -> IntoIter<V> {
        self.into_inner().into_iter()
    }

    pub fn dirty_ref(&self) -> ReadMapGuard<'_, Vec<V>> {
        let count = self.begin_read();
        let m = unsafe { &*self.dirty.get() };
        ReadMapGuard::new(count, m)
    }

    pub fn into_inner(self) -> Vec<V> {
        self.dirty.into_inner()
    }
}

impl<V> IntoIterator for SyncVec<V> {
    type Item = V;
    type IntoIter = IntoIter<V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<V> Serialize for SyncVec<V>
where
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.dirty_ref().serialize(serializer)
    }
}

impl<'de, V> serde::Deserialize<'de> for SyncVec<V>
where
    V: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let m = Vec::deserialize(deserializer)?;
        Ok(Self::from(m))
    }
}

impl<V> Debug for SyncVec<V>
where
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&*self.dirty_ref(), f)
    }
}

impl<V> Display for SyncVec<V>
where
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&*self.dirty_ref(), f)
    }
}

impl<V: PartialEq> PartialEq for SyncVec<V> {
    fn eq(&self, other: &Self) -> bool {
        (*self.dirty_ref()).eq(&*other.dirty_ref())
    }
}

impl<V: Clone> Clone for SyncVec<V> {
    fn clone(&self) -> Self {
        SyncVec::from(self.dirty_ref().to_vec())
    }
}

impl<V> Default for SyncVec<V> {
    fn default() -> Self {
        SyncVec::new()
    }
}

#[macro_export]
macro_rules! sync_vec {
    () => (
        $crate::sync::SyncVec::new()
    );
    ($elem:expr; $n:expr) => (
        $crate::sync::SyncVec::with_vec(vec![$elem;$n])
    );
    ($($x:expr),+ $(,)?) => (
        $crate::sync::SyncVec::with_vec(vec![$($x),+,])
    );
}
