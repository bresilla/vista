use std::io::Cursor;

use vista::{
    Baseline, CandidateMatcher, Config, Evaluation, Feature, IdentityNormalizer, Item,
    NormalizedItem, Normalizer, Observation, Position, Predictor, Query, ResearchExport, StreamId,
    Tokenizer, Trainer, WhitespaceTokenizer,
};

fn item(value: &str) -> Item {
    Item::new("sentence", value)
}

fn observation(stream: u64, position: u64, value: &str) -> Observation {
    Observation {
        item: item(value),
        stream: StreamId(stream),
        position: Position(position),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

fn query(stream: u64, position: u64, limit: usize) -> Query {
    Query::new(StreamId(stream), Position(position), limit)
}

fn values(predictions: &[vista::Prediction]) -> Vec<&str> {
    predictions
        .iter()
        .map(|prediction| prediction.item.value.as_str())
        .collect()
}

#[test]
fn sequence_prediction_learns_online() {
    let mut predictor = Predictor::new(Config::default());
    predictor.replay([
        observation(1, 1, "build"),
        observation(1, 2, "test"),
        observation(1, 3, "build"),
    ]);
    let predictions = predictor.predict(&query(1, 4, 5));
    assert_eq!(predictions[0].item, item("test"));
    assert!(predictions[0].probability > 0.0);
    assert!(predictions[0].score.is_finite());
}

#[test]
fn predictions_never_synthesize_unseen_surfaces() {
    let observed = ["build", "test", "deploy"];
    let mut predictor = Predictor::new(Config::default());
    for (index, value) in observed.into_iter().enumerate() {
        predictor.observe(observation(1, index as u64 + 1, value));
    }
    assert!(
        predictor
            .predict(&query(1, 4, 20))
            .iter()
            .all(|prediction| observed.contains(&prediction.item.value.as_str()))
    );
}

#[test]
fn contexts_deeper_than_three_disambiguate() {
    let mut predictor = Predictor::new(Config {
        max_order: 8,
        recent_cache_weight: 0.0,
        ..Config::default()
    });
    for (stream, sequence) in [
        (1, ["a", "b", "c", "d", "from-a"]),
        (2, ["q", "b", "c", "d", "from-q"]),
    ] {
        for (index, value) in sequence.into_iter().enumerate() {
            predictor.observe(observation(stream, index as u64 + 1, value));
        }
    }
    for (index, value) in ["a", "b", "c", "d"].into_iter().enumerate() {
        predictor.observe(observation(3, index as u64 + 1, value));
    }
    let prediction = &predictor.predict(&query(3, 5, 5))[0];
    assert_eq!(prediction.item, item("from-a"));
    assert_eq!(prediction.context_depth, 4);
}

#[test]
fn sparse_context_backs_off() {
    let mut predictor = Predictor::new(Config {
        recent_cache_weight: 0.0,
        ..Config::default()
    });
    for stream in 1..=5 {
        predictor.observe(observation(stream, 1, "common"));
        predictor.observe(observation(stream, 2, "next"));
    }
    predictor.observe(observation(9, 1, "unseen-prefix"));
    predictor.observe(observation(9, 2, "common"));
    let prediction = &predictor.predict(&query(9, 3, 5))[0];
    assert_eq!(prediction.item, item("next"));
    assert!(prediction.probability > 0.0);
}

#[test]
fn gaps_and_streams_do_not_create_transitions() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "first"));
    predictor.observe(observation(1, 3, "after-gap"));
    predictor.observe(observation(2, 1, "other"));
    let predictions = predictor.predict(&query(1, 2, 10));
    let after_gap = predictions
        .iter()
        .find(|prediction| prediction.item == item("after-gap"));
    assert!(after_gap.is_none_or(|prediction| {
        prediction
            .explanation
            .reasons
            .iter()
            .all(|reason| !reason.starts_with("matched sequence depth"))
    }));
}

