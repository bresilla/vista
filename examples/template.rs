use vista::{
    Config, Feature, Item, NormalizedItem, Normalizer, Observation, Position, Predictor, Query,
    StreamId,
};

struct CommandNormalizer;

impl Normalizer for CommandNormalizer {
    fn normalize(&self, item: &Item) -> NormalizedItem {
        if let Some(target) = item.value.strip_prefix("ssh ") {
            NormalizedItem {
                template: Item::new(item.namespace.clone(), "ssh {target}"),
                slots: vec![Feature::categorical("target", target)],
            }
        } else {
            NormalizedItem {
                template: item.clone(),
                slots: Vec::new(),
            }
        }
    }
}

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
    let mut predictor = Predictor::builder(Config::default())
        .normalizer(CommandNormalizer)
        .build();
    predictor.replay([
        observation(1, "prepare"),
        observation(2, "ssh alice@host1"),
        observation(3, "prepare"),
        observation(4, "ssh bob@host2"),
        observation(5, "prepare"),
    ]);
    for prediction in predictor.predict(&Query::new(StreamId(1), Position(6), 3)) {
        println!(
            "{:.6} {} [{}]",
            prediction.probability, prediction.item.value, prediction.template.value
        );
    }
}
