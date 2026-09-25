use crate::lock::{SyncLock, SyncLockGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::cell::UnsafeCell;
use std::fmt::{Debug, Display, Formatter};

use std::ops::{Deref, DerefMut, Index};
use std::slice::{Iter as SliceIter, IterMut as SliceIterMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::vec::IntoIter;

use super::entry::{Entry, Retired};
use super::snapshot::AtomicSnapshot;

/// A concurrent Vec with a Go `sync.Map`-style read/dirty architecture:
///
/// - `read`: an immutable snapshot, atomically published. `get` / `iter` read
///   it lock-free.
/// - `dirty`: the canonical, mutable vec, guarded by `lock`.
///
/// Every slot is an `Arc<Entry<V>>` shared between the snapshot and `dirty`.
/// The entry holds an atomic pointer to the value, so `set` swaps the pointer
/// in place (O(1)) — no snapshot rebuild — and readers always see the latest
/// value. Appends are published lazily (tracked by the `amended` flag), while
/// index-shifting operations (insert/remove/pop) rebuild the snapshot.
/// Snapshots and retired values are kept alive until the vec is dropped, so
/// references returned by `get` stay valid.
pub struct SyncVec<V> {
    dirty: UnsafeCell<Vec<Arc<Entry<V>>>>,
    lock: SyncLock,
    amended: AtomicBool,
    read: AtomicSnapshot<Vec<Arc<Entry<V>>>>,
    retired: Retired<V>,
}

/// Safety: `dirty` is only ever accessed under `lock`; the `read` snapshot is
/// immutable once published; values behind entries are immutable once
/// published and swapped out atomically; retired values and retired snapshots
/// are kept alive until the vec is dropped, so references derived from `get`
/// remain valid for the lifetime of `&self`.
unsafe impl<V> Send for SyncVec<V> {}
unsafe impl<V> Sync for SyncVec<V> {}

impl<V> SyncVec<V> {
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            dirty: UnsafeCell::new(Vec::new()),
            lock: Default::default(),
            amended: AtomicBool::new(false),
            read: AtomicSnapshot::new(Vec::new()),
            retired: Retired::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            dirty: UnsafeCell::new(Vec::with_capacity(capacity)),
            lock: Default::default(),
            amended: AtomicBool::new(false),
            read: AtomicSnapshot::new(Vec::with_capacity(capacity)),
            retired: Retired::new(),
        }
    }

    pub fn with_vec(vec: Vec<V>) -> Self {
        let dirty = vec.into_iter().map(Entry::new).map(Arc::new).collect();
        Self {
            lock: Default::default(),
            amended: AtomicBool::new(true),
            read: AtomicSnapshot::new(Vec::new()),
            dirty: UnsafeCell::new(dirty),
            retired: Retired::new(),
        }
    }

    /// Publish the current `dirty` vec as a fresh immutable snapshot.
    ///
    /// The caller must hold `lock` (or have exclusive `&mut` access).
    fn promote(&self) {
        let dirty = unsafe { &*self.dirty.get() };
        self.read.publish(dirty.clone());
        // After publishing, `read` reflects `dirty`: nothing is pending.
        self.amended.store(false, Ordering::Release);
    }

    pub fn insert(&self, index: usize, v: V) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(index, Arc::new(Entry::new(v)));
        // Inserting shifts indices, so the snapshot must be refreshed.
        self.promote();
        drop(g);
        None
    }

    pub fn set(&self, index: usize, v: V) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let entry = m.get_mut(index).expect("index out of bounds");
        // Update: swap the value in place (O(1)). The shared entry lets
        // readers observe the new value without a snapshot rebuild.
        let old = entry.swap(v);
        self.retired.push(old);
        drop(g);
        None
    }

    pub fn push(&self, v: V) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.push(Arc::new(Entry::new(v)));
        // Appending is lazy: mark `amended`; `get` on a yet-unpublished index
        // falls back to `dirty` and publishes a fresh snapshot.
        self.amended.store(true, Ordering::Release);
        drop(g);
        None
    }

    pub fn pushes(&self, arr: Vec<V>) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        for v in arr {
            m.push(Arc::new(Entry::new(v)));
        }
        self.amended.store(true, Ordering::Release);
        drop(g);
        None
    }

    pub fn push_mut(&mut self, v: V) -> Option<V> {
        unsafe { (&mut *self.dirty.get()).push(Arc::new(Entry::new(v))) };
        self.amended.store(true, Ordering::Release);
        None
    }

    /// Remove and return the last element.
    ///
    /// This requires `V: Clone` because the removed value must stay alive
    /// for concurrent readers. Use [`pop_discard`](Self::pop_discard) when
    /// the value is not `Clone` and the removed value is not needed.
    pub fn pop(&self) -> Option<V>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.pop().map(|e| e.load().clone());
        if r.is_some() {
            // Refresh the snapshot so `get` no longer serves the popped slot.
            self.promote();
        }
        drop(g);
        r
    }

    pub fn pop_mut(&mut self) -> Option<V>
    where
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.pop().map(|e| e.load().clone());
        if r.is_some() {
            self.promote();
        }
        r
    }

    /// Remove and discard the last element without returning it. Unlike
    /// [`pop`](Self::pop) this does **not** require `V: Clone`, so it works
    /// with non-`Clone` values.
    pub fn pop_discard(&self) {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if m.pop().is_some() {
            // Refresh the snapshot so `get` no longer serves the popped slot.
            self.promote();
        }
        drop(g);
    }

    pub fn pop_discard_mut(&mut self) {
        self.pop_discard()
    }

    /// Remove and return the element at `index`.
    ///
    /// This requires `V: Clone` because the removed value must stay alive
    /// for concurrent readers. Use
    /// [`remove_discard`](Self::remove_discard) when the value is not `Clone`
    /// and the removed value is not needed.
    pub fn remove(&self, index: usize) -> Option<V>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            let entry = m.remove(index);
            let v = entry.load().clone();
            // Removing shifts indices, so the snapshot must be refreshed.
            self.promote();
            drop(g);
            Some(v)
        } else {
            drop(g);
            None
        }
    }

    pub fn remove_mut(&mut self, index: usize) -> Option<V>
    where
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            let entry = m.remove(index);
            let v = entry.load().clone();
            self.promote();
            Some(v)
        } else {
            None
        }
    }

    /// Remove and discard the element at `index` without returning it. Unlike
    /// [`remove`](Self::remove) this does **not** require `V: Clone`, so it
    /// works with non-`Clone` values.
    pub fn remove_discard(&self, index: usize) {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            m.remove(index);
            // Removing shifts indices, so the snapshot must be refreshed.
            self.promote();
        }
        drop(g);
    }

    pub fn remove_discard_mut(&mut self, index: usize) {
        self.remove_discard(index)
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

    pub fn clear(&self) {
        let g = self.lock.lock();
        unsafe { (&mut *self.dirty.get()).clear() };
        self.promote();
        drop(g);
    }

    pub fn shrink_to_fit(&self) {
        let g = self.lock.lock();
        unsafe { (&mut *self.dirty.get()).shrink_to_fit() };
        drop(g);
    }

    pub fn from(vec: Vec<V>) -> Self {
        let s = Self::with_vec(vec);
        s
    }

    /// Returns a reference to the element at `index`.
    ///
    /// Reads are lock-free: the value is served from the immutable `read`
    /// snapshot through a shared entry, so `set` is visible immediately.
    /// If the index was appended to `dirty` since the last snapshot was
    /// published, a fresh snapshot is published first.
    #[inline]
    pub fn get(&self, index: usize) -> Option<&V> {
        if let Some(entry) = self.read.load().get(index) {
            return Some(entry.load());
        }
        // If nothing was written to `dirty` since the last snapshot was
        // published, a snapshot miss is a real miss: no lock is needed.
        if !self.amended.load(Ordering::Acquire) {
            return None;
        }
        // Snapshot miss: the element may have been appended to `dirty` without
        // a snapshot refresh yet (lazy promotion). Publish a fresh snapshot
        // and serve from it.
        let g = self.lock.lock();
        let found = unsafe { (&*self.dirty.get()).len() > index };
        if found {
            self.promote();
        }
        drop(g);
        if found {
            self.read.load().get(index).map(|e| e.load())
        } else {
            None
        }
    }

    #[inline]
    pub unsafe fn get_uncheck(&self, index: usize) -> &V {
        let g = self.lock.lock();
        self.promote();
        drop(g);
        unsafe { self.read.load().get_unchecked(index).load() }
    }

    /// Returns a mutable handle to the element at `index`, implemented with
    /// copy-on-write: the value is cloned, the handle mutates the clone, and
    /// the result is swapped back into the shared entry (O(1)) when the handle
    /// is dropped. Concurrent readers may observe the pre-mutation value until
    /// the handle is dropped.
    #[inline]
    pub fn get_mut(&self, index: usize) -> Option<VecRefMut<'_, V>>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let value = dirty.get(index)?.load().clone();
        drop(g);
        Some(VecRefMut {
            k: index,
            m: self,
            value: Some(value),
        })
    }

    #[inline]
    pub fn contains(&self, x: &V) -> bool
    where
        V: PartialEq,
    {
        if self.read.load().iter().any(|e| e.load() == x) {
            return true;
        }
        if !self.amended.load(Ordering::Acquire) {
            return false;
        }
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).iter().any(|e| e.load() == x) };
        drop(g);
        r
    }

    /// Iterate over the current contents. If dirty has un-published writes, a
    /// fresh snapshot is published first; otherwise the current snapshot is
    /// reused lock-free to avoid leaking retired Box allocations on every call.
    pub fn iter(&self) -> Iter<'_, V> {
        if self.amended.load(Ordering::Acquire) {
            let g = self.lock.lock();
            self.promote();
            drop(g);
        }
        Iter {
            inner: self.read.load().iter(),
        }
    }

    pub fn iter_mut(&self) -> IterMut<'_, V>
    where
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        IterMut {
            m: self,
            _g: self.lock.lock(),
            inner: Some(m.iter_mut()),
        }
    }

    pub fn into_iter(self) -> IntoIter<V> {
        self.into_inner().into_iter()
    }

    pub fn into_inner(self) -> Vec<V> {
        // Move `dirty` out; the remaining fields (snapshots, retired values,
        // lock) are dropped normally at the end of this function.
        let dirty = self.dirty.into_inner();
        dirty.into_iter().map(|e| e.take()).collect()
    }
}