#[test]
fn template_eviction_invalidates_the_pending_history() {
    let config = Config {
        max_templates: 2,
        max_surfaces: 2,
        recent_cache_weight: 0.0,
        ..Config::default()
    };
    let mut predictor = Predictor::new(config.clone());
    predictor.observe(observation(1, 1, "evicted"));
    predictor.observe(observation(1, 2, "retained"));
    predictor.observe(observation(1, 3, "replacement"));

    assert!(
        predictor
            .predict(&query(1, 4, 10))
            .iter()
            .all(|prediction| prediction
                .explanation
                .reasons
                .iter()
                .all(|reason| !reason.starts_with("matched sequence depth")))
    );

    let mut snapshot = Vec::new();
    predictor.write_snapshot(&mut snapshot).unwrap();
    Predictor::read_snapshot(
        config,
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        snapshot.as_slice(),
    )
    .unwrap();
}

#[test]
fn explicit_break_resets_sequence_and_cache() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "a"));
    predictor.observe(observation(1, 2, "b"));
    predictor.break_stream(StreamId(1));
    predictor.observe(observation(1, 3, "c"));
    assert!(
        predictor
            .predict(&query(1, 4, 10))
            .iter()
            .all(|prediction| {
                prediction
                    .explanation
                    .reasons
                    .iter()
                    .all(|reason| !reason.contains("depth 2"))
            })
    );
}

#[derive(Clone, Copy)]
struct ShellNormalizer;

impl Normalizer for ShellNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        if let Some(target) = raw.value.strip_prefix("ssh ") {
            NormalizedItem {
                template: Item::new(raw.namespace.clone(), "ssh {target}"),
                slots: vec![Feature::categorical("target", target)],
            }
        } else {
            NormalizedItem {
                template: raw.clone(),
                slots: Vec::new(),
            }
        }
    }
}

#[test]
fn normalization_predicts_templates_and_returns_surfaces() {
    let mut predictor = Predictor::builder(Config::default())
        .normalizer(ShellNormalizer)
        .build();
    for (position, value) in [
        "prepare",
        "ssh alice@host1",
        "prepare",
        "ssh bob@host2",
        "prepare",
    ]
    .into_iter()
    .enumerate()
    {
        predictor.observe(observation(1, position as u64 + 1, value));
    }
    let predictions = predictor.predict(&query(1, 6, 10));
    assert!(predictions[0].item.value.starts_with("ssh "));
    assert_eq!(predictions[0].template.value, "ssh {target}");
    assert!(
        predictions[0]
            .explanation
            .reasons
            .iter()
            .any(|reason| reason.starts_with("preferred historical surface"))
    );
    assert_eq!(predictor.stats().templates, 2);
    assert_eq!(predictor.stats().surfaces, 3);
}

#[test]
fn normalized_slots_select_a_contextual_surface() {
    let mut predictor = Predictor::builder(Config::default())
        .normalizer(ShellNormalizer)
        .build();
    predictor.observe(observation(1, 1, "ssh alice@host1"));
    predictor.observe(observation(1, 2, "ssh bob@host2"));
    let mut next = query(1, 3, 2);
    next.context
        .push(Feature::categorical("target", "alice@host1"));

    assert_eq!(predictor.predict(&next)[0].item, item("ssh alice@host1"));
}

#[test]
fn identity_normalizer_preserves_one_template_per_surface() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "one"));
    predictor.observe(observation(1, 2, "two"));
    assert_eq!(predictor.stats().templates, predictor.stats().surfaces);
}

#[derive(Clone, Copy)]
struct UnicodeSlotNormalizer;

impl Normalizer for UnicodeSlotNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        NormalizedItem {
            template: Item::new(raw.namespace.clone(), "templated"),
            slots: vec![
                Feature::categorical("duplicate", ""),
                Feature::categorical("duplicate", ""),
                Feature::categorical("🧪", "東京"),
            ],
        }
    }
}

#[test]
fn unicode_empty_and_duplicate_slots_are_deterministic() {
    let mut first = Predictor::builder(Config::default())
        .normalizer(UnicodeSlotNormalizer)
        .build();
    let mut second = Predictor::builder(Config::default())
        .normalizer(UnicodeSlotNormalizer)
        .build();
    let event = observation(1, 1, "surface");
    first.observe(event.clone());
    second.observe(event);
    let mut first_bytes = Vec::new();
    let mut second_bytes = Vec::new();
    first.write_snapshot(&mut first_bytes).unwrap();
    second.write_snapshot(&mut second_bytes).unwrap();
    assert_eq!(first_bytes, second_bytes);
    let restored = Predictor::read_snapshot(
        Config::default(),
        UnicodeSlotNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        first_bytes.as_slice(),
    )
    .unwrap();
    assert_eq!(restored.predict(&query(1, 2, 1))[0].item, item("surface"));
}

