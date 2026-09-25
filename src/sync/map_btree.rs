use crate::lock::{SyncLock, SyncLockGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{
    btree_map::IntoIter as MapIntoIter, btree_map::Iter as MapIter,
    btree_map::IterMut as MapIterMut, BTreeMap,
};
use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::entry::{Entry, Retired};
use super::snapshot::AtomicSnapshot;

/// A concurrent BTreeMap with a Go `sync.Map`-style read/dirty architecture:
///
/// - `read`: an immutable snapshot, atomically published. `get` / `iter` /
///   `Index` read it lock-free.
/// - `dirty`: the canonical, mutable map, guarded by `lock`.
///
/// Every slot is an `Arc<Entry<V>>` shared between the snapshot and `dirty`.
/// The entry holds an atomic pointer to the value, so updating an existing key
/// swaps the pointer in place (O(1)) — no snapshot rebuild — and readers
/// always see the latest value. New keys and removals are published lazily
/// (tracked by the `amended` flag). Snapshots and retired values are kept
/// alive until the map is dropped, so references returned by `get` stay valid.
pub struct SyncBtreeMap<K: Eq + Hash, V> {
    dirty: UnsafeCell<BTreeMap<K, Arc<Entry<V>>>>,
    lock: SyncLock,
    amended: AtomicBool,
    read: AtomicSnapshot<BTreeMap<K, Arc<Entry<V>>>>,
    retired: Retired<V>,
}

/// Safety: `dirty` is only ever accessed under `lock`; the `read` snapshot is
/// immutable once published; values behind entries are immutable once
/// published and swapped out atomically; retired values and retired snapshots
/// are kept alive until the map is dropped, so references derived from `get`
/// remain valid for the lifetime of `&self`.
unsafe impl<K: Eq + Hash, V> Send for SyncBtreeMap<K, V> {}
unsafe impl<K: Eq + Hash, V> Sync for SyncBtreeMap<K, V> {}

impl<K, V> std::ops::Index<&K> for SyncBtreeMap<K, V>
where
    K: Eq + Hash + Ord + Clone,
{
    type Output = V;

    fn index(&self, index: &K) -> &Self::Output {
        self.get(index).expect("key not found")
    }
}

impl<K: Eq + Hash, V> SyncBtreeMap<K, V>
where
    K: Eq + Hash,
{
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            dirty: UnsafeCell::new(BTreeMap::new()),
            lock: Default::default(),
            amended: AtomicBool::new(false),
            read: AtomicSnapshot::new(BTreeMap::new()),
            retired: Retired::new(),
        }
    }

    pub fn with_capacity(_capacity: usize) -> Self {
        Self::new()
    }

    pub fn with_map(map: BTreeMap<K, V>) -> Self
    where
        K: Ord,
    {
        let dirty = map
            .into_iter()
            .map(|(k, v)| (k, Arc::new(Entry::new(v))))
            .collect();
        Self {
            read: AtomicSnapshot::new(BTreeMap::new()),
            dirty: UnsafeCell::new(dirty),
            lock: Default::default(),
            amended: AtomicBool::new(true),
            retired: Retired::new(),
        }
    }

    /// Publish the current `dirty` map as a fresh immutable snapshot.
    ///
    /// The caller must hold `lock` (or have exclusive `&mut` access).
    fn promote(&self)
    where
        K: Clone,
    {
        let dirty = unsafe { &*self.dirty.get() };
        self.read.publish(dirty.clone());
        // After publishing, `read` reflects `dirty`: nothing is pending.
        self.amended.store(false, Ordering::Release);
    }

    /// Insert or replace the value for `k`, returning the previous value if
    /// the key already existed.
    ///
    /// This requires `V: Clone` because the previous value must stay alive
    /// for concurrent readers. Use [`set`](Self::set) when the value is not
    /// `Clone` and the previous value is not needed.
    pub fn insert(&self, k: K, v: V) -> Option<V>
    where
        K: Ord + Clone,
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if let Some(entry) = m.get(&k) {
            // Update: swap the value in place (O(1)). The shared entry lets
            // readers observe the new value without a snapshot rebuild.
            let old = entry.swap(v);
            let old_value = unsafe { (*old).clone() };
            self.retired.push(old);
            drop(g);
            return Some(old_value);
        }
        // New key: leave it for lazy promotion and mark `amended`.
        m.insert(k, Arc::new(Entry::new(v)));
        self.amended.store(true, Ordering::Release);
        drop(g);
        None
    }

    pub fn insert_mut(&mut self, k: K, v: V) -> Option<V>
    where
        K: Ord + Clone,
        V: Clone,
    {
        self.insert(k, v)
    }

    /// Insert or overwrite the value for `k` without returning the previous
    /// one. Unlike [`insert`](Self::insert) this does **not** require
    /// `V: Clone`, so it works with non-`Clone` values. Updating an existing
    /// key swaps the value in place (O(1)); readers observe the new value
    /// immediately.
    pub fn set(&self, k: K, v: V)
    where
        K: Ord,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if let Some(entry) = m.get(&k) {
            // Update: swap the value in place (O(1)). The shared entry lets
            // readers observe the new value without a snapshot rebuild.
            let old = entry.swap(v);
            self.retired.push(old);
        } else {
            // New key: leave it for lazy promotion and mark `amended`.
            m.insert(k, Arc::new(Entry::new(v)));
            self.amended.store(true, Ordering::Release);
        }
        drop(g);
    }

    pub fn set_mut(&mut self, k: K, v: V)
    where
        K: Ord,
    {
        self.set(k, v)
    }

    /// Remove `k` and return its value.
    ///
    /// This requires `V: Clone` because the removed value must stay alive
    /// for concurrent readers. Use [`delete`](Self::delete) when the value is
    /// not `Clone` and the removed value is not needed.
    pub fn remove(&self, k: &K) -> Option<V>
    where
        K: Ord + Clone,
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if let Some(entry) = m.remove(k) {
            // Clone the value out; the entry (and its value) stays alive in the
            // retired snapshot published below.
            let v = entry.load().clone();
            // Refresh the snapshot so `get` no longer serves the removed key.
            self.promote();
            drop(g);
            return Some(v);
        }
        drop(g);
        None
    }

    pub fn remove_mut(&mut self, k: &K) -> Option<V>
    where
        K: Ord + Clone,
        V: Clone,
    {
        self.remove(k)
    }

    /// Remove `k` without returning its value. Unlike
    /// [`remove`](Self::remove) this does **not** require `V: Clone`, so it
    /// works with non-`Clone` values.
    pub fn delete(&self, k: &K)
    where
        K: Ord + Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if m.remove(k).is_some() {
            // Refresh the snapshot so `get` no longer serves the removed key.
            self.promote();
        }
        drop(g);
    }

    pub fn delete_mut(&mut self, k: &K)
    where
        K: Ord + Clone,
    {
        self.delete(k)
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
        K: Eq + Hash + Clone,
    {
        let g = self.lock.lock();
        let had_entries = {
            let m = unsafe { &mut *self.dirty.get() };
            let had_entries = !m.is_empty();
            m.clear();
            had_entries
        };
        // Clearing an already-empty map changes nothing, so publishing a
        // snapshot here would only retire the current one for no reason.
        if had_entries {
            self.promote();
        }
        drop(g);
    }

    pub fn clear_mut(&mut self)
    where
        K: Eq + Hash + Clone,
    {
        self.clear()
    }

    pub fn shrink_to_fit(&self) {}

    pub fn shrink_to_fit_mut(&mut self) {}

    pub fn from(map: BTreeMap<K, V>) -> Self
    where
        K: Eq + Hash + Ord,
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
    /// snapshot through a shared entry, so updates are visible immediately.
    /// If the key was added to `dirty` since the last snapshot was published,
    /// a fresh snapshot is published first.
    ///
    /// # Examples
    ///
    /// ```
    /// use dark_std::sync::{SyncBtreeMap};
    ///
    /// let mut map = SyncBtreeMap::new();
    /// map.insert_mut(1, "a");
    /// assert_eq!(*map.get(&1).unwrap(), "a");
    /// assert_eq!(map.get(&2).is_none(), true);
    /// ```
    #[inline]
    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q> + Ord + Clone,
        Q: Hash + Eq + Ord,
    {
        if let Some(entry) = self.read.load().get(k) {
            return Some(entry.load());
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
            self.read.load().get(k).map(|e| e.load())
        } else {
            None
        }
    }

    /// Returns a mutable handle to the value for `k`, implemented with
    /// copy-on-write: the value is cloned, the handle mutates the clone, and
    /// the result is swapped back into the shared entry (O(1)) when the handle
    /// is dropped. Concurrent readers may observe the pre-mutation value until
    /// the handle is dropped.
    #[inline]
    pub fn get_mut(&self, k: &K) -> Option<BtreeMapRefMut<'_, K, V>>
    where
        K: Hash + Eq + Clone + Ord,
        V: Clone,
    {
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let value = dirty.get(k)?.load().clone();
        drop(g);
        Some(BtreeMapRefMut {
            k: k.clone(),
            m: self,
            value: Some(value),
        })
    }

    #[inline]
    pub fn contains_key(&self, x: &K) -> bool
    where
        K: PartialEq + Ord,
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

    /// Iterate over the current contents. If dirty has un-published writes, a
    /// fresh snapshot is published first; otherwise the current snapshot is
    /// reused lock-free to avoid leaking retired snapshot allocations on every
    /// call.
    pub fn iter(&self) -> Iter<'_, K, V>
    where
        K: Clone,
    {
        if self.amended.load(Ordering::Acquire) {
            let g = self.lock.lock();
            self.promote();
            drop(g);
        }
        Iter {
            inner: self.read.load().iter(),
        }
    }

    pub fn iter_mut(&self) -> IterMut<'_, K, V>
    where
        K: Clone,
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        IterMut {
            m: self,
            _g: self.lock.lock(),
            inner: Some(m.iter_mut()),
            visited: false,
        }
    }

    pub fn into_iter(self) -> MapIntoIter<K, V>
    where
        K: Ord,
    {
        self.into_inner().into_iter()
    }

    pub fn into_inner(self) -> BTreeMap<K, V>
    where
        K: Ord,
    {
        // Move `dirty` out; the remaining fields (snapshots, retired values,
        // lock) are dropped normally at the end of this function.
        let dirty = self.dirty.into_inner();
        dirty
            .into_iter()
            .map(|(k, entry)| (k, entry.take()))
            .collect()
    }
}

