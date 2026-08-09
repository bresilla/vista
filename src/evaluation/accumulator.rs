use super::*;

pub(super) struct Accumulator {
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
    pub(super) prediction_time: Duration,
    pub(super) update_time: Duration,
    pub(super) latencies: LatencyHistogram,
    pub(super) update_latencies: LatencyHistogram,
    pub(super) saved_characters: u64,
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

pub(super) struct LatencyHistogram {
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
    pub(super) fn record(&mut self, elapsed: Duration) {
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
    pub(super) fn record(
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

    pub(super) fn finish(self, predictor: Option<&Predictor>) -> EvaluationMetrics {
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
            snapshot: SnapshotMeasurement::NotMeasured,
            normalization_ratio: stats.surfaces as f64 / stats.templates.max(1) as f64,
            completion_saved_characters: self.saved_characters,
            mean_saved_characters: self.saved_characters as f64 / denominator,
        }
    }
}
