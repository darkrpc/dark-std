# dark-std
dark-std is an implementation of thread-safe containers with a read-write
separation design borrowed from Golang (reads avoid the container write-lock
and never contend with each other; writes are serialized and wait for active
readers), plus async/blocking utilities.

* defer!          (defer macro)
* SyncHashMap     (thread-safe HashMap)
* SyncBtreeMap    (thread-safe BtreeMap)
* SyncIndexMap    (thread-safe IndexMap)
* SyncVec         (thread-safe Vec)
* WaitGroup       (sync `wait()` + async `wait_async()`)
* AtomicDuration  (atomic duration)

for example:
```rust
    #[test]
    pub fn test_get() {
        let m = SyncHashMap::<i32, i32>::new();
        m.insert(1, 2);

        // get takes no container write-lock: it bumps a per-thread atomic
        // counter and returns a guard that keeps writers waiting until it is
        // dropped (the first read from a thread briefly registers its counter).
        let g = m.get(&1).unwrap();
        assert_eq!(&2, &*g);
    }
```

> **Synchronisation model (contention-free reads, serialized writes)**: reads
> (`get`/`iter`/`dirty_ref`/`len`/`contains_key`) never take the container's
> write lock and never contend with each other: each thread registers a reader
> slot in its own private counter (per-thread QSBR-style) and only touches its
> own cache line. The slot is registered lazily — the first read from a thread
> on a container briefly locks the registry to append its counter; afterwards
> reads are plain atomic increments. A reader arriving while a writer is active
> spins (yields) until the writer finishes. Writes take a mutex, raise a
> `writing` flag and wait until every thread's reader counter is zero before
> mutating the container in place — O(1)/O(log n), no whole-container copy and
> no `Clone` requirement on `K`/`V`. The counters use `SeqCst` ordering to
> close the store-buffering window, so a reader can never read while a writer
> mutates (verified with Miri against issue #3 reproductions). A read guard
> makes writers wait until it is dropped, so drop it before calling a write
> method from the same scope:
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
