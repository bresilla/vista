use crate::{Feature, Item};

pub(crate) const MAX_SLOTS_PER_ITEM: usize = 1024;

/// A reusable predictive template and the variable slots extracted from it.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedItem {
    pub template: Item,
    pub slots: Vec<Feature>,
}

/// Converts a raw item into a stable template used by the sequence model.
pub trait Normalizer: Send + Sync {
    fn normalize(&self, item: &Item) -> NormalizedItem;

    /// Rebuilds a concrete item from a predicted template and caller slots.
    ///
    /// This is the inverse of [`Normalizer::normalize`] and lets a predicted
    /// shape carry the arguments of the item being completed rather than the
    /// arguments of whichever historical surface was retained. Returning `None`
    /// rejects the template, which the default does whenever slots are present
    /// but no inverse is implemented.
    fn render(&self, template: &Item, slots: &[Feature]) -> Option<Item> {
        slots.is_empty().then(|| template.clone())
    }

    fn snapshot_key(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

/// Uses every raw item as its own template.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdentityNormalizer;

impl Normalizer for IdentityNormalizer {
    fn normalize(&self, item: &Item) -> NormalizedItem {
        NormalizedItem {
            template: item.clone(),
            slots: Vec::new(),
        }
    }

    fn snapshot_key(&self) -> &str {
        "vista::normalizer::IdentityNormalizer"
    }
}
