//! Owns the images and placements of one screen buffer. A byte quota bounds
//! the pixels it keeps; when a new image does not fit, images no placement
//! uses are evicted least recently used first, and only then placed ones
//! (with their placements), as kitty does, so the newest output always shows.

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{Image, ImageError, ImageId, MAX_IMAGES, MAX_PLACEMENTS, Placement};

#[derive(Debug, Clone)]
struct Entry {
    image: Arc<Image>,
    /// Store clock at the last insert, lookup or placement.
    last_used: u64,
}

/// Images and placements with a byte quota and LRU eviction.
#[derive(Debug, Clone)]
pub struct ImageStore {
    quota: usize,
    used: usize,
    /// Ticks on every use; also the source of image generations, which
    /// therefore never repeat.
    clock: u64,
    next_internal: u64,
    images: FxHashMap<ImageId, Entry>,
    placements: Vec<Placement>,
}

impl ImageStore {
    /// An empty store holding at most `quota` bytes of pixels.
    pub fn new(quota: usize) -> Self {
        Self {
            quota,
            used: 0,
            clock: 0,
            next_internal: ImageId::FIRST_INTERNAL.0,
            images: FxHashMap::default(),
            placements: Vec::new(),
        }
    }

    /// A fresh id outside kitty's range, never reused by this store.
    pub fn alloc_id(&mut self) -> ImageId {
        self.next_internal += 1;
        ImageId(self.next_internal - 1)
    }

    /// Stores `image` under `id`, replacing an image already there together
    /// with its placements (the kitty spec requires that on re-transmit), and
    /// evicting others until it fits.
    pub fn insert(&mut self, id: ImageId, mut image: Image) -> Result<Arc<Image>, ImageError> {
        let needed = image.pixels().len();
        if needed > self.quota {
            return Err(ImageError::Quota(needed, self.quota));
        }
        self.remove(id);
        // Terminates: each round removes an image, and an empty store fits.
        while self.used + needed > self.quota || self.images.len() >= MAX_IMAGES {
            self.evict_one();
        }
        let last_used = self.tick();
        image.set_generation(last_used);
        let image = Arc::new(image);
        self.used += needed;
        let entry = Entry {
            image: Arc::clone(&image),
            last_used,
        };
        self.images.insert(id, entry);
        Ok(image)
    }

    /// The image under `id`, marked as recently used.
    pub fn get(&mut self, id: ImageId) -> Option<Arc<Image>> {
        let now = self.tick();
        let entry = self.images.get_mut(&id)?;
        entry.last_used = now;
        Some(Arc::clone(&entry.image))
    }

    /// The image under `id` without touching its LRU position, for building
    /// frames.
    pub fn peek(&self, id: ImageId) -> Option<&Arc<Image>> {
        self.images.get(&id).map(|e| &e.image)
    }

    /// Removes the image and its placements; false when there was none.
    pub fn remove(&mut self, id: ImageId) -> bool {
        let Some(entry) = self.images.remove(&id) else {
            return false;
        };
        self.used -= entry.image.pixels().len();
        self.placements.retain(|p| p.image != id);
        true
    }

    /// Adds a placement. A non-zero placement id replaces the placement with
    /// the same image and id, which is how kitty moves one without flicker.
    pub fn place(&mut self, placement: Placement) -> Result<(), ImageError> {
        if self.get(placement.image).is_none() {
            return Err(ImageError::NotFound(placement.image));
        }
        let same = |p: &&mut Placement| p.image == placement.image && p.id == placement.id;
        if placement.id != 0
            && let Some(old) = self.placements.iter_mut().find(same)
        {
            *old = placement;
            return Ok(());
        }
        if self.placements.len() >= MAX_PLACEMENTS {
            // The oldest placement is the one most likely scrolled away.
            self.placements.remove(0);
        }
        self.placements.push(placement);
        Ok(())
    }

    /// Placements in the order they were made.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// The placements, for the terminal to re-anchor after a reflow.
    pub fn placements_mut(&mut self) -> &mut [Placement] {
        &mut self.placements
    }

    /// Keeps the placements for which `keep` is true; the images stay.
    pub fn retain_placements(&mut self, keep: impl FnMut(&Placement) -> bool) {
        self.placements.retain(keep);
    }

    /// Removes the images without placements for which `doomed` is true:
    /// kitty's upper-case deletes free data once nothing shows it.
    pub fn prune_unplaced(&mut self, mut doomed: impl FnMut(ImageId) -> bool) {
        let placed = self.placed();
        let used = &mut self.used;
        self.images.retain(|id, e| {
            let keep = placed.contains(id) || !doomed(*id);
            if !keep {
                *used -= e.image.pixels().len();
            }
            keep
        });
    }

    /// Drops every image and placement.
    pub fn clear(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.used = 0;
    }

    /// Bytes of pixels held.
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Number of stored images.
    pub fn len(&self) -> usize {
        self.images.len()
    }

    /// Whether no image is stored.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn placed(&self) -> FxHashSet<ImageId> {
        self.placements.iter().map(|p| p.image).collect()
    }

    /// Evicts the least recently used image, unplaced ones first.
    fn evict_one(&mut self) {
        let placed = self.placed();
        let victim = self
            .images
            .iter()
            .min_by_key(|(id, e)| (placed.contains(*id), e.last_used))
            .map(|(id, _)| *id);
        if let Some(id) = victim {
            self.remove(id);
        }
    }
}
