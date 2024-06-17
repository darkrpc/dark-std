use dark_std::sync::WaitGroup;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_wg() {
    let wg = WaitGroup::new();
    let wg2 = wg.clone();
    let start =std::time::Instant::now();
    tokio::spawn(async move {
        sleep(Duration::from_secs(5)).await;
        drop(wg2);
    });
    wg.wait_async().await;
    println!("all done {:?}",start.elapsed());
}

#[tokio::test]
async fn test_wg_zero() {
    let wg = WaitGroup::new();
    wg.wait_async().await;
    println!("all done");
}