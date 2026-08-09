use super::*;

fn snapshot(predictor: &Predictor) -> Vec<u8> {
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    bytes
}

#[test]
fn oversized_raw_input_is_rejected_transactionally() {
    let config = Config {
        max_string_bytes: 8,
        ..Config::default()
    };
    let mut predictor = Predictor::new(config);
    predictor.observe(observation(1, 1, "accepted")).unwrap();
    let before_stats = predictor.stats();
    let before_predictions = predictor.predict(&query(1, 2, 10));
    let before_snapshot = snapshot(&predictor);

    assert!(matches!(
        predictor.observe(observation(1, 2, "too-long-value")),
        Err(vista::InputError::StringTooLong {
            field: "raw item",
            ..
        })
    ));
    assert_eq!(predictor.stats(), before_stats);
    assert_eq!(predictor.predict(&query(1, 2, 10)), before_predictions);
    assert_eq!(snapshot(&predictor), before_snapshot);
}

#[derive(Clone, Copy)]
struct OversizedNormalizer;

impl Normalizer for OversizedNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        NormalizedItem {
            template: Item::new(raw.namespace.clone(), "derived-template"),
            slots: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct OversizedTokenizer;

impl Tokenizer for OversizedTokenizer {
    fn tokens(&self, _: &Item) -> Vec<String> {
        vec!["derived-token".into()]
    }
}

#[test]
fn oversized_normalizer_tokenizer_and_context_outputs_are_rejected() {
    let config = Config {
        max_string_bytes: 10,
        ..Config::default()
    };
    let mut normalized = Predictor::builder(config.clone())
        .normalizer(OversizedNormalizer)
        .build();
    assert!(matches!(
        normalized.observe(observation(1, 1, "raw")),
        Err(vista::InputError::StringTooLong {
            field: "normalized template",
            ..
        })
    ));

    let mut tokenized = Predictor::builder(config.clone())
        .tokenizer(OversizedTokenizer)
        .build();
    assert!(matches!(
        tokenized.observe(observation(1, 1, "raw")),
        Err(vista::InputError::StringTooLong { field: "token", .. })
    ));

    let mut contextual = Predictor::new(config);
    let mut event = observation(1, 1, "raw");
    event.context.push(Feature::categorical("aaaaa", "bbbbb"));
    assert!(matches!(
        contextual.observe(event),
        Err(vista::InputError::StringTooLong {
            field: "context key",
            ..
        })
    ));
}

#[test]
fn retained_string_budget_rejects_without_mutation() {
    let config = Config {
        max_retained_string_bytes: 1,
        ..Config::default()
    };
    let mut predictor = Predictor::new(config);
    assert!(matches!(
        predictor.observe(observation(1, 1, "a")),
        Err(vista::InputError::RetainedStringBytesExceeded { limit: 1, .. })
    ));
    assert_eq!(predictor.stats(), vista::ModelStats::default());
}

#[test]
fn eviction_and_forgetting_release_retained_string_bytes() {
    let config = Config {
        max_templates: 1,
        max_surfaces: 1,
        ..Config::default()
    };
    let mut predictor = Predictor::new(config.clone());
    predictor
        .observe(observation(1, 1, "first-long-value"))
        .unwrap();
    predictor.observe(observation(1, 2, "b")).unwrap();

    let mut only_b = Predictor::new(config);
    only_b.observe(observation(1, 1, "b")).unwrap();
    assert_eq!(
        predictor.stats().retained_string_bytes,
        only_b.stats().retained_string_bytes
    );

    predictor.forget(&|_: &Item| true);
    assert_eq!(predictor.stats().retained_string_bytes, 0);
}

#[test]
fn failed_replay_restores_the_previous_model() {
    let config = Config {
        max_string_bytes: 8,
        ..Config::default()
    };
    let mut predictor = Predictor::new(config);
    predictor.observe(observation(1, 1, "kept")).unwrap();
    let before = snapshot(&predictor);

    assert!(
        predictor
            .replay([
                observation(2, 1, "valid"),
                observation(2, 2, "too-long-value"),
            ])
            .is_err()
    );
    assert_eq!(snapshot(&predictor), before);
}

#[test]
fn snapshot_version_one_and_total_overflow_are_rejected() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "value")).unwrap();
    let mut bytes = snapshot(&predictor);
    bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
    assert!(matches!(
        Predictor::read_snapshot(
            Config::default(),
            IdentityNormalizer,
            WhitespaceTokenizer,
            vista::ContainsMatcher,
            bytes.as_slice(),
        ),
        Err(vista::SnapshotError::UnsupportedVersion(1))
    ));

    let tiny_snapshot = Config {
        max_snapshot_bytes: 64,
        ..Config::default()
    };
    let predictor = Predictor::new(tiny_snapshot);
    assert!(matches!(
        predictor.write_snapshot(Vec::new()),
        Err(vista::SnapshotError::LimitExceeded("total bytes"))
    ));
}
