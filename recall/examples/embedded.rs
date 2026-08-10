use std::hint::black_box;

use vista_recall::{Config, Item, Observation, Position, Predictor, Query, StreamId};

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

fn main() {
    let mut predictor = Predictor::new(Config::tiny());
    if predictor.observe(observation(1, "build")).is_err() {
        return;
    }
    if predictor.observe(observation(2, "test")).is_err() {
        return;
    }
    black_box(predictor.predict(&Query::new(StreamId(1), Position(3), 3)));
}
