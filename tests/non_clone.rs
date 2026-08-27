use dark_std::sync::{SyncBtreeMap, SyncHashMap, SyncIndexMap, SyncVec};

/// A deliberately non-`Clone` value. The containers must still accept it:
/// `set` / `delete` (maps) and `push` / `pop_discard` / `remove_discard`
/// (vec) must not require `V: Clone`.
#[derive(Debug, PartialEq, Eq)]
struct NonClone {
    x: i32,
}

impl Drop for NonClone {
    fn drop(&mut self) {
        // Intentionally non-trivial so a move-out bug (double free) would be
        // caught under Miri / sanitizers.
    }
}

#[test]
fn vec_non_clone() {
    let v = SyncVec::new();
    v.push(NonClone { x: 1 });
    v.push(NonClone { x: 2 });
    v.push(NonClone { x: 3 });

    assert_eq!(v.get(0).unwrap().x, 1);
    assert_eq!(v.iter().map(|e| e.x).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(v.len(), 3);

    // Remove without returning the value: no Clone needed.
    v.pop_discard();
    assert_eq!(v.len(), 2);
    assert_eq!(v.get(1).unwrap().x, 2);

    v.remove_discard(0);
    assert_eq!(v.len(), 1);
    assert_eq!(v.get(0).unwrap().x, 2);

    // The removed values stay alive (retired) so the earlier `get` reference
    // remains valid even after removal.
    let first = v.get(0).unwrap();
    assert_eq!(first.x, 2);
    let _ = first;

    let inner: Vec<NonClone> = v.into_inner();
    assert_eq!(inner.len(), 1);
    assert_eq!(inner[0].x, 2);
}

#[test]
fn vec_non_clone_mut_aliases() {
    let mut v = SyncVec::new();
    v.push(NonClone { x: 1 });
    v.push(NonClone { x: 2 });
    v.push(NonClone { x: 3 });
    v.pop_discard_mut();
    v.remove_discard_mut(0);
    assert_eq!(v.len(), 1);
    assert_eq!(v.get(0).unwrap().x, 2);
}

#[test]
fn hash_map_non_clone() {
    let m = SyncHashMap::new();
    m.set(1, NonClone { x: 1 });
    m.set(2, NonClone { x: 2 });
    // Overwrite an existing key without returning the old value.
    m.set(1, NonClone { x: 10 });

    assert_eq!(m.get(&1).unwrap().x, 10);
    assert_eq!(m.get(&2).unwrap().x, 2);
    assert_eq!(m.len(), 2);
    assert!(m.contains_key(&1));

    let mut keys: Vec<i32> = m.iter().map(|(k, _)| *k).collect();
    keys.sort();
    assert_eq!(keys, vec![1, 2]);

    // Delete without returning the value: no Clone needed.
    m.delete(&1);
    assert!(!m.contains_key(&1));
    assert_eq!(m.len(), 1);

    let map: std::collections::HashMap<i32, NonClone> = m.into_inner();
    assert_eq!(map.len(), 1);
    assert_eq!(map[&2].x, 2);
}

#[test]
fn hash_map_non_clone_mut_aliases() {
    let mut m = SyncHashMap::new();
    m.set_mut(1, NonClone { x: 1 });
    m.set_mut(1, NonClone { x: 2 });
    m.delete_mut(&1);
    assert!(m.is_empty());
}

#[test]
fn btree_map_non_clone() {
    let m = SyncBtreeMap::new();
    m.set(1, NonClone { x: 1 });
    m.set(2, NonClone { x: 2 });
    m.set(1, NonClone { x: 10 });
    m.delete(&1);

    assert_eq!(m.get(&2).unwrap().x, 2);
    assert_eq!(m.len(), 1);
    assert!(!m.contains_key(&1));
}

#[test]
fn index_map_non_clone() {
    let m = SyncIndexMap::new();
    m.set(1, NonClone { x: 1 });
    m.set(2, NonClone { x: 2 });
    m.set(1, NonClone { x: 10 });
    m.delete(&1);

    assert_eq!(m.get(&2).unwrap().x, 2);
    assert_eq!(m.len(), 1);
    assert!(!m.contains_key(&1));
}

#[test]
fn retained_reference_stays_valid_after_delete() {
    // `get` hands out references that must stay valid after mutation; values
    // are retired, never freed, until the container is dropped.
    let m = SyncHashMap::new();
    m.set(1, NonClone { x: 5 });
    let g = m.get(&1).unwrap();
    m.delete(&1);
    m.set(2, NonClone { x: 6 });
    assert_eq!(g.x, 5);
}
