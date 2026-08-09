#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use std::collections::{BTreeMap, BTreeSet};

use crate::api::Item;
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::api::SurfaceId;
#[cfg(feature = "surface-indexes")]
use crate::engine::{prune_counts, prune_counts_removed};

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

    fn snapshot_key(&self) -> &str {
        "vista::tokenizer::WhitespaceTokenizer"
    }
}

#[derive(Clone, Default)]
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
pub(crate) struct TokenIndex {
    pub(crate) items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
    #[cfg(feature = "surface-indexes")]
    pub(crate) max_tokens: usize,
    #[cfg(feature = "surface-indexes")]
    pub(crate) max_surfaces: usize,
    surface_keys: BTreeMap<SurfaceId, BTreeSet<String>>,
    string_bytes: usize,
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
            string_bytes: 0,
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn learn_normalized(&mut self, tokens: BTreeSet<String>, surface: SurfaceId) {
        for token in tokens {
            if self.items.contains_key(&token) || self.items.len() < self.max_tokens {
                if !self.items.contains_key(&token) {
                    self.string_bytes = self.string_bytes.saturating_add(token.len());
                }
                let (removed, retained) = {
                    let surfaces = self.items.entry(token.clone()).or_default();
                    let count = surfaces.entry(surface).or_default();
                    *count = count.saturating_add(1);
                    let removed = prune_counts_removed(surfaces, self.max_surfaces);
                    (removed, surfaces.contains_key(&surface))
                };
                if retained
                    && self
                        .surface_keys
                        .entry(surface)
                        .or_default()
                        .insert(token.clone())
                {
                    self.string_bytes = self.string_bytes.saturating_add(token.len());
                }
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
            self.string_bytes = self.string_bytes.saturating_sub(key.len());
            if let Some(items) = self.items.get_mut(&key) {
                items.remove(&surface);
                if items.is_empty() {
                    self.items.remove(&key);
                    self.string_bytes = self.string_bytes.saturating_sub(key.len());
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

    /// Whether history has ever produced this token.
    pub(crate) fn known(&self, token: &str) -> bool {
        self.items.contains_key(&token.to_lowercase())
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.surface_keys.clear();
        self.string_bytes = 0;
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
        let string_bytes = items
            .keys()
            .map(String::len)
            .chain(surface_keys.values().flatten().map(String::len))
            .fold(0_usize, usize::saturating_add);
        Self {
            items,
            #[cfg(feature = "surface-indexes")]
            max_tokens,
            #[cfg(feature = "surface-indexes")]
            max_surfaces,
            surface_keys,
            string_bytes,
        }
    }

    pub(crate) fn string_bytes(&self) -> usize {
        self.string_bytes
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn additional_string_bytes(
        &self,
        tokens: &BTreeSet<String>,
        surface: SurfaceId,
    ) -> usize {
        tokens.iter().fold(0_usize, |total, token| {
            let unique = if self.items.contains_key(token) {
                0
            } else {
                token.len()
            };
            let association = if self
                .items
                .get(token)
                .is_none_or(|items| !items.contains_key(&surface))
            {
                token.len()
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

#[cfg(feature = "surface-indexes")]
pub(crate) fn normalized_tokens(tokens: Vec<String>) -> BTreeSet<String> {
    tokens
        .into_iter()
        .map(|token| token.to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}
