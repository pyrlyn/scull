//! The store's quota, LRU eviction and placement bookkeeping, with a
//! model-based property test for the eviction order.

// Helpers outside #[test] functions are still test code; a failure should abort loudly.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use proptest::prelude::*;
use scull_image::{
    Crop, DEFAULT_QUOTA_BYTES, Image, ImageError, ImageId, ImageStore, MAX_PLACEMENTS, Placement,
};

/// An opaque one-row image `width` pixels wide: 4 bytes per pixel.
fn image(width: u32) -> Image {
    Image::from_rgba(width, 1, vec![0xff; width as usize * 4]).unwrap()
}

fn placement(image: ImageId, id: u32, row: u64) -> Placement {
    Placement {
        image,
        id,
        row,
        col: 0,
        cols: 1,
        rows: 1,
        offset_x: 0,
        offset_y: 0,
        crop: Crop::full(1, 1),
        z: 0,
    }
}

#[test]
fn insert_get_and_generation() {
    let mut store = ImageStore::new(1024);
    let a = store.insert(ImageId::kitty(1), image(2)).unwrap();
    let b = store.insert(ImageId::kitty(1), image(3)).unwrap();
    assert!(b.generation() > a.generation());
    assert_eq!(store.get(ImageId::kitty(1)).unwrap().width(), 3);
    assert_eq!(store.used_bytes(), 12);
    assert_eq!(store.len(), 1);
}

#[test]
fn reinsert_drops_placements() {
    let mut store = ImageStore::new(1024);
    let id = ImageId::kitty(7);
    store.insert(id, image(1)).unwrap();
    store.place(placement(id, 0, 0)).unwrap();
    store.insert(id, image(1)).unwrap();
    assert!(store.placements().is_empty());
}

#[test]
fn image_larger_than_quota_is_refused() {
    let mut store = ImageStore::new(8);
    assert_eq!(
        store.insert(ImageId::kitty(1), image(3)),
        Err(ImageError::Quota(12, 8))
    );
    assert!(store.is_empty());
}

#[test]
fn unplaced_images_are_evicted_first_then_lru() {
    // Quota of three 1-pixel images.
    let mut store = ImageStore::new(12);
    let (a, b, c, d) = (
        ImageId::kitty(1),
        ImageId::kitty(2),
        ImageId::kitty(3),
        ImageId::kitty(4),
    );
    store.insert(a, image(1)).unwrap();
    store.insert(b, image(1)).unwrap();
    store.insert(c, image(1)).unwrap();
    // `a` is the oldest but placed; `b` is older than `c`.
    store.place(placement(a, 0, 0)).unwrap();
    store.insert(d, image(1)).unwrap();
    assert!(store.peek(a).is_some() && store.peek(b).is_none() && store.peek(c).is_some());
    // Touching `c` makes `d` the least recently used unplaced image.
    store.get(c);
    store.insert(ImageId::kitty(5), image(1)).unwrap();
    assert!(store.peek(c).is_some() && store.peek(d).is_none());
}

#[test]
fn placed_images_go_when_nothing_else_is_left() {
    let mut store = ImageStore::new(8);
    let (a, b) = (ImageId::kitty(1), ImageId::kitty(2));
    store.insert(a, image(1)).unwrap();
    store.insert(b, image(1)).unwrap();
    store.place(placement(a, 0, 0)).unwrap();
    store.place(placement(b, 0, 1)).unwrap();
    store.insert(ImageId::kitty(3), image(1)).unwrap();
    assert!(
        store.peek(a).is_none(),
        "the least recently placed image goes"
    );
    assert!(store.placements().iter().all(|p| p.image != a));
    assert_eq!(store.placements().len(), 1);
}

#[test]
fn placing_a_missing_image_fails() {
    let mut store = ImageStore::new(DEFAULT_QUOTA_BYTES);
    let id = ImageId::kitty(9);
    assert_eq!(
        store.place(placement(id, 0, 0)),
        Err(ImageError::NotFound(id))
    );
}

#[test]
fn placement_id_replaces_and_zero_appends() {
    let mut store = ImageStore::new(DEFAULT_QUOTA_BYTES);
    let id = ImageId::kitty(1);
    store.insert(id, image(1)).unwrap();
    store.place(placement(id, 5, 0)).unwrap();
    store.place(placement(id, 5, 9)).unwrap();
    store.place(placement(id, 0, 1)).unwrap();
    store.place(placement(id, 0, 2)).unwrap();
    let rows: Vec<u64> = store.placements().iter().map(|p| p.row).collect();
    assert_eq!(rows, [9, 1, 2]);
}

