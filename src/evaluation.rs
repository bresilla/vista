use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::{
    Config, ContainsMatcher, IdentityNormalizer, Item, Normalizer, Observation, Predictor, Query,
    StreamId, WhitespaceTokenizer,
};

const COLD_START_OBSERVATIONS: u64 = 20;
const LOG_FLOOR: f64 = 1.0e-300;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Baseline {
    MostRecent,
    MostFrequent,
    ContextFrequency,
    FixedOrder1,
    FixedOrder3,
    FixedOrder5,
    LongestContext8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvaluationMetrics {
    pub observations: u64,
    pub predictions: u64,
    pub top_1_accuracy: f64,
    pub top_3_accuracy: f64,
    pub top_5_accuracy: f64,
    pub top_10_accuracy: f64,
    pub mean_reciprocal_rank: f64,
    pub candidate_recall: f64,
    pub coverage: f64,
    pub mean_log_loss: f64,
    pub perplexity: f64,
    pub cold_start_accuracy: f64,
    pub cold_start_log_loss: f64,
    pub macro_stream_accuracy: f64,
    pub mean_context_depth: f64,
    pub max_context_depth: usize,
    pub mean_prediction_latency: Duration,
    pub mean_update_latency: Duration,
    pub p50_prediction_latency: Duration,
    pub p95_prediction_latency: Duration,
    pub p99_prediction_latency: Duration,
    pub p50_update_latency: Duration,
    pub p95_update_latency: Duration,
    pub p99_update_latency: Duration,
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
    pub estimated_heap_bytes: usize,
    pub snapshot_bytes: usize,
    pub snapshot_load_time: Duration,
    pub normalization_ratio: f64,
    pub completion_saved_characters: u64,
    pub mean_saved_characters: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvaluationReport {
    pub variable_order: EvaluationMetrics,
    pub identity_normalization: Option<EvaluationMetrics>,
    pub baselines: BTreeMap<Baseline, EvaluationMetrics>,
}

struct Accumulator {
    observations: u64,
    predictions: u64,
    top_1: u64,
    top_3: u64,
    top_5: u64,
    top_10: u64,
    reciprocal_rank: f64,
    recalled: u64,
    log_loss: f64,
    cold: u64,
    cold_correct: u64,
    cold_log_loss: f64,
    stream_hits: BTreeMap<StreamId, (u64, u64)>,
    depth_total: u64,
    max_depth: usize,
    prediction_time: Duration,
    update_time: Duration,
    latencies: LatencyHistogram,
    update_latencies: LatencyHistogram,
    saved_characters: u64,
}

impl Default for Accumulator {
    fn default() -> Self {
        Self {
            observations: 0,
            predictions: 0,
            top_1: 0,
            top_3: 0,
            top_5: 0,
            top_10: 0,
            reciprocal_rank: 0.0,
            recalled: 0,
            log_loss: 0.0,
            cold: 0,
            cold_correct: 0,
            cold_log_loss: 0.0,
            stream_hits: BTreeMap::new(),
            depth_total: 0,
            max_depth: 0,
            prediction_time: Duration::ZERO,
            update_time: Duration::ZERO,
            latencies: LatencyHistogram::default(),
            update_latencies: LatencyHistogram::default(),
            saved_characters: 0,
        }
    }
}

struct LatencyHistogram {
    buckets: [u64; 65],
    samples: u64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self {
            buckets: [0; 65],
            samples: 0,
        }
    }
}

impl LatencyHistogram {
    fn record(&mut self, elapsed: Duration) {
        let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        let bucket = if nanos == 0 {
            0
        } else {
            nanos.ilog2() as usize + 1
        };
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.samples = self.samples.saturating_add(1);
    }

    fn percentile(&self, percentile: f64) -> Duration {
        if self.samples == 0 {
            return Duration::ZERO;
        }
        let target = (self.samples as f64 * percentile).ceil().max(1.0) as u64;
        let mut cumulative = 0_u64;
        for (bucket, count) in self.buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(*count);
            if cumulative >= target {
                let nanos = match bucket {
                    0 => 0,
                    64 => u64::MAX,
                    _ => (1_u64 << bucket) - 1,
                };
                return Duration::from_nanos(nanos);
            }
        }
        Duration::from_nanos(u64::MAX)
    }
}

