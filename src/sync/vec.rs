use std::cell::UnsafeCell;
use std::fmt::{Debug, Formatter};
use std::ops::{Deref, DerefMut, Index};
use std::sync::Arc;
use std::slice::{Iter as SliceIter, IterMut as SliceIterMut};
use serde::{Deserializer, Serialize, Serializer};
use serde::ser::SerializeSeq;


use tokio::sync::{Mutex, MutexGuard};


pub type SyncVec<V> = SyncVecImpl<V>;


pub struct SyncVecImpl<V> {
    read: UnsafeCell<Vec<V>>,
    dirty: Mutex<Vec<V>>,
}

impl<V> Drop for SyncVecImpl<V> {
    fn drop(&mut self) {
        unsafe {
            loop {
                match (&mut *self.read.get()).pop() {
                    None => {
                        break;
                    }
                    Some(v) => {
                        std::mem::forget(v)
                    }
                }
            }
        }
    }
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
            read: UnsafeCell::new(Vec::new()),
            dirty: Mutex::new(Vec::new()),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            read: UnsafeCell::new(Vec::with_capacity(capacity)),
            dirty: Mutex::new(Vec::with_capacity(capacity)),
        }
    }

    pub async fn insert(&self, index: usize, v: V) -> Option<V> {
        let mut m = self.dirty.lock().await;
        m.insert(index, v);
        let len = m.len();
        unsafe {
            let r = m.get_unchecked(len - 1);
            (&mut *self.read.get()).insert(index, std::ptr::read(r));
        }
        None
    }

    pub async fn push(&self, v: V) -> Option<V> {
        let mut m = self.dirty.lock().await;
        m.push(v);
        let len = m.len();
        unsafe {
            let r = m.get_unchecked(len - 1);
            (&mut *self.read.get()).push(std::ptr::read(r));
        }
        None
    }

    pub fn push_mut(&mut self, v: V) -> Option<V> {
        let m = self.dirty.get_mut();
        m.push(v);
        let len = m.len();
        unsafe {
            let r = m.get_unchecked(len - 1);
            (&mut *self.read.get()).push(std::ptr::read(r));
        }
        None
    }

    pub async fn pop(&self) -> Option<V> {
        let mut m = self.dirty.lock().await;
        match m.pop() {
            None => {
                return None;
            }
            Some(s) => {
                unsafe {
                    let r = (&mut *self.read.get()).pop();
                    match r {
                        None => {}
                        Some(r) => {
                            std::mem::forget(r);
                        }
                    }
                }
                return Some(s);
            }
        }
    }

    pub fn pop_mut(&mut self) -> Option<V> {
        let m = self.dirty.get_mut();
        match m.pop() {
            None => {
                return None;
            }
            Some(s) => {
                unsafe {
                    let r = (&mut *self.read.get()).pop();
                    match r {
                        None => {}
                        Some(r) => {
                            std::mem::forget(r);
                        }
                    }
                }
                return Some(s);
            }
        }
    }

    pub async fn remove(&self, index: usize) -> Option<V> {
        match self.get(index) {
            None => {
                None
            }
            Some(_) => {
                let mut m = self.dirty.lock().await;
                let v = m.remove(index);
                unsafe {
                    let r = (&mut *self.read.get()).remove(index);
                    std::mem::forget(r);
                }
                Some(v)
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
        let mut m = self.dirty.lock().await;
        m.clear();
        unsafe {
            loop {
                match (&mut *self.read.get()).pop() {
                    None => {
                        break;
                    }
                    Some(v) => {
                        std::mem::forget(v)
                    }
                }
            }
        }
    }

    pub async fn shrink_to_fit(&self) {
        let mut m = self.dirty.lock().await;
        unsafe {
            (&mut *self.read.get()).shrink_to_fit()
        }
        m.shrink_to_fit()
    }

    pub fn from(map: Vec<V>) -> Self {
        let mut s = Self::with_capacity(map.capacity());
        let m = s.dirty.get_mut();
        *m = map;
        unsafe {
            for v in m.iter() {
                (&mut *s.read.get()).push(std::ptr::read(v));
            }
        }
        drop(m);
        s
    }

    pub fn get(&self, index: usize) -> Option<&V>
    {
        unsafe {
            let k = (&*self.read.get()).get(index);
            match k {
                None => { None }
                Some(s) => {
                    Some(s)
                }
            }
        }
    }

    pub unsafe fn get_uncheck(&self, index: usize) -> Option<&V>
    {
        let k = (&*self.read.get()).get_unchecked(index);
        Some(k)
    }

    pub async fn get_mut(&self, index: usize) -> Option<VecRefMut<'_, V>>
    {
        let m = self.dirty.lock().await;
        let mut r = VecRefMut {
            g: m,
            value: None,
        };
        unsafe {
            r.value = Some(change_lifetime_mut(r.g.get_mut(index)?));
        }
        Some(r)
    }

    pub fn iter(&self) -> SliceIter<'_, V> {
        unsafe {
            (&*self.read.get()).iter()
        }
    }

    pub async fn iter_mut(&self) -> IterMut<'_, V> {
        let m = self.dirty.lock().await;
        let mut iter = IterMut {
            g: m,
            inner: None,
        };
        unsafe {
            iter.inner = Some(change_lifetime_mut(&mut iter.g).iter_mut());
        }
        return iter;
    }

    pub fn into_iter(self) -> SliceIter<'static, V> {
        unsafe {
            (&*self.read.get()).iter()
        }
    }
}


