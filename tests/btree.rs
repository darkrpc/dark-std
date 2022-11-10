
use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use dark_std::sync::SyncBtreeMap;

#[tokio::test]
pub async fn test_empty() {
    let m: SyncBtreeMap<i32, i32> = SyncBtreeMap::new();
    assert_eq!(0, m.len());
}

#[tokio::test]
pub async fn test_insert() {
    let m = SyncBtreeMap::<i32, i32>::new();
    let insert = m.insert(1, 2).await;
    assert_eq!(insert.is_none(), true);
}

#[tokio::test]
pub async fn test_insert2() {
    let m = Arc::new(SyncBtreeMap::<String, String>::new());
    m.insert("/".to_string(), "1".to_string()).await;
    m.insert("/js".to_string(), "2".to_string()).await;
    m.insert("/fn".to_string(), "3".to_string()).await;

    assert_eq!(&"1".to_string(), m.get("/").unwrap());
    assert_eq!(&"2".to_string(), m.get("/js").unwrap());
    assert_eq!(&"3".to_string(), m.get("/fn").unwrap());
}

// #[tokio::test]
// pub fn test_insert3() {
//     let m = Arc::new(SyncBtreeMap::<i32, i32>::new());
//     let wg = WaitGroup::new();
//     for _ in 0..100000 {
//         let wg1 = wg.clone();
//         let wg2 = wg.clone();
//         let m1 = m.clone();
//         let m2 = m.clone();
//         co!(move ||{
//              m1.remove(&1);
//              let insert = m1.insert(1, 2);
//              drop(wg1);
//         });
//         co!(move ||{
//              m2.remove(&1);
//              let insert = m2.insert(1, 2);
//              drop(wg2);
//         });
//     }
//     wg.wait();
// }

#[tokio::test]
pub async fn test_get() {
    let m = SyncBtreeMap::<i32, i32>::new();
    let insert = m.insert(1, 2).await;
    let g = m.get(&1).unwrap();
    assert_eq!(2, *g.deref());
}

#[tokio::test]
pub async fn test_iter() {
    let m = SyncBtreeMap::<i32, i32>::new();
    let insert = m.insert(1, 2).await;
    for (k, v) in m.iter() {
        assert_eq!(*k, 1);
        assert_eq!(*v, 2);
    }
}

#[tokio::test]
pub async fn test_iter_mut() {
    let m = SyncBtreeMap::<i32, i32>::new();
    let insert = m.insert(1, 2).await;
    for (k, v) in m.iter_mut().await {
        assert_eq!(*k, 1);
        assert_eq!(*v, 2);
    }
}
