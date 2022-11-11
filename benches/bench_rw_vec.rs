#![feature(test)]
extern crate test;

use std::sync::RwLock;
use dark_std::sync::SyncVec;

//13ns
#[bench]
fn bench_rw_vec(b: &mut test::Bencher) {
    let rw = RwLock::new(vec![1]);
    b.iter(|| {
        rw.read().unwrap().get(0);
    });
}

//0ns
#[bench]
fn bench_sync_vec(b: &mut test::Bencher) {
    let mut rw = SyncVec::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        rw.push(1).await;
    });
    assert_eq!(rw.len(), 1);
    b.iter(|| {
        rw.get(0);
    });
}