#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::api::Item;
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::api::SurfaceId;
#[cfg(feature = "surface-indexes")]
use crate::engine::prune_counts;

/// One partial-input comparison, in both concrete and template form.
///
/// `partial_template` is present only when the caller supplied a source item to
/// normalize, so a matcher can compare shapes instead of concrete arguments.
pub struct MatchInput<'a> {
    pub partial: &'a str,
    pub partial_template: Option<&'a Item>,
    pub candidate: &'a Item,
    pub candidate_template: &'a Item,
}

pub trait CandidateMatcher: Send + Sync {
    fn score(&self, partial: &str, candidate: &Item) -> Option<f64>;

    /// Scores a candidate with template context available.
    ///
    /// The default ignores the templates and defers to [`CandidateMatcher::score`].
    fn score_match(&self, input: MatchInput<'_>) -> Option<f64> {
        self.score(input.partial, input.candidate)
    }

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

    fn snapshot_key(&self) -> &str {
        "vista::matcher::ContainsMatcher"
    }
}

/// Character-trigram overlap, comparing templates when they are available.
///
/// Concrete arguments dominate edit distance between otherwise identical
/// commands, so a template comparison keeps `apt install {pkg}` close to
/// `sudo apt install {pkg}` regardless of the argument each one carried.
#[derive(Clone, Copy, Debug)]
pub struct SimilarityMatcher {
    threshold: f64,
}

impl SimilarityMatcher {
    pub fn new(threshold: f64) -> Self {
        Self {
            threshold: if threshold.is_finite() {
                threshold.clamp(0.0, 1.0)
            } else {
                0.5
            },
        }
    }

    fn similarity(&self, left: &str, right: &str) -> Option<f64> {
        let left = trigrams(left);
        let right = trigrams(right);
        if left.is_empty() || right.is_empty() {
            return None;
        }
        let shared = left.intersection(&right).count() as f64;
        let union = left.union(&right).count() as f64;
        let score = shared / union;
        (score >= self.threshold).then_some(score)
    }
}

impl Default for SimilarityMatcher {
    fn default() -> Self {
        Self::new(0.5)
    }
}

impl CandidateMatcher for SimilarityMatcher {
    fn score(&self, partial: &str, candidate: &Item) -> Option<f64> {
        self.similarity(partial, &candidate.value)
    }

    fn score_match(&self, input: MatchInput<'_>) -> Option<f64> {
        match input.partial_template {
            Some(template) => self.similarity(&template.value, &input.candidate_template.value),
            None => self.similarity(input.partial, &input.candidate.value),
        }
    }

    fn snapshot_key(&self) -> &str {
        "vista::matcher::SimilarityMatcher"
    }
}

fn trigrams(value: &str) -> BTreeSet<String> {
    let chars: Vec<_> = value.trim().to_lowercase().chars().collect();
    let width = chars.len().min(3);
    if width == 0 {
        return BTreeSet::new();
    }
    chars
        .windows(width)
        .map(|window| window.iter().collect())
        .collect()
}

#[derive(Clone, Default)]
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
    string_bytes: usize,
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
            string_bytes: 0,
        }
    }

    #[cfg(feature = "surface-indexes")]
    pub(crate) fn learn_fragments(&mut self, surface: SurfaceId, fragments: BTreeSet<String>) {
        for fragment in fragments {
            if !self.items.contains_key(&fragment) {
                self.string_bytes = self.string_bytes.saturating_add(fragment.len());
            }
            let counts = self.items.entry(fragment.clone()).or_default();
            let previous = counts.get(&surface).copied().unwrap_or(0);
            if previous == 0 {
                self.associations += 1;
                if self
                    .surface_keys
                    .entry(surface)
                    .or_default()
                    .insert(fragment.clone())
                {
                    self.string_bytes = self.string_bytes.saturating_add(fragment.len());
                }
            } else if self.order.remove(&(previous, fragment.clone(), surface)) {
                self.string_bytes = self.string_bytes.saturating_sub(fragment.len());
            }
            let next = previous.saturating_add(1);
            counts.insert(surface, next);
            if self.order.insert((next, fragment.clone(), surface)) {
                self.string_bytes = self.string_bytes.saturating_add(fragment.len());
            }
        }
        while self.associations > self.capacity {
            let Some((count, fragment, id)) = self.order.pop_first() else {
                break;
            };
            self.string_bytes = self.string_bytes.saturating_sub(fragment.len());
            if let Some(items) = self.items.get_mut(&fragment)
                && items.get(&id) == Some(&count)
            {
                items.remove(&id);
                self.associations -= 1;
                if items.is_empty() {
                    self.items.remove(&fragment);
                    self.string_bytes = self.string_bytes.saturating_sub(fragment.len());
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
        max_chars: usize,
    ) -> Self {
        #[cfg(not(feature = "surface-indexes"))]
        let _ = (capacity, max_chars);
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
            #[cfg(feature = "surface-indexes")]
            max_chars,
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
    pub(crate) fn additional_string_bytes(
        &self,
        fragments: &BTreeSet<String>,
        surface: SurfaceId,
    ) -> usize {
        fragments.iter().fold(0_usize, |total, fragment| {
            let unique = if self.items.contains_key(fragment) {
                0
            } else {
                fragment.len()
            };
            let association = if self
                .items
                .get(fragment)
                .is_none_or(|items| !items.contains_key(&surface))
            {
                fragment.len().saturating_mul(2)
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

#[cfg(feature = "surface-indexes")]
pub(crate) fn item_fragments(value: &str, max_chars: usize) -> BTreeSet<String> {
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
