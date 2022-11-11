use dark_std::sync::SyncVec;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

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
    m.push("2".to_string()).await;
    m.push("3".to_string()).await;

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
    m.push(2).await;
    let g = m.get(0).unwrap();
    assert_eq!(&2, g);
}

#[tokio::test]
pub async fn test_get_mut() {
    let m = SyncVec::<i32>::new();
    m.push(2).await;
    let mut m0 = m.get_mut(0).await.unwrap();
    *m0 = 1;
    println!("{}", *m0);
    let g = m.get(0).unwrap();
    assert_eq!(&1, g);
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
    m.push(a).await;
    let g = m.get(0).unwrap();
    let rm = m.remove(0).await.unwrap();
    println!("rm:{:?}", rm);
    drop(rm);
    assert_eq!(true, m.is_empty());
    assert_eq!(true, m.dirty_ref().is_empty());
    assert_eq!(None, m.get(0));
    assert_eq!(&A { inner: 0 }, g);
}

#[tokio::test]
pub async fn test_remove2() {
    let m = SyncVec::<String>::new();
    for _ in 0..1000000 {
        m.push(String::from("safdfasdfasdfasdfasdfasdfsadf")).await;
    }
    sleep(Duration::from_secs(2));
    println!("start clean");
    m.clear().await;
    m.shrink_to_fit().await;
    println!("done,now you can see mem usage");
    sleep(Duration::from_secs(5));
    for _ in 0..1000000 {
        m.push(String::from("safdfasdfasdfasdfasdfasdfsadf")).await;
    }
    sleep(Duration::from_secs(2));
    println!("start clean");
    m.clear().await;
    m.shrink_to_fit().await;
    println!("done,now you can see mem usage");
    sleep(Duration::from_secs(5));
}

#[tokio::test]
pub async fn test_iter() {
    let m = SyncVec::<i32>::new();
    m.push(2).await;
    for v in m.iter() {
        assert_eq!(*v, 2);
    }
}

#[tokio::test]
pub async fn test_iter_mut() {
    let m = SyncVec::<i32>::new();
    m.push(2).await;
    for v in m.iter_mut().await {
        assert_eq!(*v, 2);
    }
}
