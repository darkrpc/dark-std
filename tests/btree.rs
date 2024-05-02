use dark_std::sync::SyncBtreeMap;
use std::ops::Deref;
use std::sync::Arc;

#[test]
pub fn test_empty() {
    let m: SyncBtreeMap<i32, i32> = SyncBtreeMap::new();
    assert_eq!(0, m.len());
}

#[test]
pub fn test_insert() {
    let m = SyncBtreeMap::<i32, i32>::new();
    let insert = m.insert(1, 2);
    assert_eq!(insert.is_none(), true);
}

#[test]
pub fn test_insert2() {
    let m = Arc::new(SyncBtreeMap::<String, String>::new());
    m.insert("/".to_string(), "1".to_string());
    m.insert("/js".to_string(), "2".to_string());
    m.insert("/fn".to_string(), "3".to_string());

    assert_eq!(&"1".to_string(), m.get("/").unwrap());
    assert_eq!(&"2".to_string(), m.get("/js").unwrap());
    assert_eq!(&"3".to_string(), m.get("/fn").unwrap());
}

// #[test]
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

#[test]
pub fn test_get() {
    let m = SyncBtreeMap::<i32, i32>::new();
    m.insert(1, 2);
    let g = m.get(&1).unwrap();
    assert_eq!(2, *g.deref());
}

#[test]
pub fn test_get_mut() {
    let m = SyncBtreeMap::<i32, i32>::new();
    m.insert(1, 2);
    let mut r = m.get_mut(&1).unwrap();
    *r = 0;
    let g = m.get(&1).unwrap();
    assert_eq!(&0, g);
}

#[test]
pub fn test_iter() {
    let m = SyncBtreeMap::<i32, i32>::new();
    m.insert(1, 2);
    for (k, v) in m.iter() {
        assert_eq!(*k, 1);
        assert_eq!(*v, 2);
    }
}

#[test]
pub fn test_iter_mut() {
    let m = SyncBtreeMap::<i32, i32>::new();
    m.insert(1, 2);
    for (k, v) in m.iter_mut() {
        assert_eq!(*k, 1);
        assert_eq!(*v, 2);
    }
}

#[test]
pub fn test_get_mut_not_eq_key() {
    let m = SyncBtreeMap::<i32, i32>::new();
    m.insert(1, 1);
    m.insert(2, 2);

    let v1 = m.get_mut(&1).unwrap();
    let v2 = m.get_mut(&2).unwrap();
    assert_eq!(*v1 + 1, *v2);
}
