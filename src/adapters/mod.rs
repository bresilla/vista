mod matcher;
mod normalizer;
mod tokenizer;

pub use matcher::{CandidateMatcher, ContainsMatcher, ItemMatcher, MatchInput, SimilarityMatcher};
pub use normalizer::{IdentityNormalizer, NormalizedItem, Normalizer};
pub use tokenizer::{Tokenizer, WhitespaceTokenizer};

#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
pub(crate) use matcher::PartialIndex;
#[cfg(feature = "surface-indexes")]
pub(crate) use matcher::item_fragments;
pub(crate) use normalizer::MAX_SLOTS_PER_ITEM;
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
pub(crate) use tokenizer::TokenIndex;
#[cfg(feature = "surface-indexes")]
pub(crate) use tokenizer::normalized_tokens;
