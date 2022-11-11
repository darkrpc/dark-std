#![feature(test)]
extern crate test;
use dark_std::sync::SyncHashMap;

//6 ns/iter (+/- 0)
#[bench]
fn bench_sync_map_get(b: &mut test::Bencher) {
    let rw = SyncHashMap::new();
    rw.insert(1, 1);
    assert_eq!(rw.len(), 1);
    b.iter(|| {
        rw.get(&1);
    });
}

// //18 ns/iter (+/- 0)
// #[bench]
// fn bench_dash_map_get(b: &mut test::Bencher) {
//     let rw = dashmap::DashMap::new();
//     rw.insert(1,1);
//     b.iter(|| {
//         let _=rw.get(&1);
//     });
// }

//8 ns/iter (+/- 0)
#[bench]
fn bench_sync_map_insert(b: &mut test::Bencher) {
    let rw = SyncHashMap::new();
    b.iter(|| {
        rw.insert(1, 1);
    });
}

// //17 ns/iter (+/- 0)
// #[bench]
// fn bench_dash_map_insert(b: &mut test::Bencher) {
//     let rw = dashmap::DashMap::new();
//     b.iter(|| {
//         rw.insert(1,1);
//     });
// }