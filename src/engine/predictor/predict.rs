use super::*;

impl Predictor {
    pub fn predict(&self, query: &Query) -> Vec<Prediction> {
        if query.limit == 0 || self.dictionary.templates.is_empty() {
            return Vec::new();
        }
        let history = self
            .streams
            .continuation(query.stream, query.position)
            .map(|stream| stream.history())
            .unwrap_or_default();
        let ppm_history = self.ppm.resolve(&history);
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
            #[cfg(feature = "recent-cache")]
            history: &history,
            ppm_history: &ppm_history,
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
            let trace = self.ppm.probability_resolved(
                &ppm_history,
                surface.template,
                self.dictionary.templates.len(),
            );
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
}
