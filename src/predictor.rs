#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[cfg(any(feature = "recent-cache", feature = "snapshot"))]
use crate::cache::RecentCache;
use crate::candidates::Candidates;
use crate::config::Config;
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::context::ContextIndex;
use crate::dictionary::Dictionary;
use crate::item::{Item, SurfaceId, TemplateId};
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::matcher::PartialIndex;
use crate::matcher::{CandidateMatcher, ContainsMatcher, ItemMatcher};
use crate::normalizer::{IdentityNormalizer, Normalizer, bound_slots};
use crate::observation::{Observation, Query};
use crate::ppm::Ppm;
use crate::ranking::{Prediction, RankInput, rank};
#[cfg(feature = "surface-indexes")]
use crate::statistics::context_ratio;
use crate::statistics::surface_ratio;
use crate::stream::{StreamId, StreamTable};
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
use crate::tokenizer::TokenIndex;
use crate::tokenizer::{Tokenizer, WhitespaceTokenizer};

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
    pub estimated_heap_bytes: usize,
}

pub struct PredictorBuilder<N = IdentityNormalizer, T = WhitespaceTokenizer, M = ContainsMatcher> {
    config: Config,
    normalizer: N,
    tokenizer: T,
    matcher: M,
}

impl PredictorBuilder {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            normalizer: IdentityNormalizer,
            tokenizer: WhitespaceTokenizer,
            matcher: ContainsMatcher,
        }
    }
}

impl<N, T, M> PredictorBuilder<N, T, M> {
    pub fn normalizer<N2>(self, normalizer: N2) -> PredictorBuilder<N2, T, M> {
        PredictorBuilder {
            config: self.config,
            normalizer,
            tokenizer: self.tokenizer,
            matcher: self.matcher,
        }
    }

    pub fn tokenizer<T2>(self, tokenizer: T2) -> PredictorBuilder<N, T2, M> {
        PredictorBuilder {
            config: self.config,
            normalizer: self.normalizer,
            tokenizer,
            matcher: self.matcher,
        }
    }

    pub fn matcher<M2>(self, matcher: M2) -> PredictorBuilder<N, T, M2> {
        PredictorBuilder {
            config: self.config,
            normalizer: self.normalizer,
            tokenizer: self.tokenizer,
            matcher,
        }
    }
}

