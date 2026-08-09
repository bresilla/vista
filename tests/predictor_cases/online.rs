use super::*;

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
        cached.observe(hub.clone()).unwrap();
        uncached.observe(hub).unwrap();
        position += 1;
        let old = observation(1, position, "old");
        cached.observe(old.clone()).unwrap();
        uncached.observe(old).unwrap();
    }
    for _ in 0..10 {
        position += 1;
        let hub = observation(1, position, "hub");
        cached.observe(hub.clone()).unwrap();
        uncached.observe(hub).unwrap();
        position += 1;
        let new = observation(1, position, "new");
        cached.observe(new.clone()).unwrap();
        uncached.observe(new).unwrap();
    }
    position += 1;
    let hub = observation(1, position, "hub");
    cached.observe(hub.clone()).unwrap();
    uncached.observe(hub).unwrap();
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
    predictor.observe(observation(1, 1, "recent")).unwrap();
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
        cached.observe(event.clone()).unwrap();
        uncached.observe(event).unwrap();
    }
    let recent = observation(1, 9, "recent");
    cached.observe(recent.clone()).unwrap();
    uncached.observe(recent).unwrap();
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
    predictor.observe(observation(1, 1, "a")).unwrap();
    predictor
        .observe(observation(2, 1, "private-stream-item"))
        .unwrap();
    predictor.break_stream(StreamId(1));
    predictor.observe(observation(1, 2, "c")).unwrap();
    predictor.break_stream(StreamId(1));
    predictor.observe(observation(3, 1, "d")).unwrap();

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
        predictor
            .observe(observation(1, position as u64 + 1, value))
            .unwrap();
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
        predictor
            .observe(observation(1, position as u64 + 1, value))
            .unwrap();
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
    replayed.replay(observations.clone()).unwrap();
    let mut trainer = Trainer::new(Config::default());
    for observed in observations {
        trainer.observe(observed).unwrap();
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
    trainer.observe(observation(1, 1, "prepare")).unwrap();
    trainer
        .observe(observation(1, 2, "ssh alice@host1"))
        .unwrap();
    trainer.observe(observation(1, 3, "prepare")).unwrap();
    let predictor = trainer.finish();
    let prediction = &predictor.predict(&query(1, 4, 1))[0];
    assert_eq!(prediction.template.value, "ssh {target}");
}

#[test]
fn forgetting_removes_surfaces_without_bridging_history() {
    let mut predictor = Predictor::new(Config::default());
    for (position, value) in [(1, "a"), (2, "private"), (3, "c")] {
        predictor.observe(observation(1, position, value)).unwrap();
    }
    predictor.forget(&|candidate: &Item| candidate.value == "private");
    assert!(!values(&predictor.predict(&query(1, 4, 10))).contains(&"private"));
    predictor.observe(observation(1, 4, "d")).unwrap();
    predictor.observe(observation(2, 1, "a")).unwrap();
    predictor.observe(observation(2, 2, "c")).unwrap();
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
    predictor
        .observe(observation(1, 1, "public-before"))
        .unwrap();
    predictor
        .observe(observation(1, 2, "private-secret-value"))
        .unwrap();
    predictor
        .observe(observation(1, 3, "public-after"))
        .unwrap();
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
        predictor.observe(observed).unwrap();
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
