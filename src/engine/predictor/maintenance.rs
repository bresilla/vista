use super::*;

impl Predictor {
    pub(crate) fn retained_string_bytes(&self) -> usize {
        let bytes = self
            .dictionary
            .string_bytes()
            .saturating_add(self.corrections.string_bytes());
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        let bytes = bytes
            .saturating_add(self.context.string_bytes())
            .saturating_add(self.tokens.string_bytes())
            .saturating_add(self.partials.string_bytes());
        bytes
    }

    pub fn break_stream(&mut self, stream: StreamId) {
        self.streams.break_stream(stream);
        self.corrections.break_stream(stream);
        #[cfg(feature = "recent-cache")]
        self.cache.break_stream(stream);
    }

    /// Retypings mined from history, with how often each was observed.
    pub fn corrections(&self) -> Vec<(CorrectionPair, u64)> {
        self.corrections
            .pairs()
            .map(|(pair, count)| (pair.clone(), count))
            .collect()
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

    pub(super) fn remove_surface_indexes(&mut self, surface: SurfaceId) {
        #[cfg(feature = "surface-indexes")]
        {
            self.context.remove_surface(surface);
            self.tokens.remove_surface(surface);
            self.partials.remove_surface(surface);
        }
        #[cfg(not(feature = "surface-indexes"))]
        let _ = surface;
    }

    pub(super) fn remove_template_indexes(&mut self, template: TemplateId) {
        self.ppm.remove_template(template);
        #[cfg(feature = "recent-cache")]
        self.cache.remove_template(template);
        self.streams.remove_template(template);
    }

    pub fn clear(&mut self) {
        self.dictionary.clear();
        self.streams.clear();
        self.ppm.clear();
        self.corrections.clear();
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
        let followers = self.ppm.follower_count();
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
            contexts: self.ppm.context_count(),
            followers,
            zero_order_entries: self.ppm.zero.len(),
            cache_entries,
            stream_history_entries,
            context_associations,
            tokens,
            token_associations,
            partial_associations,
            observations: self.clock,
            correction_pairs: self.corrections.len(),
            retained_string_bytes: self.retained_string_bytes(),
            estimated_heap_bytes: 0,
        };
        let context_members = self.ppm.context_member_count();
        let reverse_context_associations = self.ppm.reverse_association_count();
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
            reverse_context_associations.saturating_mul(24),
            stats.retained_string_bytes,
        ]
        .into_iter()
        .fold(0_usize, usize::saturating_add);
        stats
    }
}