impl Accumulator {
    fn record(
        &mut self,
        ranked: &[Item],
        actual: &Item,
        probability: f64,
        cold: bool,
        stream: StreamId,
        depth: usize,
    ) {
        self.observations += 1;
        if !ranked.is_empty() {
            self.predictions += 1;
        }
        let stream_entry = self.stream_hits.entry(stream).or_default();
        stream_entry.1 += 1;
        if let Some(index) = ranked.iter().position(|item| item == actual) {
            self.recalled += 1;
            self.reciprocal_rank += 1.0 / (index + 1) as f64;
            self.top_1 += u64::from(index < 1);
            self.top_3 += u64::from(index < 3);
            self.top_5 += u64::from(index < 5);
            self.top_10 += u64::from(index < 10);
            if index == 0 {
                stream_entry.0 += 1;
                if cold {
                    self.cold_correct += 1;
                }
            }
        }
        let loss = -probability.max(LOG_FLOOR).ln();
        self.log_loss += loss;
        if cold {
            self.cold += 1;
            self.cold_log_loss += loss;
        }
        self.depth_total += depth as u64;
        self.max_depth = self.max_depth.max(depth);
    }

    fn finish(self, predictor: Option<&Predictor>) -> EvaluationMetrics {
        let denominator = self.observations.max(1) as f64;
        let macro_stream_accuracy = if self.stream_hits.is_empty() {
            0.0
        } else {
            self.stream_hits
                .values()
                .map(|(hits, total)| *hits as f64 / (*total).max(1) as f64)
                .sum::<f64>()
                / self.stream_hits.len() as f64
        };
        let stats = predictor.map(Predictor::stats).unwrap_or_default();
        let log_loss = self.log_loss / denominator;
        EvaluationMetrics {
            observations: self.observations,
            predictions: self.predictions,
            top_1_accuracy: self.top_1 as f64 / denominator,
            top_3_accuracy: self.top_3 as f64 / denominator,
            top_5_accuracy: self.top_5 as f64 / denominator,
            top_10_accuracy: self.top_10 as f64 / denominator,
            mean_reciprocal_rank: self.reciprocal_rank / denominator,
            candidate_recall: self.recalled as f64 / denominator,
            coverage: self.predictions as f64 / denominator,
            mean_log_loss: log_loss,
            perplexity: log_loss.exp(),
            cold_start_accuracy: self.cold_correct as f64 / self.cold.max(1) as f64,
            cold_start_log_loss: self.cold_log_loss / self.cold.max(1) as f64,
            macro_stream_accuracy,
            mean_context_depth: self.depth_total as f64 / denominator,
            max_context_depth: self.max_depth,
            mean_prediction_latency: mean_duration(self.prediction_time, self.observations),
            mean_update_latency: mean_duration(self.update_time, self.observations),
            p50_prediction_latency: self.latencies.percentile(0.50),
            p95_prediction_latency: self.latencies.percentile(0.95),
            p99_prediction_latency: self.latencies.percentile(0.99),
            p50_update_latency: self.update_latencies.percentile(0.50),
            p95_update_latency: self.update_latencies.percentile(0.95),
            p99_update_latency: self.update_latencies.percentile(0.99),
            templates: stats.templates,
            surfaces: stats.surfaces,
            streams: stats.streams,
            contexts: stats.contexts,
            followers: stats.followers,
            zero_order_entries: stats.zero_order_entries,
            cache_entries: stats.cache_entries,
            stream_history_entries: stats.stream_history_entries,
            context_associations: stats.context_associations,
            tokens: stats.tokens,
            token_associations: stats.token_associations,
            partial_associations: stats.partial_associations,
            estimated_heap_bytes: stats.estimated_heap_bytes,
            snapshot_bytes: 0,
            snapshot_load_time: Duration::ZERO,
            normalization_ratio: stats.surfaces as f64 / stats.templates.max(1) as f64,
            completion_saved_characters: self.saved_characters,
            mean_saved_characters: self.saved_characters as f64 / denominator,
        }
    }
}