/// Iterator over `(&K, &V)`, served from the immutable snapshot.
pub struct Iter<'a, K, V> {
    inner: MapIter<'a, K, Arc<Entry<V>>>,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, e)| (k, e.load()))
    }
}

impl<'a, K, V> ExactSizeIterator for Iter<'a, K, V> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

/// Mutable iterator over `(&K, &mut V)`. Entries shared with snapshots are
/// replaced with fresh unique ones; mutations are published when the iterator
/// is dropped.
pub struct IterMut<'a, K: Eq + Hash + Clone, V: Clone> {
    m: &'a SyncBtreeMap<K, V>,
    _g: SyncLockGuard<'a>,
    inner: Option<MapIterMut<'a, K, Arc<Entry<V>>>>,
    visited: bool,
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Drop for IterMut<'a, K, V> {
    fn drop(&mut self) {
        // Drop the `&mut` borrows into `dirty` first, then publish the
        // mutations into a fresh snapshot. The lock (`_g`) is still held.
        self.inner.take();
        // Only a handed-out `&mut V` can have changed anything; dropping an
        // untouched iterator must not retire a snapshot.
        if self.visited {
            self.m.promote();
        }
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        let (k, entry) = self.inner.as_mut().unwrap().next()?;
        self.visited = true;
        // Make the entry uniquely owned so we can hand out `&mut V`.
        if Arc::get_mut(entry).is_none() {
            let current = entry.load().clone();
            *entry = Arc::new(Entry::new(current));
        }
        Some((k, Arc::get_mut(entry).unwrap().get_mut()))
    }
}

