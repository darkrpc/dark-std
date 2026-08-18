use dark_std::sync::SyncVec;
use dark_std::sync_vec;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

#[test]
pub fn test_debug() {
    let m: SyncVec<i32> = SyncVec::new();
    m.push(1);
    println!("{:?}", m);
    assert_eq!(format!("{:?}", m), "[1]");
}

#[test]
pub fn test_empty() {
    let m: SyncVec<i32> = SyncVec::new();
    assert_eq!(0, m.len());
}

#[test]
pub fn test_push() {
    let m = SyncVec::<i32>::new();
    let insert = m.push(1);
    assert_eq!(insert.is_none(), true);
}

#[test]
pub fn test_push2() {
    let m = Arc::new(SyncVec::<String>::new());
    m.push("1".to_string());
    m.push("2".to_string());
    m.push("3".to_string());

    assert_eq!(&"1".to_string(), &*m.get(0).unwrap());
    assert_eq!(&"2".to_string(), &*m.get(1).unwrap());
    assert_eq!(&"3".to_string(), &*m.get(2).unwrap());
}

#[test]
pub fn test_get() {
    let m = SyncVec::<i32>::new();
    m.push(2);
    let g = m.get(0).unwrap();
    assert_eq!(&2, &*g);
}

#[test]
pub fn test_get_mut() {
    let m = SyncVec::<i32>::new();
    m.push(2);
    let mut m0 = m.get_mut(0).unwrap();
    *m0 = 1;
    println!("{}", *m0);
    drop(m0); // write guard holds the lock; drop it before reading
    let g = m.get(0).unwrap();
    assert_eq!(&1, &*g);
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

#[test]
pub fn test_remove() {
    let a = A { inner: 0 };
    let m = SyncVec::<A>::new();
    m.push(a);
    let g = m.get(0).unwrap();
    assert_eq!(&A { inner: 0 }, &*g);
    drop(g); // read guard holds the lock; drop it before writing
    let rm = m.remove(0).unwrap();
    println!("rm:{:?}", rm);
    drop(rm);
    assert_eq!(true, m.is_empty());
    assert_eq!(true, m.dirty_ref().is_empty());
    assert_eq!(None, m.get(0));
}

#[test]
pub fn test_remove2() {
    let m = SyncVec::<String>::new();
    for _ in 0..1000000 {
        m.push(String::from("safdfasdfasdfasdfasdfasdfsadf"));
    }
    sleep(Duration::from_secs(2));
    println!("start clean");
    m.clear();
    m.shrink_to_fit();
    println!("done,now you can see mem usage");
    sleep(Duration::from_secs(5));
    for _ in 0..1000000 {
        m.push(String::from("safdfasdfasdfasdfasdfasdfsadf"));
    }
    sleep(Duration::from_secs(2));
    println!("start clean");
    m.clear();
    m.shrink_to_fit();
    println!("done,now you can see mem usage");
    sleep(Duration::from_secs(5));
}

#[test]
pub fn test_iter() {
    let m = SyncVec::<i32>::new();
    m.push(2);
    for v in m.iter() {
        assert_eq!(*v, 2);
    }
}

#[test]
pub fn test_iter_mut() {
    let m = SyncVec::<i32>::new();
    m.push(2);
    for v in m.iter_mut() {
        assert_eq!(*v, 2);
    }
}

#[test]
pub fn test_macro() {
    let v = sync_vec![];
    v.push(1);
    assert_eq!(v, sync_vec![1]);
}

#[test]
pub fn test_macro2() {
    let v = sync_vec![1];
    v.push(2);
    assert_eq!(v, sync_vec![1, 2]);
}

#[test]
pub fn test_macro3() {
    let v = sync_vec![1;2];
    assert_eq!(&*v.dirty_ref(), &vec![1; 2]);
}

// Regression for the rbatis scenario: an async executor iterates the
// container (e.g. running interceptors) while holding the iterator across an
// `.await` inside a `tokio::spawn`ed task. That requires the iterator to be
// `Send`, otherwise the future "cannot be sent between threads safely".
#[tokio::test]
pub async fn test_iter_send_across_await() {
    let v = Arc::new(SyncVec::new());
    v.push(1);
    v.push(2);
    v.push(3);

    let v2 = v.clone();
    let sum = tokio::spawn(async move {
        let mut sum = 0;
        for item in v2.iter() {
            // keep the iterator alive across an await
            tokio::task::yield_now().await;
            sum += *item;
        }
        sum
    })
    .await
    .expect("spawned task panicked");
    assert_eq!(sum, 6);
}

// Compile-time guard: the read iterator must be `Send` for `V: Sync`.
#[test]
pub fn test_vec_iter_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<dark_std::sync::VecIter<'static, i32>>();
}

// `&SyncVec` must implement `IntoIterator` (the pattern `for x in &v`, used
// e.g. by fast_log's `for x in &self.modules`), not just the consuming one.
#[test]
pub fn test_iter_ref() {
    let v = SyncVec::new();
    v.push(1);
    v.push(2);
    v.push(3);
    let mut sum = 0;
    for x in &v {
        sum += *x;
    }
    assert_eq!(sum, 6);

    let mut via_into_iter = 0;
    for x in (&v).into_iter() {
        via_into_iter += *x;
    }
    assert_eq!(via_into_iter, 6);
}