#[test]
fn surface_eviction_keeps_its_shared_template_valid() {
    let config = Config {
        max_templates: 1,
        max_surfaces: 1,
        ..Config::default()
    };
    let mut predictor = Predictor::builder(config.clone())
        .normalizer(ShellNormalizer)
        .build();
    predictor.observe(observation(1, 1, "ssh alice@host1"));
    predictor.observe(observation(1, 2, "ssh bob@host2"));
    assert_eq!(predictor.stats().templates, 1);
    assert_eq!(predictor.stats().surfaces, 1);
    assert_eq!(
        predictor.predict(&query(1, 3, 2))[0].item.value,
        "ssh bob@host2"
    );
    let mut snapshot = Vec::new();
    predictor.write_snapshot(&mut snapshot).unwrap();
    assert!(
        Predictor::read_snapshot(
            config,
            ShellNormalizer,
            WhitespaceTokenizer,
            vista::ContainsMatcher,
            snapshot.as_slice(),
        )
        .is_ok()
    );
}

struct PrefixMatcher;

impl CandidateMatcher for PrefixMatcher {
    fn score(&self, partial: &str, candidate: &Item) -> Option<f64> {
        candidate.value.starts_with(partial).then_some(1.0)
    }
}

#[test]
fn partial_retrieval_and_custom_matcher_filter_candidates() {
    let mut predictor = Predictor::builder(Config {
        max_candidate_templates: 1,
        ..Config::default()
    })
    .matcher(PrefixMatcher)
    .build();
    predictor.observe(observation(1, 1, "needle target"));
    for position in 1..=10 {
        predictor.observe(observation(
            2,
            position,
            if position % 2 == 0 { "alpha" } else { "beta" },
        ));
    }
    let mut next = query(9, 1, 10);
    next.partial = Some("needle".into());
    assert_eq!(predictor.predict(&next)[0].item, item("needle target"));
}

#[derive(Clone, Copy)]
struct MagicTokenizer;

impl Tokenizer for MagicTokenizer {
    fn tokens(&self, item: &Item) -> Vec<String> {
        vec![if item.value == "alpha" {
            "magic".into()
        } else {
            "ordinary".into()
        }]
    }

    fn query_tokens(&self, _: &str) -> Vec<String> {
        vec!["magic".into()]
    }
}

#[test]
fn custom_query_tokenization_drives_partial_retrieval() {
    let mut predictor = Predictor::builder(Config {
        max_candidate_templates: 1,
        max_candidates: 1,
        max_partial_associations: 1,
        ..Config::default()
    })
    .tokenizer(MagicTokenizer)
    .build();
    predictor.observe(observation(1, 1, "alpha"));
    for position in 2..=10 {
        predictor.observe(observation(1, position, "zzzz"));
    }
    let mut next = query(1, 11, 1);
    next.partial = Some("alp".into());

    assert_eq!(predictor.predict(&next)[0].item, item("alpha"));
}

#[test]
fn recent_cache_adapts_without_becoming_a_second_model() {
    let mut cached = Predictor::new(Config {
        max_order: 1,
        recent_cache_weight: 0.5,
        recent_cache_half_life: 4,
        ..Config::default()
    });
    let mut uncached = Predictor::new(Config {
        max_order: 1,
        recent_cache_weight: 0.0,
        ..Config::default()
    });
    let mut position = 0;
    for _ in 0..20 {
        position += 1;
        let hub = observation(1, position, "hub");
        cached.observe(hub.clone());
        uncached.observe(hub);
        position += 1;
        let old = observation(1, position, "old");
        cached.observe(old.clone());
        uncached.observe(old);
    }
    for _ in 0..10 {
        position += 1;
        let hub = observation(1, position, "hub");
        cached.observe(hub.clone());
        uncached.observe(hub);
        position += 1;
        let new = observation(1, position, "new");
        cached.observe(new.clone());
        uncached.observe(new);
    }
    position += 1;
    let hub = observation(1, position, "hub");
    cached.observe(hub.clone());
    uncached.observe(hub);
    assert_eq!(
        cached.predict(&query(1, position + 1, 2))[0].item,
        item("new")
    );
    assert_eq!(
        uncached.predict(&query(1, position + 1, 2))[0].item,
        item("old")
    );
}

