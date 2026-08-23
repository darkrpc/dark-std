use super::lock::{SyncLock, SyncLockGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::cell::UnsafeCell;
use std::fmt::{Debug, Display, Formatter};

use std::ops::{Deref, DerefMut, Index};
use std::slice::{Iter as SliceIter, IterMut as SliceIterMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::vec::IntoIter;

use super::snapshot::AtomicSnapshot;

/// A concurrent Vec with a Go `sync.Map`-style read/dirty architecture:
///
/// - `read`: an immutable snapshot, atomically published. `get` / `iter` read
///   it lock-free.
/// - `dirty`: the canonical, mutable vec, guarded by `lock`. Every write goes
///   here and is lazily published into a fresh snapshot.
///
/// Snapshots are immutable and kept alive until the vec is dropped, so
/// references returned by `get` stay valid even while the vec is mutated.
/// Methods that publish a fresh snapshot require `V: Clone`. Suitable for
/// read-mostly workloads (many readers, few writers).
pub struct SyncVec<V> {
    dirty: UnsafeCell<Vec<V>>,
    lock: SyncLock,
    amended: AtomicBool,
    read: AtomicSnapshot<Vec<V>>,
}

/// Safety: `dirty` is only ever accessed under `lock`; the `read` snapshot is
/// immutable once published and is kept alive until the vec is dropped, so
/// references derived from it remain valid for the lifetime of `&self`.
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
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            dirty: UnsafeCell::new(Vec::with_capacity(capacity)),
            lock: Default::default(),
            amended: AtomicBool::new(false),
            read: AtomicSnapshot::new(Vec::with_capacity(capacity)),
        }
    }

    pub fn with_vec(vec: Vec<V>) -> Self {
        Self {
            lock: Default::default(),
            amended: AtomicBool::new(true),
            read: AtomicSnapshot::new(Vec::new()),
            dirty: UnsafeCell::new(vec),
        }
    }

    /// Publish the current `dirty` vec as a fresh immutable snapshot.
    ///
    /// The caller must hold `lock` (or have exclusive `&mut` access).
    fn promote(&self)
    where
        V: Clone,
    {
        let dirty = unsafe { &*self.dirty.get() };
        self.read.publish(dirty.clone());
        // After publishing, `read` reflects `dirty`: nothing is pending.
        self.amended.store(false, Ordering::Release);
    }

    pub fn insert(&self, index: usize, v: V) -> Option<V>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(index, v);
        // Inserting shifts indices, so the snapshot must be refreshed.
        self.promote();
        drop(g);
        None
    }

    pub fn set(&self, index: usize, v: V) -> Option<V>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m[index] = v;
        // Updating an existing slot must refresh the snapshot, otherwise `get`
        // would keep serving the stale value from `read`.
        self.promote();
        drop(g);
        None
    }

    pub fn push(&self, v: V) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.push(v);
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
            m.push(v);
        }
        self.amended.store(true, Ordering::Release);
        drop(g);
        None
    }

    pub fn push_mut(&mut self, v: V) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        m.push(v);
        self.amended.store(true, Ordering::Release);
        None
    }

    pub fn pop(&self) -> Option<V>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.pop();
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
        let r = m.pop();
        if r.is_some() {
            self.promote();
        }
        r
    }

    pub fn remove(&self, index: usize) -> Option<V>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            let v = m.remove(index);
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
            let v = m.remove(index);
            self.promote();
            Some(v)
        } else {
            None
        }
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
        V: Clone,
    {
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
    /// snapshot. If the index was written to `dirty` since the last snapshot
    /// was published, a fresh snapshot is published first and the value is
    /// served from it, so the returned reference always points into immutable,
    /// retained storage.
    #[inline]
    pub fn get(&self, index: usize) -> Option<&V>
    where
        V: Clone,
    {
        if let Some(v) = self.read.load().get(index) {
            return Some(v);
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
            self.read.load().get(index)
        } else {
            None
        }
    }

    #[inline]
    pub unsafe fn get_uncheck(&self, index: usize) -> &V
    where
        V: Clone,
    {
        let g = self.lock.lock();
        self.promote();
        drop(g);
        self.read.load().get_unchecked(index)
    }

    /// Returns a mutable handle to the element at `index`, implemented with
    /// copy-on-write: the value is cloned, the handle mutates the clone, and
    /// the result is written back (and published into a fresh snapshot) when
    /// the handle is dropped. The returned reference stays valid as long as
    /// the handle is held; concurrent readers may observe the pre-mutation
    /// value until the handle is dropped.
    #[inline]
    pub fn get_mut(&self, index: usize) -> Option<VecRefMut<'_, V>>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        let dirty = unsafe { &*self.dirty.get() };
        let value = dirty.get(index)?.clone();
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
        if self.read.load().contains(x) {
            return true;
        }
        if !self.amended.load(Ordering::Acquire) {
            return false;
        }
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).contains(x) };
        drop(g);
        r
    }

    /// Iterate over the current contents. A fresh snapshot is published first,
    /// so all elements written so far are visible.
    pub fn iter(&self) -> std::slice::Iter<'_, V>
    where
        V: Clone,
    {
        let g = self.lock.lock();
        self.promote();
        drop(g);
        self.read.load().iter()
    }

    pub fn iter_mut(&self) -> VecIterMut<'_, V>
    where
        V: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        VecIterMut {
            m: self,
            _g: self.lock.lock(),
            inner: Some(m.iter_mut()),
        }
    }

    pub fn into_iter(self) -> IntoIter<V> {
        self.dirty.into_inner().into_iter()
    }

    pub fn into_inner(self) -> Vec<V> {
        self.dirty.into_inner()
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
            if let Some(slot) = dirty.get_mut(self.k) {
                *slot = v;
            }
            // If the slot disappeared (concurrent pop/remove/clear) the
            // mutation is dropped; the removal wins.
            self.m.promote();
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

pub struct Iter<'a, V> {
    inner: Option<SliceIter<'a, *const V>>,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.inner.as_mut().unwrap().next();
        match next {
            None => None,
            Some(v) => {
                if v.is_null() {
                    None
                } else {
                    unsafe { Some(&**v) }
                }
            }
        }
    }
}