pub unsafe fn change_lifetime_const<'a, 'b, T>(x: &'a T) -> &'b T {
    &*(x as *const T)
}

pub unsafe fn change_lifetime_mut<'a, 'b, T>(x: &'a mut T) -> &'b mut T {
    &mut *(x as *mut T)
}

pub struct VecRefMut<'a, V> {
    g: MutexGuard<'a, Vec<V>>,
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

impl<'a, V> Debug for VecRefMut<'_, V> where V: Debug {
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
            None => { None }
            Some(v) => {
                if v.is_null() {
                    None
                } else {
                    unsafe {
                        Some(&**v)
                    }
                }
            }
        }
    }
}


pub struct IterMut<'a, V> {
    g: MutexGuard<'a, Vec<V>>,
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
    type IntoIter = SliceIter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}


impl<V> serde::Serialize for SyncVecImpl<V> where V: Serialize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        let mut m = serializer.serialize_seq(Some(self.len()))?;
        for v in self.iter() {
            m.serialize_element(v)?;
        }
        m.end()
    }
}

impl<'de, V> serde::Deserialize<'de> for SyncVecImpl<V> where V: serde::Deserialize<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
        let m = Vec::deserialize(deserializer)?;
        Ok(Self::from(m))
    }
}

impl<V> Debug for SyncVecImpl<V> where V: Debug {
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

#[cfg(test)]
mod test {
    use std::ops::Deref;
    use std::sync::Arc;
    use std::sync::atomic::{Ordering};
    use std::thread::sleep;
    use std::time::Duration;
    use crate::sync::vec::SyncVec;

    #[tokio::test]
    pub async fn test_debug() {
        let m: SyncVec<i32> = SyncVec::new();
        m.push(1).await;
        println!("{:?}", m);
        assert_eq!(format!("{:?}", m), "[1]");
    }

    #[test]
    pub fn test_empty() {
        let m: SyncVec<i32> = SyncVec::new();
        assert_eq!(0, m.len());
    }

    #[tokio::test]
    pub async fn test_push() {
        let m = SyncVec::<i32>::new();
        let insert = m.push(1).await;
        assert_eq!(insert.is_none(), true);
    }

