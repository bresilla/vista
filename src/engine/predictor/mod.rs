#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

mod builder;
mod maintenance;
mod observe;
mod predict;

pub use builder::PredictorBuilder;

use crate::adapters::{CandidateMatcher, ItemMatcher, MatchInput, Normalizer, Tokenizer};
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::adapters::{PartialIndex, TokenIndex};
use crate::api::{Config, Item, Observation, Query, StreamId, StreamTable, SurfaceId, TemplateId};
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::engine::ContextIndex;
use crate::engine::{Candidates, Channel, Prediction, RankInput, rank, repair};
#[cfg(any(feature = "recent-cache", feature = "snapshot"))]
use crate::model::RecentCache;
#[cfg(feature = "surface-indexes")]
use crate::model::context_ratio;
use crate::model::{CorrectionLog, CorrectionPair, Dictionary, Ppm, surface_ratio};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModelStats {
    pub templates: usize,
    pub surfaces: usize,
    pub streams: usize,
    pub contexts: usize,
    pub followers: usize,
    pub zero_order_entries: usize,
    pub cache_entries: usize,
    pub stream_history_entries: usize,
    pub context_associations: usize,
    pub tokens: usize,
    pub token_associations: usize,
    pub partial_associations: usize,
    pub observations: u64,
    pub correction_pairs: usize,
    pub retained_string_bytes: usize,
    pub estimated_heap_bytes: usize,
}

/// One bounded online predictor for historical sentence surfaces.
pub struct Predictor {
    pub(crate) config: Config,
    pub(crate) normalizer: Box<dyn Normalizer>,
    #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
    pub(crate) tokenizer: Box<dyn Tokenizer>,
    pub(crate) matcher: Box<dyn CandidateMatcher>,
    pub(crate) dictionary: Dictionary,
    pub(crate) streams: StreamTable,
    pub(crate) ppm: Ppm,
    #[cfg(any(feature = "recent-cache", feature = "snapshot"))]
    pub(crate) cache: RecentCache,
    #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
    pub(crate) context: ContextIndex,
    #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
    pub(crate) tokens: TokenIndex,
    #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
    pub(crate) partials: PartialIndex,
    pub(crate) corrections: CorrectionLog,
    pub(crate) clock: u64,
}

impl Predictor {
    pub fn new(config: Config) -> Self {
        PredictorBuilder::new(config).build()
    }

    pub fn builder(config: Config) -> PredictorBuilder {
        PredictorBuilder::new(config)
    }

    pub fn with_components<N, T, M>(config: Config, normalizer: N, tokenizer: T, matcher: M) -> Self
    where
        N: Normalizer + 'static,
        T: Tokenizer + 'static,
        M: CandidateMatcher + 'static,
    {
        let config = config.normalise();
        #[cfg(not(any(feature = "snapshot", feature = "surface-indexes")))]
        let _ = tokenizer;
        Self {
            dictionary: Dictionary::new(config.max_templates, config.max_surfaces),
            streams: StreamTable::new(config.max_streams),
            ppm: Ppm::new(
                config.max_contexts,
                config.max_followers_per_context,
                config.max_order,
            ),
            #[cfg(any(feature = "recent-cache", feature = "snapshot"))]
            cache: RecentCache::new(
                config.recent_cache_items,
                config.recent_cache_half_life,
                config.max_streams,
            ),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            context: ContextIndex::new(config.max_context_associations),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            tokens: TokenIndex::new(
                config.max_tokens,
                config.max_surface_candidates_per_template,
            ),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            partials: PartialIndex::new(
                config.max_partial_associations,
                config.max_partial_chars_per_item,
            ),
            corrections: CorrectionLog::new(config.max_correction_pairs),
            normalizer: Box::new(normalizer),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            tokenizer: Box::new(tokenizer),
            matcher: Box::new(matcher),
            clock: 0,
            config,
        }
    }
}
