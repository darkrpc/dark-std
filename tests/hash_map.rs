use dark_std::sync::SyncHashMap;

#[tokio::test]
async fn test_hashmap(){
    let m = SyncHashMap::new();
    m.insert(1,1).await;
    for (k,v) in m {
        println!("{},{}",k,v);
    }
}