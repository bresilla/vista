#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use std::collections::{BTreeMap, BTreeSet};

use crate::item::Item;
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::item::SurfaceId;
#[cfg(feature = "surface-indexes")]
use crate::pruning::{prune_counts, prune_counts_removed};

pub trait Tokenizer: Send + Sync {
    fn tokens(&self, item: &Item) -> Vec<String>;

    fn query_tokens(&self, text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_lowercase).collect()
    }

    fn snapshot_key(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WhitespaceTokenizer;

impl Tokenizer for WhitespaceTokenizer {
    fn tokens(&self, item: &Item) -> Vec<String> {
        item.value
            .split_whitespace()
            .map(str::to_lowercase)
            .collect()
    }
}

#[derive(Default)]
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
pub(crate) struct TokenIndex {
    pub(crate) items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
    #[cfg(feature = "surface-indexes")]
    pub(crate) max_tokens: usize,
    #[cfg(feature = "surface-indexes")]
    pub(crate) max_surfaces: usize,
    surface_keys: BTreeMap<SurfaceId, BTreeSet<String>>,
}

#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
impl TokenIndex {
    pub(crate) fn new(max_tokens: usize, max_surfaces: usize) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = (max_tokens, max_surfaces);
        Self {
            items: BTreeMap::new(),
            #[cfg(feature = "surface-indexes")]
            max_tokens,
            #[cfg(feature = "surface-indexes")]
            max_surfaces,
            surface_keys: BTreeMap::new(),
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn learn(&mut self, tokens: Vec<String>, surface: SurfaceId) {
        let tokens: BTreeSet<_> = tokens
            .into_iter()
            .take(self.max_tokens)
            .map(|token| token.to_lowercase())
            .filter(|token| !token.is_empty())
            .collect();
        for token in tokens {
            if self.items.contains_key(&token) || self.items.len() < self.max_tokens {
                let removed = {
                    let surfaces = self.items.entry(token.clone()).or_default();
                    let count = surfaces.entry(surface).or_default();
                    *count = count.saturating_add(1);
                    prune_counts_removed(surfaces, self.max_surfaces)
                };
                self.surface_keys
                    .entry(surface)
                    .or_default()
                    .insert(token.clone());
                for removed_surface in removed {
                    self.remove_reverse_key(removed_surface, &token);
                }
            }
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn remove_surface(&mut self, surface: SurfaceId) {
        let keys = self.surface_keys.remove(&surface).unwrap_or_default();
        for key in keys {
            if let Some(items) = self.items.get_mut(&key) {
                items.remove(&surface);
                if items.is_empty() {
                    self.items.remove(&key);
                }
            }
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn candidates(&self, tokens: &[String], limit: usize) -> BTreeMap<SurfaceId, u64> {
        let mut candidates = BTreeMap::<SurfaceId, u64>::new();
        let tokens: BTreeSet<_> = tokens
            .iter()
            .take(self.max_tokens)
            .map(|token| token.to_lowercase())
            .filter(|token| !token.is_empty())
            .collect();
        for token in tokens {
            if let Some(items) = self.items.get(&token) {
                for (id, count) in items {
                    let entry = candidates.entry(*id).or_default();
                    *entry = entry.saturating_add(*count);
                    if candidates.len() > limit {
                        prune_counts(&mut candidates, limit);
                    }
                }
            }
        }
        candidates
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.surface_keys.clear();
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn restore(
        items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
        max_tokens: usize,
        max_surfaces: usize,
    ) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = (max_tokens, max_surfaces);
        let mut surface_keys = BTreeMap::<SurfaceId, BTreeSet<String>>::new();
        for (key, surfaces) in &items {
            for surface in surfaces.keys() {
                surface_keys
                    .entry(*surface)
                    .or_default()
                    .insert(key.clone());
            }
        }
        Self {
            items,
            #[cfg(feature = "surface-indexes")]
            max_tokens,
            #[cfg(feature = "surface-indexes")]
            max_surfaces,
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
