use indexmap::map::{
    IndexMap as Map, IntoIter as MapIntoIter, Iter as MapIter, IterMut as MapIterMut,
};
use super::lock::{SyncLock, SyncLockGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::snapshot::AtomicSnapshot;

/// A concurrent IndexMap with a Go `sync.Map`-style read/dirty architecture:
///
/// - `read`: an immutable snapshot, atomically published. `get` / `iter` /
///   `Index` read it lock-free.
/// - `dirty`: the canonical, mutable map, guarded by `lock`. Every write goes
///   here and is lazily published into a fresh snapshot.
///
/// Snapshots are immutable and kept alive until the map is dropped, so
/// references returned by `get` stay valid even while the map is mutated.
/// Methods that publish a fresh snapshot require `K: Clone + V: Clone`.
/// Suitable for read-mostly workloads (many readers, few writers).
pub struct SyncIndexMap<K: Eq + Hash, V> {
    dirty: UnsafeCell<Map<K, V>>,
    lock: SyncLock,
    amended: AtomicBool,
    read: AtomicSnapshot<Map<K, V>>,
}

/// Safety: `dirty` is only ever accessed under `lock`; the `read` snapshot is
/// immutable once published and is kept alive until the map is dropped, so
/// references derived from it remain valid for the lifetime of `&self`.
unsafe impl<K: Eq + Hash, V> Send for SyncIndexMap<K, V> {}
unsafe impl<K: Eq + Hash, V> Sync for SyncIndexMap<K, V> {}

impl<K, V> std::ops::Index<&K> for SyncIndexMap<K, V>
where
    K: Eq + Hash,
    K: Clone,
    V: Clone,
{
    type Output = V;

    fn index(&self, index: &K) -> &Self::Output {
        self.get(index).expect("key not found")
    }
}

impl<K, V> SyncIndexMap<K, V>
where
    K: Eq + Hash,
{
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            dirty: UnsafeCell::new(Map::new()),
            lock: Default::default(),
            amended: AtomicBool::new(false),
            read: AtomicSnapshot::new(Map::new()),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            dirty: UnsafeCell::new(Map::with_capacity(capacity)),
            lock: Default::default(),
            amended: AtomicBool::new(false),
            read: AtomicSnapshot::new(Map::with_capacity(capacity)),
        }
    }

    pub fn with_map(map: Map<K, V>) -> Self {
        Self {
            read: AtomicSnapshot::new(Map::new()),
            dirty: UnsafeCell::new(map),
            lock: Default::default(),
            amended: AtomicBool::new(true),
        }
    }

    /// Publish the current `dirty` map as a fresh immutable snapshot.
    ///
    /// The caller must hold `lock` (or have exclusive `&mut` access).
    fn promote(&self)
    where
        K: Clone,
        V: Clone,
    {
        let dirty = unsafe { &*self.dirty.get() };
        self.read.publish(dirty.clone());
        // After publishing, `read` reflects `dirty`: nothing is pending.
        self.amended.store(false, Ordering::Release);
    }

    pub fn insert(&self, k: K, v: V) -> Option<V>
    where
        K: Clone,
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.insert(k, v);
        // Updating an existing key must refresh the snapshot, otherwise `get`
        // would keep serving the stale value from `read`.
        if r.is_some() {
            self.promote();
        } else {
            // New key: leave it for lazy promotion and mark `amended`.
            self.amended.store(true, Ordering::Release);
        }
        drop(g);
        r
    }

    pub fn insert_mut(&mut self, k: K, v: V) -> Option<V>
    where
        K: Clone,
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.insert(k, v);
        if r.is_some() {
            self.promote();
        } else {
            // New key: leave it for lazy promotion and mark `amended`.
            self.amended.store(true, Ordering::Release);
        }
        r
    }

    pub fn remove(&self, k: &K) -> Option<V>
    where
        K: Clone,
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.swap_remove(k);
        if r.is_some() {
            // Refresh the snapshot so `get` no longer serves the removed key.
            self.promote();
        }
        drop(g);
        r
    }

    pub fn remove_mut(&mut self, k: &K) -> Option<V>
    where
        K: Clone,
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.swap_remove(k);
        if r.is_some() {
            self.promote();
        }
        r
    }

    pub fn len(&self) -> usize {
        if !self.amended.load(Ordering::Acquire) {
            return self.read.load().len();
        }
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).len() };
        drop(g);
        r
    }

    pub fn is_empty(&self) -> bool {
        if !self.amended.load(Ordering::Acquire) {
            return self.read.load().is_empty();
        }
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).is_empty() };
        drop(g);
        r
    }

    pub fn clear(&self)
    where
        K: Clone,
        V: Clone,
    {
        let g = self.lock.lock();
        unsafe { (&mut *self.dirty.get()).clear() };
        self.promote();
        drop(g);
    }

    pub fn clear_mut(&mut self)
    where
        K: Clone,
        V: Clone,
    {
        unsafe { (&mut *self.dirty.get()).clear() };
        self.promote();
    }

    pub fn shrink_to_fit(&self) {
        let g = self.lock.lock();
        unsafe { (&mut *self.dirty.get()).shrink_to_fit() };
        drop(g);
    }

    pub fn shrink_to_fit_mut(&mut self) {
        unsafe { (&mut *self.dirty.get()).shrink_to_fit() }
    }

    pub fn from(map: Map<K, V>) -> Self
    where
        K: Eq + Hash,
    {
        let s = Self::with_map(map);
        s
    }

    /// Returns a reference to the value corresponding to the key.
    ///
    /// The key may be any borrowed form of the map's key type, but
    /// [`Hash`] and [`Eq`] on the borrowed form *must* match those for
    /// the key type.
    ///
    /// Reads are lock-free: the value is served from the immutable `read`
    /// snapshot. If the key was written to `dirty` since the last snapshot was
    /// published, a fresh snapshot is published first and the value is served
    /// from it, so the returned reference always points into immutable,
    /// retained storage.
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
    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q> + Clone,
        Q: Hash + Eq,
        V: Clone,
    {
        if let Some(v) = self.read.load().get(k) {
            return Some(v);
        }
        // If nothing was written to `dirty` since the last snapshot was
        // published, a snapshot miss is a real miss: no lock is needed.
        if !self.amended.load(Ordering::Acquire) {
            return None;
        }
        // Snapshot miss: the key may have been written to `dirty` without a
        // snapshot refresh yet (lazy promotion). Publish a fresh snapshot and
        // serve from it so the reference points into immutable, retained
        // storage instead of the lock-guarded `dirty` map.
        let g = self.lock.lock();
        let found = unsafe { (&*self.dirty.get()).contains_key(k) };
        if found {
            self.promote();
        }
        drop(g);
        if found {
            self.read.load().get(k)
        } else {
            None
        }
    }

    /// Returns a mutable handle to the value for `k`, implemented with
    /// copy-on-write: the value is cloned, the handle mutates the clone, and
    /// the result is written back (and published into a fresh snapshot) when
    /// the handle is dropped. The returned reference stays valid as long as
    /// the handle is held; concurrent readers may observe the pre-mutation
    /// value until the handle is dropped.
    #[inline]
    pub fn get_mut(&self, k: &K) -> Option<HashMapRefMut<'_, K, V>>
    where
        K: Hash + Eq + Clone,
        V: Clone,
    {
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let value = dirty.get(k)?.clone();
        drop(g);
        Some(HashMapRefMut {
            k: k.clone(),
            m: self,
            value: Some(value),
        })
    }

    #[inline]
    pub fn contains_key(&self, x: &K) -> bool
    where
        K: PartialEq,
    {
        if self.read.load().contains_key(x) {
            return true;
        }
        if !self.amended.load(Ordering::Acquire) {
            return false;
        }
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).contains_key(x) };
        drop(g);
        r
    }

    /// Iterate over the current contents. A fresh snapshot is published first,
    /// so all entries written so far are visible.
    pub fn iter(&self) -> MapIter<'_, K, V>
    where
        K: Clone,
        V: Clone,
    {
        let g = self.lock.lock();
        self.promote();
        drop(g);
        self.read.load().iter()
    }

    pub fn iter_mut(&self) -> HashIterMut<'_, K, V>
    where
        K: Clone,
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        HashIterMut {
            m: self,
            _g: self.lock.lock(),
            inner: Some(m.iter_mut()),
        }
    }

    pub fn into_iter(self) -> MapIntoIter<K, V> {
        self.dirty.into_inner().into_iter()
    }

    pub fn into_inner(self) -> Map<K, V> {
        self.dirty.into_inner()
    }
}

