use std::collections::{BTreeMap, BTreeSet};

use crate::api::SurfaceId;

/// Longest item indexed by shape, in tokens.
const MAX_SHAPE_TOKENS: usize = 24;

/// Positional token classes of an item, ignoring the words themselves.
///
/// Repair works by structural analogy: `crane index filter --help` can fix
/// `hexe mux float --hlp` because they share a shape, not a vocabulary. Two
/// items with the same shape align token for token, which is the arrangement
/// alignment can actually use.
pub(crate) fn shape(value: &str) -> Option<String> {
    let mut shape = String::new();
    for token in value.split_whitespace() {
        if shape.len() >= MAX_SHAPE_TOKENS {
            return None;
        }
        shape.push(if token.starts_with('-') {
            'f'
        } else if token.contains('/') {
            'p'
        } else if token.chars().all(|c| c.is_ascii_digit()) {
            'n'
        } else {
            'w'
        });
    }
    (shape.len() > 1).then_some(shape)
}

/// Surfaces grouped by shape, so a damaged item can find items arranged
/// like it. Derived from retained surfaces, so snapshots rebuild it instead
/// of storing it.
#[derive(Clone, Default)]
pub(crate) struct ShapeIndex {
    items: BTreeMap<String, BTreeSet<SurfaceId>>,
    shapes: BTreeMap<SurfaceId, String>,
    capacity: usize,
}

impl ShapeIndex {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            ..Self::default()
        }
    }

    pub(crate) fn learn(&mut self, value: &str, surface: SurfaceId) {
        let Some(shape) = shape(value) else { return };
        if self.shapes.insert(surface, shape.clone()).is_none() {
            self.items.entry(shape).or_default().insert(surface);
        }
        while self.shapes.len() > self.capacity {
            let Some((&oldest, _)) = self.shapes.iter().next() else {
                break;
            };
            self.remove_surface(oldest);
        }
    }

    pub(crate) fn matching(&self, shape: &str) -> impl Iterator<Item = SurfaceId> {
        self.items
            .get(shape)
            .into_iter()
            .flat_map(|surfaces| surfaces.iter().copied())
    }

    pub(crate) fn remove_surface(&mut self, surface: SurfaceId) {
        let Some(shape) = self.shapes.remove(&surface) else {
            return;
        };
        if let Some(surfaces) = self.items.get_mut(&shape) {
            surfaces.remove(&surface);
            if surfaces.is_empty() {
                self.items.remove(&shape);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.shapes.clear();
    }
}

/// Edit distance, abandoned once it cannot fall within `budget`.
pub(crate) fn distance_within(left: &[char], right: &[char], budget: usize) -> Option<usize> {
    if left.len().abs_diff(right.len()) > budget {
        return None;
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0_usize; right.len() + 1];
    for (row, from) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, to) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(from != to);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        if current.iter().min().copied().unwrap_or(0) > budget {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[right.len()];
    (distance <= budget).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_classifies_tokens_by_position() {
        assert_eq!(shape("hexe mux float --hlp").as_deref(), Some("wwwf"));
        assert_eq!(shape("crane index filter --help").as_deref(), Some("wwwf"));
        assert_eq!(shape("cat /etc/hosts").as_deref(), Some("wp"));
        assert_eq!(shape("kill -9 1234").as_deref(), Some("wfn"));
    }

    #[test]
    fn single_token_and_oversized_items_are_not_indexed() {
        assert_eq!(shape("ls"), None);
        assert_eq!(shape(&"x ".repeat(MAX_SHAPE_TOKENS + 1)), None);
    }

    #[test]
    fn removal_drops_the_surface_from_its_bucket() {
        let mut index = ShapeIndex::new(8);
        index.learn("git checkout main", SurfaceId(1));
        assert_eq!(index.matching("www").count(), 1);
        index.remove_surface(SurfaceId(1));
        assert_eq!(index.matching("www").count(), 0);
    }

    #[test]
    fn the_index_respects_its_bound() {
        let mut index = ShapeIndex::new(2);
        for id in 0..6 {
            index.learn("git checkout main", SurfaceId(id));
        }
        assert!(index.matching("www").count() <= 2);
    }
}
