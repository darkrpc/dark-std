use indexmap::map::{
    IndexMap as Map, IntoIter as MapIntoIter, Iter as MapIter, IterMut as MapIterMut,
};
use parking_lot::Mutex;
use serde::{Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::{ReadGuard, ReadMapGuard, WriteGuard, WriteLock};

/// Read guard returned by [`SyncIndexMap::get`].
pub type IndexMapGet<'a, V> = ReadGuard<'a, V>;

/// Write guard returned by [`SyncIndexMap::get_mut`].
pub struct IndexMapRefMut<'a, K, V> {
    inner: WriteGuard<'a, V>,
    _k: PhantomData<&'a K>,
}

impl<'a, K, V> IndexMapRefMut<'a, K, V> {
    #[inline]
    pub(crate) fn new(inner: WriteGuard<'a, V>) -> Self {
        IndexMapRefMut {
            inner,
            _k: PhantomData,
        }
    }
}

impl<'a, K, V> Deref for IndexMapRefMut<'a, K, V> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, K, V> DerefMut for IndexMapRefMut<'a, K, V> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'a, K, V: Debug> Debug for IndexMapRefMut<'a, K, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&*self.inner, f)
    }
}

impl<'a, K, V: Display> Display for IndexMapRefMut<'a, K, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&*self.inner, f)
    }
}

impl<'a, K, V: PartialEq> PartialEq for IndexMapRefMut<'a, K, V> {
    fn eq(&self, other: &Self) -> bool {
        *self.inner == *other.inner
    }
}

impl<'a, K, V: Eq> Eq for IndexMapRefMut<'a, K, V> {}

/// Backwards-compatible alias kept for callers of
/// `dark_std::sync::map_index::HashMapRefMut`.
pub type HashMapRefMut<'a, K, V> = IndexMapRefMut<'a, K, V>;

/// Read iterator returned by [`SyncIndexMap::iter`].
pub struct IndexMapIter<'a, K, V> {
    count: &'a AtomicUsize,
    inner: MapIter<'a, K, V>,
    _not_send: PhantomData<*const ()>,
}

impl<'a, K, V> Drop for IndexMapIter<'a, K, V> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Release);
    }
}

impl<'a, K, V> Iterator for IndexMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// Write iterator returned by [`SyncIndexMap::iter_mut`].
pub struct IndexMapIterMut<'a, K, V> {
    _w: WriteLock<'a>,
    inner: MapIterMut<'a, K, V>,
}

impl<'a, K, V> Iterator for IndexMapIterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// this sync map used to many reader,writer less.space-for-time strategy
///
/// Reads are lock-free: `get`/`iter`/`dirty_ref`/`len`/`contains_key` only
/// register a reader slot with an atomic counter and then read the map without
/// any lock (readers never block each other and never touch a lock word).
/// Writes take a mutex, raise a `writing` flag and wait until all in-flight
/// readers are gone before mutating the map in place — O(1), no whole-container
/// copy and no `Clone` requirement on `K`/`V`.
///
/// # Deadlock note
/// A read guard makes writers wait until it is dropped. Do not call a write
/// method while a read/write guard is alive in the same scope: drop the guard
/// first (e.g. `drop(g)` before `insert`/`remove`/`get_mut`), otherwise the
/// writer waits for its own guard and deadlocks.
pub struct SyncIndexMap<K: Eq + Hash, V> {
    dirty: UnsafeCell<Map<K, V>>,
    write: Mutex<()>,
    id: usize,
    writing: AtomicBool,
    registry: Mutex<Vec<std::boxed::Box<AtomicUsize>>>,
}

// SAFETY: all writers hold `write` and wait for `readers` to drain before
// touching `dirty`; readers either see a consistent snapshot or retry while a
// writer is active, so concurrent access to `dirty` is race-free.
unsafe impl<K: Eq + Hash, V: Send> Send for SyncIndexMap<K, V> {}
unsafe impl<K: Eq + Hash, V: Sync> Sync for SyncIndexMap<K, V> {}