#[test]
fn unseen_stream_uses_the_global_recent_cache() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "recent"));
    let prediction = &predictor.predict(&query(99, 1, 5))[0];
    assert!(
        prediction
            .explanation
            .reasons
            .iter()
            .any(|reason| reason.starts_with("recent-cache probability"))
    );
}

#[test]
fn global_cache_falls_back_to_unconditional_recent_items() {
    let cached_config = Config {
        recent_cache_weight: 0.5,
        recent_cache_half_life: 1,
        ..Config::default()
    };
    let uncached_config = Config {
        recent_cache_weight: 0.0,
        ..cached_config.clone()
    };
    let mut cached = Predictor::new(cached_config);
    let mut uncached = Predictor::new(uncached_config);
    for position in 1..=8 {
        let event = observation(1, position, "old");
        cached.observe(event.clone());
        uncached.observe(event);
    }
    let recent = observation(1, 9, "recent");
    cached.observe(recent.clone());
    uncached.observe(recent);
    let unseen = query(99, 1, 10);
    assert!(
        cached.probability_of(&unseen, &item("recent"))
            > uncached.probability_of(&unseen, &item("recent"))
    );
}

#[test]
fn stream_eviction_removes_its_private_cache() {
    let mut predictor = Predictor::new(Config {
        max_streams: 2,
        recent_cache_weight: 0.5,
        recent_cache_half_life: 1_000,
        ..Config::default()
    });
    predictor.observe(observation(1, 1, "a"));
    predictor.observe(observation(2, 1, "private-stream-item"));
    predictor.break_stream(StreamId(1));
    predictor.observe(observation(1, 2, "c"));
    predictor.break_stream(StreamId(1));
    predictor.observe(observation(3, 1, "d"));

    let probability = predictor.probability_of(&query(2, 2, 10), &item("private-stream-item"));
    assert!(probability < 0.4);
}

#[test]
fn probabilities_are_finite_and_bounded() {
    let mut predictor = Predictor::new(Config {
        recent_cache_weight: 0.0,
        ..Config::default()
    });
    for (position, value) in ["a", "b", "a", "c", "a"].into_iter().enumerate() {
        predictor.observe(observation(1, position as u64 + 1, value));
    }
    for value in ["a", "b", "c", "unseen"] {
        let probability = predictor.probability_of(&query(1, 6, 10), &item(value));
        assert!(probability.is_finite());
        assert!(probability > 0.0 && probability <= 1.0);
    }
}

#[test]
fn template_probabilities_and_unknown_mass_are_conserved() {
    let mut predictor = Predictor::new(Config {
        recent_cache_weight: 0.2,
        ..Config::default()
    });
    for (position, value) in ["hub", "a", "hub", "b", "hub"].into_iter().enumerate() {
        predictor.observe(observation(1, position as u64 + 1, value));
    }
    let next = query(1, 6, 10);
    let total: f64 = ["hub", "a", "b", "never-observed"]
        .into_iter()
        .map(|value| predictor.probability_of(&next, &item(value)))
        .sum();
    assert!(
        (total - 1.0).abs() < 1.0e-12,
        "probability mass was {total}"
    );
}

#[test]
fn replay_and_streaming_trainer_are_equivalent() {
    let observations = [
        observation(1, 1, "a"),
        observation(1, 2, "b"),
        observation(1, 3, "a"),
    ];
    let mut replayed = Predictor::new(Config::default());
    replayed.replay(observations.clone());
    let mut trainer = Trainer::new(Config::default());
    for observed in observations {
        trainer.observe(observed);
    }
    let trained = trainer.finish();
    assert_eq!(replayed.stats(), trained.stats());
    assert_eq!(
        replayed.predict(&query(1, 4, 10)),
        trained.predict(&query(1, 4, 10))
    );
}

