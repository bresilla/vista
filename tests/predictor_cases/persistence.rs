use super::*;

#[test]
fn ranking_and_snapshots_are_deterministic() {
    let observations = [observation(1, 1, "z"), observation(2, 1, "a")];
    let mut first = Predictor::new(Config::default());
    let mut second = Predictor::new(Config::default());
    first.replay(observations.clone()).unwrap();
    second.replay(observations).unwrap();
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
    original
        .replay([
            observation(1, 1, "build"),
            observation(1, 2, "test"),
            observation(1, 3, "build"),
        ])
        .unwrap();
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
    original.observe(observation(1, 4, "test")).unwrap();
    restored.observe(observation(1, 4, "test")).unwrap();
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
        predictor.observe(observation(stream, 1, "hub")).unwrap();
        predictor.observe(observation(stream, 2, next)).unwrap();
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
    predictor.observe(observation(1, 1, "a")).unwrap();
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
    existing
        .replay([
            observation(1, 1, "build"),
            observation(1, 2, "test"),
            observation(1, 3, "build"),
        ])
        .unwrap();
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

    let mut offset = 8 + 4 + 8 + 8 + 26 * 8;
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
    predictor.observe(observation(1, 1, "a")).unwrap();
    predictor.observe(observation(1, 2, "b")).unwrap();
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

#[test]
fn built_in_snapshot_keys_are_stable() {
    assert_eq!(
        IdentityNormalizer.snapshot_key(),
        "vista::normalizer::IdentityNormalizer"
    );
    assert_eq!(
        WhitespaceTokenizer.snapshot_key(),
        "vista::tokenizer::WhitespaceTokenizer"
    );
    assert_eq!(
        ContainsMatcher.snapshot_key(),
        "vista::matcher::ContainsMatcher"
    );
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
    predictor.observe(observation(1, 1, "surface")).unwrap();
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
fn excessive_normalized_slots_are_rejected_without_mutation() {
    let config = Config::default();
    let mut predictor = Predictor::builder(config.clone())
        .normalizer(ManySlotsNormalizer)
        .build();
    let before = predictor.stats();
    assert!(matches!(
        predictor.observe(observation(1, 1, "surface")),
        Err(vista::InputError::TooManySlots { limit: 1_024, .. })
    ));
    assert_eq!(predictor.stats(), before);
}
