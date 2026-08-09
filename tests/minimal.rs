use vista::{Config, Item, Observation, Position, Predictor, Query, StreamId};

fn observation(position: u64, value: &str) -> Observation {
    Observation {
        item: Item::new("command", value),
        stream: StreamId(1),
        position: Position(position),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

fn trained() -> Predictor {
    let mut predictor = Predictor::new(Config::tiny());
    predictor.observe(observation(1, "build")).unwrap();
    predictor.observe(observation(2, "test")).unwrap();
    predictor.observe(observation(3, "build")).unwrap();
    predictor
}

#[test]
fn sequence_only_core_learns_with_tiny_bounds() {
    let predictor = trained();
    let predictions = predictor.predict(&Query::new(StreamId(1), Position(4), 3));

    assert_eq!(predictions[0].item.value, "test");
    assert!(predictor.stats().templates <= Config::tiny().max_templates);
    #[cfg(not(feature = "recent-cache"))]
    assert_eq!(predictor.stats().cache_entries, 0);
    #[cfg(not(feature = "surface-indexes"))]
    assert_eq!(predictor.stats().context_associations, 0);
}

#[cfg(feature = "snapshot")]
#[test]
fn snapshot_only_build_round_trips_sequence_state() {
    use std::io::Cursor;

    use vista::{ContainsMatcher, IdentityNormalizer, WhitespaceTokenizer};

    let predictor = trained();
    let query = Query::new(StreamId(1), Position(4), 3);
    let expected = predictor.predict(&query);
    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes).unwrap();
    let restored = Predictor::read_snapshot(
        Config::tiny(),
        IdentityNormalizer,
        WhitespaceTokenizer,
        ContainsMatcher,
        Cursor::new(bytes),
    )
    .unwrap();

    assert_eq!(restored.predict(&query), expected);
}
