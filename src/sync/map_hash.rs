use parking_lot::{ReentrantMutex, ReentrantMutexGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::{
    hash_map::IntoIter as MapIntoIter, hash_map::Iter as MapIter, hash_map::IterMut as MapIterMut,
    HashMap as Map, HashMap,
};
use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// this sync map used to many reader,writer less.space-for-time strategy
pub struct SyncHashMap<K: Eq + Hash, V> {
    locks: UnsafeCell<Map<K, ReentrantMutex<()>>>,
    dirty: UnsafeCell<Map<K, V>>,
    lock: ReentrantMutex<()>,
}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash, V> Send for SyncHashMap<K, V> {}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash, V> Sync for SyncHashMap<K, V> {}

impl<K, V> std::ops::Index<&K> for SyncHashMap<K, V>
    where
        K: Eq + Hash,
{
    type Output = V;

    fn index(&self, index: &K) -> &Self::Output {
        unsafe { &(&*self.dirty.get())[index] }
    }
}

impl<K, V> SyncHashMap<K, V>
    where
        K: Eq + Hash,
{
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            locks: UnsafeCell::new(Map::new()),
            dirty: UnsafeCell::new(Map::new()),
            lock: Default::default(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            locks: UnsafeCell::new(Map::new()),
            dirty: UnsafeCell::new(Map::with_capacity(capacity)),
            lock: Default::default(),
        }
    }

    pub fn with_map(map: Map<K, V>) -> Self {
        Self {
            locks: UnsafeCell::new(Map::new()),
            dirty: UnsafeCell::new(map),
            lock: Default::default(),
        }
    }

    pub fn insert(&self, k: K, v: V) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.insert(k, v);
        drop(g);
        r
    }

    pub fn insert_mut(&mut self, k: K, v: V) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(k, v)
    }

    pub fn remove(&self, k: &K) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.remove(k);
        drop(g);
        r
    }

    pub fn remove_mut(&mut self, k: &K) -> Option<V> {
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
    pub fn get_mut(&self, k: &K) -> Option<HashMapRefMut<'_, K, V>>
        where
            K: Hash + Eq + Clone,
    {
        let m = unsafe { &mut *self.locks.get() };
        if m.contains_key(k) == false {
            let g = ReentrantMutex::new(());
            m.insert(k.clone(), g);
        }
        let g = m.get(k).unwrap();
        Some(HashMapRefMut {
            k: unsafe { std::mem::transmute(&k) },
            m: self,
            _g: g.lock(),
            value: {
                let m = unsafe { &mut *self.dirty.get() };
                m.get_mut(k)?
            },
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

    pub fn iter_mut(&self) -> HashIterMut<'_, K, V> {
        return HashIterMut {
            _g: self.lock.lock(),
            inner: {
                let m = unsafe { &mut *self.dirty.get() };
                m.iter_mut()
            },
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

pub struct HashMapRefMut<'a, K: Eq + Hash, V> {
    k: &'a K,
    m: &'a SyncHashMap<K, V>,
    _g: ReentrantMutexGuard<'a, ()>,
    value: &'a mut V,
}

impl<'a, K: Eq + Hash, V> Drop for HashMapRefMut<'a, K, V> {
    fn drop(&mut self) {
        let m = unsafe { &mut *self.m.locks.get() };
        _ = m.remove(self.k);
    }
}

impl<'a, K: Eq + Hash, V> Deref for HashMapRefMut<'_, K, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, K: Eq + Hash, V> DerefMut for HashMapRefMut<'_, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

impl<'a, K: Eq + Hash, V> Debug for HashMapRefMut<'_, K, V>
    where
        V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<'a, K: Eq + Hash, V> Display for HashMapRefMut<'_, K, V>
    where
        V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<'a, K: Eq + Hash, V> PartialEq<Self> for HashMapRefMut<'_, K, V>
    where
        V: Eq,
{
    fn eq(&self, other: &Self) -> bool {
        self.value.eq(&other.value)
    }
}

impl<'a, K: Eq + Hash, V> Eq for HashMapRefMut<'_, K, V> where V: Eq {}

pub struct HashIterMut<'a, K, V> {
    _g: ReentrantMutexGuard<'a, ()>,
    inner: MapIterMut<'a, K, V>,
}

impl<'a, K, V> Deref for HashIterMut<'a, K, V> {
    type Target = MapIterMut<'a, K, V>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, K, V> DerefMut for HashIterMut<'a, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'a, K, V> Iterator for HashIterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<'a, K, V> IntoIterator for &'a SyncHashMap<K, V>
    where
        K: Eq + Hash,
{
    type Item = (&'a K, &'a V);
    type IntoIter = MapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K, V> IntoIterator for SyncHashMap<K, V>
    where
        K: Eq + Hash,
{
    type Item = (K, V);
    type IntoIter = MapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<K: Eq + Hash, V> From<Map<K, V>> for SyncHashMap<K, V> {
    fn from(arg: Map<K, V>) -> Self {
        Self::from(arg)
    }
}

impl<K, V> serde::Serialize for SyncHashMap<K, V>
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

impl<'de, K, V> serde::Deserialize<'de> for SyncHashMap<K, V>
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

impl<K, V> Debug for SyncHashMap<K, V>
    where
        K: Eq + Hash + Debug,
        V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.dirty_ref().fmt(f)
    }
}

impl<K, V> Display for SyncHashMap<K, V>
    where
        K: Eq + Hash + Display,
        V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Pointer;
        self.dirty_ref().fmt(f)
    }
}


impl<K: Clone + Eq + Hash, V: Clone> Clone for SyncHashMap<K, V> {
    fn clone(&self) -> Self {
        let c = (*self.dirty_ref()).clone();
        SyncHashMap::from(c)
    }
}