pub struct VecIterMut<'a, V: Clone> {
    m: &'a SyncVec<V>,
    _g: SyncLockGuard<'a>,
    inner: Option<SliceIterMut<'a, V>>,
}

impl<'a, V: Clone> Drop for VecIterMut<'a, V> {
    fn drop(&mut self) {
        // Drop the `&mut` borrows into `dirty` first, then publish the
        // mutations into a fresh snapshot. The lock (`_g`) is still held.
        self.inner.take();
        self.m.promote();
    }
}

impl<'a, V: Clone> Deref for VecIterMut<'a, V> {
    type Target = SliceIterMut<'a, V>;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<'a, V: Clone> DerefMut for VecIterMut<'a, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, V: Clone> Iterator for VecIterMut<'a, V> {
    type Item = &'a mut V;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().unwrap().next()
    }
}

impl<'a, V: Clone> IntoIterator for &'a SyncVec<V> {
    type Item = &'a V;
    type IntoIter = std::slice::Iter<'a, V>;

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
        let g = self.lock.lock();
        let r = unsafe { (&*self.dirty.get()).serialize(serializer) };
        drop(g);
        r
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

impl<V: Clone> Index<usize> for SyncVec<V> {
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
        let r = a.eq(b);
        drop(g2);
        drop(g1);
        r
    }
}

impl<V: Clone> Clone for SyncVec<V> {
    fn clone(&self) -> Self {
        let g = self.lock.lock();
        let c = unsafe { (&*self.dirty.get()).clone() };
        drop(g);
        SyncVec::from(c)
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


