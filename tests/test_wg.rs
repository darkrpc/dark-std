use std::time::Duration;
use tokio::time::sleep;
use dark_std::sync::WaitGroup;

#[tokio::test]
async fn test_wg() {
    let wg = WaitGroup::new();
    let wg2 = wg.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;
        wg2.done_async().await;
    });
    let wg2 = wg.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;
        wg2.done_async().await;
    });
    wg.wait_async().await;
    println!("all done");
}