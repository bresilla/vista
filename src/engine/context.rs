use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "surface-indexes")]
use crate::api::Feature;
use crate::api::SurfaceId;
#[cfg(feature = "surface-indexes")]
use crate::api::association_keys;
#[cfg(feature = "surface-indexes")]
use crate::engine::prune_counts;

#[derive(Clone, Default)]
pub(crate) struct ContextIndex {
    pub(crate) items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
    #[cfg(feature = "surface-indexes")]
    pub(crate) capacity: usize,
    associations: usize,
    order: BTreeSet<(u64, String, SurfaceId)>,
    surface_keys: BTreeMap<SurfaceId, BTreeSet<String>>,
    string_bytes: usize,
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
            string_bytes: 0,
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn learn_keys<I>(&mut self, keys: I, surface: SurfaceId)
    where
        I: IntoIterator<Item = String>,
    {
        for key in keys {
            if !self.items.contains_key(&key) {
                self.string_bytes = self.string_bytes.saturating_add(key.len());
            }
            let counts = self.items.entry(key.clone()).or_default();
            let previous = counts.get(&surface).copied().unwrap_or(0);
            if previous == 0 {
                self.associations += 1;
                if self
                    .surface_keys
                    .entry(surface)
                    .or_default()
                    .insert(key.clone())
                {
                    self.string_bytes = self.string_bytes.saturating_add(key.len());
                }
            } else if self.order.remove(&(previous, key.clone(), surface)) {
                self.string_bytes = self.string_bytes.saturating_sub(key.len());
            }
            let next = previous.saturating_add(1);
            counts.insert(surface, next);
            if self.order.insert((next, key.clone(), surface)) {
                self.string_bytes = self.string_bytes.saturating_add(key.len());
            }
        }
        while self.associations > self.capacity {
            let Some((count, key, surface)) = self.order.pop_first() else {
                break;
            };
            self.string_bytes = self.string_bytes.saturating_sub(key.len());
            if let Some(items) = self.items.get_mut(&key)
                && items.get(&surface) == Some(&count)
            {
                items.remove(&surface);
                self.associations -= 1;
                if items.is_empty() {
                    self.items.remove(&key);
                    self.string_bytes = self.string_bytes.saturating_sub(key.len());
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
            self.string_bytes = self.string_bytes.saturating_sub(key.len());
            if let Some(items) = self.items.get_mut(&key)
                && let Some(count) = items.remove(&surface)
            {
                if self.order.remove(&(count, key.clone(), surface)) {
                    self.string_bytes = self.string_bytes.saturating_sub(key.len());
                }
                self.associations = self.associations.saturating_sub(1);
                if items.is_empty() {
                    self.items.remove(&key);
                    self.string_bytes = self.string_bytes.saturating_sub(key.len());
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
        self.string_bytes = 0;
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn restore(
        items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
        capacity: usize,
    ) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = capacity;
        let associations = items.values().map(BTreeMap::len).sum();
        let order: BTreeSet<(u64, String, SurfaceId)> = items
            .iter()
            .flat_map(|(key, values)| values.iter().map(|(id, count)| (*count, key.clone(), *id)))
            .collect();
        let surface_keys = reverse_keys(&items);
        let string_bytes = items
            .keys()
            .map(String::len)
            .chain(order.iter().map(|(_, key, _)| key.len()))
            .chain(surface_keys.values().flatten().map(String::len))
            .fold(0_usize, usize::saturating_add);
        Self {
            items,
            #[cfg(feature = "surface-indexes")]
            capacity,
            associations,
            order,
            surface_keys,
            string_bytes,
        }
    }

    pub(crate) fn string_bytes(&self) -> usize {
        self.string_bytes
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn additional_string_bytes(&self, keys: &[String], surface: SurfaceId) -> usize {
        keys.iter().fold(0_usize, |total, key| {
            let unique = if self.items.contains_key(key) {
                0
            } else {
                key.len()
            };
            let association = if self
                .items
                .get(key)
                .is_none_or(|items| !items.contains_key(&surface))
            {
                key.len().saturating_mul(2)
            } else {
                0
            };
            total.saturating_add(unique).saturating_add(association)
        })
    }

    #[cfg(feature = "surface-indexes")]
    fn remove_reverse_key(&mut self, surface: SurfaceId, key: &str) {
        if let Some(keys) = self.surface_keys.get_mut(&surface) {
            if keys.remove(key) {
                self.string_bytes = self.string_bytes.saturating_sub(key.len());
            }
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
