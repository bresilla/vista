/// Bounded presentation adjustments applied after sequence probability.
#[derive(Clone, Debug, PartialEq)]
pub struct Weights {
    pub context: f64,
    pub surface: f64,
    pub outcome: f64,
    pub partial: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            context: 0.35,
            surface: 0.20,
            outcome: 0.15,
            partial: 0.50,
        }
    }
}

/// Hard memory limits and prediction parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub max_string_bytes: usize,
    pub max_retained_string_bytes: usize,
    pub max_snapshot_bytes: usize,
    pub max_templates: usize,
    pub max_surfaces: usize,
    pub max_streams: usize,
    pub max_order: usize,
    pub max_contexts: usize,
    pub max_followers_per_context: usize,
    pub max_context_associations: usize,
    pub max_tokens: usize,
    pub max_partial_chars_per_item: usize,
    pub max_partial_associations: usize,
    pub max_candidate_templates: usize,
    pub max_surface_candidates_per_template: usize,
    pub max_candidates: usize,
    pub recent_cache_items: usize,
    pub recent_cache_weight: f64,
    pub recent_cache_half_life: u64,
    pub max_repair_iterations: usize,
    pub max_correction_pairs: usize,
    pub channel_weight: f64,
    pub weights: Weights,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_string_bytes: 65_536,
            max_retained_string_bytes: 67_108_864,
            max_snapshot_bytes: 134_217_728,
            max_templates: 16_384,
            max_surfaces: 32_768,
            max_streams: 256,
            max_order: 8,
            max_contexts: 262_144,
            max_followers_per_context: 64,
            max_context_associations: 65_536,
            max_tokens: 32_768,
            max_partial_chars_per_item: 512,
            max_partial_associations: 65_536,
            max_candidate_templates: 128,
            max_surface_candidates_per_template: 8,
            max_candidates: 128,
            recent_cache_items: 256,
            recent_cache_weight: 0.20,
            recent_cache_half_life: 32,
            max_repair_iterations: 3,
            max_correction_pairs: 4_096,
            channel_weight: 1.0,
            weights: Weights::default(),
        }
    }
}

impl Config {
    /// Strict low-memory preset for embedded hosts.
    ///
    /// This retains less history and considers fewer candidates than
    /// [`Config::default`]. Applications should evaluate its recall on their
    /// own chronological data before deployment.
    pub const fn tiny() -> Self {
        Self {
            max_string_bytes: 1_024,
            max_retained_string_bytes: 65_536,
            max_snapshot_bytes: 1_048_576,
            max_templates: 64,
            max_surfaces: 128,
            max_streams: 4,
            max_order: 3,
            max_contexts: 256,
            max_followers_per_context: 4,
            max_context_associations: 128,
            max_tokens: 64,
            max_partial_chars_per_item: 64,
            max_partial_associations: 128,
            max_candidate_templates: 12,
            max_surface_candidates_per_template: 3,
            max_candidates: 12,
            recent_cache_items: 16,
            recent_cache_weight: 0.20,
            recent_cache_half_life: 16,
            max_repair_iterations: 1,
            max_correction_pairs: 32,
            channel_weight: 1.0,
            weights: Weights {
                context: 0.35,
                surface: 0.20,
                outcome: 0.15,
                partial: 0.50,
            },
        }
    }

    pub(crate) fn normalise(mut self) -> Self {
        self.max_string_bytes = self.max_string_bytes.max(1);
        self.max_retained_string_bytes = self.max_retained_string_bytes.max(1);
        self.max_snapshot_bytes = self.max_snapshot_bytes.max(1);
        self.max_templates = self.max_templates.max(1).min(u32::MAX as usize);
        self.max_surfaces = self.max_surfaces.max(1).min(u32::MAX as usize);
        self.max_streams = self.max_streams.max(1);
        self.max_order = self.max_order.clamp(1, 32);
        self.max_contexts = self.max_contexts.max(1).min(u32::MAX as usize);
        self.max_followers_per_context = self.max_followers_per_context.max(1);
        self.max_context_associations = self.max_context_associations.max(1);
        self.max_tokens = self.max_tokens.max(1);
        self.max_partial_chars_per_item = self.max_partial_chars_per_item.max(1);
        self.max_partial_associations = self.max_partial_associations.max(1);
        self.max_candidate_templates = self.max_candidate_templates.max(1);
        self.max_surface_candidates_per_template = self.max_surface_candidates_per_template.max(1);
        self.max_candidates = self.max_candidates.max(1);
        self.recent_cache_items = self.recent_cache_items.max(1);
        if !self.recent_cache_weight.is_finite() {
            self.recent_cache_weight = 0.20;
        }
        self.recent_cache_weight = self.recent_cache_weight.clamp(0.0, 0.5);
        self.recent_cache_half_life = self.recent_cache_half_life.max(1);
        self.max_repair_iterations = self.max_repair_iterations.clamp(1, 8);
        self.max_correction_pairs = self.max_correction_pairs.max(1);
        if !self.channel_weight.is_finite() {
            self.channel_weight = 1.0;
        }
        self.channel_weight = self.channel_weight.clamp(0.0, 10.0);
        let defaults = Weights::default();
        normalise_weight(&mut self.weights.context, defaults.context);
        normalise_weight(&mut self.weights.surface, defaults.surface);
        normalise_weight(&mut self.weights.outcome, defaults.outcome);
        normalise_weight(&mut self.weights.partial, defaults.partial);
        self
    }
}

fn normalise_weight(value: &mut f64, fallback: f64) {
    if !value.is_finite() {
        *value = fallback;
    }
    *value = value.clamp(-10.0, 10.0);
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn tiny_preset_uses_strict_bounds() {
        let config = Config::tiny();

        assert_eq!(config.max_templates, 64);
        assert_eq!(config.max_string_bytes, 1_024);
        assert_eq!(config.max_retained_string_bytes, 65_536);
        assert_eq!(config.max_snapshot_bytes, 1_048_576);
        assert_eq!(config.max_surfaces, 128);
        assert_eq!(config.max_contexts, 256);
        assert_eq!(config.max_followers_per_context, 4);
        assert_eq!(config.max_candidates, 12);
        assert_eq!(config.recent_cache_items, 16);
        assert_eq!(config.clone().normalise(), config);
    }
}
