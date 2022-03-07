# dark-std
dark-std is an Implementation of asynchronous containers build on tokio.
It uses a read-write separation design borrowed from Golang


* SyncHashMap   asynchronous HashMap
* SyncBtreeMap   asynchronous BtreeMap
* SyncVec   asynchronous Vec

for example:
```rust
    #[tokio::test]
    pub async fn test_get() {
        let m = SyncHashMap::<i32, i32>::new();
        let insert = m.insert(1, 2).await;
        
        let g = m.get(&1).unwrap();//don't need lock and await
        assert_eq!(&2, g);
    }
```