pub struct Evaluation;

impl Evaluation {
    pub fn run<I>(config: Config, observations: I) -> EvaluationReport
    where
        I: IntoIterator<Item = Observation>,
    {
        let mut observations: Vec<_> = observations.into_iter().collect();
        observations.sort_by_key(|observation| {
            (
                observation.timestamp,
                observation.stream,
                observation.position,
            )
        });
        Self::run_ordered(config, observations)
    }

    pub fn run_ordered<I>(config: Config, observations: I) -> EvaluationReport
    where
        I: IntoIterator<Item = Observation>,
    {
        let predictor = Predictor::new(config.clone());
        Self::run_predictors(config, predictor, None, IdentityNormalizer, observations)
    }

    pub fn run_ordered_with_normalizer<I, N>(
        config: Config,
        observations: I,
        normalizer: N,
    ) -> EvaluationReport
    where
        I: IntoIterator<Item = Observation>,
        N: Clone + Normalizer + 'static,
    {
        let predictor = Predictor::builder(config.clone())
            .normalizer(normalizer.clone())
            .build();
        let identity = Predictor::new(config.clone());
        Self::run_predictors(config, predictor, Some(identity), normalizer, observations)
    }

    fn run_predictors<I, N>(
        config: Config,
        mut predictor: Predictor,
        mut identity: Option<Predictor>,
        restore_normalizer: N,
        observations: I,
    ) -> EvaluationReport
    where
        I: IntoIterator<Item = Observation>,
        N: Normalizer + 'static,
    {
        let limit = config.max_candidates.max(10);
        let restore_config = config.clone();
        let mut model = Accumulator::default();
        let mut identity_metrics = Accumulator::default();
        let mut baseline_state = BaselineState::default();
        let mut baselines: BTreeMap<_, _> = [
            Baseline::MostRecent,
            Baseline::MostFrequent,
            Baseline::ContextFrequency,
            Baseline::FixedOrder1,
            Baseline::FixedOrder3,
            Baseline::FixedOrder5,
            Baseline::LongestContext8,
        ]
        .into_iter()
        .map(|baseline| (baseline, Accumulator::default()))
        .collect();

        for observation in observations {
            let cold = predictor.stats().observations < COLD_START_OBSERVATIONS;
            let query = Query {
                stream: observation.stream,
                position: observation.position,
                context: observation.context.clone(),
                partial: None,
                limit,
            };
            let started = Instant::now();
            let predictions = predictor.predict(&query);
            let elapsed = started.elapsed();
            model.prediction_time += elapsed;
            model.latencies.record(elapsed);
            let ranked: Vec<_> = predictions
                .iter()
                .map(|prediction| prediction.item.clone())
                .collect();
            let probability = predictor.probability_of(&query, &observation.item);
            let depth = predictions
                .first()
                .map(|prediction| prediction.context_depth)
                .unwrap_or(0);
            model.record(
                &ranked,
                &observation.item,
                probability,
                cold,
                observation.stream,
                depth,
            );
            model.saved_characters += completion_savings(&predictor, &query, &observation.item, 1);
            if let Some(identity) = &mut identity {
                let started = Instant::now();
                let predictions = identity.predict(&query);
                let elapsed = started.elapsed();
                identity_metrics.prediction_time += elapsed;
                identity_metrics.latencies.record(elapsed);
                let ranked: Vec<_> = predictions
                    .iter()
                    .map(|prediction| prediction.item.clone())
                    .collect();
                identity_metrics.record(
                    &ranked,
                    &observation.item,
                    identity.probability_of(&query, &observation.item),
                    cold,
                    observation.stream,
                    0,
                );
                let started = Instant::now();
                identity.observe(observation.clone());
                let elapsed = started.elapsed();
                identity_metrics.update_time += elapsed;
                identity_metrics.update_latencies.record(elapsed);
            }
            for (kind, metrics) in &mut baselines {
                let ranked = baseline_state.predict(*kind, &observation, limit);
                let probability = baseline_state.probability(*kind, &observation);
                metrics.record(
                    &ranked,
                    &observation.item,
                    probability,
                    cold,
                    observation.stream,
                    0,
                );
            }
            let started = Instant::now();
            predictor.observe(observation.clone());
            let elapsed = started.elapsed();
            model.update_time += elapsed;
            model.update_latencies.record(elapsed);
            baseline_state.observe(&observation);
        }
        let mut variable_order = model.finish(Some(&predictor));
        measure_snapshot(
            &mut variable_order,
            &predictor,
            restore_config.clone(),
            restore_normalizer,
        );
        let identity_normalization = identity.as_ref().map(|predictor| {
            let mut metrics = identity_metrics.finish(Some(predictor));
            measure_snapshot(&mut metrics, predictor, restore_config, IdentityNormalizer);
            metrics
        });
        EvaluationReport {
            variable_order,
            identity_normalization,
            baselines: baselines
                .into_iter()
                .map(|(kind, metrics)| (kind, metrics.finish(None)))
                .collect(),
        }
    }
}

