use super::*;

#[test]
fn evaluation_is_chronological_and_reports_all_baselines() {
    let report = Evaluation::run(
        Config::default(),
        [
            observation(1, 2, "b"),
            observation(1, 1, "a"),
            observation(1, 3, "a"),
            observation(1, 4, "b"),
        ],
    );
    assert_eq!(report.variable_order.observations, 4);
    assert!(report.variable_order.candidate_recall >= report.variable_order.top_5_accuracy);
    assert!(report.variable_order.mean_log_loss.is_finite());
    assert!(report.variable_order.cold_start_log_loss.is_finite());
    assert!(report.variable_order.p99_update_latency >= report.variable_order.p50_update_latency);
    assert!(report.baselines.contains_key(&Baseline::FixedOrder5));
    assert!(report.baselines.contains_key(&Baseline::LongestContext8));
    assert!(
        report.variable_order.top_5_accuracy
            >= report.baselines[&Baseline::FixedOrder3].top_5_accuracy
    );
    assert!(
        report.variable_order.mean_log_loss
            < report.baselines[&Baseline::MostFrequent].mean_log_loss
    );
}

#[test]
fn evaluation_respects_gaps_and_reports_macro_stream_accuracy() {
    let mut observations = Vec::new();
    for position in 1..=20 {
        observations.push(observation(
            1,
            position,
            if position % 2 == 0 { "b" } else { "a" },
        ));
    }
    observations.push(observation(2, 1, "unique"));
    observations.push(observation(2, 3, "after-gap"));
    let report = Evaluation::run_ordered(Config::default(), observations);
    assert_ne!(
        report.variable_order.top_1_accuracy,
        report.variable_order.macro_stream_accuracy
    );
    assert!(
        report.variable_order.mean_context_depth <= report.variable_order.max_context_depth as f64
    );
}

#[test]
fn completion_savings_counts_unicode_scalars() {
    let corpus = include_str!("../fixtures/workflow.txt");
    let observations = corpus
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
        .map(|(index, line)| observation(1, index as u64 + 1, line));
    let report = Evaluation::run_ordered(Config::default(), observations);
    assert!(report.variable_order.completion_saved_characters > 0);
    assert!(report.variable_order.mean_saved_characters.is_finite());
}

#[test]
fn evaluation_compares_template_normalization_with_identity() {
    let observations: Vec<_> = (1..=20)
        .map(|position| {
            observation(
                1,
                position,
                if position % 2 == 0 {
                    "ssh alice@host1"
                } else {
                    "ssh bob@host2"
                },
            )
        })
        .collect();
    let report =
        Evaluation::run_ordered_with_normalizer(Config::default(), observations, ShellNormalizer);
    let identity = report.identity_normalization.as_ref().unwrap();
    assert!(report.variable_order.normalization_ratio > 1.0);
    assert!(report.variable_order.estimated_heap_bytes < identity.estimated_heap_bytes);
    assert!(matches!(
        &report.variable_order.snapshot,
        vista::SnapshotMeasurement::Success { bytes, .. } if *bytes > 0
    ));
    assert!(matches!(
        &identity.snapshot,
        vista::SnapshotMeasurement::Success { bytes, .. } if *bytes > 0
    ));
    assert!(matches!(
        &report.baselines[&Baseline::FixedOrder1].snapshot,
        vista::SnapshotMeasurement::NotMeasured
    ));
}

#[test]
fn evaluation_reports_snapshot_write_failure() {
    let report = Evaluation::run_ordered(
        Config {
            max_snapshot_bytes: 64,
            ..Config::default()
        },
        [observation(1, 1, "value")],
    );
    assert!(matches!(
        &report.variable_order.snapshot,
        vista::SnapshotMeasurement::Failed {
            stage: vista::SnapshotStage::Write,
            ..
        }
    ));
    assert_eq!(report.variable_order.observations, 1);
}

struct DivergentCloneNormalizer(bool);

impl Clone for DivergentCloneNormalizer {
    fn clone(&self) -> Self {
        Self(true)
    }
}

impl Normalizer for DivergentCloneNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        NormalizedItem {
            template: Item::new(
                raw.namespace.clone(),
                if self.0 { "trained" } else { "loaded" },
            ),
            slots: Vec::new(),
        }
    }
}

#[test]
fn evaluation_reports_snapshot_read_failure() {
    let report = Evaluation::run_ordered_with_normalizer(
        Config::default(),
        [observation(1, 1, "value")],
        DivergentCloneNormalizer(false),
    );
    assert!(matches!(
        &report.variable_order.snapshot,
        vista::SnapshotMeasurement::Failed {
            stage: vista::SnapshotStage::Read,
            ..
        }
    ));
}

#[test]
fn production_candidate_limits_reach_ninety_nine_percent_recall() {
    let observations = (1..=1_000).map(|position| {
        observation(
            1,
            position,
            match position % 4 {
                0 => "push",
                1 => "status",
                2 => "add",
                _ => "commit",
            },
        )
    });
    let report = Evaluation::run_ordered(Config::default(), observations);
    assert!(report.variable_order.candidate_recall >= 0.99);
    assert!(
        report.variable_order.mean_log_loss
            < report.baselines[&Baseline::FixedOrder1].mean_log_loss
    );
}

#[test]
fn context_and_outcomes_adjust_surface_ranking() {
    let mut predictor = Predictor::new(Config::default());
    for stream in 1..=4 {
        let mut alpha = observation(stream, 1, "deploy alpha");
        alpha.context.push(Feature::categorical("project", "alpha"));
        alpha.outcome.push(Feature::categorical("success", "true"));
        predictor.observe(alpha).unwrap();
        let mut beta = observation(stream + 10, 1, "deploy beta");
        beta.context.push(Feature::categorical("project", "beta"));
        beta.outcome.push(Feature::categorical("success", "false"));
        predictor.observe(beta).unwrap();
    }
    let mut next = query(99, 1, 5);
    next.context.push(Feature::categorical("project", "alpha"));
    assert_eq!(predictor.predict(&next)[0].item, item("deploy alpha"));
}

#[test]
fn invalid_numeric_configuration_and_match_scores_stay_safe() {
    struct BrokenMatcher;
    impl CandidateMatcher for BrokenMatcher {
        fn score(&self, _: &str, _: &Item) -> Option<f64> {
            Some(f64::NAN)
        }
    }
    let mut config = Config {
        recent_cache_weight: f64::NAN,
        ..Config::default()
    };
    config.weights.context = f64::INFINITY;
    let mut predictor = Predictor::builder(config).matcher(BrokenMatcher).build();
    predictor.observe(observation(1, 1, "candidate")).unwrap();
    let mut next = query(1, 2, 10);
    next.partial = Some("can".into());
    assert!(predictor.predict(&next).is_empty());
}

#[test]
fn fifty_thousand_events_remain_bounded() {
    let config = Config {
        max_templates: 128,
        max_surfaces: 128,
        max_contexts: 2_048,
        max_followers_per_context: 16,
        max_partial_associations: 4_096,
        ..Config::default()
    };
    let mut trainer = Trainer::new(config);
    for position in 1..=50_000_u64 {
        trainer
            .observe(observation(1, position, &format!("item-{}", position % 64)))
            .unwrap();
    }
    let predictor = trainer.finish();
    let stats = predictor.stats();
    assert_eq!(stats.observations, 50_000);
    assert!(stats.templates <= 128);
    assert!(stats.contexts <= 2_048);
    assert!(!predictor.predict(&query(1, 50_001, 10)).is_empty());
}