impl<'a, K: Eq + Hash + Clone, V: Clone> ExactSizeIterator for IterMut<'a, K, V> {
    fn len(&self) -> usize {
        self.inner.as_ref().unwrap().len()
    }
}

pub struct BtreeMapRefMut<'a, K: Eq + Hash + Ord + Clone, V: Clone> {
    k: K,
    m: &'a SyncBtreeMap<K, V>,
    value: Option<V>,
}

impl<'a, K: Eq + Hash + Ord + Clone, V: Clone> Drop for BtreeMapRefMut<'a, K, V> {
    fn drop(&mut self) {
        if let Some(v) = self.value.take() {
            let g = self.m.lock.lock();
            let dirty = unsafe { &mut *self.m.dirty.get() };
            match dirty.get_mut(&self.k) {
                Some(entry) => {
                    let old = entry.swap(v);
                    self.m.retired.push(old);
                }
                // The key was removed while the handle was held: keep the
                // mutation by re-inserting it.
                None => {
                    dirty.insert(self.k.clone(), Arc::new(Entry::new(v)));
                    self.m.amended.store(true, Ordering::Release);
                }
            }
            drop(g);
        }
    }
}

impl<'a, K: Eq + Hash + Ord + Clone, V: Clone> Deref for BtreeMapRefMut<'_, K, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, K: Eq + Hash + Ord + Clone, V: Clone> DerefMut for BtreeMapRefMut<'_, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<'a, K: Eq + Hash + Ord + Clone, V: Clone> Debug for BtreeMapRefMut<'_, K, V>
where
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.as_ref().unwrap().fmt(f)
    }
}

