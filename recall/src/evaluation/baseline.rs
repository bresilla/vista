use std::collections::{BTreeMap, VecDeque};

use crate::{Item, Observation, StreamId};

use super::{Baseline, LOG_FLOOR};

#[derive(Default)]
pub(super) struct BaselineState {
    frequencies: BTreeMap<Item, u64>,
    streams: BTreeMap<StreamId, (u64, VecDeque<Item>)>,
    transitions: BTreeMap<Vec<Item>, BTreeMap<Item, u64>>,
    contexts: BTreeMap<String, BTreeMap<Item, u64>>,
}

impl BaselineState {
    pub(super) fn probability(&self, baseline: Baseline, observation: &Observation) -> f64 {
        let scores = self.scores(baseline, observation);
        let selected = scores.get(&observation.item).copied().unwrap_or(0) as f64;
        let total = scores.values().copied().fold(0_u64, u64::saturating_add) as f64;
        let vocabulary = self.frequencies.len() as f64;
        let denominator = total + 0.5 * (vocabulary + 1.0);
        if denominator > 0.0 {
            ((selected + 0.5) / denominator).max(LOG_FLOOR)
        } else {
            1.0
        }
    }

    pub(super) fn predict(
        &self,
        baseline: Baseline,
        observation: &Observation,
        limit: usize,
    ) -> Vec<Item> {
        let mut ranked: Vec<_> = self.scores(baseline, observation).into_iter().collect();
        ranked.sort_by(|(a_item, a), (b_item, b)| b.cmp(a).then_with(|| a_item.cmp(b_item)));
        ranked
            .into_iter()
            .take(limit)
            .map(|(item, _)| item)
            .collect()
    }

    fn scores(&self, baseline: Baseline, observation: &Observation) -> BTreeMap<Item, u64> {
        match baseline {
            Baseline::MostRecent => self
                .streams
                .get(&observation.stream)
                .filter(|(position, _)| position.checked_add(1) == Some(observation.position.0))
                .and_then(|(_, history)| history.back())
                .cloned()
                .map(|item| BTreeMap::from([(item, 1)]))
                .unwrap_or_default(),
            Baseline::MostFrequent => self.frequencies.clone(),
            Baseline::ContextFrequency => {
                let mut scores = BTreeMap::<Item, u64>::new();
                for key in observation.context.iter().map(|feature| feature.key()) {
                    if let Some(items) = self.contexts.get(&key) {
                        for (item, count) in items {
                            let score = scores.entry(item.clone()).or_default();
                            *score = score.saturating_add(*count);
                        }
                    }
                }
                scores
            }
            Baseline::FixedOrder1
            | Baseline::FixedOrder3
            | Baseline::FixedOrder5
            | Baseline::LongestContext8 => {
                let requested_depth = match baseline {
                    Baseline::FixedOrder1 => 1,
                    Baseline::FixedOrder3 => 3,
                    Baseline::FixedOrder5 => 5,
                    _ => 8,
                };
                let Some((position, history)) = self.streams.get(&observation.stream) else {
                    return BTreeMap::new();
                };
                if position.checked_add(1) != Some(observation.position.0) {
                    return BTreeMap::new();
                }
                let maximum = requested_depth.min(history.len());
                if baseline == Baseline::LongestContext8 {
                    (1..=maximum)
                        .rev()
                        .find_map(|depth| {
                            self.transitions
                                .get(
                                    &history
                                        .iter()
                                        .skip(history.len() - depth)
                                        .cloned()
                                        .collect::<Vec<_>>(),
                                )
                                .cloned()
                        })
                        .unwrap_or_default()
                } else {
                    self.transitions
                        .get(
                            &history
                                .iter()
                                .skip(history.len() - maximum)
                                .cloned()
                                .collect::<Vec<_>>(),
                        )
                        .cloned()
                        .unwrap_or_default()
                }
            }
        }
    }

    pub(super) fn observe(&mut self, observation: &Observation) {
        let frequency = self
            .frequencies
            .entry(observation.item.clone())
            .or_default();
        *frequency = frequency.saturating_add(1);
        for key in observation.context.iter().map(|feature| feature.key()) {
            let count = self
                .contexts
                .entry(key)
                .or_default()
                .entry(observation.item.clone())
                .or_default();
            *count = count.saturating_add(1);
        }
        let stream = self.streams.entry(observation.stream).or_default();
        let continuous =
            stream.0.checked_add(1) == Some(observation.position.0) && !stream.1.is_empty();
        if !continuous {
            stream.1.clear();
        }
        if continuous {
            for depth in 1..=stream.1.len().min(8) {
                let state = stream
                    .1
                    .iter()
                    .skip(stream.1.len() - depth)
                    .cloned()
                    .collect::<Vec<_>>();
                let count = self
                    .transitions
                    .entry(state)
                    .or_default()
                    .entry(observation.item.clone())
                    .or_default();
                *count = count.saturating_add(1);
            }
        }
        stream.0 = observation.position.0;
        stream.1.push_back(observation.item.clone());
        while stream.1.len() > 8 {
            stream.1.pop_front();
        }
    }
}
