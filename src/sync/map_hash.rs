use parking_lot::{Mutex, MutexGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{
    hash_map::IntoIter as MapIntoIter, hash_map::Iter as MapIter, hash_map::IterMut as MapIterMut,
    HashMap as Map, HashMap,
};
use std::fmt::{Debug, Formatter};
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// this sync map used to many reader,writer less.space-for-time strategy
pub struct SyncHashMap<K: Eq + Hash + Clone, V> {
    dirty: UnsafeCell<Map<K, V>>,
    lock: Mutex<()>,
}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash + Clone, V> Send for SyncHashMap<K, V> {}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash + Clone, V> Sync for SyncHashMap<K, V> {}

impl<K, V> SyncHashMap<K, V>
where
    K: Eq + Hash + Clone,
{
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            dirty: UnsafeCell::new(Map::new()),
            lock: Default::default(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            dirty: UnsafeCell::new(Map::with_capacity(capacity)),
            lock: Default::default(),
        }
    }

    pub fn with_map(map: Map<K, V>) -> Self {
        Self {
            dirty: UnsafeCell::new(map),
            lock: Default::default(),
        }
    }

    pub fn insert(&self, k: K, v: V) -> Option<V>
    where
        K: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.insert(k.clone(), v);
        drop(g);
        r
    }

    pub fn insert_mut(&mut self, k: K, v: V) -> Option<V>
    where
        K: Clone,
    {
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(k.clone(), v)
    }

    pub fn remove(&self, k: &K) -> Option<V>
    where
        K: Clone,
    {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.remove(k);
        drop(g);
        r
    }

    pub fn remove_mut(&mut self, k: &K) -> Option<V>
    where
        K: Clone,
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

    pub fn clear(&self) {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.clear();
        drop(g);
    }

    pub fn clear_mut(&mut self) {
        let m = unsafe { &mut *self.dirty.get() };
        m.clear();
    }

    pub fn shrink_to_fit(&self) {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.shrink_to_fit();
        drop(g);
    }

    pub fn shrink_to_fit_mut(&mut self) {
        let m = unsafe { &mut *self.dirty.get() };
        m.shrink_to_fit()
    }

    pub fn from(map: Map<K, V>) -> Self
    where
        K: Clone + Eq + Hash,
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
    /// use dark_std::sync::{SyncHashMap};
    ///
    /// let mut map = SyncHashMap::new();
    /// map.insert_mut(1, "a");
    /// assert_eq!(*map.get(&1).unwrap(), "a");
    /// assert_eq!(map.get(&2).is_none(), true);
    /// ```
    #[inline]
    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        unsafe { (&*self.dirty.get()).get(k) }
    }

    #[inline]
    pub fn get_mut<Q: ?Sized>(&self, k: &Q) -> Option<SyncMapRefMut<'_, V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
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
        K: PartialEq,
    {
        let m = unsafe { &mut *self.dirty.get() };
        m.contains_key(x)
    }

    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, K, V> {
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

    pub fn dirty_ref(&self) -> &HashMap<K, V> {
        unsafe { &*self.dirty.get() }
    }

    pub fn into_inner(self) -> HashMap<K, V> {
        self.dirty.into_inner()
    }
}

pub struct SyncMapRefMut<'a, V> {
    _g: MutexGuard<'a, ()>,
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
    _g: MutexGuard<'a, ()>,
    inner: MapIterMut<'a, K, V>,
}

impl<'a, K, V> Deref for IterMut<'a, K, V> {
    type Target = MapIterMut<'a, K, V>;

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

impl<'a, K, V> IntoIterator for &'a SyncHashMap<K, V>
where
    K: Eq + Hash + Clone,
{
    type Item = (&'a K, &'a V);
    type IntoIter = MapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K, V> IntoIterator for SyncHashMap<K, V>
where
    K: Eq + Hash + Clone,
{
    type Item = (K, V);
    type IntoIter = MapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<K: Eq + Hash + Clone, V> From<Map<K, V>> for SyncHashMap<K, V> {
    fn from(arg: Map<K, V>) -> Self {
        Self::from(arg)
    }
}

impl<K, V> serde::Serialize for SyncHashMap<K, V>
where
    K: Eq + Hash + Clone + Serialize,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.dirty_ref().serialize(serializer)
    }
}

impl<'de, K, V> serde::Deserialize<'de> for SyncHashMap<K, V>
where
    K: Eq + Hash + Clone + serde::Deserialize<'de>,
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

impl<K, V> Debug for SyncHashMap<K, V>
where
    K: Eq + Hash + Clone + Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.dirty_ref().fmt(f)
    }
}
