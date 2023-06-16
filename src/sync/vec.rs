use parking_lot::{ReentrantMutex, ReentrantMutexGuard};
use serde::{Deserializer, Serialize, Serializer};
use std::cell::UnsafeCell;
use std::fmt::{Debug, Display, Formatter};
use std::ops::{Deref, DerefMut, Index};
use std::slice::{Iter as SliceIter, IterMut as SliceIterMut};
use std::sync::Arc;
use std::vec::IntoIter;

pub struct SyncVec<V> {
    dirty: UnsafeCell<Vec<V>>,
    lock: ReentrantMutex<()>,
}

/// this is safety, dirty mutex ensure
unsafe impl<V> Send for SyncVec<V> {}

/// this is safety, dirty mutex ensure
unsafe impl<V> Sync for SyncVec<V> {}

impl<V> SyncVec<V> {
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

    pub fn insert(&self, index: usize, v: V) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(index, v);
        drop(g);
        None
    }

    pub fn insert_mut(&mut self, index: usize, v: V) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        m.insert(index, v);
        None
    }

    pub fn push(&self, v: V) -> Option<V> {
        let g = self.lock.lock();
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

    pub fn pop(&self) -> Option<V> {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        let r = m.pop();
        drop(g);
        r
    }

    pub fn pop_mut(&mut self) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        m.pop()
    }

    pub fn remove(&self, index: usize) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            let v = m.remove(index);
            Some(v)
        } else {
            None
        }
    }

    pub fn remove_mut(&mut self, index: usize) -> Option<V> {
        let m = unsafe { &mut *self.dirty.get() };
        if m.len() > index {
            let v = m.remove(index);
            Some(v)
        } else {
            None
        }
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

    pub fn shrink_to_fit(&self) {
        let g = self.lock.lock();
        let m = unsafe { &mut *self.dirty.get() };
        m.shrink_to_fit();
        drop(g);
    }

    pub fn from(vec: Vec<V>) -> Self {
        let s = Self::with_vec(vec);
        s
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&V> {
        unsafe {
            return (&*self.dirty.get()).get(index);
        }
    }

    #[inline]
    pub fn get_uncheck(&self, index: usize) -> &V {
        unsafe { (&*self.dirty.get()).get_unchecked(index) }
    }

    #[inline]
    pub fn get_mut(&self, index: usize) -> Option<VecRefMut<'_, V>> {
        let m = unsafe { &mut *self.dirty.get() };
        Some(VecRefMut {
            _g: self.lock.lock(),
            value: Some(m.get_mut(index)?),
        })
    }

    #[inline]
    pub fn contains(&self, x: &V) -> bool
        where
            V: PartialEq,
    {
        let m = unsafe { &mut *self.dirty.get() };
        m.contains(x)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, V> {
        unsafe { (&*self.dirty.get()).iter() }
    }

    pub fn iter_mut(&self) -> VecIterMut<'_, V> {
        let m = unsafe { &mut *self.dirty.get() };
        let mut iter = VecIterMut {
            _g: self.lock.lock(),
            inner: None,
        };
        iter.inner = Some(m.iter_mut());
        return iter;
    }

    pub fn into_iter(self) -> IntoIter<V> {
        let m = self.dirty.into_inner();
        m.into_iter()
    }

    pub fn dirty_ref(&self) -> &Vec<V> {
        unsafe { &*self.dirty.get() }
    }

    pub fn into_inner(self) -> Vec<V> {
        self.dirty.into_inner()
    }
}

pub struct VecRefMut<'a, V> {
    _g: ReentrantMutexGuard<'a, ()>,
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

impl<'a, V> Display for VecRefMut<'_, V>
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

pub struct VecIterMut<'a, V> {
    _g: ReentrantMutexGuard<'a, ()>,
    inner: Option<SliceIterMut<'a, V>>,
}

impl<'a, V> Deref for VecIterMut<'a, V> {
    type Target = SliceIterMut<'a, V>;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<'a, V> DerefMut for VecIterMut<'a, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, V> Iterator for VecIterMut<'a, V> {
    type Item = &'a mut V;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().unwrap().next()
    }
}

impl<'a, V> IntoIterator for &'a SyncVec<V> {
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
        self.dirty_ref().serialize(serializer)
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
        self.dirty_ref().fmt(f)
    }
}

impl<V> Display for SyncVec<V>
    where
        V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Pointer;
        self.dirty_ref().fmt(f)
    }
}

impl<V> Index<usize> for SyncVec<V> {
    type Output = V;

    fn index(&self, index: usize) -> &Self::Output {
        self.get_uncheck(index)
    }
}

impl<V: PartialEq> PartialEq for SyncVec<V> {
    fn eq(&self, other: &Self) -> bool {
        self.dirty_ref().eq(other.dirty_ref())
    }
}

impl<V: Clone> Clone for SyncVec<V> {
    fn clone(&self) -> Self {
        SyncVec::from(self.dirty_ref().to_vec())
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
