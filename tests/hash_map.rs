use dark_std::sync::SyncHashMap;

use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

#[test]
pub fn test_debug() {
    let m: SyncHashMap<i32, i32> = SyncHashMap::new();
    m.insert(1, 1);
    println!("{:?}", m);
    assert_eq!(format!("{:?}", m), "{1: 1}");
}

#[test]
pub fn test_empty() {
    let m: SyncHashMap<i32, i32> = SyncHashMap::new();
    assert_eq!(0, m.len());
}

#[test]
pub fn test_insert() {
    let m = SyncHashMap::<i32, i32>::new();
    let insert = m.insert(1, 2);
    assert_eq!(insert.is_none(), true);
}

#[test]
pub fn test_insert2() {
    let m = Arc::new(SyncHashMap::<String, String>::new());
    m.insert("/".to_string(), "1".to_string());
    m.insert("/js".to_string(), "2".to_string());
    m.insert("/fn".to_string(), "3".to_string());

    assert_eq!(&"1".to_string(), m.get("/").unwrap());
    assert_eq!(&"2".to_string(), m.get("/js").unwrap());
    assert_eq!(&"3".to_string(), m.get("/fn").unwrap());
}

// #[test]
// pub fn test_insert3() {
//     let m = Arc::new(SyncHashMap::<i32, i32>::new());
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

// #[test]
// pub fn test_insert4() {
//     let m = Arc::new(SyncHashMap::<i32, i32>::new());
//     let wg = WaitGroup::new();
//     for _ in 0..8 {
//         let wg1 = wg.clone();
//         let wg2 = wg.clone();
//         let m1 = m.clone();
//         let m2 = m.clone();
//         co!(move ||{
//              for i in 0..10000{
//                  m1.remove(&i);
//                  let insert = m1.insert(i, i);
//              }
//              drop(wg1);
//         });
//         co!(move ||{
//              for i in 0..10000{
//                  m2.remove(&i);
//                  let insert = m2.insert(i, i);
//              }
//              drop(wg2);
//         });
//     }
//     wg.wait();
// }

#[test]
pub fn test_get() {
    let m = SyncHashMap::<i32, i32>::new();
    m.insert(1, 2);
    let g = m.get(&1).unwrap();
    assert_eq!(&2, g);
}

#[test]
pub fn test_get_mut() {
    let m = SyncHashMap::<i32, i32>::new();
    m.insert(1, 2);
    let mut r = m.get_mut(&1).unwrap();
    *r = 0;
    let g = m.get(&1).unwrap();
    assert_eq!(&0, g);
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
    let m = SyncHashMap::<i32, A>::new();
    m.insert(1, a);
    let g = m.get(&1).unwrap();
    let rm = m.remove(&1).unwrap();
    println!("rm:{:?}", rm);
    drop(rm);
    assert_eq!(true, m.is_empty());
    assert_eq!(true, m.dirty_ref().is_empty());
    assert_eq!(None, m.get(&1));
    assert_eq!(&A { inner: 0 }, g);
}

#[test]
pub fn test_remove2() {
    let m = SyncHashMap::<i32, String>::new();
    for i in 0..1000000 {
        m.insert(i, String::from("safdfasdfasdfasdfasdfasdfsadf"));
    }
    sleep(Duration::from_secs(2));
    println!("start clean");
    m.clear();
    m.shrink_to_fit();
    println!("done,now you can see mem usage");
    sleep(Duration::from_secs(5));
    for i in 0..1000000 {
        m.insert(i, String::from("safdfasdfasdfasdfasdfasdfsadf"));
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
    let m = SyncHashMap::<i32, i32>::new();
    m.insert(1, 2);
    for (k, v) in m.iter() {
        assert_eq!(*k, 1);
        assert_eq!(*v, 2);
    }
}

#[test]
pub fn test_iter_mut() {
    let m = SyncHashMap::<i32, i32>::new();
    m.insert(1, 2);
    for (k, v) in m.iter_mut() {
        assert_eq!(*k, 1);
        assert_eq!(*v, 2);
    }
}

#[test]
pub fn test_get_mut_not_eq_key() {
    let m = SyncHashMap::<i32, i32>::new();
    m.insert(1, 1);
    m.insert(2, 2);

    let v1 = m.get_mut(&1).unwrap();
    let v2 = m.get_mut(&2).unwrap();
    assert_eq!(*v1 + 1, *v2);
}

// #[test]
// pub fn test_smoke2() {
//     let wait1 = WaitGroup::new();
//     let m1 = Arc::new(SyncHashMap::<i32, i32>::new());
//     for i in 0..10000 {
//         let wg = wait1.clone();
//         let m = m1.clone();
//
//         let wg2 = wait1.clone();
//         let m2 = m1.clone();
//         co!(move ||{
//             let insert = m.insert(i, i);
//             let g = m.get(&i).unwrap();
//             assert_eq!(i, *g.deref());
//             drop(wg);
//             println!("done{}",i);
//         });
//         co!(move ||{
//              let g = m2.remove(&i);
//               if g.is_some(){
//               println!("done remove {}",i);
//               drop(wg2);} });
//     }
//     wait1.wait();
// }

// #[test]
// pub fn test_smoke3() {
//     let wait1 = WaitGroup::new();
//     let m1 = Arc::new(SyncHashMap::<i32, i32>::new());
//     for mut i in 0..10000 {
//         i = 1;
//         let wg = wait1.clone();
//         let m = m1.clone();
//         co!(move ||{
//             let insert = m.insert(i, i);
//             let g = m.get(&i).unwrap();
//             assert_eq!(i, *g.deref());
//             drop(wg);
//             println!("done{}",i);
//         });
//         let wg2 = wait1.clone();
//         let m2 = m1.clone();
//         co!(move ||{
//              let g = m2.remove(&i);
//              drop(wg2);
//         });
//     }
//     wait1.wait();
// }
