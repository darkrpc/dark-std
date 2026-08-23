//! Regression tests for https://github.com/darkrpc/dark-std/issues/3
//!
//! These are the exact single-access reproductions from the issue. Previously
//! `get`/`dirty_ref` returned plain references into the shared storage without
//! any synchronisation, so running these under Miri reported data races. All
//! access is now synchronised (per-thread reader slots + writer waits), so
//! these run race-free.

use dark_std::sync::{SyncBtreeMap, SyncHashMap, SyncIndexMap, SyncVec};

#[test]
fn SyncBtreeMap_race() {
    let map: SyncBtreeMap<bool, bool> = SyncBtreeMap::new();
    map.insert(true, true);
    std::thread::scope(|s| {
        s.spawn(|| {
            map.dirty_ref();
        });
        map.remove(&true);
    });
}

#[test]
fn SyncHashMap_race() {
    let map: SyncHashMap<bool, bool> = SyncHashMap::new();
    std::thread::scope(|s| {
        s.spawn(|| {
            map.get(&true);
        });
        map.insert(true, true);
    });
}

#[test]
fn SyncVec_race() {
    let vec: SyncVec<bool> = SyncVec::new();
    vec.push(true);
    std::thread::scope(|s| {
        s.spawn(|| {
            vec.get(0);
        });
        vec.push(false);
    });
}

#[test]
fn SyncIndexMap_race() {
    let map: SyncIndexMap<bool, bool> = SyncIndexMap::new();
    std::thread::scope(|s| {
        s.spawn(|| {
            map.dirty_ref();
        });
        map.insert(true, true);
    });
}
