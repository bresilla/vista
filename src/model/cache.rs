use std::collections::{BTreeMap, VecDeque};

use crate::api::{StreamId, TemplateId};

type Entry = (u64, Option<TemplateId>, TemplateId);

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RecentCache {
    pub(crate) global: VecDeque<Entry>,
    pub(crate) streams: BTreeMap<StreamId, VecDeque<Entry>>,
    pub(crate) capacity: usize,
    pub(crate) half_life: u64,
    pub(crate) max_streams: usize,
}

impl RecentCache {
    pub(crate) fn new(capacity: usize, half_life: u64, max_streams: usize) -> Self {
        Self {
            capacity,
            half_life,
            max_streams,
            ..Self::default()
        }
    }

    #[cfg(feature = "recent-cache")]
    pub(crate) fn observe(
        &mut self,
        stream: StreamId,
        clock: u64,
        previous: Option<TemplateId>,
        next: TemplateId,
    ) {
        push(&mut self.global, self.capacity, (clock, previous, next));
        if !self.streams.contains_key(&stream)
            && self.streams.len() >= self.max_streams
            && let Some(oldest) = self
                .streams
                .iter()
                .min_by_key(|(id, entries)| {
                    (entries.back().map(|entry| entry.0).unwrap_or(0), **id)
                })
                .map(|(id, _)| *id)
        {
            self.streams.remove(&oldest);
        }
        push(
            self.streams.entry(stream).or_default(),
            self.capacity,
            (clock, previous, next),
        );
    }

    #[cfg(feature = "recent-cache")]
    pub(crate) fn candidates(
        &self,
        stream: StreamId,
        previous: Option<TemplateId>,
        clock: u64,
        limit: usize,
    ) -> Vec<TemplateId> {
        let (source, conditional) = self.source(stream, previous);
        let mut counts = BTreeMap::<TemplateId, f64>::new();
        for (seen, seen_previous, next) in source.iter().rev() {
            if !conditional || *seen_previous == previous {
                let count = counts.entry(*next).or_default();
                *count += half_life_weight(clock.saturating_sub(*seen), self.half_life);
            }
        }
        let mut ranked: Vec<_> = counts.into_iter().collect();
        ranked.sort_by(|(a_id, a), (b_id, b)| b.total_cmp(a).then_with(|| a_id.cmp(b_id)));
        ranked.into_iter().take(limit).map(|(id, _)| id).collect()
    }

    #[cfg(feature = "recent-cache")]
    pub(crate) fn probability(
        &self,
        stream: StreamId,
        previous: Option<TemplateId>,
        candidate: TemplateId,
        clock: u64,
    ) -> Option<f64> {
        let (source, conditional) = self.source(stream, previous);
        let mut total = 0.0;
        let mut selected = 0.0;
        for (seen, seen_previous, next) in source {
            if conditional && *seen_previous != previous {
                continue;
            }
            let weight = half_life_weight(clock.saturating_sub(*seen), self.half_life);
            total += weight;
            if *next == candidate {
                selected += weight;
            }
        }
        (total > 0.0).then_some(selected / total)
    }

    #[cfg(feature = "recent-cache")]
    pub(crate) fn unknown_probability(
        &self,
        stream: StreamId,
        previous: Option<TemplateId>,
    ) -> Option<f64> {
        let (source, conditional) = self.source(stream, previous);
        source
            .iter()
            .any(|(_, seen_previous, _)| !conditional || *seen_previous == previous)
            .then_some(0.0)
    }

    #[cfg(feature = "recent-cache")]
    fn source(&self, stream: StreamId, previous: Option<TemplateId>) -> (&VecDeque<Entry>, bool) {
        if let Some(cache) = self.streams.get(&stream).filter(|cache| {
            cache
                .iter()
                .any(|(_, seen_previous, _)| *seen_previous == previous)
        }) {
            return (cache, true);
        }
        if previous.is_some()
            && self
                .global
                .iter()
                .any(|(_, seen_previous, _)| *seen_previous == previous)
        {
            (&self.global, true)
        } else {
            (&self.global, false)
        }
    }

    #[cfg(feature = "recent-cache")]
    pub(crate) fn break_stream(&mut self, stream: StreamId) {
        self.streams.remove(&stream);
    }

    #[cfg(feature = "recent-cache")]
    pub(crate) fn remove_template(&mut self, template: TemplateId) {
        self.global
            .retain(|(_, previous, next)| *previous != Some(template) && *next != template);
        self.streams.retain(|_, entries| {
            entries.retain(|(_, previous, next)| *previous != Some(template) && *next != template);
            !entries.is_empty()
        });
    }

    #[cfg(feature = "recent-cache")]
    pub(crate) fn clear(&mut self) {
        self.global.clear();
        self.streams.clear();
    }
}

#[cfg(feature = "recent-cache")]
fn push<T>(queue: &mut VecDeque<T>, capacity: usize, value: T) {
    queue.push_back(value);
    while queue.len() > capacity {
        queue.pop_front();
    }
}

#[cfg(feature = "recent-cache")]
fn half_life_weight(age: u64, half_life: u64) -> f64 {
    let halves = age / half_life;
    let base = match halves {
        0..=1022 => f64::from_bits((1023 - halves) << 52),
        1023..=1074 => f64::from_bits(1 << (1074 - halves)),
        _ => 0.0,
    };
    let fraction = (age % half_life) as f64 / half_life as f64;
    base * (1.0 - 0.5 * fraction)
}

#[cfg(all(test, feature = "recent-cache"))]
mod tests {
    use super::*;

    #[test]
    fn candidates_use_decayed_weight_instead_of_raw_frequency() {
        let old = TemplateId(1);
        let recent = TemplateId(2);
        let mut cache = RecentCache::new(32, 1, 4);
        for clock in 1..=10 {
            cache.observe(StreamId(1), clock, None, old);
        }
        cache.observe(StreamId(1), 100, None, recent);

        assert_eq!(cache.candidates(StreamId(1), None, 100, 1), vec![recent]);
    }

    #[test]
    fn cache_decay_hits_exact_half_life_boundaries() {
        assert_eq!(half_life_weight(0, 32), 1.0);
        assert_eq!(half_life_weight(32, 32), 0.5);
        assert_eq!(half_life_weight(64, 32), 0.25);
        assert_eq!(half_life_weight(u64::MAX, 1), 0.0);
    }
}
