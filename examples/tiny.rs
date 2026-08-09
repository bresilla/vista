use vista::{Config, Item, Observation, Position, Predictor, StreamId};

fn main() {
    let mut predictor = Predictor::new(Config::tiny());
    for position in 1..=100_000 {
        predictor
            .observe(Observation {
                item: Item::new("command", format!("command-{}", position % 64)),
                stream: StreamId(1),
                position: Position(position),
                timestamp: position as i64,
                context: Vec::new(),
                outcome: Vec::new(),
            })
            .unwrap();
    }
    let stats = predictor.stats();
    println!("events={}", stats.observations);
    println!("templates={}", stats.templates);
    println!("surfaces={}", stats.surfaces);
    println!("contexts={}", stats.contexts);
    println!("followers={}", stats.followers);
    println!("estimated_heap_bytes={}", stats.estimated_heap_bytes);
    println!("retained_string_bytes={}", stats.retained_string_bytes);
}