#[test]
fn streaming_trainer_accepts_custom_adapters() {
    let mut trainer =
        Trainer::from_builder(Predictor::builder(Config::default()).normalizer(ShellNormalizer));
    trainer.observe(observation(1, 1, "prepare"));
    trainer.observe(observation(1, 2, "ssh alice@host1"));
    trainer.observe(observation(1, 3, "prepare"));
    let predictor = trainer.finish();
    let prediction = &predictor.predict(&query(1, 4, 1))[0];
    assert_eq!(prediction.template.value, "ssh {target}");
}

#[test]
fn forgetting_removes_surfaces_without_bridging_history() {
    let mut predictor = Predictor::new(Config::default());
    for (position, value) in [(1, "a"), (2, "private"), (3, "c")] {
        predictor.observe(observation(1, position, value));
    }
    predictor.forget(&|candidate: &Item| candidate.value == "private");
    assert!(!values(&predictor.predict(&query(1, 4, 10))).contains(&"private"));
    predictor.observe(observation(1, 4, "d"));
    predictor.observe(observation(2, 1, "a"));
    predictor.observe(observation(2, 2, "c"));
    let d = predictor
        .predict(&query(2, 3, 10))
        .into_iter()
        .find(|prediction| prediction.item == item("d"));
    assert!(d.is_none_or(|prediction| {
        prediction
            .explanation
            .reasons
            .iter()
            .all(|reason| !reason.contains("depth 2"))
    }));
}

#[test]
fn forgotten_surfaces_cannot_be_recovered_from_snapshots() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "public-before"));
    predictor.observe(observation(1, 2, "private-secret-value"));
    predictor.observe(observation(1, 3, "public-after"));
    predictor.forget(&|candidate: &Item| candidate.value == "private-secret-value");
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    assert!(
        !bytes
            .windows("private-secret-value".len())
            .any(|window| window == b"private-secret-value")
    );
    let restored = Predictor::read_snapshot(
        Config::default(),
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        bytes.as_slice(),
    )
    .unwrap();
    assert!(!values(&restored.predict(&query(1, 4, 10))).contains(&"private-secret-value"));
}

#[test]
fn every_collection_respects_configured_bounds() {
    let config = Config {
        max_templates: 16,
        max_surfaces: 20,
        max_streams: 4,
        max_contexts: 32,
        max_followers_per_context: 3,
        max_context_associations: 24,
        max_tokens: 12,
        max_partial_associations: 30,
        max_candidates: 8,
        ..Config::default()
    };
    let mut predictor = Predictor::new(config.clone());
    for index in 0..1_000_u64 {
        let mut observed = observation(index % 8, index / 8 + 1, &format!("item-{index}"));
        observed
            .context
            .push(Feature::categorical("bucket", (index % 50).to_string()));
        predictor.observe(observed);
    }
    let stats = predictor.stats();
    assert!(stats.templates <= 16);
    assert!(stats.surfaces <= 20);
    assert!(stats.streams <= 4);
    assert!(stats.contexts <= 32);
    assert!(stats.zero_order_entries <= 16);
    assert!(stats.cache_entries <= 256 * 5);
    assert!(stats.stream_history_entries <= 4 * 8);
    assert!(stats.context_associations <= 24);
    assert!(stats.tokens <= 12);
    assert!(stats.token_associations <= 12 * 8);
    assert!(stats.partial_associations <= 30);
    assert!(predictor.predict(&query(1, 200, 100)).len() <= 8);
    let mut snapshot = Vec::new();
    predictor.write_snapshot(&mut snapshot).unwrap();
    assert!(
        Predictor::read_snapshot(
            config,
            IdentityNormalizer,
            WhitespaceTokenizer,
            vista::ContainsMatcher,
            snapshot.as_slice(),
        )
        .is_ok()
    );
}

#[test]
fn ranking_and_snapshots_are_deterministic() {
    let observations = [observation(1, 1, "z"), observation(2, 1, "a")];
    let mut first = Predictor::new(Config::default());
    let mut second = Predictor::new(Config::default());
    first.replay(observations.clone());
    second.replay(observations);
    assert_eq!(
        first.predict(&query(3, 1, 10)),
        second.predict(&query(3, 1, 10))
    );
    let mut first_bytes = Vec::new();
    let mut second_bytes = Vec::new();
    first.write_snapshot(&mut first_bytes).unwrap();
    second.write_snapshot(&mut second_bytes).unwrap();
    assert_eq!(first_bytes, second_bytes);
}

