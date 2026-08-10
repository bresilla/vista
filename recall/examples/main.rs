use vista_recall::{Config, Item, Observation, Position, Predictor, Query, StreamId};

fn observation(value: &str, position: u64) -> Observation {
    Observation {
        item: Item::new("sentence", value),
        stream: StreamId(1),
        position: Position(position),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

fn main() {
    let mut predictor = Predictor::new(Config::default());
    predictor
        .replay([
            observation("build the project", 1),
            observation("run the tests", 2),
            observation("build the project", 3),
        ])
        .unwrap();
    let query = Query::new(StreamId(1), Position(4), 3);
    for prediction in predictor.predict(&query) {
        println!(
            "p={:.6} score={:.3} {}",
            prediction.probability, prediction.score, prediction.item.value
        );
    }
}
