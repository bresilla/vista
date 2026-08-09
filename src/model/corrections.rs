use std::collections::BTreeMap;

use crate::api::{Item, StreamId};

const MAX_PAIR_CHARS: usize = 512;

/// A retyping the caller performed: `typed` failed, `corrected` then succeeded.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CorrectionPair {
    pub typed: Item,
    pub corrected: Item,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Attempt {
    pub(crate) item: Item,
    pub(crate) position: u64,
    pub(crate) failed: bool,
}

/// Bounded log of observed retypings, mined without annotation or rules.
#[derive(Clone, Default)]
pub(crate) struct CorrectionLog {
    pairs: BTreeMap<CorrectionPair, u64>,
    order: BTreeMap<u64, CorrectionPair>,
    clocks: BTreeMap<CorrectionPair, u64>,
    pending: BTreeMap<StreamId, Attempt>,
    string_bytes: usize,
    capacity: usize,
}

impl CorrectionLog {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            ..Self::default()
        }
    }

    /// Records the observation and, when it directly repairs the previous one,
    /// the resulting pair.
    pub(crate) fn observe(&mut self, stream: StreamId, item: &Item, position: u64, failed: bool) {
        let previous = self.pending.insert(
            stream,
            Attempt {
                item: item.clone(),
                position,
                failed,
            },
        );
        if failed {
            return;
        }
        let Some(previous) = previous else { return };
        if !previous.failed || previous.position.checked_add(1) != Some(position) {
            return;
        }
        if previous.item == *item || !plausible_retyping(&previous.item.value, &item.value) {
            return;
        }
        self.record(CorrectionPair {
            typed: previous.item,
            corrected: item.clone(),
        });
    }

    fn record(&mut self, pair: CorrectionPair) {
        let clock = self.clocks.len() as u64 + 1;
        if let Some(previous) = self.clocks.insert(pair.clone(), clock) {
            self.order.remove(&previous);
        } else {
            self.string_bytes = self
                .string_bytes
                .saturating_add(pair_bytes(&pair).saturating_mul(2));
        }
        *self.pairs.entry(pair.clone()).or_default() += 1;
        self.order.insert(clock, pair);
        while self.pairs.len() > self.capacity {
            let Some((clock, victim)) = self.order.pop_first() else {
                break;
            };
            let _ = clock;
            self.pairs.remove(&victim);
            self.clocks.remove(&victim);
            self.string_bytes = self
                .string_bytes
                .saturating_sub(pair_bytes(&victim).saturating_mul(2));
        }
    }

    pub(crate) fn pairs(&self) -> impl Iterator<Item = (&CorrectionPair, u64)> {
        self.pairs.iter().map(|(pair, count)| (pair, *count))
    }

    /// Observed rate at which `typed` was retyped as `corrected`, over every
    /// retyping of `typed` recorded so far.
    pub(crate) fn retyped_rate(&self, typed: &str, corrected: &str) -> Option<f64> {
        let mut matching = 0_u64;
        let mut total = 0_u64;
        for (pair, count) in &self.pairs {
            for (from, to) in aligned_tokens(&pair.typed.value, &pair.corrected.value) {
                if from != typed {
                    continue;
                }
                total = total.saturating_add(*count);
                if to == corrected {
                    matching = matching.saturating_add(*count);
                }
            }
        }
        (total > 0).then(|| matching as f64 / total as f64)
    }

    pub(crate) fn len(&self) -> usize {
        self.pairs.len()
    }

    pub(crate) fn string_bytes(&self) -> usize {
        self.string_bytes
    }

    pub(crate) fn break_stream(&mut self, stream: StreamId) {
        self.pending.remove(&stream);
    }

    pub(crate) fn clear(&mut self) {
        self.pairs.clear();
        self.order.clear();
        self.clocks.clear();
        self.pending.clear();
        self.string_bytes = 0;
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn restore(capacity: usize, pairs: Vec<(CorrectionPair, u64)>) -> Self {
        let mut log = Self::new(capacity);
        for (pair, count) in pairs {
            log.record(pair.clone());
            if let Some(entry) = log.pairs.get_mut(&pair) {
                *entry = count.max(1);
            }
        }
        log
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn ordered(&self) -> Vec<(&CorrectionPair, u64)> {
        self.pairs
            .iter()
            .map(|(pair, count)| (pair, *count))
            .collect()
    }
}

/// One point change per five characters, the gate used to mine query logs.
fn plausible_retyping(typed: &str, corrected: &str) -> bool {
    if typed.len() > MAX_PAIR_CHARS || corrected.len() > MAX_PAIR_CHARS {
        return false;
    }
    let typed: Vec<char> = typed.chars().collect();
    let corrected: Vec<char> = corrected.chars().collect();
    let budget = 1 + (typed.len() + corrected.len()) / 10;
    distance_within(&typed, &corrected, budget).is_some()
}

/// Levenshtein distance, abandoned once it cannot fall within `budget`.
fn distance_within(left: &[char], right: &[char], budget: usize) -> Option<usize> {
    if left.len().abs_diff(right.len()) > budget {
        return None;
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0_usize; right.len() + 1];
    for (row, from) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, to) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(from != to);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        if current.iter().min().copied().unwrap_or(0) > budget {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[right.len()];
    (distance <= budget).then_some(distance)
}

/// Positionally aligned token pairs that differ, over equal-length gaps.
fn aligned_tokens<'a>(typed: &'a str, corrected: &'a str) -> Vec<(&'a str, &'a str)> {
    let typed: Vec<&str> = typed.split_whitespace().collect();
    let corrected: Vec<&str> = corrected.split_whitespace().collect();
    if typed.len() != corrected.len() {
        return Vec::new();
    }
    typed
        .into_iter()
        .zip(corrected)
        .filter(|(from, to)| from != to)
        .collect()
}