#[test]
fn empty_snapshot_round_trip_is_valid() {
    let predictor = Predictor::new(Config::default());
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let restored = Predictor::read_snapshot(
        Config::default(),
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        bytes.as_slice(),
    )
    .unwrap();
    assert_eq!(restored.stats(), predictor.stats());
    assert!(restored.predict(&query(1, 1, 10)).is_empty());
}

#[test]
fn snapshot_round_trip_restores_and_continues_learning() {
    let mut original = Predictor::new(Config::default());
    original.replay([
        observation(1, 1, "build"),
        observation(1, 2, "test"),
        observation(1, 3, "build"),
    ]);
    let mut bytes = Vec::new();
    original.write_snapshot(&mut bytes).unwrap();
    let expected_bytes = bytes.clone();
    let mut restored = Predictor::read_snapshot(
        Config::default(),
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        Cursor::new(bytes),
    )
    .unwrap();
    let mut restored_bytes = Vec::new();
    restored.write_snapshot(&mut restored_bytes).unwrap();
    assert_eq!(restored_bytes, expected_bytes);
    assert_eq!(original.stats(), restored.stats());
    assert_eq!(
        original.predict(&query(1, 4, 10)),
        restored.predict(&query(1, 4, 10))
    );
    original.observe(observation(1, 4, "test"));
    restored.observe(observation(1, 4, "test"));
    assert_eq!(
        original.predict(&query(1, 5, 10)),
        restored.predict(&query(1, 5, 10))
    );
}

#[test]
fn pruned_probability_mass_survives_snapshot() {
    let config = Config {
        max_followers_per_context: 1,
        recent_cache_weight: 0.0,
        ..Config::default()
    };
    let mut predictor = Predictor::new(config.clone());
    for (stream, next) in [(1, "a"), (2, "b"), (3, "a"), (4, "c")] {
        predictor.observe(observation(stream, 1, "hub"));
        predictor.observe(observation(stream, 2, next));
    }
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let restored = Predictor::read_snapshot(
        config,
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        Cursor::new(bytes),
    )
    .unwrap();
    for candidate in ["a", "b", "c"] {
        let probability = restored.probability_of(&query(99, 1, 5), &item(candidate));
        assert!(probability.is_finite() && probability > 0.0);
    }
}

#[test]
fn corrupt_truncated_and_trailing_snapshots_are_rejected() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "a"));
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let load = |bytes: Vec<u8>| {
        Predictor::read_snapshot(
            Config::default(),
            IdentityNormalizer,
            WhitespaceTokenizer,
            vista::ContainsMatcher,
            Cursor::new(bytes),
        )
    };
    let mut corrupt = bytes.clone();
    let checksum_byte = corrupt.len() - 1;
    corrupt[checksum_byte] ^= 1;
    assert!(load(corrupt).is_err());
    let mut bit_flip = bytes.clone();
    let payload_byte = bit_flip.len() - 9;
    bit_flip[payload_byte] ^= 1;
    assert!(load(bit_flip).is_err());
    assert!(load(bytes[..bytes.len() - 1].to_vec()).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(load(trailing).is_err());
}

#[test]
fn failed_snapshot_load_leaves_existing_predictor_unchanged() {
    let mut existing = Predictor::new(Config::default());
    existing.replay([
        observation(1, 1, "build"),
        observation(1, 2, "test"),
        observation(1, 3, "build"),
    ]);
    let before_stats = existing.stats();
    let before_predictions = existing.predict(&query(1, 4, 10));
    let failed = Predictor::read_snapshot(
        Config::default(),
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        Cursor::new(b"not a snapshot"),
    );
    assert!(failed.is_err());
    assert_eq!(existing.stats(), before_stats);
    assert_eq!(existing.predict(&query(1, 4, 10)), before_predictions);
}

