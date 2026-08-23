//! Regression tests for https://github.com/darkrpc/dark-std/issues/3
//!
//! Previously `get`/`dirty_ref` returned plain references into the shared
//! storage without any synchronisation, so running the code below against the
//! old implementation under Miri reported data races. All access is now
//! synchronised (structure protected by a read-write lock, values kept alive
//! behind `Arc`), so these run race-free.

use dark_std::sync::{SyncBtreeMap, SyncHashMap, SyncIndexMap, SyncVec};

#[test]
fn sync_btree_map_race() {
    let map: SyncBtreeMap<bool, bool> = SyncBtreeMap::new();
    map.insert(true, true);
    std::thread::scope(|s| {
        s.spawn(|| {
            for _ in 0..100 {
                let _ = map.dirty_ref();
            }
        });
        for _ in 0..100 {
            let _ = map.remove(&true);
            map.insert(true, true);
        }
    });
}

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

#[test]
fn sync_index_map_race() {
    let map: SyncIndexMap<bool, bool> = SyncIndexMap::new();
    map.insert(true, true);
    std::thread::scope(|s| {
        s.spawn(|| {
            for _ in 0..100 {
                let _ = map.dirty_ref();
            }
        });
        for _ in 0..100 {
            let _ = map.remove(&true);
            map.insert(true, true);
        }
    });
}