pub struct HashMapRefMut<'a, K: Eq + Hash + Clone, V: Clone> {
    k: K,
    m: &'a SyncIndexMap<K, V>,
    value: Option<V>,
}

impl<'a, K: Clone + Eq + Hash, V: Clone> Drop for HashMapRefMut<'a, K, V> {
    fn drop(&mut self) {
        if let Some(v) = self.value.take() {
            let g = self.m.lock.lock();
            let dirty = unsafe { &mut *self.m.dirty.get() };
            match dirty.get_mut(&self.k) {
                Some(slot) => *slot = v,
                // The key was removed while the handle was held: keep the
                // mutation by re-inserting it.
                None => {
                    dirty.insert(self.k.clone(), v);
                }
            }
            self.m.promote();
            drop(g);
        }
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Deref for HashMapRefMut<'_, K, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> DerefMut for HashMapRefMut<'_, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Debug for HashMapRefMut<'_, K, V>
where
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.as_ref().unwrap().fmt(f)
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Display for HashMapRefMut<'_, K, V>
where
    V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.as_ref().unwrap().fmt(f)
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> PartialEq<Self> for HashMapRefMut<'_, K, V>
where
    V: Eq,
{
    fn eq(&self, other: &Self) -> bool {
        self.value.as_ref().unwrap().eq(&other.value.as_ref().unwrap())
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Eq for HashMapRefMut<'_, K, V> where V: Eq {}

pub struct HashIterMut<'a, K: Eq + Hash + Clone, V: Clone> {
    m: &'a SyncIndexMap<K, V>,
    _g: SyncLockGuard<'a>,
    inner: Option<MapIterMut<'a, K, V>>,
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Drop for HashIterMut<'a, K, V> {
    fn drop(&mut self) {
        // Drop the `&mut` borrows into `dirty` first, then publish the
        // mutations into a fresh snapshot. The lock (`_g`) is still held.
        self.inner.take();
        self.m.promote();
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Deref for HashIterMut<'a, K, V> {
    type Target = MapIterMut<'a, K, V>;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> DerefMut for HashIterMut<'a, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Iterator for HashIterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().unwrap().next()
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> IntoIterator for &'a SyncIndexMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = MapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).serialize(serializer) };
        drop(g);
        r
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
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).fmt(f) };
        drop(g);
        r
    }
}

impl<K, V> Display for SyncIndexMap<K, V>
where
    K: Eq + Hash + Display,
    V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Pointer;
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).fmt(f) };
        drop(g);
        r
    }
}

impl<K: Clone + Eq + Hash, V: Clone> Clone for SyncIndexMap<K, V> {
    fn clone(&self) -> Self {
        let g = self.lock.lock();
        let c = unsafe { (&*self.dirty.get()).clone() };
        drop(g);
        SyncIndexMap::from(c)
    }
}

impl<K: Eq + Hash, V> Default for SyncIndexMap<K, V> {
    fn default() -> Self {
        SyncIndexMap::new()
    }
}


// `Index<&K>` access, kept for pre-0.2.17 API compatibility.
#[test]
pub fn test_index() {
    let m = SyncIndexMap::<i32, i32>::new();
    m.insert(1, 10);
    m.insert(2, 20);
    assert_eq!(m[&1], 10);
    assert_eq!(m[&2], 20);
}

// `iter_mut` must deref to the inner map iterator (pre-0.2.17 API).
#[test]
pub fn test_iter_mut_deref() {
    let m = SyncIndexMap::<i32, i32>::new();
    m.insert(1, 10);
    m.insert(2, 20);
    let it = m.iter_mut();
    assert_eq!(it.len(), 2); // via Deref to the inner iterator
}


