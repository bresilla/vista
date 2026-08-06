#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use std::collections::{BTreeMap, BTreeSet};

use crate::item::Item;
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::item::SurfaceId;
#[cfg(feature = "surface-indexes")]
use crate::pruning::prune_counts;

pub trait CandidateMatcher: Send + Sync {
    fn score(&self, partial: &str, candidate: &Item) -> Option<f64>;

    fn snapshot_key(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

pub trait ItemMatcher {
    fn matches(&self, item: &Item) -> bool;
}

impl<F> ItemMatcher for F
where
    F: Fn(&Item) -> bool,
{
    fn matches(&self, item: &Item) -> bool {
        self(item)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ContainsMatcher;

impl CandidateMatcher for ContainsMatcher {
    fn score(&self, partial: &str, candidate: &Item) -> Option<f64> {
        let partial = partial.trim().to_lowercase();
        if partial.is_empty() {
            return Some(0.0);
        }
        let value = candidate.value.to_lowercase();
        value
            .contains(&partial)
            .then(|| if value == partial { 1.0 } else { 0.6 })
    }
}

#[derive(Default)]
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
pub(crate) struct PartialIndex {
    pub(crate) items: BTreeMap<String, BTreeMap<SurfaceId, u64>>,
    #[cfg(feature = "surface-indexes")]
    pub(crate) capacity: usize,
    #[cfg(feature = "surface-indexes")]
    pub(crate) max_chars: usize,
    associations: usize,
    order: BTreeSet<(u64, String, SurfaceId)>,
    surface_keys: BTreeMap<SurfaceId, BTreeSet<String>>,
}

#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
impl PartialIndex {
    pub(crate) fn new(capacity: usize, max_chars: usize) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = (capacity, max_chars);
        Self {
            items: BTreeMap::new(),
            #[cfg(feature = "surface-indexes")]
            capacity,
            #[cfg(feature = "surface-indexes")]
            max_chars,
            associations: 0,
            order: BTreeSet::new(),
            surface_keys: BTreeMap::new(),
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn learn(&mut self, surface: SurfaceId, item: &Item) {
        for fragment in item_fragments(&item.value, self.max_chars) {
            let counts = self.items.entry(fragment.clone()).or_default();
            let previous = counts.get(&surface).copied().unwrap_or(0);
            if previous == 0 {
                self.associations += 1;
                self.surface_keys
                    .entry(surface)
                    .or_default()
                    .insert(fragment.clone());
            } else {
                self.order.remove(&(previous, fragment.clone(), surface));
            }
            let next = previous.saturating_add(1);
            counts.insert(surface, next);
            self.order.insert((next, fragment, surface));
        }
        while self.associations > self.capacity {
            let Some((count, fragment, id)) = self.order.pop_first() else {
                break;
            };
            if let Some(items) = self.items.get_mut(&fragment)
                && items.get(&id) == Some(&count)
            {
                items.remove(&id);
                self.associations -= 1;
                if items.is_empty() {
                    self.items.remove(&fragment);
                }
                self.remove_reverse_key(id, &fragment);
            }
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn candidates(&self, partial: &str, limit: usize) -> BTreeMap<SurfaceId, u64> {
        let mut candidates = BTreeMap::<SurfaceId, u64>::new();
        for fragment in query_fragments(partial, self.max_chars) {
            if let Some(items) = self.items.get(&fragment) {
                for id in items.keys() {
                    let count = candidates.entry(*id).or_default();
                    *count = count.saturating_add(1);
                    if candidates.len() > limit {
                        prune_counts(&mut candidates, limit);
                    }
                }
            }
        }
        candidates
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
        max_chars: usize,
    ) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = (capacity, max_chars);
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
            #[cfg(feature = "surface-indexes")]
            max_chars,
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

#[cfg(feature = "surface-indexes")]
fn item_fragments(value: &str, max_chars: usize) -> BTreeSet<String> {
    let chars: Vec<_> = value.to_lowercase().chars().take(max_chars).collect();
    let mut fragments = BTreeSet::new();
    for width in 1..=3.min(chars.len()) {
        for window in chars.windows(width) {
            fragments.insert(window.iter().collect());
        }
    }
    fragments
}

#[cfg(feature = "surface-indexes")]
fn query_fragments(partial: &str, max_chars: usize) -> BTreeSet<String> {
    let chars: Vec<_> = partial
        .trim()
        .to_lowercase()
        .chars()
        .take(max_chars)
        .collect();
    let width = chars.len().min(3);
    if width == 0 {
        return BTreeSet::new();
    }
    chars
        .windows(width)
        .map(|window| window.iter().collect())
        .collect()
}
