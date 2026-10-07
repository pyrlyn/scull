//! A capped interning table with mark-and-sweep reclaim, shared by the style
//! and cluster tables. Separate so the cap, the free list and the sweep are
//! written and tested once.
//!
//! Reclaim is a sweep rather than a refcount: cells stay `Copy` and a cell
//! write is one store, at the price of an occasional walk over every row
//! (kitty garbage-collects its text cache the same way, `docs/research/kitty.md`
//! §6). The grid decides when a sweep is worth it; this module only counts.

use std::borrow::Borrow;
use std::hash::Hash;

use rustc_hash::FxHashMap;

const WORD_BITS: usize = u64::BITS as usize;

/// One bit per id: which entries a sweep must keep.
#[derive(Debug, Clone)]
pub struct Marks(Vec<u64>);

impl Marks {
    pub(crate) fn new(len: usize) -> Self {
        Self(vec![0; len.div_ceil(WORD_BITS)])
    }

    /// Keeps `id` alive through the next sweep. Ids past the table are ignored.
    pub fn mark(&mut self, id: u32) {
        let id = id as usize;
        if let Some(word) = self.0.get_mut(id / WORD_BITS) {
            *word |= 1 << (id % WORD_BITS);
        }
    }

    pub(crate) fn is_marked(&self, id: usize) -> bool {
        self.0
            .get(id / WORD_BITS)
            .is_some_and(|word| word & (1 << (id % WORD_BITS)) != 0)
    }
}

/// Values in, small ids out, at most `cap` live at once. The first `pinned`
/// ids are never swept.
#[derive(Debug, Clone)]
pub(crate) struct Interner<T> {
    slots: Vec<Option<T>>,
    index: FxHashMap<T, u32>,
    free: Vec<u32>,
    cap: usize,
    pinned: usize,
    inserts_since_sweep: usize,
}

impl<T: Hash + Eq + Clone> Interner<T> {
    pub(crate) fn new(cap: usize, pinned: impl IntoIterator<Item = T>) -> Self {
        let mut table = Self {
            slots: Vec::new(),
            index: FxHashMap::default(),
            free: Vec::new(),
            cap,
            pinned: 0,
            inserts_since_sweep: 0,
        };
        for value in pinned {
            if table.intern(&value, T::clone).is_some() {
                table.pinned += 1;
            }
        }
        table.inserts_since_sweep = 0;
        table
    }

    pub(crate) fn get(&self, id: u32) -> Option<&T> {
        self.slots.get(id as usize).and_then(Option::as_ref)
    }

    /// The id of `key`, inserting `make(key)` when absent. `None` when the
    /// table holds `cap` live entries.
    pub(crate) fn intern<Q>(&mut self, key: &Q, make: impl FnOnce(&Q) -> T) -> Option<u32>
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(&id) = self.index.get(key) {
            return Some(id);
        }
        if self.len() >= self.cap {
            return None;
        }
        let value = make(key);
        let id = match self.free.pop() {
            Some(id) => {
                let slot = self.slots.get_mut(id as usize)?;
                *slot = Some(value.clone());
                id
            }
            None => {
                let id = u32::try_from(self.slots.len()).ok()?;
                self.slots.push(Some(value.clone()));
                id
            }
        };
        self.index.insert(value, id);
        self.inserts_since_sweep += 1;
        Some(id)
    }

    /// Live entries.
    pub(crate) fn len(&self) -> usize {
        self.index.len()
    }

    pub(crate) fn cap(&self) -> usize {
        self.cap
    }

    /// Inserts since the last sweep, so the grid can tell whether a sweep
    /// now would pay for itself.
    pub(crate) fn inserts_since_sweep(&self) -> usize {
        self.inserts_since_sweep
    }

    /// A blank mark set sized for this table.
    pub(crate) fn marks(&self) -> Marks {
        Marks::new(self.slots.len())
    }

    /// Frees every unpinned entry `live` does not mark; returns how many.
    pub(crate) fn sweep(&mut self, live: &Marks) -> usize {
        let mut freed = 0;
        for (id, slot) in self.slots.iter_mut().enumerate().skip(self.pinned) {
            if live.is_marked(id) {
                continue;
            }
            if let (Some(value), Ok(id)) = (slot.take(), u32::try_from(id)) {
                self.index.remove(&value);
                self.free.push(id);
                freed += 1;
            }
        }
        self.inserts_since_sweep = 0;
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: usize = 4;

    fn table() -> Interner<u32> {
        Interner::new(CAP, [0])
    }

    #[test]
    fn same_value_gets_same_id() {
        let mut t = table();
        let a = t.intern(&7, |v| *v);
        assert_eq!(a, t.intern(&7, |v| *v));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn insert_past_cap_is_refused_but_lookup_still_works() {
        let mut t = table();
        for v in 1..CAP as u32 {
            assert!(t.intern(&v, |v| *v).is_some());
        }
        assert_eq!(t.intern(&99, |v| *v), None);
        assert_eq!(t.intern(&1, |v| *v), Some(1));
        assert_eq!(t.len(), CAP);
    }

    #[test]
    fn sweep_frees_unmarked_ids_for_reuse_and_keeps_pinned() {
        let mut t = table();
        let a = t.intern(&1, |v| *v).unwrap();
        let b = t.intern(&2, |v| *v).unwrap();
        let mut live = t.marks();
        live.mark(a);
        assert_eq!(t.sweep(&live), 1);
        assert_eq!(t.get(0), Some(&0), "pinned survives without a mark");
        assert_eq!(t.get(a), Some(&1));
        assert_eq!(t.get(b), None);
        assert_eq!(t.intern(&3, |v| *v), Some(b), "freed id is reused");
        assert_eq!(t.inserts_since_sweep(), 1);
    }

    #[test]
    fn mark_past_the_table_is_ignored() {
        let t = table();
        let mut live = t.marks();
        live.mark(u32::MAX);
        assert!(!live.is_marked(u32::MAX as usize));
    }
}
