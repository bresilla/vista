use std::hint::black_box;

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

fn main() {
    let mut predictor = Predictor::new(Config::tiny());
    predictor.observe(observation(1, "build"));
    predictor.observe(observation(2, "test"));
    black_box(predictor.predict(&Query::new(StreamId(1), Position(3), 3)));
}
