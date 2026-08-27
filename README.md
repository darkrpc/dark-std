# dark-std
dark-std is an Implementation of asynchronous

The sync containers (`SyncHashMap`, `SyncBtreeMap`, `SyncIndexMap`, `SyncVec`) use a Go
`sync.Map`-style **read/dirty + atomic snapshot** architecture: reads are lock-free and
served from an immutable, atomically published snapshot, while writes go to a `dirty` map
under a lock and are lazily published into a fresh snapshot — a **read-fast, write-slow**
design optimized for many readers, few writers.

* defer!          (defer macro)
* SyncHashMap     (async HashMap)
* SyncBtreeMap    (async BtreeMap)
* SyncVec         (async Vec)
* WaitGroup       (async/blocking all support WaitGroup)
* AtomicDuration  (atomic duration)

The containers do **not** require `V: Clone`. Because `get` hands out
references that stay valid until the container is dropped, the APIs that
return an owned value out of the container (`insert` / `remove` / `pop`,
and the copy-on-write `get_mut` / `iter_mut`) need `V: Clone`; use the
non-`Clone` variants when the value is not `Clone` and the old value is not
needed:

* `SyncHashMap` / `SyncBtreeMap` / `SyncIndexMap`:
  [`set(k, v)`] and [`delete(k)`] instead of `insert` / `remove`.
* `SyncVec`: [`pop_discard()`] and [`remove_discard(i)`] instead of `pop` / `remove`.

for example:
```rust
    #[tokio::test]
    pub async fn test_get() {
        let m = SyncHashMap::<i32, i32>::new();
        let insert = m.insert(1, 2);
        
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
        drop(wg2);
    });
    let wg2 = wg.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;
        drop(wg2);
    });
    wg.wait_async().await;
    println!("all done");
}
```