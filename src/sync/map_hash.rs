use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::fmt::{Debug, Formatter};
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::collections::{HashMap as Map, hash_map::Iter as MapIter, hash_map::IterMut as MapIterMut, hash_map::IntoIter as MapIntoIter, HashMap};
use serde::ser::SerializeMap;
use serde::{Deserializer, Serialize, Serializer};

use tokio::sync::{Mutex, MutexGuard};

pub type SyncHashMap<K, V> = SyncMapImpl<K, V>;

/// this sync map used to many reader,writer less.space-for-time strategy
///
/// Map is like a Go map[interface{}]interface{} but is safe for concurrent use
/// by multiple goroutines without additional locking or coordination.
/// Loads, stores, and deletes run in amortized constant time.
///
/// The Map type is specialized. Most code should use a plain Go map instead,
/// with separate locking or coordination, for better type safety and to make it
/// easier to maintain other invariants along with the map content.
///
/// The Map type is optimized for two common use cases: (1) when the entry for a given
/// key is only ever written once but read many times, as in caches that only grow,
/// or (2) when multiple goroutines read, write, and overwrite entries for disjoint
/// sets of keys. In these two cases, use of a Map may significantly reduce lock
/// contention compared to a Go map paired with a separate Mutex or RWMutex.
///
/// The zero Map is empty and ready for use. A Map must not be copied after first use.
pub struct SyncMapImpl<K: Eq + Hash + Clone, V> {
    read: UnsafeCell<Map<K, V>>,
    dirty: Option<Mutex<Map<K, V>>>,
}

impl<K: Eq + Hash + Clone, V> Drop for SyncMapImpl<K, V> {
    fn drop(&mut self) {
        unsafe {
            let k = (&mut *self.read.get()).keys().clone();
            for x in k {
                let v = (&mut *self.read.get()).remove(x);
                match v {
                    None => {}
                    Some(v) => {
                        std::mem::forget(v);
                    }
                }
            }
        }
    }
}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash + Clone, V> Send for SyncMapImpl<K, V> {}

/// this is safety, dirty mutex ensure
unsafe impl<K: Eq + Hash + Clone, V> Sync for SyncMapImpl<K, V> {}


impl<K, V> SyncMapImpl<K, V> where K: std::cmp::Eq + Hash + Clone {
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            read: UnsafeCell::new(Map::new()),
            dirty: Some(Mutex::new(Map::new())),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            read: UnsafeCell::new(Map::with_capacity(capacity)),
            dirty: Some(Mutex::new(Map::with_capacity(capacity))),
        }
    }


    pub async fn insert(&self, k: K, v: V) -> Option<V> where K: Clone {
        if self.dirty.is_none() {
            return None;
        }
        let mut m = self.dirty.as_ref().unwrap().lock().await;
        let op = m.insert(k.clone(), v);
        match op {
            None => {
                let r = m.get(&k);
                unsafe {
                    (&mut *self.read.get()).insert(k, std::mem::transmute_copy(r.unwrap()));
                }
                None
            }
            Some(v) => {
                Some(v)
            }
        }
    }

    pub fn insert_mut(&mut self, k: K, v: V) -> Option<V> where K: Clone {
        let m = self.dirty.as_mut().expect("dirty is removed").get_mut();
        let op = m.insert(k.clone(), v);
        match op {
            None => {
                let r = m.get(&k);
                unsafe {
                    (&mut *self.read.get()).insert(k, std::mem::transmute_copy(r.unwrap()));
                }
                None
            }
            Some(v) => {
                Some(v)
            }
        }
    }

    pub async fn remove(&self, k: &K) -> Option<V> where K: Clone {
        if self.dirty.is_none() {
            return None;
        }
        let mut m = self.dirty.as_ref().unwrap().lock().await;
        let op = m.remove(k);
        match op {
            Some(v) => {
                unsafe {
                    let r = (&mut *self.read.get()).remove(k);
                    match r {
                        None => {}
                        Some(r) => {
                            std::mem::forget(r);
                        }
                    }
                }
                Some(v)
            }
            None => {
                None
            }
        }
    }

    pub fn remove_mut(&mut self, k: &K) -> Option<V> where K: Clone {
        let m = self.dirty.as_mut().expect("dirty is removed").get_mut();
        let op = m.remove(k);
        match op {
            Some(v) => {
                unsafe {
                    let r = (&mut *self.read.get()).remove(k);
                    match r {
                        None => {}
                        Some(r) => {
                            std::mem::forget(r);
                        }
                    }
                }
                Some(v)
            }
            None => {
                None
            }
        }
    }


    pub fn len(&self) -> usize {
        unsafe {
            (&*self.read.get()).len()
        }
    }

    pub fn is_empty(&self) -> bool {
        unsafe {
            (&*self.read.get()).is_empty()
        }
    }

    pub async fn clear(&self) {
        if self.dirty.is_none() {
            return;
        }
        let mut m = self.dirty.as_ref().unwrap().lock().await;
        m.clear();
        unsafe {
            let k = (&mut *self.read.get()).keys().clone();
            for x in k {
                let v = (&mut *self.read.get()).remove(x);
                match v {
                    None => {}
                    Some(v) => {
                        std::mem::forget(v);
                    }
                }
            }
        }
    }

    pub fn clear_mut(&mut self) {
        let m = self.dirty.as_mut().expect("dirty is removed").get_mut();
        m.clear();
        unsafe {
            let k = (&mut *self.read.get()).keys().clone();
            for x in k {
                let v = (&mut *self.read.get()).remove(x);
                match v {
                    None => {}
                    Some(v) => {
                        std::mem::forget(v);
                    }
                }
            }
        }
    }

    pub async fn shrink_to_fit(&self) {
        if self.dirty.is_none() {
            return;
        }
        let mut m = self.dirty.as_ref().unwrap().lock().await;
        unsafe {
            (&mut *self.read.get()).shrink_to_fit()
        }
        m.shrink_to_fit()
    }

    pub async fn shrink_to_fit_mut(&mut self) {
        if self.dirty.is_none() {
            return;
        }
        let m = self.dirty.as_mut().unwrap().get_mut();
        unsafe {
            (&mut *self.read.get()).shrink_to_fit()
        }
        m.shrink_to_fit()
    }

    pub fn from(map: Map<K, V>) -> Self where K: Clone + Eq + Hash {
        let mut s = Self::with_capacity(map.capacity());
        let m = s.dirty.as_mut().expect("dirty is removed").get_mut();
        *m = map;
        unsafe {
            for (k, v) in m.iter() {
                (&mut *s.read.get()).insert(k.clone(), std::mem::transmute_copy(v));
            }
        }
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
            let k = (&*self.read.get()).get(k);
            match k {
                None => { None }
                Some(s) => {
                    Some(s)
                }
            }
        }
    }

    pub async fn get_mut<Q: ?Sized>(&self, k: &Q) -> Option<SyncMapRefMut<'_, K, V>>
        where
            K: Borrow<Q>,
            Q: Hash + Eq,
    {
        if self.dirty.is_none() {
            return None;
        }
        let m = self.dirty.as_ref().unwrap().lock().await;
        let mut r = SyncMapRefMut {
            g: m,
            value: None,
        };
        unsafe {
            r.value = Some(change_lifetime_mut(r.g.get_mut(k)?));
        }
        Some(r)
    }

    pub fn iter(&self) -> MapIter<'_, K, V> {
        unsafe {
            (&*self.read.get()).iter()
        }
    }

    pub async fn iter_mut(&self) -> IterMut<'_, K, V> {
        let m = self.dirty.as_ref().expect("dirty is removed").lock().await;
        let mut iter = IterMut {
            g: m,
            inner: None,
        };
        unsafe {
            iter.inner = Some(change_lifetime_mut(&mut iter.g).iter_mut());
        }
        return iter;
    }

    pub fn into_iter(mut self) -> MapIntoIter<K, V> {
        unsafe {
            (*self.read.get()).clear();
        }
        self.dirty.take().expect("dirty is None!").into_inner().into_iter()
    }

    pub async fn dirty_ref(&self) -> MutexGuard<'_, HashMap<K, V>> {
        self.dirty.as_ref().expect("dirty is removed").lock().await
    }
}

