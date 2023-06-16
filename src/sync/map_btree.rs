use parking_lot::{ReentrantMutex, ReentrantMutexGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{btree_map::IntoIter as MapIntoIter, btree_map::Iter as MapIter, BTreeMap};
use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// this sync map used to many reader,writer less.space-for-time strategy
pub struct SyncBtreeMap<K: Eq + Hash, V> {
    dirty: UnsafeCell<BTreeMap<K, V>>,
    lock: ReentrantMutex<()>,
}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash, V> Send for SyncBtreeMap<K, V> {}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash, V> Sync for SyncBtreeMap<K, V> {}

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
        }
    }

    pub fn with_capacity(_capacity: usize) -> Self {
        Self::new()
    }

    pub fn with_map(map: BTreeMap<K, V>) -> Self {
        Self {
            dirty: UnsafeCell::new(map),
            lock: Default::default(),
        }
    }

    pub fn insert(&self, k: K, v: V) -> Option<V>
        where
            K: Ord,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.insert(k, v);
        drop(g);
        r
    }

    pub fn insert_mut(&mut self, k: K, v: V) -> Option<V>
        where
            K: Ord,
    {
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(k, v)
    }

    pub fn remove(&self, k: &K) -> Option<V>
        where
            K: Ord,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.remove(k);
        drop(g);
        r
    }

    pub fn remove_mut(&mut self, k: &K) -> Option<V>
        where
            K: Ord,
    {
        let m = unsafe { &mut *self.dirty.get() };
        m.remove(k)
    }

    pub fn len(&self) -> usize {
        unsafe { (&*self.dirty.get()).len() }
    }

    pub fn is_empty(&self) -> bool {
        unsafe { (&*self.dirty.get()).is_empty() }
    }

    pub fn clear(&self)
        where
            K: Eq + Hash,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.clear();
        drop(g);
    }

    pub fn clear_mut(&mut self)
        where
            K: Eq + Hash,
    {
        let m = unsafe { &mut *self.dirty.get() };
        m.clear();
    }

    pub fn shrink_to_fit(&self) {}

    pub fn shrink_to_fit_mut(&mut self) {}

    pub fn from(map: BTreeMap<K, V>) -> Self
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
    /// Since reading a map is unlocked, it is very fast
    ///
    /// test bench_sync_hash_map_read   ... bench:           8 ns/iter (+/- 0)
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
            K: Borrow<Q> + Ord,
            Q: Hash + Eq + Ord,
    {
        unsafe { (&*self.dirty.get()).get(k) }
    }

    #[inline]
    pub fn get_mut<Q: ?Sized>(&self, k: &Q) -> Option<SyncMapRefMut<'_, V>>
        where
            K: Borrow<Q> + Ord,
            Q: Hash + Eq + Ord,
    {
        let m = unsafe { &mut *self.dirty.get() };
        Some(SyncMapRefMut {
            _g: self.lock.lock(),
            value: m.get_mut(k)?,
        })
    }

    #[inline]
    pub fn contains_key(&self, x: &K) -> bool
        where
            K: PartialEq + Ord,
    {
        let m = unsafe { &mut *self.dirty.get() };
        m.contains_key(x)
    }

    pub fn iter(&self) -> MapIter<'_, K, V> {
        unsafe { (&*self.dirty.get()).iter() }
    }

    pub fn iter_mut(&self) -> IterMut<'_, K, V> {
        let m = unsafe { &mut *self.dirty.get() };
        return IterMut {
            _g: self.lock.lock(),
            inner: m.iter_mut(),
        };
    }

    pub fn into_iter(self) -> MapIntoIter<K, V> {
        self.dirty.into_inner().into_iter()
    }

    pub fn dirty_ref(&self) -> &BTreeMap<K, V> {
        unsafe { &*self.dirty.get() }
    }

    pub fn into_inner(self) -> BTreeMap<K, V> {
        self.dirty.into_inner()
    }
}

pub struct SyncMapRefMut<'a, V> {
    _g: ReentrantMutexGuard<'a, ()>,
    value: &'a mut V,
}

impl<'a, V> Deref for SyncMapRefMut<'_, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, V> DerefMut for SyncMapRefMut<'_, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

impl<'a, V> Debug for SyncMapRefMut<'_, V>
    where
        V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<'a, V> Display for SyncMapRefMut<'_, V>
    where
        V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<'a, V> PartialEq<Self> for SyncMapRefMut<'_, V>
    where
        V: Eq,
{
    fn eq(&self, other: &Self) -> bool {
        self.value.eq(&other.value)
    }
}

impl<'a, V> Eq for SyncMapRefMut<'_, V> where V: Eq {}

pub struct IterMut<'a, K, V> {
    _g: ReentrantMutexGuard<'a, ()>,
    inner: std::collections::btree_map::IterMut<'a, K, V>,
}

impl<'a, K, V> Deref for IterMut<'a, K, V> {
    type Target = std::collections::btree_map::IterMut<'a, K, V>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, K, V> DerefMut for IterMut<'a, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<'a, K: Eq + Hash, V> IntoIterator for &'a SyncBtreeMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = MapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K: Eq + Hash, V> IntoIterator for SyncBtreeMap<K, V> {
    type Item = (K, V);
    type IntoIter = MapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<K: Eq + Hash, V> From<BTreeMap<K, V>> for SyncBtreeMap<K, V> {
    fn from(arg: BTreeMap<K, V>) -> Self {
        Self::from(arg)
    }
}

impl<K: Eq + Hash, V> serde::Serialize for SyncBtreeMap<K, V>
    where
        K: Eq + Hash + Serialize + Ord,
        V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
    {
        self.dirty_ref().serialize(serializer)
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
        self.dirty_ref().fmt(f)
    }
}

impl<K: Eq + Hash, V> Display for SyncBtreeMap<K, V>
    where
        K: Eq + Hash + Display,
        V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Pointer;
        self.dirty_ref().fmt(f)
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

impl<K: Clone + Eq + Hash, V: Clone> Clone for SyncBtreeMap<K, V> {
    fn clone(&self) -> Self {
        let c = (*self.dirty_ref()).clone();
        SyncBtreeMap::from(c)
    }
}
