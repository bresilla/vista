use std::collections::{BTreeMap, VecDeque};

use crate::api::TemplateId;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StreamId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Position(pub u64);

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StreamState {
    pub(crate) last_position: Option<u64>,
    pub(crate) recent: VecDeque<TemplateId>,
    pub(crate) last_seen: u64,
}

impl StreamState {
    pub(crate) fn is_continuous(&self, position: Position) -> bool {
        self.last_position
            .is_some_and(|last| last.checked_add(1) == Some(position.0))
    }

    pub(crate) fn history(&self) -> Vec<TemplateId> {
        self.recent.iter().copied().collect()
    }

    fn reset(&mut self) {
        self.recent.clear();
        self.last_position = None;
    }
}

#[derive(Clone, Default)]
pub(crate) struct StreamTable {
    pub(crate) streams: BTreeMap<StreamId, StreamState>,
    pub(crate) capacity: usize,
}

impl StreamTable {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            streams: BTreeMap::new(),
            capacity,
        }
    }

    pub(crate) fn open(&mut self, id: StreamId, position: Position) -> (bool, Option<StreamId>) {
        let mut evicted = None;
        if !self.streams.contains_key(&id)
            && self.streams.len() >= self.capacity
            && let Some(oldest) = self
                .streams
                .iter()
                .min_by_key(|(id, state)| (state.last_seen, **id))
                .map(|(id, _)| *id)
        {
            self.streams.remove(&oldest);
            evicted = Some(oldest);
        }
        let state = self.streams.entry(id).or_default();
        let continuous = state.is_continuous(position);
        if !continuous {
            state.recent.clear();
        }
        (continuous, evicted)
    }

    pub(crate) fn history(&self, id: StreamId) -> Vec<TemplateId> {
        self.streams
            .get(&id)
            .map(StreamState::history)
            .unwrap_or_default()
    }

    pub(crate) fn continuation(&self, id: StreamId, position: Position) -> Option<&StreamState> {
        self.streams
            .get(&id)
            .filter(|state| state.is_continuous(position))
    }

    pub(crate) fn advance(
        &mut self,
        id: StreamId,
        template: TemplateId,
        position: Position,
        depth: usize,
        clock: u64,
    ) {
        if let Some(state) = self.streams.get_mut(&id) {
            state.last_position = Some(position.0);
            state.last_seen = clock;
            state.recent.push_back(template);
            while state.recent.len() > depth {
                state.recent.pop_front();
            }
        }
    }

    pub(crate) fn break_stream(&mut self, id: StreamId) {
        if let Some(state) = self.streams.get_mut(&id) {
            state.reset();
        }
    }

    pub(crate) fn remove_template(&mut self, template: TemplateId) {
        for state in self.streams.values_mut() {
            if state.recent.contains(&template) {
                state.reset();
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.streams.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.streams.len()
    }
}
