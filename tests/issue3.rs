//! Regression tests for https://github.com/darkrpc/dark-std/issues/3
//!
//! The containers use a Go `sync.Map`-style read/dirty architecture: reads
//! (`get`) hit an immutable, atomically published snapshot while all writes go
//! to the `dirty` map under a lock, so running the code below against the old
//! implementation under Miri reported data races. All access is now
//! synchronised, so these run race-free.

use dark_std::sync::{SyncHashMap, SyncVec};

#[test]
fn sync_hash_map_race() {
    let map: SyncHashMap<bool, bool> = SyncHashMap::new();
    std::thread::scope(|s| {
        s.spawn(|| {
            for _ in 0..100 {
                let _ = map.get(&true);
            }
        });
        for _ in 0..100 {
            map.insert(true, true);
        }
    });
}

#[test]
fn sync_vec_race() {
    let vec: SyncVec<usize> = SyncVec::new();
    vec.push(1);
    std::thread::scope(|s| {
        s.spawn(|| {
            for _ in 0..100 {
                let _ = vec.get(0);
            }
        });
        for i in 0..100 {
            vec.push(i);
        }
    });
}
