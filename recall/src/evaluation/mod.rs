use std::collections::BTreeMap;
use std::time::{Duration, Instant};

mod accumulator;
mod baseline;
mod correction;
mod runner;

use accumulator::Accumulator;
use baseline::BaselineState;
pub use correction::{
    CorrectionAttempt, CorrectionEvaluation, CorrectionMetrics, CorrectionReport,
};
pub use runner::Evaluation;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotStage {
    Write,
    Read,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum SnapshotMeasurement {
    Success {
        bytes: usize,
        load_time: Duration,
    },
    Failed {
        stage: SnapshotStage,
        error: String,
    },
    #[default]
    NotMeasured,
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
    pub snapshot: SnapshotMeasurement,
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