impl<N, T, M> PredictorBuilder<N, T, M>
where
    N: Normalizer + 'static,
    T: Tokenizer + 'static,
    M: CandidateMatcher + 'static,
{
    pub fn build(self) -> Predictor {
        Predictor::with_components(self.config, self.normalizer, self.tokenizer, self.matcher)
    }
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
            normalizer: Box::new(normalizer),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            tokenizer: Box::new(tokenizer),
            matcher: Box::new(matcher),
            clock: 0,
            config,
        }
    }

    pub fn replay<I>(&mut self, observations: I)
    where
        I: IntoIterator<Item = Observation>,
    {
        self.clear();
        for observation in observations {
            self.observe(observation);
        }
    }

    pub fn observe(&mut self, observation: Observation) {
        self.clock = self.clock.saturating_add(1);
        let (continuous, evicted_stream) =
            self.streams.open(observation.stream, observation.position);
        #[cfg(feature = "recent-cache")]
        if let Some(stream) = evicted_stream {
            self.cache.break_stream(stream);
        }
        #[cfg(not(feature = "recent-cache"))]
        let _ = evicted_stream;
        #[cfg(feature = "recent-cache")]
        if !continuous {
            self.cache.break_stream(observation.stream);
        }
        let mut history = if continuous {
            self.streams.history(observation.stream)
        } else {
            Vec::new()
        };
        let normalized = bound_slots(self.normalizer.normalize(&observation.item));
        #[cfg(feature = "surface-indexes")]
        let slots = normalized.slots.clone();
        let Some(admission) = self.dictionary.admit(
            &observation.item,
            normalized,
            &observation.outcome,
            self.clock,
        ) else {
            self.streams.break_stream(observation.stream);
            #[cfg(feature = "recent-cache")]
            self.cache.break_stream(observation.stream);
            return;
        };
        let invalidated_history = admission
            .removed_templates
            .iter()
            .any(|template| history.contains(template));
        for surface in admission.removed_surfaces {
            self.remove_surface_indexes(surface);
        }
        for template in admission.removed_templates {
            self.remove_template_indexes(template);
        }
        if invalidated_history {
            history.clear();
        }

        self.ppm.learn(&history, admission.template, self.clock);
        #[cfg(feature = "surface-indexes")]
        {
            let mut context: Vec<_> = observation
                .context
                .iter()
                .take(self.config.max_context_associations)
                .cloned()
                .collect();
            context.extend(
                slots.into_iter().take(
                    self.config
                        .max_context_associations
                        .saturating_sub(context.len()),
                ),
            );
            self.context.learn(&context, admission.surface);
            self.tokens
                .learn(self.tokenizer.tokens(&observation.item), admission.surface);
            self.partials.learn(admission.surface, &observation.item);
        }
        #[cfg(feature = "recent-cache")]
        self.cache.observe(
            observation.stream,
            self.clock,
            history.last().copied(),
            admission.template,
        );
        self.streams.advance(
            observation.stream,
            admission.template,
            observation.position,
            self.config.max_order,
            self.clock,
        );
    }

    pub fn predict(&self, query: &Query) -> Vec<Prediction> {
        if query.limit == 0 || self.dictionary.templates.is_empty() {
            return Vec::new();
        }
        let history = self
            .streams
            .continuation(query.stream, query.position)
            .map(|stream| stream.history())
            .unwrap_or_default();
        #[cfg(feature = "surface-indexes")]
        let context_candidates = self
            .context
            .candidates(&query.context, self.config.max_candidates);
        #[cfg(feature = "surface-indexes")]
        let query_tokens = query
            .partial
            .as_deref()
            .map(|partial| self.tokenizer.query_tokens(partial))
            .unwrap_or_default();
        let surfaces = Candidates {
            ppm: &self.ppm,
            #[cfg(feature = "recent-cache")]
            cache: &self.cache,
            dictionary: &self.dictionary,
            #[cfg(feature = "surface-indexes")]
            context_candidates: &context_candidates,
            #[cfg(feature = "surface-indexes")]
            partials: &self.partials,
            #[cfg(feature = "surface-indexes")]
            tokens: &self.tokens,
            #[cfg(feature = "surface-indexes")]
            query_tokens: &query_tokens,
            history: &history,
            #[cfg(feature = "recent-cache")]
            clock: self.clock,
            template_limit: self.config.max_candidate_templates,
            surfaces_per_template: self.config.max_surface_candidates_per_template,
            candidate_limit: self.config.max_candidates,
        }
        .generate(query);
        #[cfg(feature = "surface-indexes")]
        let context_counts = self.context.counts_for(&query.context, &surfaces);

        let mut predictions = Vec::with_capacity(surfaces.len());
        for surface_id in surfaces {
            let Some(surface) = self.dictionary.surface(surface_id) else {
                continue;
            };
            let Some(template) = self.dictionary.template(surface.template) else {
                continue;
            };
            let partial = match query.partial.as_deref() {
                Some(value) => match self.matcher.score(value, &surface.item) {
                    Some(score) if score.is_finite() => score,
                    _ => continue,
                },
                None => 0.0,
            };
            let trace =
                self.ppm
                    .probability(&history, surface.template, self.dictionary.templates.len());
            #[cfg(feature = "recent-cache")]
            let cache_probability = self.cache.probability(
                query.stream,
                history.last().copied(),
                surface.template,
                self.clock,
            );
            #[cfg(feature = "recent-cache")]
            let probability = cache_probability.map_or(trace.probability, |cache| {
                (1.0 - self.config.recent_cache_weight) * trace.probability
                    + self.config.recent_cache_weight * cache
            });
            #[cfg(not(feature = "recent-cache"))]
            let probability = trace.probability;
            #[cfg(feature = "surface-indexes")]
            let context = context_ratio(
                context_counts.get(&surface_id).copied().unwrap_or(0),
                &surface.stats,
            );
            #[cfg(not(feature = "surface-indexes"))]
            let context = 0.0;
            let surface_evidence = surface_ratio(&surface.stats, &template.stats, self.clock);
            predictions.push(rank(
                RankInput {
                    item: surface.item.clone(),
                    template: template.item.clone(),
                    probability,
                    #[cfg(feature = "explanations")]
                    long_term_probability: trace.probability,
                    context,
                    surface: surface_evidence,
                    outcome: surface.stats.quality().unwrap_or(0.0),
                    partial,
                    deepest: trace.deepest,
                    #[cfg(feature = "explanations")]
                    backoffs: trace.backoffs,
                    #[cfg(feature = "explanations")]
                    count: trace.count,
                    #[cfg(feature = "explanations")]
                    total: trace.total,
                    #[cfg(feature = "explanations")]
                    cache_probability: {
                        #[cfg(feature = "recent-cache")]
                        {
                            cache_probability
                        }
                        #[cfg(not(feature = "recent-cache"))]
                        {
                            None
                        }
                    },
                },
                &self.config.weights,
            ));
        }
        predictions.sort_by(Prediction::cmp_rank);
        predictions.truncate(query.limit.min(self.config.max_candidates));
        predictions
    }

    pub fn probability_of(&self, query: &Query, item: &Item) -> f64 {
        let normalized = self.normalizer.normalize(item);
        let history = self
            .streams
            .continuation(query.stream, query.position)
            .map(|stream| stream.history())
            .unwrap_or_default();
        let Some(template) = self.dictionary.template_id(&normalized.template) else {
            let ppm = self
                .ppm
                .unknown_probability(&history, self.dictionary.templates.len());
            #[cfg(feature = "recent-cache")]
            return self
                .cache
                .unknown_probability(query.stream, history.last().copied())
                .map_or(ppm, |_| (1.0 - self.config.recent_cache_weight) * ppm);
            #[cfg(not(feature = "recent-cache"))]
            return ppm;
        };
        let ppm = self
            .ppm
            .probability(&history, template, self.dictionary.templates.len())
            .probability;
        #[cfg(feature = "recent-cache")]
        return self
            .cache
            .probability(query.stream, history.last().copied(), template, self.clock)
            .map_or(ppm, |cache| {
                (1.0 - self.config.recent_cache_weight) * ppm
                    + self.config.recent_cache_weight * cache
            });
        #[cfg(not(feature = "recent-cache"))]
        ppm
    }

    pub fn break_stream(&mut self, stream: StreamId) {
        self.streams.break_stream(stream);
        #[cfg(feature = "recent-cache")]
        self.cache.break_stream(stream);
    }

    pub fn forget(&mut self, matcher: &dyn ItemMatcher) {
        let surfaces: Vec<_> = self
            .dictionary
            .surfaces
            .iter()
            .filter(|(_, record)| matcher.matches(&record.item))
            .map(|(id, _)| *id)
            .collect();
        let mut templates = BTreeSet::new();
        for surface in surfaces {
            if let Some(record) = self.dictionary.remove_surface(surface) {
                templates.insert(record.template);
                self.remove_surface_indexes(surface);
            }
        }
        for template in templates {
            let empty = self
                .dictionary
                .template(template)
                .is_some_and(|record| record.surfaces.is_empty());
            if empty {
                self.dictionary.remove_template(template);
                self.remove_template_indexes(template);
            }
        }
    }

    fn remove_surface_indexes(&mut self, surface: SurfaceId) {
        #[cfg(feature = "surface-indexes")]
        {
            self.context.remove_surface(surface);
            self.tokens.remove_surface(surface);
            self.partials.remove_surface(surface);
        }
        #[cfg(not(feature = "surface-indexes"))]
        let _ = surface;
    }

    fn remove_template_indexes(&mut self, template: TemplateId) {
        self.ppm.remove_template(template);
        #[cfg(feature = "recent-cache")]
        self.cache.remove_template(template);
        self.streams.remove_template(template);
    }

    pub fn clear(&mut self) {
        self.dictionary.clear();
        self.streams.clear();
        self.ppm.clear();
        #[cfg(feature = "recent-cache")]
        self.cache.clear();
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        {
            self.context.clear();
            self.tokens.clear();
            self.partials.clear();
        }
        self.clock = 0;
    }

    pub fn stats(&self) -> ModelStats {
        let followers = self
            .ppm
            .contexts
            .values()
            .map(|state| state.followers.len())
            .sum();
        #[cfg(feature = "recent-cache")]
        let cache_entries = self.cache.global.len()
            + self
                .cache
                .streams
                .values()
                .map(|entries| entries.len())
                .sum::<usize>();
        #[cfg(not(feature = "recent-cache"))]
        let cache_entries = 0;
        let stream_history_entries = self
            .streams
            .streams
            .values()
            .map(|stream| stream.recent.len())
            .sum::<usize>();
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        let context_associations = self.context.associations();
        #[cfg(not(any(feature = "snapshot", feature = "surface-indexes")))]
        let context_associations = 0;
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        let tokens = self.tokens.len();
        #[cfg(not(any(feature = "snapshot", feature = "surface-indexes")))]
        let tokens = 0;
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        let token_associations = self.tokens.items.values().map(BTreeMap::len).sum();
        #[cfg(not(any(feature = "snapshot", feature = "surface-indexes")))]
        let token_associations = 0;
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        let partial_associations = self.partials.associations();
        #[cfg(not(any(feature = "snapshot", feature = "surface-indexes")))]
        let partial_associations = 0;
        let mut stats = ModelStats {
            templates: self.dictionary.templates.len(),
            surfaces: self.dictionary.surfaces.len(),
            streams: self.streams.len(),
            contexts: self.ppm.contexts.len(),
            followers,
            zero_order_entries: self.ppm.zero.len(),
            cache_entries,
            stream_history_entries,
            context_associations,
            tokens,
            token_associations,
            partial_associations,
            observations: self.clock,
            estimated_heap_bytes: 0,
        };
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        let index_string_bytes = self
            .context
            .items
            .keys()
            .map(String::len)
            .chain(self.tokens.items.keys().map(String::len))
            .chain(self.partials.items.keys().map(String::len))
            .fold(0_usize, usize::saturating_add)
            .saturating_add(self.context.reverse_key_bytes().saturating_mul(2))
            .saturating_add(self.tokens.reverse_key_bytes())
            .saturating_add(self.partials.reverse_key_bytes().saturating_mul(2));
        #[cfg(not(any(feature = "snapshot", feature = "surface-indexes")))]
        let index_string_bytes = 0;
        let string_bytes = self
            .dictionary
            .templates
            .values()
            .map(|record| record.item.namespace.len() + record.item.value.len())
            .chain(self.dictionary.surfaces.values().map(|record| {
                record.item.namespace.len()
                    + record.item.value.len()
                    + record
                        .slots
                        .iter()
                        .map(|feature| match feature {
                            crate::Feature::Categorical { name, value } => name.len() + value.len(),
                            crate::Feature::Numeric { name, .. } => name.len(),
                        })
                        .sum::<usize>()
            }))
            .fold(0_usize, usize::saturating_add)
            .saturating_add(index_string_bytes);
        let context_members = self.ppm.contexts.keys().map(Vec::len).sum::<usize>();
        stats.estimated_heap_bytes = [
            stats.templates.saturating_mul(160),
            stats.surfaces.saturating_mul(192),
            stats.streams.saturating_mul(160),
            stats.contexts.saturating_mul(96),
            stats.followers.saturating_mul(32),
            stats.zero_order_entries.saturating_mul(24),
            stats.context_associations.saturating_mul(24),
            stats.tokens.saturating_mul(64),
            stats.token_associations.saturating_mul(24),
            stats.partial_associations.saturating_mul(24),
            stats
                .context_associations
                .saturating_add(stats.token_associations)
                .saturating_add(stats.partial_associations)
                .saturating_mul(24),
            stats
                .context_associations
                .saturating_add(stats.partial_associations)
                .saturating_mul(24),
            stats.cache_entries.saturating_mul(24),
            stats
                .stream_history_entries
                .saturating_mul(std::mem::size_of::<TemplateId>()),
            context_members.saturating_mul(std::mem::size_of::<TemplateId>()),
            string_bytes,
        ]
        .into_iter()
        .fold(0_usize, usize::saturating_add);
        stats
    }
}
