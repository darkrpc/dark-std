# dark-std
dark-std is an Implementation of asynchronous containers build on tokio.
It uses a read-write separation design borrowed from Golang

* defer!          (defer macro)
* SyncHashMap     (async HashMap)
* SyncBtreeMap    (async BtreeMap)
* SyncVec         (async Vec)
* WaitGroup       (async/blocking all support WaitGroup)

for example:
```rust
    #[tokio::test]
    pub async fn test_get() {
        let m = SyncHashMap::<i32, i32>::new();
        let insert = m.insert(1, 2).await;
        
        let g = m.get(&1).unwrap();//don't need lock and await
        assert_eq!(&2, g);
    }
```


wait group:
```rust
use std::time::Duration;
use tokio::time::sleep;
use dark_std::sync::WaitGroup;
#[tokio::test]
async fn test_wg() {
    let wg = WaitGroup::new();
    let wg2 = wg.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;
        wg2.done_async().await;
    });
    let wg2 = wg.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;
        wg2.done_async().await;
    });
    wg.wait_async().await;
    println!("all done");
}
```