pub struct VecRefMut<'a, V: Clone> {
    k: usize,
    m: &'a SyncVec<V>,
    value: Option<V>,
}

impl<'a, V: Clone> Drop for VecRefMut<'a, V> {
    fn drop(&mut self) {
        if let Some(v) = self.value.take() {
            let g = self.m.lock.lock();
            let dirty = unsafe { &mut *self.m.dirty.get() };
            if let Some(entry) = dirty.get_mut(self.k) {
                let old = entry.swap(v);
                self.m.retired.push(old);
            }
            // If the slot disappeared (concurrent pop/remove/clear) the
            // mutation is dropped; the removal wins.
            drop(g);
        }
    }
}

impl<'a, V: Clone> Deref for VecRefMut<'_, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, V: Clone> DerefMut for VecRefMut<'_, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<'a, V: Clone> Debug for VecRefMut<'_, V>
where
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.as_ref().unwrap().fmt(f)
    }
}

impl<'a, V: Clone> Display for VecRefMut<'_, V>
where
    V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.as_ref().unwrap().fmt(f)
    }
}

/// Iterator over `&V`, served from the immutable snapshot.
pub struct Iter<'a, V> {
    inner: SliceIter<'a, Arc<Entry<V>>>,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|e| e.load())
    }
}

impl<'a, V> ExactSizeIterator for Iter<'a, V> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

