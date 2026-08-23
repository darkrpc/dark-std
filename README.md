# dark-std
dark-std is an Implementation of asynchronous

* defer!          (defer macro)
* SyncHashMap     (async HashMap)
* SyncBtreeMap    (async BtreeMap)
* SyncVec         (async Vec)
* WaitGroup       (async/blocking all support WaitGroup)
* AtomicDuration  (atomic duration)

for example:
```rust
    #[tokio::test]
    pub async fn test_get() {
        let m = SyncHashMap::<i32, i32>::new();
        m.insert(1, 2);

        // get is lock-free: it only registers a reader slot with an atomic
        // counter (no mutex, no blocking), then returns a guard that keeps
        // writers waiting until it is dropped.
        let g = m.get(&1).unwrap();
        assert_eq!(&2, &*g);
    }
```

> **Synchronisation model (contention-free reads, locked writes)**: reads
> (`get`/`iter`/`dirty_ref`/`len`/`contains_key`) never take a lock, never
> block and never contend with each other. Each thread registers a reader slot
> in its own private counter (per-thread QSBR-style); concurrent readers only
> touch their own cache line. Writes take a mutex, raise a `writing` flag and
> wait until every thread's reader counter is zero before mutating the
> container in place — O(1)/O(log n), no whole-container copy and no `Clone`
> requirement on `K`/`V`. The counters use `SeqCst` ordering to close the
> store-buffering window, so a reader can never read while a writer mutates
> (verified with Miri against issue #3 reproductions). A read guard makes
> writers wait until it is dropped, so drop it before calling a write method
> from the same scope:
>
> ```rust
> # use dark_std::sync::{SyncHashMap};
> # let m = SyncHashMap::<i32, i32>::new();
> # m.insert(1, 2);
> let g = m.get(&1).unwrap();
> assert_eq!(&2, &*g);
> drop(g); // release the reader slot before writing
> m.insert(2, 3);
> ```

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
