use super::*;

use crate::adapters::{MAX_SLOTS_PER_ITEM, NormalizedItem};
#[cfg(feature = "surface-indexes")]
use crate::adapters::{item_fragments, normalized_tokens};
#[cfg(feature = "surface-indexes")]
use crate::api::association_keys;
use crate::api::{Feature, InputError};

#[derive(Clone)]
struct PreparedObservation {
    observation: Observation,
    normalized: NormalizedItem,
    #[cfg(feature = "surface-indexes")]
    context_keys: Vec<String>,
    #[cfg(feature = "surface-indexes")]
    tokens: BTreeSet<String>,
    #[cfg(feature = "surface-indexes")]
    fragments: BTreeSet<String>,
}

struct ModelCheckpoint {
    dictionary: Dictionary,
    streams: StreamTable,
    ppm: Ppm,
    #[cfg(any(feature = "recent-cache", feature = "snapshot"))]
    cache: RecentCache,
    #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
    context: ContextIndex,
    #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
    tokens: TokenIndex,
    #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
    partials: PartialIndex,
    clock: u64,
}

impl ModelCheckpoint {
    fn capture(predictor: &Predictor) -> Self {
        Self {
            dictionary: predictor.dictionary.clone(),
            streams: predictor.streams.clone(),
            ppm: predictor.ppm.clone(),
            #[cfg(any(feature = "recent-cache", feature = "snapshot"))]
            cache: predictor.cache.clone(),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            context: predictor.context.clone(),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            tokens: predictor.tokens.clone(),
            #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
            partials: predictor.partials.clone(),
            clock: predictor.clock,
        }
    }

    fn restore(self, predictor: &mut Predictor) {
        predictor.dictionary = self.dictionary;
        predictor.streams = self.streams;
        predictor.ppm = self.ppm;
        #[cfg(any(feature = "recent-cache", feature = "snapshot"))]
        {
            predictor.cache = self.cache;
        }
        #[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
        {
            predictor.context = self.context;
            predictor.tokens = self.tokens;
            predictor.partials = self.partials;
        }
        predictor.clock = self.clock;
    }
}

