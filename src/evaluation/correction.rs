use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::{Config, Item, Observation, Position, Predictor, Query, StreamId};

use super::mean_duration;

/// One repair opportunity replayed against the model as it stood before it.
///
/// `intended` is `None` for a control, an item that needed no repair at all.
#[derive(Clone, Debug, PartialEq)]
pub struct CorrectionAttempt {
    pub stream: StreamId,
    pub position: Position,
    pub typed: Item,
    pub intended: Option<Item>,
}

impl CorrectionAttempt {
    pub fn repair(stream: StreamId, position: Position, typed: Item, intended: Item) -> Self {
        Self {
            stream,
            position,
            typed,
            intended: Some(intended),
        }
    }

    pub fn control(stream: StreamId, position: Position, typed: Item) -> Self {
        Self {
            stream,
            position,
            typed,
            intended: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CorrectionMetrics {
    pub opportunities: u64,
    pub controls: u64,
    pub suggestions: u64,
    /// Correct repairs over repairs offered.
    pub precision: f64,
    /// Correct repairs over attempts that needed one.
    pub recall: f64,
    pub top_1_accuracy: f64,
    pub top_3_accuracy: f64,
    /// Repairs offered for an item that was already correct.
    pub false_positive_rate: f64,
    /// Attempts left untouched, correctly or not.
    pub abstention_rate: f64,
    pub mean_iterations: f64,
    pub mean_latency: Duration,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CorrectionReport {
    pub metrics: CorrectionMetrics,
}

#[derive(Default)]
struct Tally {
    opportunities: u64,
    controls: u64,
    suggestions: u64,
    correct: u64,
    top_3: u64,
    false_positives: u64,
    abstentions: u64,
    iterations: u64,
    latency: Duration,
    attempts: u64,
}

impl Tally {
    fn record(&mut self, attempt: &CorrectionAttempt, ranked: &[Item], iterations: u64) {
        self.attempts += 1;
        self.iterations += iterations;
        match &attempt.intended {
            Some(intended) => {
                self.opportunities += 1;
                if ranked.is_empty() {
                    self.abstentions += 1;
                    return;
                }
                self.suggestions += 1;
                if ranked[0] == *intended {
                    self.correct += 1;
                }
                if ranked.iter().take(3).any(|item| item == intended) {
                    self.top_3 += 1;
                }
            }
            None => {
                self.controls += 1;
                if ranked.is_empty() {
                    self.abstentions += 1;
                } else {
                    self.suggestions += 1;
                    self.false_positives += 1;
                }
            }
        }
    }

    fn finish(self) -> CorrectionMetrics {
        let opportunities = self.opportunities.max(1) as f64;
        CorrectionMetrics {
            opportunities: self.opportunities,
            controls: self.controls,
            suggestions: self.suggestions,
            precision: self.correct as f64 / self.suggestions.max(1) as f64,
            recall: self.correct as f64 / opportunities,
            top_1_accuracy: self.correct as f64 / opportunities,
            top_3_accuracy: self.top_3 as f64 / opportunities,
            false_positive_rate: self.false_positives as f64 / self.controls.max(1) as f64,
            abstention_rate: self.abstentions as f64 / self.attempts.max(1) as f64,
            mean_iterations: self.iterations as f64 / self.attempts.max(1) as f64,
            mean_latency: mean_duration(self.latency, self.attempts),
        }
    }
}

/// Chronological repair evaluator: replay history, repair, score, then learn.
pub struct CorrectionEvaluation;

impl CorrectionEvaluation {
    pub fn run<I, A>(config: Config, observations: I, attempts: A) -> CorrectionReport
    where
        I: IntoIterator<Item = Observation>,
        A: IntoIterator<Item = CorrectionAttempt>,
    {
        let mut observations: Vec<_> = observations.into_iter().collect();
        observations.sort_by_key(|observation| {
            (
                observation.timestamp,
                observation.stream,
                observation.position,
            )
        });
        let mut pending: BTreeMap<(StreamId, Position), Vec<CorrectionAttempt>> = BTreeMap::new();
        for attempt in attempts {
            pending
                .entry((attempt.stream, attempt.position))
                .or_default()
                .push(attempt);
        }

        let mut predictor = Predictor::new(config.clone());
        let mut tally = Tally::default();
        for observation in observations {
            let key = (observation.stream, observation.position);
            if let Some(attempts) = pending.remove(&key) {
                Self::score(&predictor, &config, &attempts, &mut tally);
            }
            let _ = predictor.observe(observation);
        }
        for attempts in pending.into_values() {
            Self::score(&predictor, &config, &attempts, &mut tally);
        }
        CorrectionReport {
            metrics: tally.finish(),
        }
    }

    fn score(
        predictor: &Predictor,
        config: &Config,
        attempts: &[CorrectionAttempt],
        tally: &mut Tally,
    ) {
        for attempt in attempts {
            let mut query = Query::new(attempt.stream, attempt.position, config.max_candidates);
            query.limit = 3.min(config.max_candidates.max(1));
            let started = Instant::now();
            let repairs = predictor.predict_aligned(&query, &attempt.typed);
            tally.latency += started.elapsed();
            let iterations = repairs
                .first()
                .map(|prediction| prediction.repair_iterations.max(1) as u64)
                .unwrap_or(0);
            let ranked: Vec<_> = repairs
                .into_iter()
                .map(|prediction| prediction.item)
                .collect();
            tally.record(attempt, &ranked, iterations);
        }
    }
}