fn measure_snapshot<N>(
    metrics: &mut EvaluationMetrics,
    predictor: &Predictor,
    config: Config,
    normalizer: N,
) where
    N: Normalizer + 'static,
{
    let mut snapshot = Vec::new();
    if predictor.write_snapshot(&mut snapshot).is_err() {
        return;
    }
    metrics.snapshot_bytes = snapshot.len();
    let started = Instant::now();
    if Predictor::read_snapshot(
        config,
        normalizer,
        WhitespaceTokenizer,
        ContainsMatcher,
        snapshot.as_slice(),
    )
    .is_ok()
    {
        metrics.snapshot_load_time = started.elapsed();
    }
}

#[derive(Default)]
struct BaselineState {
    frequencies: BTreeMap<Item, u64>,
    streams: BTreeMap<StreamId, (u64, VecDeque<Item>)>,
    transitions: BTreeMap<Vec<Item>, BTreeMap<Item, u64>>,
    contexts: BTreeMap<String, BTreeMap<Item, u64>>,
}

impl BaselineState {
    fn probability(&self, baseline: Baseline, observation: &Observation) -> f64 {
        let scores = self.scores(baseline, observation);
        let selected = scores.get(&observation.item).copied().unwrap_or(0) as f64;
        let total = scores.values().copied().fold(0_u64, u64::saturating_add) as f64;
        let vocabulary = self.frequencies.len() as f64;
        let denominator = total + 0.5 * (vocabulary + 1.0);
        if denominator > 0.0 {
            ((selected + 0.5) / denominator).max(LOG_FLOOR)
        } else {
            1.0
        }
    }

    fn predict(&self, baseline: Baseline, observation: &Observation, limit: usize) -> Vec<Item> {
        let mut ranked: Vec<_> = self.scores(baseline, observation).into_iter().collect();
        ranked.sort_by(|(a_item, a), (b_item, b)| b.cmp(a).then_with(|| a_item.cmp(b_item)));
        ranked
            .into_iter()
            .take(limit)
            .map(|(item, _)| item)
            .collect()
    }

