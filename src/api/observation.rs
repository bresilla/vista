use crate::api::{Feature, Item, Position, StreamId};

/// One completed and learnable event.
///
/// Events that were hidden, rejected, or incomplete should never become
/// observations; their positions should still be consumed so that the next
/// visible observation carries a detectable gap.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub item: Item,
    pub stream: StreamId,
    pub position: Position,
    pub timestamp: i64,
    pub context: Vec<Feature>,
    pub outcome: Vec<Feature>,
}

/// A request for ranked candidates at one position of one stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub stream: StreamId,
    pub position: Position,
    pub context: Vec<Feature>,
    pub partial: Option<String>,
    pub limit: usize,
}

impl Query {
    pub fn new(stream: StreamId, position: Position, limit: usize) -> Self {
        Self {
            stream,
            position,
            context: Vec::new(),
            partial: None,
            limit,
        }
    }
}
