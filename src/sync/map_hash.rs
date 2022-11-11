use serde::ser::SerializeMap;
use serde::{Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{hash_map::IntoIter as MapIntoIter, hash_map::Iter as MapIter, hash_map::IterMut as MapIterMut, HashMap as Map, HashMap};
use std::fmt::{Debug, Formatter};
use std::hash::Hash;

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use tokio::sync::{Mutex, MutexGuard};

pub type SyncHashMap<K, V> = SyncMapImpl<K, V>;

/// this sync map used to many reader,writer less.space-for-time strategy
pub struct SyncMapImpl<K: Eq + Hash + Clone, V> {
    dirty: UnsafeCell<Map<K, V>>,
    lock: Mutex<()>,
}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash + Clone, V> Send for SyncMapImpl<K, V> {}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash + Clone, V> Sync for SyncMapImpl<K, V> {}

impl<K, V> SyncMapImpl<K, V>
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

    pub fn with_map(map:Map<K,V>) -> Self {
        Self {
            dirty: UnsafeCell::new(map),
            lock: Default::default(),
        }
    }

    pub async fn insert(&self, k: K, v: V) -> Option<V>
        where
            K: Clone,
    {
        let g = self.lock.lock().await;
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

    pub async fn remove(&self, k: &K) -> Option<V>
        where
            K: Clone,
    {
        let g = self.lock.lock().await;
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

    pub async fn clear(&self) {
        let g = self.lock.lock().await;
        let m = unsafe { &mut *self.dirty.get() };
        m.clear();
        drop(g);
    }

    pub fn clear_mut(&mut self) {
        let m = unsafe { &mut *self.dirty.get() };
        m.clear();
    }

    pub async fn shrink_to_fit(&self) {
        let g = self.lock.lock().await;
        let m = unsafe { &mut *self.dirty.get() };
        m.shrink_to_fit();
        drop(g);
    }

    pub async fn shrink_to_fit_mut(&mut self) {
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
    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<&V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq,
    {
        unsafe {
            (&*self.dirty.get()).get(k)
        }
    }

    pub async fn get_mut<Q: ?Sized>(&self, k: &Q) -> Option<SyncMapRefMut<'_, V>>
        where
            K: Borrow<Q>,
            Q: Hash + Eq,
    {
        let m = unsafe { &mut *self.dirty.get() };
        let mut r = SyncMapRefMut { _g: self.lock.lock().await, value: None };
        r.value = Some(m.get_mut(k)?);
        Some(r)
    }

    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, K, V> {
        unsafe { (&*self.dirty.get()).iter() }
    }

    pub async fn iter_mut(&self) -> IterMut<'_, K, V> {
        let m = unsafe { &mut *self.dirty.get() };
        let mut iter = IterMut { _g: self.lock.lock().await, inner: None };
            iter.inner = Some(m.iter_mut());
        return iter;
    }

    pub fn into_iter(self) -> MapIntoIter<K, V> {
        self.dirty.into_inner().into_iter()
    }

    pub fn dirty_ref(&self) -> &HashMap<K, V> {
        unsafe { &*self.dirty.get() }
    }
}

pub unsafe fn change_lifetime_const<'a, 'b, T>(x: &'a T) -> &'b T {
    &*(x as *const T)
}

pub unsafe fn change_lifetime_mut<'a, 'b, T>(x: &'a mut T) -> &'b mut T {
    &mut *(x as *mut T)
}

pub struct SyncMapRefMut<'a, V> {
    _g: MutexGuard<'a, ()>,
    value: Option<&'a mut V>,
}

impl<'a, V> Deref for SyncMapRefMut<'_, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, V> DerefMut for SyncMapRefMut<'_, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
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
    inner: Option<MapIterMut<'a, K, V>>,
}

impl<'a, K, V> Deref for IterMut<'a, K, V> {
    type Target = MapIterMut<'a, K, V>;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<'a, K, V> DerefMut for IterMut<'a, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().unwrap().next()
    }
}

impl<'a, K, V> IntoIterator for &'a SyncMapImpl<K, V>
    where
        K: Eq + Hash + Clone,
{
    type Item = (&'a K, &'a V);
    type IntoIter = MapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K, V> IntoIterator for SyncMapImpl<K, V>
    where
        K: Eq + Hash + Clone,
{
    type Item = (K, V);
    type IntoIter = MapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<K: Eq + Hash + Clone, V> From<Map<K, V>> for SyncMapImpl<K, V> {
    fn from(arg: Map<K, V>) -> Self {
        Self::from(arg)
    }
}

impl<K, V> serde::Serialize for SyncMapImpl<K, V>
    where
        K: Eq + Hash + Clone + Serialize,
        V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
    {
        let mut m = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self.iter() {
            m.serialize_key(k)?;
            m.serialize_value(v)?;
        }
        m.end()
    }
}

impl<'de, K, V> serde::Deserialize<'de> for SyncMapImpl<K, V>
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

impl<K, V> Debug for SyncMapImpl<K, V>
    where
        K: std::cmp::Eq + Hash + Clone + Debug,
        V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut m = f.debug_map();
        for (k, v) in self.iter() {
            m.key(k);
            m.value(v);
        }
        m.finish()
    }
}
