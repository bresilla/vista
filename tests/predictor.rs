use std::io::Cursor;

use vista::{
    Baseline, CandidateMatcher, Config, ContainsMatcher, Evaluation, Feature, IdentityNormalizer,
    Item, NormalizedItem, Normalizer, Observation, Position, Predictor, Query, ResearchExport,
    StreamId, Tokenizer, Trainer, WhitespaceTokenizer,
};

mod predictor_cases;

fn item(value: &str) -> Item {
    Item::new("sentence", value)
}

fn observation(stream: u64, position: u64, value: &str) -> Observation {
    Observation {
        item: item(value),
        stream: StreamId(stream),
        position: Position(position),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

fn query(stream: u64, position: u64, limit: usize) -> Query {
    Query::new(StreamId(stream), Position(position), limit)
}

fn values(predictions: &[vista::Prediction]) -> Vec<&str> {
    predictions
        .iter()
        .map(|prediction| prediction.item.value.as_str())
        .collect()
}

#[derive(Clone, Copy)]
struct ShellNormalizer;

impl Normalizer for ShellNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        if let Some(target) = raw.value.strip_prefix("ssh ") {
            NormalizedItem {
                template: Item::new(raw.namespace.clone(), "ssh {target}"),
                slots: vec![Feature::categorical("target", target)],
            }
        } else {
            NormalizedItem {
                template: raw.clone(),
                slots: Vec::new(),
            }
        }
    }
}

fn dictionary_identifier_offsets(bytes: &[u8]) -> (Vec<usize>, Vec<usize>) {
    fn read_u64(bytes: &[u8], offset: &mut usize) -> u64 {
        let value = u64::from_le_bytes(bytes[*offset..*offset + 8].try_into().unwrap());
        *offset += 8;
        value
    }
    fn skip_string(bytes: &[u8], offset: &mut usize) {
        let length = read_u64(bytes, offset) as usize;
        *offset += length;
    }
    fn skip_item(bytes: &[u8], offset: &mut usize) {
        skip_string(bytes, offset);
        skip_string(bytes, offset);
    }

    let mut offset = 8 + 4 + 8 + 8 + 26 * 8;
    for _ in 0..3 {
        skip_string(bytes, &mut offset);
    }
    offset += 8 + 4 + 4;
    let template_count = read_u64(bytes, &mut offset) as usize;
    let mut template_ids = Vec::new();
    for _ in 0..template_count {
        template_ids.push(offset);
        offset += 4;
        skip_item(bytes, &mut offset);
        offset += 8 * 4;
    }
    let surface_count = read_u64(bytes, &mut offset) as usize;
    let mut surface_templates = Vec::new();
    for _ in 0..surface_count {
        offset += 4;
        surface_templates.push(offset);
        offset += 4;
        skip_item(bytes, &mut offset);
        offset += 8 * 4;
        let slots = read_u64(bytes, &mut offset) as usize;
        assert_eq!(slots, 0);
    }
    (template_ids, surface_templates)
}
