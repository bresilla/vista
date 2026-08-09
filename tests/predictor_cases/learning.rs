use super::*;

#[test]
fn sequence_prediction_learns_online() {
    let mut predictor = Predictor::new(Config::default());
    predictor
        .replay([
            observation(1, 1, "build"),
            observation(1, 2, "test"),
            observation(1, 3, "build"),
        ])
        .unwrap();
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
        predictor
            .observe(observation(1, index as u64 + 1, value))
            .unwrap();
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
            predictor
                .observe(observation(stream, index as u64 + 1, value))
                .unwrap();
        }
    }
    for (index, value) in ["a", "b", "c", "d"].into_iter().enumerate() {
        predictor
            .observe(observation(3, index as u64 + 1, value))
            .unwrap();
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
        predictor.observe(observation(stream, 1, "common")).unwrap();
        predictor.observe(observation(stream, 2, "next")).unwrap();
    }
    predictor
        .observe(observation(9, 1, "unseen-prefix"))
        .unwrap();
    predictor.observe(observation(9, 2, "common")).unwrap();
    let prediction = &predictor.predict(&query(9, 3, 5))[0];
    assert_eq!(prediction.item, item("next"));
    assert!(prediction.probability > 0.0);
}

#[test]
fn gaps_and_streams_do_not_create_transitions() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "first")).unwrap();
    predictor.observe(observation(1, 3, "after-gap")).unwrap();
    predictor.observe(observation(2, 1, "other")).unwrap();
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
fn interleaved_streams_learn_only_their_own_transitions() {
    let mut predictor = Predictor::new(Config {
        recent_cache_weight: 0.0,
        ..Config::default()
    });
    predictor
        .observe(observation(1, 1, "stream-one-start"))
        .unwrap();
    predictor
        .observe(observation(2, 1, "stream-two-start"))
        .unwrap();
    predictor
        .observe(observation(1, 2, "stream-one-next"))
        .unwrap();
    predictor
        .observe(observation(2, 2, "stream-two-next"))
        .unwrap();

    predictor
        .observe(observation(3, 1, "stream-one-start"))
        .unwrap();
    assert_eq!(
        predictor.predict(&query(3, 2, 5))[0].item,
        item("stream-one-next")
    );

    predictor
        .observe(observation(4, 1, "stream-two-start"))
        .unwrap();
    assert_eq!(
        predictor.predict(&query(4, 2, 5))[0].item,
        item("stream-two-next")
    );
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
    predictor.observe(observation(1, 1, "evicted")).unwrap();
    predictor.observe(observation(1, 2, "retained")).unwrap();
    predictor.observe(observation(1, 3, "replacement")).unwrap();

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
    predictor.observe(observation(1, 1, "a")).unwrap();
    predictor.observe(observation(1, 2, "b")).unwrap();
    predictor.break_stream(StreamId(1));
    predictor.observe(observation(1, 3, "c")).unwrap();
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
        predictor
            .observe(observation(1, position as u64 + 1, value))
            .unwrap();
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
    predictor
        .observe(observation(1, 1, "ssh alice@host1"))
        .unwrap();
    predictor
        .observe(observation(1, 2, "ssh bob@host2"))
        .unwrap();
    let mut next = query(1, 3, 2);
    next.context
        .push(Feature::categorical("target", "alice@host1"));

    assert_eq!(predictor.predict(&next)[0].item, item("ssh alice@host1"));
}

#[test]
fn identity_normalizer_preserves_one_template_per_surface() {
    let mut predictor = Predictor::new(Config::default());
    predictor.observe(observation(1, 1, "one")).unwrap();
    predictor.observe(observation(1, 2, "two")).unwrap();
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
    first.observe(event.clone()).unwrap();
    second.observe(event).unwrap();
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
    predictor
        .observe(observation(1, 1, "ssh alice@host1"))
        .unwrap();
    predictor
        .observe(observation(1, 2, "ssh bob@host2"))
        .unwrap();
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
    predictor
        .observe(observation(1, 1, "needle target"))
        .unwrap();
    for position in 1..=10 {
        predictor
            .observe(observation(
                2,
                position,
                if position % 2 == 0 { "alpha" } else { "beta" },
            ))
            .unwrap();
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
    predictor.observe(observation(1, 1, "alpha")).unwrap();
    for position in 2..=10 {
        predictor.observe(observation(1, position, "zzzz")).unwrap();
    }
    let mut next = query(1, 11, 1);
    next.partial = Some("alp".into());

    assert_eq!(predictor.predict(&next)[0].item, item("alpha"));
}