fn pair_bytes(pair: &CorrectionPair) -> usize {
    pair.typed.namespace.len()
        + pair.typed.value.len()
        + pair.corrected.namespace.len()
        + pair.corrected.value.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(value: &str) -> Item {
        Item::new("command", value)
    }

    fn log() -> CorrectionLog {
        CorrectionLog::new(8)
    }

    #[test]
    fn a_failure_followed_by_a_retyping_is_recorded() {
        let mut log = log();
        log.observe(StreamId(1), &item("git chekout main"), 1, true);
        log.observe(StreamId(1), &item("git checkout main"), 2, false);
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn gaps_streams_and_successes_record_nothing() {
        let mut log = log();
        log.observe(StreamId(1), &item("git chekout main"), 1, true);
        log.observe(StreamId(1), &item("git checkout main"), 3, false);
        log.observe(StreamId(2), &item("git chekout main"), 1, true);
        log.observe(StreamId(3), &item("git checkout main"), 2, false);
        log.observe(StreamId(4), &item("ls"), 1, false);
        log.observe(StreamId(4), &item("ls -la"), 2, false);
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn a_distant_retyping_is_not_a_correction() {
        let mut log = log();
        log.observe(StreamId(1), &item("cargo build"), 1, true);
        log.observe(StreamId(1), &item("ssh alice@example.com"), 2, false);
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn the_log_respects_its_bound() {
        let mut log = CorrectionLog::new(2);
        for index in 0..6 {
            let typo = format!("commandx{index}");
            let fixed = format!("command{index}");
            log.observe(StreamId(1), &item(&typo), index * 2 + 1, true);
            log.observe(StreamId(1), &item(&fixed), index * 2 + 2, false);
        }
        assert!(log.len() <= 2);
    }

    #[test]
    fn breaking_a_stream_forgets_the_pending_attempt() {
        let mut log = log();
        log.observe(StreamId(1), &item("git chekout main"), 1, true);
        log.break_stream(StreamId(1));
        log.observe(StreamId(1), &item("git checkout main"), 2, false);
        assert_eq!(log.len(), 0);
    }
}
