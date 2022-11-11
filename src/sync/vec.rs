
use serde::ser::SerializeSeq;
use serde::{Deserializer, Serialize, Serializer};
use std::cell::UnsafeCell;
use std::fmt::{Debug, Formatter};
use std::ops::{Deref, DerefMut, Index};
use std::slice::{Iter as SliceIter, IterMut as SliceIterMut};
use std::sync::Arc;
use std::vec::IntoIter;

use tokio::sync::{Mutex, MutexGuard};

pub type SyncVec<V> = SyncVecImpl<V>;

pub struct SyncVecImpl<V> {
    dirty: UnsafeCell<Vec<V>>,
    lock: Mutex<()>,
}

/// this is safety, dirty mutex ensure
unsafe impl<V> Send for SyncVecImpl<V> {}

/// this is safety, dirty mutex ensure
unsafe impl<V> Sync for SyncVecImpl<V> {}

impl<V> SyncVecImpl<V> {
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn new() -> Self {
        Self {
            dirty: UnsafeCell::new(Vec::new()),
            lock: Default::default(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            dirty: UnsafeCell::new(Vec::with_capacity(capacity)),
            lock: Default::default(),
        }
    }

    pub fn with_vec(vec: Vec<V>) -> Self {
        Self {
            dirty: UnsafeCell::new(vec),
            lock: Default::default(),
        }
    }

    pub async fn insert(&self, index: usize, v: V) -> Option<V> {
        let g=self.lock.lock().await;
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(index, v);
        drop(g);
        None
    }

    pub async fn push(&self, v: V) -> Option<V> {
        let g=self.lock.lock().await;
        let m = unsafe { &mut *self.dirty.get() };
        m.push(v);
        drop(g);
        None
    }

    pub fn push_mut(&mut self, v: V) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        m.push(v);
        None
    }

    pub async fn pop(&self) -> Option<V> {
        let g=self.lock.lock().await;
        let m = unsafe { &mut *self.dirty.get() };
        match m.pop() {
            None => {
                return None;
            }
            Some(s) => {
                drop(g);
                return Some(s);
            }
        }
    }

    pub fn pop_mut(&mut self) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        match m.pop() {
            None => {
                return None;
            }
            Some(s) => {
                return Some(s);
            }
        }
    }

    pub async fn remove(&self, index: usize) -> Option<V> {
        let g=self.lock.lock().await;
        match self.get(index) {
            None => None,
            Some(_) => {
                drop(g);
                let m = unsafe { &mut *self.dirty.get() };
                let v = m.remove(index);
                Some(v)
            }
        }
    }

    pub fn len(&self) -> usize {
        unsafe { (&*self.dirty.get()).len() }
    }

    pub fn is_empty(&self) -> bool {
        unsafe { (&*self.dirty.get()).is_empty() }
    }

    pub async fn clear(&self) {
        let g=self.lock.lock().await;
        let m = unsafe { &mut *self.dirty.get() };
        m.clear();
        drop(g);
    }

    pub async fn shrink_to_fit(&self) {
        let g=self.lock.lock().await;
        let m = unsafe { &mut *self.dirty.get() };
        m.shrink_to_fit();
        drop(g);
    }

    pub fn from(vec: Vec<V>) -> Self {
        let s = Self::with_vec(vec);
        s
    }

    pub fn get(&self, index: usize) -> Option<&V> {
        unsafe {
            return (&*self.dirty.get()).get(index);
        }
    }

    pub unsafe fn get_uncheck(&self, index: usize) -> &V {
        (&*self.dirty.get()).get_unchecked(index)
    }

    pub async fn get_mut(&self, index: usize) -> Option<VecRefMut<'_, V>> {
        let m = unsafe { &mut *self.dirty.get() };
        let mut r = VecRefMut { _g: self.lock.lock().await, value: None };
        r.value = Some(m.get_mut(index)?);
        Some(r)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, V> {
        unsafe { (&*self.dirty.get()).iter() }
    }

    pub async fn iter_mut(&self) -> IterMut<'_, V> {
        let m = unsafe { &mut *self.dirty.get() };
        let mut iter = IterMut { _g: self.lock.lock().await, inner: None };
        iter.inner = Some(m.iter_mut());
        return iter;
    }

    pub fn into_iter(self) -> IntoIter<V> {
        let m = self.dirty.into_inner();
        m.into_iter()
    }

    pub async fn dirty_ref(&self) -> &mut Vec<V> {
        unsafe { &mut *self.dirty.get() }
    }
}

pub struct VecRefMut<'a, V> {
    _g: MutexGuard<'a, ()>,
    value: Option<&'a mut V>,
}

impl<'a, V> Deref for VecRefMut<'_, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, V> DerefMut for VecRefMut<'_, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<'a, V> Debug for VecRefMut<'_, V>
    where
        V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
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

pub struct IterMut<'a, V> {
    _g: MutexGuard<'a, ()>,
    inner: Option<SliceIterMut<'a, V>>,
}

impl<'a, V> Deref for IterMut<'a, V> {
    type Target = SliceIterMut<'a, V>;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<'a, V> DerefMut for IterMut<'a, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, V> Iterator for IterMut<'a, V> {
    type Item = &'a mut V;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().unwrap().next()
    }
}

impl<'a, V> IntoIterator for &'a SyncVecImpl<V> {
    type Item = &'a V;
    type IntoIter = std::slice::Iter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<V> IntoIterator for SyncVecImpl<V> {
    type Item = V;
    type IntoIter = IntoIter<V>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<V> Serialize for SyncVecImpl<V>
    where
        V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
    {
        let mut m = serializer.serialize_seq(Some(self.len()))?;
        for v in self.iter() {
            m.serialize_element(v)?;
        }
        m.end()
    }
}

impl<'de, V> serde::Deserialize<'de> for SyncVecImpl<V>
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

impl<V> Debug for SyncVecImpl<V>
    where
        V: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut m = f.debug_list();
        for v in self.iter() {
            m.entry(v);
        }
        m.finish()
    }
}

impl<V> Index<usize> for SyncVecImpl<V> {
    type Output = V;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).unwrap()
    }
}