impl<'a, K: Eq + Hash + Ord + Clone, V: Clone> Display for BtreeMapRefMut<'_, K, V>
where
    V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.as_ref().unwrap().fmt(f)
    }
}

impl<'a, K: Eq + Hash + Ord + Clone, V: Clone> PartialEq<Self> for BtreeMapRefMut<'_, K, V>
where
    V: Eq,
{
    fn eq(&self, other: &Self) -> bool {
        self.value
            .as_ref()
            .unwrap()
            .eq(&other.value.as_ref().unwrap())
    }
}

impl<'a, K: Eq + Hash + Ord + Clone, V: Clone> Eq for BtreeMapRefMut<'_, K, V> where V: Eq {}

impl<'a, K: Clone, V> IntoIterator for &'a SyncBtreeMap<K, V>
where
    K: Eq + Hash,
{
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K: Eq + Hash + Ord, V> IntoIterator for SyncBtreeMap<K, V> {
    type Item = (K, V);
    type IntoIter = MapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<K: Eq + Hash + Ord, V> From<BTreeMap<K, V>> for SyncBtreeMap<K, V> {
    fn from(arg: BTreeMap<K, V>) -> Self {
        Self::from(arg)
    }
}

impl<K, V> serde::Serialize for SyncBtreeMap<K, V>
where
    K: Eq + Hash + Serialize + Ord,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let mut m = serializer.serialize_map(Some(dirty.len()))?;
        for (k, e) in dirty.iter() {
            m.serialize_entry(k, e.load())?;
        }
        drop(g);
        m.end()
    }
}

impl<'de, K, V> serde::Deserialize<'de> for SyncBtreeMap<K, V>
where
    K: Eq + Hash + Ord + serde::Deserialize<'de>,
    V: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let m = BTreeMap::deserialize(deserializer)?;
        Ok(Self::from(m))
    }
}

impl<K: Eq + Hash, V> Debug for SyncBtreeMap<K, V>
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

impl<K: Eq + Hash, V> Display for SyncBtreeMap<K, V>
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

pub struct BtreeIter<'a, K, V> {
    inner: MapIter<'a, K, *const V>,
}

impl<'a, K, V> Iterator for BtreeIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            None => None,
            Some((k, v)) => Some((k, unsafe { v.as_ref().unwrap() })),
        }
    }
}

impl<K: Clone + Eq + Hash + Ord, V: Clone> Clone for SyncBtreeMap<K, V> {
    fn clone(&self) -> Self {
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let m = dirty
            .iter()
            .map(|(k, e)| (k.clone(), e.load().clone()))
            .collect();
        drop(g);
        SyncBtreeMap::from(m)
    }
}

impl<K: Eq + Hash, V> Default for SyncBtreeMap<K, V> {
    fn default() -> Self {
        SyncBtreeMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SyncBtreeMap<i32, i32> {
        let mut map = BTreeMap::new();
        map.insert(1, 1);
        map.insert(2, 2);
        SyncBtreeMap::with_map(map)
    }

    #[test]
    fn repeated_iter_does_not_retire_snapshots() {
        let m = sample();
        // The first call publishes the writes made by `with_map`.
        assert_eq!(m.iter().count(), 2);
        let retired = m.read.retired_len();
        for _ in 0..16 {
            assert_eq!(m.iter().count(), 2);
        }
        assert_eq!(m.read.retired_len(), retired);
    }

    #[test]
    fn iter_still_sees_pending_inserts() {
        let m = SyncBtreeMap::new();
        m.set(2, 2);
        m.set(1, 1);
        let got: Vec<(i32, i32)> = m.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(got, vec![(1, 1), (2, 2)]);
    }

    #[test]
    fn repeated_clear_on_empty_does_not_retire_snapshots() {
        let m: SyncBtreeMap<i32, i32> = SyncBtreeMap::new();
        m.clear();
        let retired = m.read.retired_len();
        for _ in 0..16 {
            m.clear();
        }
        assert_eq!(m.read.retired_len(), retired);
    }

    #[test]
    fn dropped_unused_iter_mut_does_not_retire_snapshots() {
        let m = sample();
        assert_eq!(m.iter().count(), 2);
        let retired = m.read.retired_len();
        for _ in 0..16 {
            drop(m.iter_mut());
        }
        assert_eq!(m.read.retired_len(), retired);
    }

    #[test]
    fn iter_mut_publishes_mutations() {
        let m = sample();
        for (_, v) in m.iter_mut() {
            *v *= 10;
        }
        assert_eq!(*m.get(&1).unwrap(), 10);
        assert_eq!(m.iter().map(|(_, v)| *v).sum::<i32>(), 30);
    }
}