    #[tokio::test]
    pub async fn test_push2() {
        let m = Arc::new(SyncVec::<String>::new());
        m.push("1".to_string()).await;
        ;
        m.push("2".to_string()).await;
        ;
        m.push("3".to_string()).await;
        ;

        assert_eq!(&"1".to_string(), m.get(0).unwrap());
        assert_eq!(&"2".to_string(), m.get(1).unwrap());
        assert_eq!(&"3".to_string(), m.get(2).unwrap());
    }

    // #[test]
    // pub fn test_insert3() {
    //     let m = Arc::new(SyncVec::<i32>::new());
    //     let wg = WaitGroup::new();
    //     for _ in 0..100000 {
    //         let wg1 = wg.clone();
    //         let wg2 = wg.clone();
    //         let m1 = m.clone();
    //         let m2 = m.clone();
    //         co!(move ||{
    //              m1.pop();
    //              let insert = m1.push( 2);
    //              drop(wg1);
    //         });
    //         co!(move ||{
    //              m2.pop();
    //              let insert = m2.push( 2);
    //              drop(wg2);
    //         });
    //     }
    //     wg.wait();
    // }

    // #[test]
    // pub fn test_insert4() {
    //     let m = Arc::new(SyncVec::<i32>::new());
    //     let wg = WaitGroup::new();
    //     for _ in 0..8 {
    //         let wg1 = wg.clone();
    //         let wg2 = wg.clone();
    //         let m1 = m.clone();
    //         let m2 = m.clone();
    //         co!(move ||{
    //              for i in 0..10000{
    //                  m1.pop();
    //                  let insert = m1.push( i);
    //              }
    //              drop(wg1);
    //         });
    //         co!(move ||{
    //              for i in 0..10000{
    //                  m2.pop();
    //                  let insert = m2.push( i);
    //              }
    //              drop(wg2);
    //         });
    //     }
    //     wg.wait();
    // }

    #[tokio::test]
    pub async fn test_get() {
        let m = SyncVec::<i32>::new();
        let insert = m.push(2).await;
        let g = m.get(0).unwrap();
        assert_eq!(&2, g);
    }

    #[derive(Clone, Debug, Eq, PartialEq, Hash)]
    pub struct A {
        inner: i32,
    }

    impl Drop for A {
        fn drop(&mut self) {
            println!("droped");
        }
    }

    #[tokio::test]
    pub async fn test_remove() {
        let a = A { inner: 0 };
        let m = SyncVec::<A>::new();
        let insert = m.push(a).await;
        let g = m.get(0).unwrap();
        let rm = m.remove(0).await.unwrap();
        println!("rm:{:?}", rm);
        drop(rm);
        assert_eq!(true, m.is_empty());
        assert_eq!(true, m.dirty.lock().await.is_empty());
        assert_eq!(None, m.get(0));
        assert_eq!(&A { inner: 0 }, g);
    }

    #[tokio::test]
    pub async fn test_remove2() {
        let m = SyncVec::<String>::new();
        for i in 0..1000000 {
            m.push(String::from("safdfasdfasdfasdfasdfasdfsadf")).await;
            ;
        }
        sleep(Duration::from_secs(2));
        println!("start clean");
        m.clear();
        m.shrink_to_fit();
        println!("done,now you can see mem usage");
        sleep(Duration::from_secs(5));
        for i in 0..1000000 {
            m.push(String::from("safdfasdfasdfasdfasdfasdfsadf")).await;
            ;
        }
        sleep(Duration::from_secs(2));
        println!("start clean");
        m.clear();
        m.shrink_to_fit();
        println!("done,now you can see mem usage");
        sleep(Duration::from_secs(5));
    }

    #[tokio::test]
    pub async fn test_iter() {
        let m = SyncVec::<i32>::new();
        let insert = m.push(2).await;
        ;
        for v in m.iter() {
            assert_eq!(*v, 2);
        }
    }

    #[tokio::test]
    pub async fn test_iter_mut() {
        let m = SyncVec::<i32>::new();
        let insert = m.push(2).await;
        ;
        for v in m.iter_mut().await {
            assert_eq!(*v, 2);
        }
    }
}