/// Mutable iterator over `&mut V`. Entries shared with snapshots are replaced
/// with fresh unique ones; mutations are published when the iterator is
/// dropped.
pub struct IterMut<'a, V: Clone> {
    m: &'a SyncVec<V>,
    _g: SyncLockGuard<'a>,
    inner: Option<SliceIterMut<'a, Arc<Entry<V>>>>,
}

impl<'a, V: Clone> Drop for IterMut<'a, V> {
    fn drop(&mut self) {
        // Drop the `&mut` borrows into `dirty` first, then publish the
        // mutations into a fresh snapshot. The lock (`_g`) is still held.
        self.inner.take();
        self.m.promote();
    }
}

impl<'a, V: Clone> Iterator for IterMut<'a, V> {
    type Item = &'a mut V;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.inner.as_mut().unwrap().next()?;
        // Make the entry uniquely owned so we can hand out `&mut V`.
        if Arc::get_mut(entry).is_none() {
            let current = entry.load().clone();
            *entry = Arc::new(Entry::new(current));
        }
        Some(Arc::get_mut(entry).unwrap().get_mut())
    }
}

impl<'a, V: Clone> ExactSizeIterator for IterMut<'a, V> {
    fn len(&self) -> usize {
        self.inner.as_ref().unwrap().len()
    }
}

impl<'a, V> IntoIterator for &'a SyncVec<V> {
    type Item = &'a V;
    type IntoIter = Iter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
        use serde::ser::SerializeSeq;
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let mut seq = serializer.serialize_seq(Some(dirty.len()))?;
        for e in dirty.iter() {
            seq.serialize_element(e.load())?;
        }
        drop(g);
        seq.end()
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
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).fmt(f) };
        drop(g);
        r
    }
}

impl<V> Display for SyncVec<V>
where
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

impl<V> Index<usize> for SyncVec<V> {
    type Output = V;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("index out of bounds")
    }
}

impl<V: PartialEq> PartialEq for SyncVec<V> {
    fn eq(&self, other: &Self) -> bool {
        // Comparing a vec with itself must not re-lock the same mutex.
        if std::ptr::eq(self, other) {
            return true;
        }
        let g1 = self.lock.lock();
        let g2 = other.lock.lock();
        let a = unsafe { &*self.dirty.get() };
        let b = unsafe { &*other.dirty.get() };
        let r = a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.load() == y.load());
        drop(g2);
        drop(g1);
        r
    }
}

impl<V: Clone> Clone for SyncVec<V> {
    fn clone(&self) -> Self {
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let v: Vec<V> = dirty.iter().map(|e| e.load().clone()).collect();
        drop(g);
        SyncVec::from(v)
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
