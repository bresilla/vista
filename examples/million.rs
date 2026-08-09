use std::time::{Duration, Instant};

use vista::{Config, Item, Observation, Position, Predictor, Query, StreamId, Trainer};

fn observation(position: u64) -> Observation {
    let phase = (position / 50_000) % 4;
    let step = position % 16;
    Observation {
        item: Item::new("command", format!("phase-{phase}-step-{step}")),
        stream: StreamId(position % 8),
        position: Position(position / 8 + 1),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

fn percentile(values: &mut [Duration], percentile: f64) -> Duration {
    values.sort_unstable();
    values[((values.len() - 1) as f64 * percentile).round() as usize]
}

fn main() {
    const EVENTS: u64 = 1_000_000;
    let config = Config {
        max_templates: 1_024,
        max_surfaces: 1_024,
        max_contexts: 100_000,
        max_followers_per_context: 32,
        max_partial_associations: 32_768,
        ..Config::default()
    };
    let started = Instant::now();
    let mut trainer = Trainer::new(config.clone());
    for position in 0..EVENTS {
        trainer.observe(observation(position)).unwrap();
    }
    let predictor = trainer.finish();
    let ingest = started.elapsed();
    let mut latencies = Vec::with_capacity(10_000);
    for index in 0..10_000 {
        let stream = StreamId(index % 8);
        let query = Query::new(stream, Position(EVENTS / 8 + 1), 10);
        let started = Instant::now();
        let _ = predictor.predict(&query);
        latencies.push(started.elapsed());
    }
    let mut snapshot = Vec::new();
    predictor.write_snapshot(&mut snapshot).unwrap();
    let queries: Vec<_> = (0..8)
        .map(|stream| Query::new(StreamId(stream), Position(EVENTS / 8 + 1), 10))
        .collect();
    let expected: Vec<_> = queries
        .iter()
        .map(|query| predictor.predict(query))
        .collect();
    let load_started = Instant::now();
    let restored = Predictor::read_snapshot(
        config,
        vista::IdentityNormalizer,
        vista::WhitespaceTokenizer,
        vista::ContainsMatcher,
        snapshot.as_slice(),
    )
    .unwrap();
    let load = load_started.elapsed();
    assert_eq!(predictor.stats(), restored.stats());
    for (query, expected) in queries.iter().zip(expected) {
        assert_eq!(restored.predict(query), expected);
    }
    let stats = predictor.stats();
    assert!(stats.templates <= 1_024);
    assert!(stats.surfaces <= 1_024);
    assert!(stats.streams <= 256);
    assert!(stats.contexts <= 100_000);
    assert!(stats.followers <= stats.contexts * 32);
    assert!(stats.zero_order_entries <= 1_024);
    assert!(stats.cache_entries <= 256 * 257);
    assert!(stats.stream_history_entries <= 256 * 8);
    assert!(stats.context_associations <= 65_536);
    assert!(stats.tokens <= 32_768);
    assert!(stats.token_associations <= 32_768 * 8);
    assert!(stats.partial_associations <= 32_768);
    println!("events={EVENTS}");
    println!(
        "ingest_events_per_second={:.0}",
        EVENTS as f64 / ingest.as_secs_f64()
    );
    println!(
        "p50_predict_us={}",
        percentile(&mut latencies, 0.50).as_micros()
    );
    println!(
        "p95_predict_us={}",
        percentile(&mut latencies, 0.95).as_micros()
    );
    println!(
        "p99_predict_us={}",
        percentile(&mut latencies, 0.99).as_micros()
    );
    println!("templates={}", stats.templates);
    println!("surfaces={}", stats.surfaces);
    println!("contexts={}", stats.contexts);
    println!("followers={}", stats.followers);
    println!("zero_order_entries={}", stats.zero_order_entries);
    println!("cache_entries={}", stats.cache_entries);
    println!("stream_history_entries={}", stats.stream_history_entries);
    println!("token_associations={}", stats.token_associations);
    println!("estimated_heap_bytes={}", stats.estimated_heap_bytes);
    println!("retained_string_bytes={}", stats.retained_string_bytes);
    println!("snapshot_bytes={}", snapshot.len());
    println!("snapshot_load_ms={}", load.as_millis());
}
