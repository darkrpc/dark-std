#![feature(test)]
extern crate test;

use dark_std::sync::SyncVec;
use std::sync::RwLock;

//test bench_rw_vec        ... bench:           4 ns/iter (+/- 0)
#[bench]
fn bench_rw_vec(b: &mut test::Bencher) {
    let rw = RwLock::new(vec![1]);
    b.iter(|| {
        let _ = rw.read().unwrap().get(0);
    });
}

//test bench_sync_vec      ... bench:           0 ns/iter (+/- 0)
#[bench]
fn bench_sync_vec(b: &mut test::Bencher) {
    let rw = SyncVec::new();
    rw.push(1);
    assert_eq!(rw.len(), 1);
    b.iter(|| {
        rw.get(0);
    });
}

//test bench_sync_vec_push ... bench:           5 ns/iter (+/- 0)
#[bench]
fn bench_sync_vec_push(b: &mut test::Bencher) {
    let rw = SyncVec::new();
    b.iter(|| {
        rw.push(0);
    });
}