#[test]
fn placement_count_is_capped_oldest_first() {
    let mut store = ImageStore::new(DEFAULT_QUOTA_BYTES);
    let id = ImageId::kitty(1);
    store.insert(id, image(1)).unwrap();
    for row in 0..=MAX_PLACEMENTS as u64 {
        store.place(placement(id, 0, row)).unwrap();
    }
    assert_eq!(store.placements().len(), MAX_PLACEMENTS);
    assert_eq!(store.placements()[0].row, 1);
}

#[test]
fn retain_and_prune() {
    let mut store = ImageStore::new(DEFAULT_QUOTA_BYTES);
    let (a, b) = (ImageId::kitty(1), ImageId::kitty(2));
    store.insert(a, image(1)).unwrap();
    store.insert(b, image(1)).unwrap();
    store.place(placement(a, 0, 0)).unwrap();
    store.place(placement(b, 0, 1)).unwrap();
    store.retain_placements(|p| p.row != 0);
    store.prune_unplaced(|_| true);
    assert!(store.peek(a).is_none(), "a lost its only placement");
    assert!(store.peek(b).is_some(), "b is still placed");
    assert_eq!(store.used_bytes(), 4);
}

#[test]
fn internal_ids_never_collide_with_kitty_ids() {
    let mut store = ImageStore::new(DEFAULT_QUOTA_BYTES);
    let first = store.alloc_id();
    let second = store.alloc_id();
    assert_ne!(first, second);
    assert_eq!(first.as_kitty(), None);
    assert_eq!(ImageId::kitty(u32::MAX).as_kitty(), Some(u32::MAX));
}

#[test]
fn crop_clamps_to_the_image() {
    let crop = Crop {
        x: 8,
        y: 2,
        width: 100,
        height: 0,
    };
    assert_eq!(
        crop.clamp(10, 5),
        Crop {
            x: 8,
            y: 2,
            width: 2,
            height: 3
        }
    );
    assert_eq!(Crop::full(4, 4).clamp(2, 2), Crop::full(2, 2));
}

#[derive(Debug, Clone)]
enum Op {
    Insert(u32, u32),
    Get(u32),
    Place(u32),
    Unplace(u32),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..12u32, 1..6u32).prop_map(|(id, w)| Op::Insert(id, w)),
        (0..12u32).prop_map(Op::Get),
        (0..12u32).prop_map(Op::Place),
        (0..12u32).prop_map(Op::Unplace),
    ]
}

/// The model's view of one stored image: (placements, last use, bytes).
type Model = BTreeMap<u32, (usize, u64, usize)>;

proptest! {
    #[test]
    fn quota_and_lru_hold(ops in prop::collection::vec(op(), 1..200)) {
        const QUOTA: usize = 64;
        let mut store = ImageStore::new(QUOTA);
        let mut model = Model::new();
        let mut clock = 0u64;
        for op in ops {
            clock += 1;
            match op {
                Op::Insert(id, w) => {
                    let before = model.clone();
                    model.remove(&id);
                    store.insert(ImageId::kitty(id), image(w)).unwrap();
                    let evicted: Vec<_> = before.iter()
                        .filter(|(k, _)| **k != id && store.peek(ImageId::kitty(**k)).is_none())
                        .map(|(k, v)| (*k, *v)).collect();
                    for (k, _) in &evicted {
                        model.remove(k);
                    }
                    // Every evicted image ranked below every survivor.
                    let key = |(placed, used, _): (usize, u64, usize)| (placed > 0, used);
                    for (_, e) in &evicted {
                        for s in model.values() {
                            prop_assert!(key(*e) < key(*s));
                        }
                    }
                    model.insert(id, (0, clock, w as usize * 4));
                }
                Op::Get(id) => {
                    if store.get(ImageId::kitty(id)).is_some() {
                        model.get_mut(&id).unwrap().1 = clock;
                    }
                }
                Op::Place(id) => {
                    if store.place(placement(ImageId::kitty(id), 0, 0)).is_ok() {
                        let entry = model.get_mut(&id).unwrap();
                        entry.0 += 1;
                        entry.1 = clock;
                    }
                }
                Op::Unplace(id) => {
                    store.retain_placements(|p| p.image != ImageId::kitty(id));
                    if let Some(entry) = model.get_mut(&id) {
                        entry.0 = 0;
                    }
                }
            }
            let bytes: usize = model.values().map(|e| e.2).sum();
            prop_assert_eq!(bytes, store.used_bytes());
            prop_assert!(store.used_bytes() <= QUOTA);
            prop_assert_eq!(store.len(), model.len());
            for p in store.placements() {
                prop_assert!(store.peek(p.image).is_some());
            }
        }
    }
}