impl Predictor {
    pub fn replay<I>(&mut self, observations: I) -> Result<(), InputError>
    where
        I: IntoIterator<Item = Observation>,
    {
        let checkpoint = ModelCheckpoint::capture(self);
        self.clear();
        for observation in observations {
            if let Err(error) = self.observe(observation) {
                checkpoint.restore(self);
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn observe(&mut self, observation: Observation) -> Result<(), InputError> {
        let prepared = self.prepare(observation)?;
        self.dictionary
            .validate_admission(&prepared.observation.item, &prepared.normalized)?;
        self.validate_retained_budget(&prepared)?;
        self.apply_prepared(prepared);
        Ok(())
    }

    fn prepare(&self, observation: Observation) -> Result<PreparedObservation, InputError> {
        validate_item(&observation.item, "raw item", self.config.max_string_bytes)?;
        validate_features(
            &observation.context,
            "context feature",
            self.config.max_string_bytes,
        )?;
        validate_features(
            &observation.outcome,
            "outcome feature",
            self.config.max_string_bytes,
        )?;
        let normalized = self.normalizer.normalize(&observation.item);
        validate_item(
            &normalized.template,
            "normalized template",
            self.config.max_string_bytes,
        )?;
        if normalized.slots.len() > MAX_SLOTS_PER_ITEM {
            return Err(InputError::TooManySlots {
                count: normalized.slots.len(),
                limit: MAX_SLOTS_PER_ITEM,
            });
        }
        validate_features(
            &normalized.slots,
            "normalized slot",
            self.config.max_string_bytes,
        )?;

        #[cfg(feature = "surface-indexes")]
        let (context_keys, tokens, fragments) = {
            let mut context: Vec<_> = observation
                .context
                .iter()
                .take(self.config.max_context_associations)
                .cloned()
                .collect();
            context.extend(
                normalized
                    .slots
                    .iter()
                    .take(
                        self.config
                            .max_context_associations
                            .saturating_sub(context.len()),
                    )
                    .cloned(),
            );
            let context_keys = association_keys(&context, self.config.max_context_associations);
            validate_strings(
                context_keys.iter().map(String::as_str),
                "context key",
                self.config.max_string_bytes,
            )?;

            let raw_tokens = self.tokenizer.tokens(&observation.item);
            validate_strings(
                raw_tokens.iter().map(String::as_str),
                "token",
                self.config.max_string_bytes,
            )?;
            for token in &raw_tokens {
                validate_string(
                    "normalized token",
                    &token.to_lowercase(),
                    self.config.max_string_bytes,
                )?;
            }
            let tokens = normalized_tokens(raw_tokens);
            let fragments = item_fragments(
                &observation.item.value,
                self.config.max_partial_chars_per_item,
            );
            validate_strings(
                fragments.iter().map(String::as_str),
                "partial fragment",
                self.config.max_string_bytes,
            )?;
            (context_keys, tokens, fragments)
        };

        Ok(PreparedObservation {
            observation,
            normalized,
            #[cfg(feature = "surface-indexes")]
            context_keys,
            #[cfg(feature = "surface-indexes")]
            tokens,
            #[cfg(feature = "surface-indexes")]
            fragments,
        })
    }

    fn validate_retained_budget(&self, prepared: &PreparedObservation) -> Result<(), InputError> {
        let additional = self
            .dictionary
            .additional_string_bytes(&prepared.observation.item, &prepared.normalized);
        #[cfg(feature = "surface-indexes")]
        let additional = {
            let surface = self
                .dictionary
                .surface_id(&prepared.observation.item)
                .unwrap_or(SurfaceId(self.dictionary.next_surface));
            additional
                .saturating_add(
                    self.context
                        .additional_string_bytes(&prepared.context_keys, surface),
                )
                .saturating_add(
                    self.tokens
                        .additional_string_bytes(&prepared.tokens, surface),
                )
                .saturating_add(
                    self.partials
                        .additional_string_bytes(&prepared.fragments, surface),
                )
        };
        let upper = self.retained_string_bytes().saturating_add(additional);
        if upper <= self.config.max_retained_string_bytes {
            return Ok(());
        }
        let actual = self.simulated_retained_string_bytes(prepared);
        if actual > self.config.max_retained_string_bytes {
            return Err(InputError::RetainedStringBytesExceeded {
                bytes: actual,
                limit: self.config.max_retained_string_bytes,
            });
        }
        Ok(())
    }

    fn simulated_retained_string_bytes(&self, prepared: &PreparedObservation) -> usize {
        let mut dictionary = self.dictionary.clone();
        let admission = dictionary
            .admit(
                &prepared.observation.item,
                prepared.normalized.clone(),
                &prepared.observation.outcome,
                self.clock.saturating_add(1),
            )
            .expect("validated dictionary admission");
        #[cfg(not(feature = "surface-indexes"))]
        let _ = admission;
        let bytes = dictionary.string_bytes();
        #[cfg(feature = "surface-indexes")]
        {
            let mut context = self.context.clone();
            let mut tokens = self.tokens.clone();
            let mut partials = self.partials.clone();
            for surface in admission.removed_surfaces {
                context.remove_surface(surface);
                tokens.remove_surface(surface);
                partials.remove_surface(surface);
            }
            context.learn_keys(prepared.context_keys.clone(), admission.surface);
            tokens.learn_normalized(prepared.tokens.clone(), admission.surface);
            partials.learn_fragments(admission.surface, prepared.fragments.clone());
            bytes
                .saturating_add(context.string_bytes())
                .saturating_add(tokens.string_bytes())
                .saturating_add(partials.string_bytes())
        }
        #[cfg(not(feature = "surface-indexes"))]
        bytes
    }

    fn apply_prepared(&mut self, prepared: PreparedObservation) {
        self.clock = self.clock.saturating_add(1);
        let observation = prepared.observation;
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
        let admission = self
            .dictionary
            .admit(
                &observation.item,
                prepared.normalized,
                &observation.outcome,
                self.clock,
            )
            .expect("validated dictionary admission");
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
            self.context
                .learn_keys(prepared.context_keys, admission.surface);
            self.tokens
                .learn_normalized(prepared.tokens, admission.surface);
            self.partials
                .learn_fragments(admission.surface, prepared.fragments);
        }
        #[cfg(feature = "recent-cache")]
        self.cache.observe(
            observation.stream,
            self.clock,
            history.last().copied(),
            admission.template,
        );
        self.corrections.observe(
            observation.stream,
            &observation.item,
            observation.position.0,
            observation
                .outcome
                .iter()
                .filter_map(Feature::quality)
                .any(|quality| quality <= 0.0),
        );
        self.streams.advance(
            observation.stream,
            admission.template,
            observation.position,
            self.config.max_order,
            self.clock,
        );
    }
}

fn validate_item(item: &Item, field: &'static str, limit: usize) -> Result<(), InputError> {
    validate_string(field, &item.namespace, limit)?;
    validate_string(field, &item.value, limit)
}

fn validate_features(
    features: &[Feature],
    field: &'static str,
    limit: usize,
) -> Result<(), InputError> {
    for feature in features {
        match feature {
            Feature::Categorical { name, value } => {
                validate_string(field, name, limit)?;
                validate_string(field, value, limit)?;
            }
            Feature::Numeric { name, .. } => validate_string(field, name, limit)?,
        }
    }
    Ok(())
}

#[cfg(feature = "surface-indexes")]
fn validate_strings<'a, I>(values: I, field: &'static str, limit: usize) -> Result<(), InputError>
where
    I: IntoIterator<Item = &'a str>,
{
    for value in values {
        validate_string(field, value, limit)?;
    }
    Ok(())
}

fn validate_string(field: &'static str, value: &str, limit: usize) -> Result<(), InputError> {
    if value.len() > limit {
        return Err(InputError::StringTooLong {
            field,
            bytes: value.len(),
            limit,
        });
    }
    Ok(())
}