#[test]
fn unsupported_and_oversized_snapshots_are_rejected() {
    let predictor = Predictor::new(Config::default());
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();

    let mut unsupported = bytes.clone();
    unsupported[8..12].copy_from_slice(&99_u32.to_le_bytes());
    let load = |bytes: Vec<u8>| {
        Predictor::read_snapshot(
            Config::default(),
            IdentityNormalizer,
            WhitespaceTokenizer,
            vista::ContainsMatcher,
            Cursor::new(bytes),
        )
    };
    assert!(load(unsupported).is_err());

    let mut unsupported_features = bytes.clone();
    unsupported_features[12..20].copy_from_slice(&1_u64.to_le_bytes());
    assert!(load(unsupported_features).is_err());

    let mut offset = 8 + 4 + 8 + 8 + 20 * 8;
    for _ in 0..3 {
        let length = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8 + length;
    }
    offset += 8 + 4 + 4;
    bytes[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(load(bytes).is_err());
}

#[test]
fn overflowing_snapshot_limits_are_rejected() {
    let config = Config {
        max_tokens: usize::MAX,
        max_surface_candidates_per_template: 2,
        ..Config::default()
    };
    let predictor = Predictor::new(config.clone());
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let restored = Predictor::read_snapshot(
        config,
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        bytes.as_slice(),
    );
    assert!(restored.is_err());
}

#[test]
fn duplicate_and_dangling_snapshot_identifiers_are_rejected() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "a"));
    predictor.observe(observation(1, 2, "b"));
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let (template_ids, surface_templates) = dictionary_identifier_offsets(&bytes);
    assert!(template_ids.len() >= 2);
    assert!(!surface_templates.is_empty());

    let load = |bytes: Vec<u8>| {
        Predictor::read_snapshot(
            Config::default(),
            IdentityNormalizer,
            WhitespaceTokenizer,
            vista::ContainsMatcher,
            bytes.as_slice(),
        )
    };
    let mut duplicate = bytes.clone();
    let first_id = duplicate[template_ids[0]..template_ids[0] + 4].to_vec();
    duplicate[template_ids[1]..template_ids[1] + 4].copy_from_slice(&first_id);
    assert!(load(duplicate).is_err());

    let mut dangling = bytes;
    dangling[surface_templates[0]..surface_templates[0] + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(load(dangling).is_err());
}

#[test]
fn incompatible_snapshot_configuration_is_rejected() {
    let predictor = Predictor::new(Config::default());
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let error = Predictor::read_snapshot(
        Config {
            max_order: 4,
            ..Config::default()
        },
        IdentityNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        Cursor::new(bytes),
    );
    assert!(error.is_err());
}

#[test]
fn incompatible_snapshot_adapters_are_rejected() {
    struct OtherMatcher;
    impl CandidateMatcher for OtherMatcher {
        fn score(&self, _: &str, _: &Item) -> Option<f64> {
            Some(1.0)
        }
    }
    let predictor = Predictor::new(Config::default());
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let restored = Predictor::read_snapshot(
        Config::default(),
        IdentityNormalizer,
        WhitespaceTokenizer,
        OtherMatcher,
        Cursor::new(bytes),
    );
    assert!(restored.is_err());
}

struct SameKeyNormalizer(&'static str);

impl Normalizer for SameKeyNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        NormalizedItem {
            template: Item::new(raw.namespace.clone(), self.0),
            slots: vec![Feature::numeric("variant", self.0.len() as f32)],
        }
    }

    fn snapshot_key(&self) -> &str {
        "same-key-normalizer"
    }
}

#[test]
fn snapshot_revalidates_normalizer_output_even_when_keys_match() {
    let config = Config::default();
    let mut predictor = Predictor::builder(config.clone())
        .normalizer(SameKeyNormalizer("original"))
        .build();
    predictor.observe(observation(1, 1, "surface"));
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();

    assert!(matches!(
        Predictor::read_snapshot(
            config,
            SameKeyNormalizer("changed"),
            WhitespaceTokenizer,
            vista::ContainsMatcher,
            bytes.as_slice(),
        ),
        Err(vista::SnapshotError::IncompatibleConfig)
    ));
}

#[derive(Clone, Copy)]
struct ManySlotsNormalizer;

