use std::collections::BTreeMap;

#[cfg(feature = "surface-indexes")]
use crate::adapters::PartialIndex;
#[cfg(feature = "surface-indexes")]
use crate::adapters::TokenIndex;
use crate::api::{Query, SurfaceId, TemplateId};
#[cfg(feature = "recent-cache")]
use crate::model::RecentCache;
use crate::model::{Dictionary, Ppm, PpmHistory};

pub(crate) struct Candidates<'a> {
    pub(crate) ppm: &'a Ppm,
    #[cfg(feature = "recent-cache")]
    pub(crate) cache: &'a RecentCache,
    pub(crate) dictionary: &'a Dictionary,
    #[cfg(feature = "surface-indexes")]
    pub(crate) context_candidates: &'a BTreeMap<SurfaceId, u64>,
    #[cfg(feature = "surface-indexes")]
    pub(crate) partials: &'a PartialIndex,
    #[cfg(feature = "surface-indexes")]
    pub(crate) tokens: &'a TokenIndex,
    #[cfg(feature = "surface-indexes")]
    pub(crate) query_tokens: &'a [String],
    #[cfg(feature = "recent-cache")]
    pub(crate) history: &'a [TemplateId],
    pub(crate) ppm_history: &'a PpmHistory,
    #[cfg(feature = "recent-cache")]
    pub(crate) clock: u64,
    pub(crate) template_limit: usize,
    pub(crate) surfaces_per_template: usize,
    pub(crate) candidate_limit: usize,
}

impl Candidates<'_> {
    pub(crate) fn generate(&self, query: &Query) -> Vec<SurfaceId> {
        #[cfg(not(any(feature = "recent-cache", feature = "surface-indexes")))]
        let _ = query;
        let mut template_weights = BTreeMap::<TemplateId, u64>::new();
        #[cfg(feature = "recent-cache")]
        let sources = [
            (
                300,
                self.ppm
                    .candidates_resolved(self.ppm_history, self.template_limit),
            ),
            (
                200,
                self.cache.candidates(
                    query.stream,
                    self.history.last().copied(),
                    self.clock,
                    self.template_limit,
                ),
            ),
            (100, self.dictionary.global_templates(self.template_limit)),
        ];
        #[cfg(not(feature = "recent-cache"))]
        let sources = [
            (
                300,
                self.ppm
                    .candidates_resolved(self.ppm_history, self.template_limit),
            ),
            (100, self.dictionary.global_templates(self.template_limit)),
        ];
        for (source_weight, candidates) in sources {
            for (rank, id) in candidates.into_iter().enumerate() {
                let evidence = u64::try_from(self.template_limit.saturating_sub(rank))
                    .unwrap_or(u64::MAX)
                    .saturating_mul(source_weight);
                let entry = template_weights.entry(id).or_default();
                *entry = entry.saturating_add(evidence);
            }
        }
        let mut templates: Vec<_> = template_weights.into_iter().collect();
        templates.sort_by(|(a_id, a), (b_id, b)| b.cmp(a).then_with(|| a_id.cmp(b_id)));
        templates.truncate(self.template_limit);

        let mut weighted = BTreeMap::<SurfaceId, u64>::new();
        #[cfg(feature = "surface-indexes")]
        if let Some(partial) = &query.partial {
            for (id, evidence) in self.partials.candidates(partial, self.candidate_limit) {
                let entry = weighted.entry(id).or_default();
                *entry = entry.saturating_add(evidence.saturating_mul(1_000));
            }
            for (id, evidence) in self
                .tokens
                .candidates(self.query_tokens, self.candidate_limit)
            {
                let entry = weighted.entry(id).or_default();
                *entry = entry.saturating_add(evidence.saturating_mul(100));
            }
        }
        #[cfg(feature = "surface-indexes")]
        for (id, evidence) in self.context_candidates {
            let entry = weighted.entry(*id).or_default();
            *entry = entry.saturating_add(evidence.saturating_mul(500));
        }
        for (template_rank, (template, _)) in templates.into_iter().enumerate() {
            for (surface_rank, id) in self
                .dictionary
                .surfaces_for(template, self.surfaces_per_template)
                .into_iter()
                .enumerate()
            {
                let template_weight =
                    u64::try_from(self.template_limit.saturating_sub(template_rank))
                        .unwrap_or(u64::MAX)
                        .saturating_mul(10);
                let surface_weight =
                    u64::try_from(self.surfaces_per_template.saturating_sub(surface_rank))
                        .unwrap_or(u64::MAX);
                let entry = weighted.entry(id).or_default();
                *entry = entry.saturating_add(template_weight.saturating_add(surface_weight));
            }
        }
        let mut ranked: Vec<_> = weighted.into_iter().collect();
        ranked.sort_by(|(a_id, a), (b_id, b)| b.cmp(a).then_with(|| a_id.cmp(b_id)));
        ranked
            .into_iter()
            .take(self.candidate_limit)
            .map(|(id, _)| id)
            .collect()
    }
}
