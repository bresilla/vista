use super::*;

/// How partial input is used once it has driven candidate retrieval.
enum PartialMode<'a> {
    /// Consult the candidate matcher, optionally comparing templates.
    Filtered(Option<&'a Item>),
    /// Retrieve on the partial but let the caller discriminate.
    Retrieval,
}

impl Predictor {
    pub fn predict(&self, query: &Query) -> Vec<Prediction> {
        self.ranked(query, PartialMode::Filtered(None), query.limit)
    }

    /// Ranks predicted templates and refills them with `source`'s own slots.
    ///
    /// Matching compares template shapes rather than concrete arguments, and
    /// each template is returned once, carrying the arguments of `source`
    /// instead of those of the historical surface that was retained. Templates
    /// the normalizer cannot render are dropped. When `query.partial` is unset
    /// the source value is used, so partial retrieval still applies.
    pub fn predict_rendered(&self, query: &Query, source: &Item) -> Vec<Prediction> {
        if query.limit == 0 {
            return Vec::new();
        }
        let normalized = self.normalizer.normalize(source);
        let mut effective = query.clone();
        if effective.partial.is_none() {
            effective.partial = Some(source.value.clone());
        }
        let mut seen = BTreeSet::new();
        self.ranked(
            &effective,
            PartialMode::Filtered(Some(&normalized.template)),
            self.config.max_candidates,
        )
        .into_iter()
        .filter(|prediction| seen.insert(prediction.template.clone()))
        .filter_map(|mut prediction| {
            prediction.item = self
                .normalizer
                .render(&prediction.template, &normalized.slots)?;
            Some(prediction)
        })
        .take(query.limit)
        .collect()
    }

    /// Repairs `source` from the structure of the commands history already
    /// contains, without templates, slots, or a normalizer.
    ///
    /// Each ranked candidate is token-aligned against `source`: shared tokens
    /// are structure, tokens only the candidate carries are the repair, and
    /// differing tokens are kept from whichever side they belong to. Results
    /// that reproduce `source` unchanged are not repairs and are dropped, as
    /// are duplicates that different candidates repair to the same command.
    /// `Prediction::template` still identifies the history that matched.
    pub fn predict_aligned(&self, query: &Query, source: &Item) -> Vec<Prediction> {
        if query.limit == 0 {
            return Vec::new();
        }
        let mut current = source.clone();
        let mut visited = BTreeSet::from([source.value.clone()]);
        let mut repairs = Vec::new();
        for iteration in 1..=self.config.max_repair_iterations {
            let round = self.repair_round(query, &current, iteration);
            let Some(best) = round.first().map(|p| p.item.value.clone()) else {
                break;
            };
            repairs = round;
            if !visited.insert(best.clone()) {
                break;
            }
            current = Item::new(source.namespace.clone(), best);
        }
        repairs.truncate(query.limit);
        repairs
    }

    /// One repair pass over the candidates retrieved for `current`.
    fn repair_round(&self, query: &Query, current: &Item, iteration: usize) -> Vec<Prediction> {
        let mut effective = query.clone();
        effective.partial = Some(current.value.clone());
        // History decides which tokens are real; nothing describes the syntax.
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        let known = |token: &str| self.tokens.known(token);
        #[cfg(not(any(feature = "snapshot", feature = "surface-indexes")))]
        let known = |_: &str| false;
        let retyped =
            |typed: &str, corrected: &str| self.corrections.retyped_rate(typed, corrected);
        let channel = Channel {
            known: &known,
            retyped: &retyped,
            weight: self.config.channel_weight,
        };
        let mut seen = BTreeSet::new();
        self.ranked(
            &effective,
            PartialMode::Retrieval,
            self.config.max_candidates,
        )
        .into_iter()
        .filter_map(|mut prediction| {
            let repaired = repair(&current.value, &prediction.item.value, &channel)?;
            if repaired == current.value || !seen.insert(repaired.clone()) {
                return None;
            }
            prediction.item = Item::new(prediction.item.namespace.clone(), repaired);
            prediction.repair_iterations = iteration;
            Some(prediction)
        })
        .collect()
    }

    fn ranked(&self, query: &Query, mode: PartialMode<'_>, limit: usize) -> Vec<Prediction> {
        if limit == 0 || self.dictionary.templates.is_empty() {
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
            let partial = match (query.partial.as_deref(), &mode) {
                (Some(value), PartialMode::Filtered(partial_template)) => {
                    match self.matcher.score_match(MatchInput {
                        partial: value,
                        partial_template: *partial_template,
                        candidate: &surface.item,
                        candidate_template: &template.item,
                    }) {
                        Some(score) if score.is_finite() => score,
                        _ => continue,
                    }
                }
                _ => 0.0,
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
        predictions.truncate(limit.min(self.config.max_candidates));
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