impl Normalizer for ManySlotsNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        NormalizedItem {
            template: raw.clone(),
            slots: (0..2_000)
                .map(|index| Feature::categorical("slot", index.to_string()))
                .collect(),
        }
    }
}

#[test]
fn normalized_slots_are_bounded_and_snapshot_compatible() {
    let config = Config::default();
    let mut predictor = Predictor::builder(config.clone())
        .normalizer(ManySlotsNormalizer)
        .build();
    predictor.observe(observation(1, 1, "surface"));
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();

    Predictor::read_snapshot(
        config,
        ManySlotsNormalizer,
        WhitespaceTokenizer,
        vista::ContainsMatcher,
        bytes.as_slice(),
    )
    .unwrap();
}

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
    let corpus = include_str!("fixtures/workflow.txt");
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
    assert!(report.variable_order.snapshot_bytes > 0);
    assert!(identity.snapshot_bytes > 0);
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
        predictor.observe(alpha);
        let mut beta = observation(stream + 10, 1, "deploy beta");
        beta.context.push(Feature::categorical("project", "beta"));
        beta.outcome.push(Feature::categorical("success", "false"));
        predictor.observe(beta);
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
    predictor.observe(observation(1, 1, "candidate"));
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
        trainer.observe(observation(1, position, &format!("item-{}", position % 64)));
    }
    let predictor = trainer.finish();
    let stats = predictor.stats();
    assert_eq!(stats.observations, 50_000);
    assert!(stats.templates <= 128);
    assert!(stats.contexts <= 2_048);
    assert!(!predictor.predict(&query(1, 50_001, 10)).is_empty());
}

#[test]
fn research_export_is_deterministic_and_preserves_gaps() {
    let observations = [
        observation(1, 1, "a"),
        observation(1, 2, "b"),
        observation(1, 4, "a"),
    ];
    let export = ResearchExport::from_observations(observations).unwrap();
    assert_eq!(export.dictionary, vec![item("a"), item("b")]);
    assert_eq!(export.sequences, vec![vec![0, 1], vec![0]]);
    let mut spmf = Vec::new();
    export.write_spmf(&mut spmf).unwrap();
    assert_eq!(String::from_utf8(spmf).unwrap(), "0 -1 1 -1 -2\n0 -1 -2\n");
}

#[test]
fn research_export_orders_interleaved_sessions_chronologically() {
    let export = ResearchExport::from_observations([
        observation(2, 1, "stream-two"),
        observation(1, 1, "stream-one"),
        observation(2, 2, "stream-two-next"),
    ])
    .unwrap();
    assert_eq!(export.sequences, vec![vec![0, 2], vec![1]]);
}

fn dictionary_identifier_offsets(bytes: &[u8]) -> (Vec<usize>, Vec<usize>) {
    fn read_u64(bytes: &[u8], offset: &mut usize) -> u64 {
        let value = u64::from_le_bytes(bytes[*offset..*offset + 8].try_into().unwrap());
        *offset += 8;
        value
    }
    fn skip_string(bytes: &[u8], offset: &mut usize) {
        let length = read_u64(bytes, offset) as usize;
        *offset += length;
    }
    fn skip_item(bytes: &[u8], offset: &mut usize) {
        skip_string(bytes, offset);
        skip_string(bytes, offset);
    }

    let mut offset = 8 + 4 + 8 + 8 + 20 * 8;
    for _ in 0..3 {
        skip_string(bytes, &mut offset);
    }
    offset += 8 + 4 + 4;
    let template_count = read_u64(bytes, &mut offset) as usize;
    let mut template_ids = Vec::new();
    for _ in 0..template_count {
        template_ids.push(offset);
        offset += 4;
        skip_item(bytes, &mut offset);
        offset += 8 * 4;
    }
    let surface_count = read_u64(bytes, &mut offset) as usize;
    let mut surface_templates = Vec::new();
    for _ in 0..surface_count {
        offset += 4;
        surface_templates.push(offset);
        offset += 4;
        skip_item(bytes, &mut offset);
        offset += 8 * 4;
        let slots = read_u64(bytes, &mut offset) as usize;
        assert_eq!(slots, 0);
    }
    (template_ids, surface_templates)
}