impl<K, V> SyncIndexMap<K, V>
where
    K: Eq + Hash,
{
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
            dirty: UnsafeCell::new(Map::new()),
            write: Mutex::new(()),
            id: super::CONTAINER_ID.fetch_add(1, Ordering::Relaxed),
            writing: AtomicBool::new(false),
            registry: Mutex::new(Vec::new()),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            dirty: UnsafeCell::new(Map::with_capacity(capacity)),
            write: Mutex::new(()),
            id: super::CONTAINER_ID.fetch_add(1, Ordering::Relaxed),
            writing: AtomicBool::new(false),
            registry: Mutex::new(Vec::new()),
        }
    }

    pub fn with_map(map: Map<K, V>) -> Self {
        Self {
            dirty: UnsafeCell::new(map),
            write: Mutex::new(()),
            id: super::CONTAINER_ID.fetch_add(1, Ordering::Relaxed),
            writing: AtomicBool::new(false),
            registry: Mutex::new(Vec::new()),
        }
    }

    pub fn insert(&self, k: K, v: V) -> Option<V> {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.insert(k, v)
    }

    pub fn insert_mut(&mut self, k: K, v: V) -> Option<V> {
        unsafe { &mut *self.dirty.get() }.insert(k, v)
    }

    pub fn remove(&self, k: &K) -> Option<V> {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.swap_remove(k)
    }

    pub fn remove_mut(&mut self, k: &K) -> Option<V> {
        unsafe { &mut *self.dirty.get() }.swap_remove(k)
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

    pub fn clear_mut(&mut self) {
        unsafe { &mut *self.dirty.get() }.clear();
    }

    pub fn shrink_to_fit(&self) {
        let _w = self.begin_write();
        unsafe { &mut *self.dirty.get() }.shrink_to_fit();
    }

    pub fn shrink_to_fit_mut(&mut self) {
        unsafe { &mut *self.dirty.get() }.shrink_to_fit()
    }

    pub fn from(map: Map<K, V>) -> Self
    where
        K: Eq + Hash,
    {
        Self::with_map(map)
    }

    /// Returns a read-guarded reference to the value corresponding to the key.
    ///
    /// The key may be any borrowed form of the map's key type, but
    /// [`Hash`] and [`Eq`] on the borrowed form *must* match those for
    /// the key type.
    ///
    /// The read is lock-free: it only registers a reader slot, so concurrent
    /// reads never block each other and never take a lock. Writers wait for
    /// the returned guard to be dropped before mutating the map.
    ///
    /// # Examples
    ///
    /// ```
    /// use dark_std::sync::{SyncIndexMap};
    ///
    /// let mut map = SyncIndexMap::new();
    /// map.insert_mut(1, "a");
    /// assert_eq!(*map.get(&1).unwrap(), "a");
    /// assert_eq!(map.get(&2).is_none(), true);
    /// ```
    #[inline]
    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<IndexMapGet<'_, V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let count = self.begin_read();
        let m = unsafe { &*self.dirty.get() };
        match m.get(k) {
            Some(v) => Some(ReadGuard::new(count, v)),
            None => {
                count.fetch_sub(1, Ordering::Release);
                None
            }
        }
    }

    /// Returns a write-guarded mutable reference to the value of the key.
    ///
    /// The guard holds the writer lock (writers are mutually exclusive and
    /// wait for in-flight readers) until it is dropped, so the mutable
    /// reference can never race with concurrent readers or writers. Drop it
    /// before calling another method from the same scope.
    #[inline]
    pub fn get_mut(&self, k: &K) -> Option<IndexMapRefMut<'_, K, V>> {
        let w = self.begin_write();
        let m = unsafe { &mut *self.dirty.get() };
        match m.get_mut(k) {
            Some(v) => Some(IndexMapRefMut::new(WriteGuard::new(w, v))),
            None => None,
        }
    }

    #[inline]
    pub fn contains_key<Q: ?Sized>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let count = self.begin_read();
        let b = unsafe { &*self.dirty.get() }.contains_key(k);
        count.fetch_sub(1, Ordering::Release);
        b
    }

    pub fn iter(&self) -> IndexMapIter<'_, K, V> {
        let count = self.begin_read();
        let m = unsafe { &*self.dirty.get() };
        IndexMapIter {
            count,
            inner: m.iter(),
            _not_send: PhantomData,
        }
    }

    pub fn iter_mut(&self) -> IndexMapIterMut<'_, K, V> {
        let w = self.begin_write();
        let m = unsafe { &mut *self.dirty.get() };
        IndexMapIterMut {
            _w: w,
            inner: m.iter_mut(),
        }
    }

    pub fn into_iter(self) -> MapIntoIter<K, V> {
        self.into_inner().into_iter()
    }

    pub fn dirty_ref(&self) -> ReadMapGuard<'_, Map<K, V>> {
        let count = self.begin_read();
        let m = unsafe { &*self.dirty.get() };
        ReadMapGuard::new(count, m)
    }

    pub fn into_inner(self) -> Map<K, V> {
        self.dirty.into_inner()
    }
}

impl<K, V> IntoIterator for SyncIndexMap<K, V>
where
    K: Eq + Hash,
{
    type Item = (K, V);
    type IntoIter = MapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<'a, K: Eq + Hash, V> IntoIterator for &'a SyncIndexMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = IndexMapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K: Eq + Hash, V> From<Map<K, V>> for SyncIndexMap<K, V> {
    fn from(arg: Map<K, V>) -> Self {
        Self::from(arg)
    }
}

impl<K, V> serde::Serialize for SyncIndexMap<K, V>
where
    K: Eq + Hash + Serialize,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.dirty_ref().serialize(serializer)
    }
}

impl<'de, K, V> serde::Deserialize<'de> for SyncIndexMap<K, V>
where
    K: Eq + Hash + serde::Deserialize<'de>,
    V: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let m = Map::deserialize(deserializer)?;
        Ok(Self::from(m))
    }
}

impl<K, V> Debug for SyncIndexMap<K, V>
where
    K: Eq + Hash + Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&*self.dirty_ref(), f)
    }
}

impl<K, V> Display for SyncIndexMap<K, V>
where
    K: Eq + Hash + Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&*self.dirty_ref(), f)
    }
}

impl<K: Clone + Eq + Hash, V: Clone> Clone for SyncIndexMap<K, V> {
    fn clone(&self) -> Self {
        let c = (*self.dirty_ref()).clone();
        SyncIndexMap::from(c)
    }
}

impl<K: Eq + Hash, V> Default for SyncIndexMap<K, V> {
    fn default() -> Self {
        SyncIndexMap::new()
    }
}