    fn scores(&self, baseline: Baseline, observation: &Observation) -> BTreeMap<Item, u64> {
        match baseline {
            Baseline::MostRecent => self
                .streams
                .get(&observation.stream)
                .filter(|(position, _)| position.checked_add(1) == Some(observation.position.0))
                .and_then(|(_, history)| history.back())
                .cloned()
                .map(|item| BTreeMap::from([(item, 1)]))
                .unwrap_or_default(),
            Baseline::MostFrequent => self.frequencies.clone(),
            Baseline::ContextFrequency => {
                let mut scores = BTreeMap::<Item, u64>::new();
                for key in observation.context.iter().map(|feature| feature.key()) {
                    if let Some(items) = self.contexts.get(&key) {
                        for (item, count) in items {
                            let score = scores.entry(item.clone()).or_default();
                            *score = score.saturating_add(*count);
                        }
                    }
                }
                scores
            }
            Baseline::FixedOrder1
            | Baseline::FixedOrder3
            | Baseline::FixedOrder5
            | Baseline::LongestContext8 => {
                let requested_depth = match baseline {
                    Baseline::FixedOrder1 => 1,
                    Baseline::FixedOrder3 => 3,
                    Baseline::FixedOrder5 => 5,
                    _ => 8,
                };
                let Some((position, history)) = self.streams.get(&observation.stream) else {
                    return BTreeMap::new();
                };
                if position.checked_add(1) != Some(observation.position.0) {
                    return BTreeMap::new();
                }
                let maximum = requested_depth.min(history.len());
                if baseline == Baseline::LongestContext8 {
                    (1..=maximum)
                        .rev()
                        .find_map(|depth| {
                            self.transitions
                                .get(
                                    &history
                                        .iter()
                                        .skip(history.len() - depth)
                                        .cloned()
                                        .collect::<Vec<_>>(),
                                )
                                .cloned()
                        })
                        .unwrap_or_default()
                } else {
                    self.transitions
                        .get(
                            &history
                                .iter()
                                .skip(history.len() - maximum)
                                .cloned()
                                .collect::<Vec<_>>(),
                        )
                        .cloned()
                        .unwrap_or_default()
                }
            }
        }
    }

    fn observe(&mut self, observation: &Observation) {
        let frequency = self
            .frequencies
            .entry(observation.item.clone())
            .or_default();
        *frequency = frequency.saturating_add(1);
        for key in observation.context.iter().map(|feature| feature.key()) {
            let count = self
                .contexts
                .entry(key)
                .or_default()
                .entry(observation.item.clone())
                .or_default();
            *count = count.saturating_add(1);
        }
        let stream = self.streams.entry(observation.stream).or_default();
        let continuous =
            stream.0.checked_add(1) == Some(observation.position.0) && !stream.1.is_empty();
        if !continuous {
            stream.1.clear();
        }
        if continuous {
            for depth in 1..=stream.1.len().min(8) {
                let state = stream
                    .1
                    .iter()
                    .skip(stream.1.len() - depth)
                    .cloned()
                    .collect::<Vec<_>>();
                let count = self
                    .transitions
                    .entry(state)
                    .or_default()
                    .entry(observation.item.clone())
                    .or_default();
                *count = count.saturating_add(1);
            }
        }
        stream.0 = observation.position.0;
        stream.1.push_back(observation.item.clone());
        while stream.1.len() > 8 {
            stream.1.pop_front();
        }
    }
}

fn completion_savings(
    predictor: &Predictor,
    query: &Query,
    actual: &Item,
    acceptance_cost: usize,
) -> u64 {
    let chars: Vec<_> = actual.value.chars().collect();
    for length in 0..=chars.len() {
        let mut partial_query = query.clone();
        partial_query.partial = Some(chars[..length].iter().collect());
        if predictor
            .predict(&partial_query)
            .first()
            .is_some_and(|prediction| &prediction.item == actual)
        {
            return chars
                .len()
                .saturating_sub(length)
                .saturating_sub(acceptance_cost) as u64;
        }
    }
    0
}

fn mean_duration(total: Duration, observations: u64) -> Duration {
    Duration::from_secs_f64(total.as_secs_f64() / observations.max(1) as f64)
}
