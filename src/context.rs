use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "surface-indexes")]
use crate::Feature;
#[cfg(feature = "surface-indexes")]
use crate::feature::association_keys;
use crate::item::SurfaceId;
#[cfg(feature = "surface-indexes")]
use crate::pruning::prune_counts;

#[derive(Default)]
pub(crate) struct ContextIndex {
    pub(crate) items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
    #[cfg(feature = "surface-indexes")]
    pub(crate) capacity: usize,
    associations: usize,
    order: BTreeSet<(u64, String, SurfaceId)>,
    surface_keys: BTreeMap<SurfaceId, BTreeSet<String>>,
}

impl ContextIndex {
    pub(crate) fn new(capacity: usize) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = capacity;
        Self {
            items: BTreeMap::new(),
            #[cfg(feature = "surface-indexes")]
            capacity,
            associations: 0,
            order: BTreeSet::new(),
            surface_keys: BTreeMap::new(),
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn learn(&mut self, features: &[Feature], surface: SurfaceId) {
        for key in association_keys(features, self.capacity) {
            let counts = self.items.entry(key.clone()).or_default();
            let previous = counts.get(&surface).copied().unwrap_or(0);
            if previous == 0 {
                self.associations += 1;
                self.surface_keys
                    .entry(surface)
                    .or_default()
                    .insert(key.clone());
            } else {
                self.order.remove(&(previous, key.clone(), surface));
            }
            let next = previous.saturating_add(1);
            counts.insert(surface, next);
            self.order.insert((next, key, surface));
        }
        while self.associations > self.capacity {
            let Some((count, key, surface)) = self.order.pop_first() else {
                break;
            };
            if let Some(items) = self.items.get_mut(&key)
                && items.get(&surface) == Some(&count)
            {
                items.remove(&surface);
                self.associations -= 1;
                if items.is_empty() {
                    self.items.remove(&key);
                }
                self.remove_reverse_key(surface, &key);
            }
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn candidates(
        &self,
        features: &[Feature],
        limit: usize,
    ) -> BTreeMap<SurfaceId, u64> {
        let mut result = BTreeMap::<SurfaceId, u64>::new();
        for key in association_keys(features, self.capacity) {
            if let Some(items) = self.items.get(&key) {
                for (id, count) in items {
                    let entry = result.entry(*id).or_default();
                    *entry = entry.saturating_add(*count);
                    if result.len() > limit {
                        prune_counts(&mut result, limit);
                    }
                }
            }
        }
        result
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn counts_for(
        &self,
        features: &[Feature],
        surfaces: &[SurfaceId],
    ) -> BTreeMap<SurfaceId, u64> {
        let keys = association_keys(features, self.capacity);
        surfaces
            .iter()
            .copied()
            .map(|surface| {
                let count = keys
                    .iter()
                    .filter_map(|key| self.items.get(key).and_then(|items| items.get(&surface)))
                    .fold(0_u64, |total, count| total.saturating_add(*count));
                (surface, count)
            })
            .collect()
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn remove_surface(&mut self, surface: SurfaceId) {
        let keys = self.surface_keys.remove(&surface).unwrap_or_default();
        for key in keys {
            if let Some(items) = self.items.get_mut(&key)
                && let Some(count) = items.remove(&surface)
            {
                self.order.remove(&(count, key.clone(), surface));
                self.associations = self.associations.saturating_sub(1);
                if items.is_empty() {
                    self.items.remove(&key);
                }
            }
        }
    }

    pub(crate) fn associations(&self) -> usize {
        self.associations
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
        self.surface_keys.clear();
        self.associations = 0;
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn restore(
        items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
        capacity: usize,
    ) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = capacity;
        let associations = items.values().map(BTreeMap::len).sum();
        let order = items
            .iter()
            .flat_map(|(key, values)| values.iter().map(|(id, count)| (*count, key.clone(), *id)))
            .collect();
        let surface_keys = reverse_keys(&items);
        Self {
            items,
            #[cfg(feature = "surface-indexes")]
            capacity,
            associations,
            order,
            surface_keys,
        }
    }

    pub(crate) fn reverse_key_bytes(&self) -> usize {
        self.surface_keys
            .values()
            .flat_map(BTreeSet::iter)
            .map(String::len)
            .fold(0_usize, usize::saturating_add)
    }

    #[cfg(feature = "surface-indexes")]
    fn remove_reverse_key(&mut self, surface: SurfaceId, key: &str) {
        if let Some(keys) = self.surface_keys.get_mut(&surface) {
            keys.remove(key);
            if keys.is_empty() {
                self.surface_keys.remove(&surface);
            }
        }
    }
}

#[cfg(feature = "snapshot")]
fn reverse_keys(
    items: &BTreeMap<String, BTreeMap<SurfaceId, u64>>,
) -> BTreeMap<SurfaceId, BTreeSet<String>> {
    let mut reverse = BTreeMap::<SurfaceId, BTreeSet<String>>::new();
    for (key, surfaces) in items {
        for surface in surfaces.keys() {
            reverse.entry(*surface).or_default().insert(key.clone());
        }
    }
    reverse
}