pub unsafe fn change_lifetime_const<'a, 'b, T>(x: &'a T) -> &'b T {
    &*(x as *const T)
}

pub unsafe fn change_lifetime_mut<'a, 'b, T>(x: &'a mut T) -> &'b mut T {
    &mut *(x as *mut T)
}

pub struct SyncMapRefMut<'a, K, V> {
    g: MutexGuard<'a, Map<K, V>>,
    value: Option<&'a mut V>,
}


impl<'a, K, V> Deref for SyncMapRefMut<'_, K, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, K, V> DerefMut for SyncMapRefMut<'_, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<'a, K, V> Debug for SyncMapRefMut<'_, K, V> where V: Debug {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}


impl<'a, K, V> PartialEq<Self> for SyncMapRefMut<'_, K, V> where V: Eq {
    fn eq(&self, other: &Self) -> bool {
        self.value.eq(&other.value)
    }
}

impl<'a, K, V> Eq for SyncMapRefMut<'_, K, V> where V: Eq {}


pub struct Iter<'a, K, V> {
    inner: Option<MapIter<'a, K, *const V>>,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.inner.as_mut().unwrap().next();
        match next {
            None => { None }
            Some((k, v)) => {
                if v.is_null() {
                    None
                } else {
                    unsafe {
                        Some((k, &**v))
                    }
                }
            }
        }
    }
}

pub struct IterMut<'a, K, V> {
    g: MutexGuard<'a, Map<K, V>>,
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

impl<'a, K, V> IntoIterator for &'a SyncMapImpl<K, V> where K: Eq + Hash + Clone {
    type Item = (&'a K, &'a V);
    type IntoIter = MapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K, V> IntoIterator for SyncMapImpl<K, V> where K: Eq + Hash + Clone {
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

impl<K, V> serde::Serialize for SyncMapImpl<K, V> where K: Eq + Hash + Clone + Serialize, V: Serialize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        let mut m = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self.iter() {
            m.serialize_key(k)?;
            m.serialize_value(v)?;
        }
        m.end()
    }
}

impl<'de, K, V> serde::Deserialize<'de> for SyncMapImpl<K, V> where K: Eq + Hash + Clone + serde::Deserialize<'de>, V: serde::Deserialize<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
        let m = Map::deserialize(deserializer)?;
        Ok(Self::from(m))
    }
}

impl<K, V> Debug for SyncMapImpl<K, V> where K: std::cmp::Eq + Hash + Clone + Debug, V: Debug {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut m = f.debug_map();
        for (k, v) in self.iter() {
            m.key(k);
            m.value(v);
        }
        m.finish()
    }
}

