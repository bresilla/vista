use super::*;

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
                identity
                    .observe(observation.clone())
                    .expect("evaluation observation rejected");
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
            predictor
                .observe(observation.clone())
                .expect("evaluation observation rejected");
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
    if let Err(error) = predictor.write_snapshot(&mut snapshot) {
        metrics.snapshot = SnapshotMeasurement::Failed {
            stage: SnapshotStage::Write,
            error: snapshot_error(&error),
        };
        return;
    }
    let started = Instant::now();
    match Predictor::read_snapshot(
        config,
        normalizer,
        WhitespaceTokenizer,
        ContainsMatcher,
        snapshot.as_slice(),
    ) {
        Ok(_) => {
            metrics.snapshot = SnapshotMeasurement::Success {
                bytes: snapshot.len(),
                load_time: started.elapsed(),
            };
        }
        Err(error) => {
            metrics.snapshot = SnapshotMeasurement::Failed {
                stage: SnapshotStage::Read,
                error: snapshot_error(&error),
            };
        }
    }
}

fn snapshot_error(error: &crate::SnapshotError) -> String {
    match error {
        crate::SnapshotError::Io(_) => "I/O error",
        crate::SnapshotError::InvalidMagic => "invalid magic",
        crate::SnapshotError::UnsupportedVersion(_) => "unsupported version",
        crate::SnapshotError::UnsupportedFeatures(_) => "unsupported features",
        crate::SnapshotError::IncompatibleConfig => "incompatible configuration",
        crate::SnapshotError::Corrupt(_) => "corrupt snapshot",
        crate::SnapshotError::LimitExceeded(_) => "snapshot limit exceeded",
        crate::SnapshotError::ChecksumMismatch => "checksum mismatch",
        crate::SnapshotError::TrailingData => "trailing data",
    }
    .into()
}
