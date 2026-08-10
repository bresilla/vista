use std::collections::{BTreeMap, BTreeSet};

use crate::api::TemplateId;

const MIN_PROBABILITY: f64 = 1.0e-300;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ContextId(u32);

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FollowerState {
    pub(crate) count: u64,
    pub(crate) last_seen: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ContextState {
    pub(crate) followers: BTreeMap<TemplateId, FollowerState>,
    pub(crate) total: u64,
    pub(crate) pruned_count: u64,
    pub(crate) last_seen: u64,
}

impl ContextState {
    fn evidence(&self) -> u64 {
        self.total.saturating_add(self.pruned_count)
    }
}

#[derive(Clone)]
struct ContextRecord {
    members: Vec<TemplateId>,
    state: ContextState,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProbabilityTrace {
    pub(crate) probability: f64,
    pub(crate) deepest: usize,
    pub(crate) backoffs: usize,
    pub(crate) count: u64,
    pub(crate) total: u64,
}

pub(crate) struct PpmHistory {
    contexts: Vec<Option<ContextId>>,
}

#[derive(Clone)]
pub(crate) struct Ppm {
    contexts: Vec<Option<ContextRecord>>,
    context_count: usize,
    context_buckets: BTreeMap<u64, Vec<ContextId>>,
    pub(crate) zero: BTreeMap<TemplateId, u64>,
    pub(crate) zero_total: u64,
    pub(crate) max_contexts: usize,
    pub(crate) max_followers: usize,
    pub(crate) max_order: usize,
    context_order: BTreeSet<(u64, u64, ContextId)>,
    member_contexts: BTreeMap<TemplateId, BTreeSet<ContextId>>,
    follower_contexts: BTreeMap<TemplateId, BTreeSet<ContextId>>,
    free_contexts: BTreeSet<ContextId>,
}

impl Ppm {
    pub(crate) fn new(max_contexts: usize, max_followers: usize, max_order: usize) -> Self {
        Self {
            contexts: Vec::new(),
            context_count: 0,
            context_buckets: BTreeMap::new(),
            zero: BTreeMap::new(),
            zero_total: 0,
            max_contexts,
            max_followers,
            max_order,
            context_order: BTreeSet::new(),
            member_contexts: BTreeMap::new(),
            follower_contexts: BTreeMap::new(),
            free_contexts: BTreeSet::new(),
        }
    }

    pub(crate) fn learn(&mut self, history: &[TemplateId], next: TemplateId, clock: u64) {
        *self.zero.entry(next).or_default() =
            self.zero.get(&next).copied().unwrap_or(0).saturating_add(1);
        self.zero_total = self.zero_total.saturating_add(1);
        for depth in 1..=history.len().min(self.max_order) {
            let members = &history[history.len() - depth..];
            let context = if let Some(id) = self.context_id(members) {
                id
            } else {
                if self.context_count >= self.max_contexts {
                    self.evict_context();
                }
                let Some(id) = self.insert_context(members.to_vec(), ContextState::default())
                else {
                    continue;
                };
                id
            };
            let state = &mut self
                .contexts
                .get_mut(context.0 as usize)
                .and_then(Option::as_mut)
                .expect("context index is valid")
                .state;
            self.context_order
                .remove(&(state.evidence(), state.last_seen, context));
            state.total = state.total.saturating_add(1);
            state.last_seen = clock;
            let is_new_follower = !state.followers.contains_key(&next);
            let follower = state.followers.entry(next).or_default();
            follower.count = follower.count.saturating_add(1);
            follower.last_seen = clock;
            if is_new_follower {
                self.follower_contexts
                    .entry(next)
                    .or_default()
                    .insert(context);
            }
            if state.followers.len() > self.max_followers
                && let Some(victim) = state
                    .followers
                    .iter()
                    .min_by_key(|(id, follower)| (follower.count, follower.last_seen, **id))
                    .map(|(id, follower)| (*id, follower.clone()))
            {
                state.followers.remove(&victim.0);
                state.total = state.total.saturating_sub(victim.1.count);
                if let Some(contexts) = self.follower_contexts.get_mut(&victim.0) {
                    contexts.remove(&context);
                    if contexts.is_empty() {
                        self.follower_contexts.remove(&victim.0);
                    }
                }
                state.pruned_count = state.pruned_count.saturating_add(victim.1.count);
            }
            self.context_order
                .insert((state.evidence(), state.last_seen, context));
        }
    }

    fn insert_context(
        &mut self,
        members: Vec<TemplateId>,
        state: ContextState,
    ) -> Option<ContextId> {
        let id = self
            .free_contexts
            .pop_first()
            .or_else(|| u32::try_from(self.contexts.len()).ok().map(ContextId))?;
        let hash = context_hash(&members);
        self.context_buckets.entry(hash).or_default().push(id);
        for member in members.iter().copied().collect::<BTreeSet<_>>() {
            self.member_contexts.entry(member).or_default().insert(id);
        }
        for follower in state.followers.keys() {
            self.follower_contexts
                .entry(*follower)
                .or_default()
                .insert(id);
        }
        self.context_order
            .insert((state.evidence(), state.last_seen, id));
        let record = ContextRecord { members, state };
        if id.0 as usize == self.contexts.len() {
            self.contexts.push(Some(record));
        } else {
            self.contexts[id.0 as usize] = Some(record);
        }
        self.context_count += 1;
        Some(id)
    }

    fn context_id(&self, members: &[TemplateId]) -> Option<ContextId> {
        self.context_buckets
            .get(&context_hash(members))?
            .iter()
            .copied()
            .find(|id| {
                self.contexts
                    .get(id.0 as usize)
                    .and_then(Option::as_ref)
                    .is_some_and(|record| record.members == members)
            })
    }

    fn evict_context(&mut self) {
        if let Some((_, _, context)) = self.context_order.pop_first() {
            self.remove_context(context);
        }
    }

    fn remove_context(&mut self, context: ContextId) -> Option<ContextRecord> {
        let record = self.contexts.get_mut(context.0 as usize)?.take()?;
        self.context_count = self.context_count.saturating_sub(1);
        self.context_order
            .remove(&(record.state.evidence(), record.state.last_seen, context));
        let hash = context_hash(&record.members);
        if let Some(contexts) = self.context_buckets.get_mut(&hash) {
            contexts.retain(|candidate| *candidate != context);
            if contexts.is_empty() {
                self.context_buckets.remove(&hash);
            }
        }
        for member in record.members.iter().copied().collect::<BTreeSet<_>>() {
            remove_context_id(&mut self.member_contexts, member, context);
        }
        for follower in record.state.followers.keys() {
            remove_context_id(&mut self.follower_contexts, *follower, context);
        }
        self.free_contexts.insert(context);
        Some(record)
    }

    pub(crate) fn resolve(&self, history: &[TemplateId]) -> PpmHistory {
        PpmHistory {
            contexts: (1..=history.len().min(self.max_order))
                .map(|depth| self.context_id(&history[history.len() - depth..]))
                .collect(),
        }
    }

    pub(crate) fn candidates_resolved(
        &self,
        history: &PpmHistory,
        limit: usize,
    ) -> Vec<TemplateId> {
        let mut weighted = BTreeMap::<TemplateId, (usize, u64)>::new();
        for (index, context) in history.contexts.iter().enumerate().rev() {
            let Some(state) = context.and_then(|id| self.state(id)) else {
                continue;
            };
            let depth = index + 1;
            for (id, follower) in &state.followers {
                let entry = weighted.entry(*id).or_default();
                entry.0 = entry.0.max(depth);
                entry.1 = entry.1.saturating_add(follower.count);
            }
        }
        let mut ranked: Vec<_> = weighted.into_iter().collect();
        ranked.sort_by(|(a_id, a), (b_id, b)| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a_id.cmp(b_id))
        });
        ranked.into_iter().take(limit).map(|(id, _)| id).collect()
    }

    pub(crate) fn probability(
        &self,
        history: &[TemplateId],
        candidate: TemplateId,
        vocabulary: usize,
    ) -> ProbabilityTrace {
        self.probability_resolved(&self.resolve(history), candidate, vocabulary)
    }

    pub(crate) fn probability_resolved(
        &self,
        history: &PpmHistory,
        candidate: TemplateId,
        vocabulary: usize,
    ) -> ProbabilityTrace {
        let denominator = self.zero_total as f64 + 0.5 * (vocabulary as f64 + 1.0);
        let base_count = self.zero.get(&candidate).copied().unwrap_or(0);
        let mut probability = if denominator > 0.0 {
            (base_count as f64 + 0.5) / denominator
        } else {
            1.0 / (vocabulary.max(1) + 1) as f64
        };
        let mut deepest = 0;
        let mut backoffs = 0;
        let mut trace_count = base_count;
        let mut trace_total = self.zero_total;
        for (index, context) in history.contexts.iter().enumerate() {
            let depth = index + 1;
            let Some(state) = context.and_then(|id| self.state(id)) else {
                backoffs += 1;
                continue;
            };
            let distinct = state.followers.len() as f64;
            let denominator = state.total.saturating_add(state.pruned_count) as f64 + distinct;
            if denominator <= 0.0 {
                backoffs += 1;
                continue;
            }
            let count = state
                .followers
                .get(&candidate)
                .map(|follower| follower.count)
                .unwrap_or(0);
            let escape = (distinct + state.pruned_count as f64) / denominator;
            probability = count as f64 / denominator + escape * probability;
            deepest = depth;
            trace_count = count;
            trace_total = state.total.saturating_add(state.pruned_count);
        }
        ProbabilityTrace {
            probability: probability.clamp(MIN_PROBABILITY, 1.0),
            deepest,
            backoffs,
            count: trace_count,
            total: trace_total,
        }
    }

    pub(crate) fn unknown_probability(&self, history: &[TemplateId], vocabulary: usize) -> f64 {
        self.unknown_probability_resolved(&self.resolve(history), vocabulary)
    }

    pub(crate) fn unknown_probability_resolved(
        &self,
        history: &PpmHistory,
        vocabulary: usize,
    ) -> f64 {
        let denominator = self.zero_total as f64 + 0.5 * (vocabulary as f64 + 1.0);
        let mut probability = if denominator > 0.0 {
            0.5 / denominator
        } else {
            1.0 / (vocabulary.max(1) + 1) as f64
        };
        for context in &history.contexts {
            let Some(state) = context.and_then(|id| self.state(id)) else {
                continue;
            };
            let distinct = state.followers.len() as f64;
            let denominator = state.total.saturating_add(state.pruned_count) as f64 + distinct;
            if denominator > 0.0 {
                probability *= (distinct + state.pruned_count as f64) / denominator;
            }
        }
        probability.clamp(MIN_PROBABILITY, 1.0)
    }

    pub(crate) fn remove_template(&mut self, template: TemplateId) {
        if let Some(count) = self.zero.remove(&template) {
            self.zero_total = self.zero_total.saturating_sub(count);
        }
        let member_contexts = self.member_contexts.remove(&template).unwrap_or_default();
        for context in member_contexts {
            self.remove_context(context);
        }
        let follower_contexts = self.follower_contexts.remove(&template).unwrap_or_default();
        for context in follower_contexts {
            let remove = if let Some(record) = self
                .contexts
                .get_mut(context.0 as usize)
                .and_then(Option::as_mut)
            {
                self.context_order.remove(&(
                    record.state.evidence(),
                    record.state.last_seen,
                    context,
                ));
                if let Some(follower) = record.state.followers.remove(&template) {
                    record.state.total = record.state.total.saturating_sub(follower.count);
                }
                if record.state.followers.is_empty() {
                    true
                } else {
                    self.context_order.insert((
                        record.state.evidence(),
                        record.state.last_seen,
                        context,
                    ));
                    false
                }
            } else {
                false
            };
            if remove {
                self.remove_context(context);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.contexts.clear();
        self.context_count = 0;
        self.context_buckets.clear();
        self.zero.clear();
        self.context_order.clear();
        self.member_contexts.clear();
        self.follower_contexts.clear();
        self.free_contexts.clear();
        self.zero_total = 0;
    }

    pub(crate) fn context_count(&self) -> usize {
        self.context_count
    }

    pub(crate) fn follower_count(&self) -> usize {
        self.contexts
            .iter()
            .filter_map(Option::as_ref)
            .map(|record| record.state.followers.len())
            .sum()
    }

    pub(crate) fn context_member_count(&self) -> usize {
        self.contexts
            .iter()
            .filter_map(Option::as_ref)
            .map(|record| record.members.len())
            .sum()
    }

    pub(crate) fn reverse_association_count(&self) -> usize {
        self.context_buckets.values().map(Vec::len).sum::<usize>()
            + self
                .member_contexts
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
            + self
                .follower_contexts
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
            + self.context_order.len()
            + self.free_contexts.len()
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn ordered_contexts(&self) -> Vec<(&[TemplateId], &ContextState)> {
        let mut contexts: Vec<_> = self
            .contexts
            .iter()
            .filter_map(Option::as_ref)
            .map(|record| (record.members.as_slice(), &record.state))
            .collect();
        contexts.sort_by(|left, right| left.0.cmp(right.0));
        contexts
    }

    #[cfg(test)]
    fn context_state(&self, members: &[TemplateId]) -> Option<&ContextState> {
        let id = self.context_id(members)?;
        self.state(id)
    }

    fn state(&self, id: ContextId) -> Option<&ContextState> {
        self.contexts
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .map(|record| &record.state)
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn restore(
        contexts: BTreeMap<Vec<TemplateId>, ContextState>,
        zero: BTreeMap<TemplateId, u64>,
        zero_total: u64,
        max_contexts: usize,
        max_followers: usize,
        max_order: usize,
    ) -> Self {
        let mut ppm = Self::new(max_contexts, max_followers, max_order);
        ppm.zero = zero;
        ppm.zero_total = zero_total;
        for (members, state) in contexts {
            ppm.insert_context(members, state)
                .expect("validated context count fits identifiers");
        }
        ppm
    }
}

fn context_hash(members: &[TemplateId]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in (members.len() as u64)
        .to_le_bytes()
        .into_iter()
        .chain(members.iter().flat_map(|id| id.0.to_le_bytes()))
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn remove_context_id(
    index: &mut BTreeMap<TemplateId, BTreeSet<ContextId>>,
    template: TemplateId,
    context: ContextId,
) {
    if let Some(contexts) = index.get_mut(&template) {
        contexts.remove(&context);
        if contexts.is_empty() {
            index.remove(&template);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_probability_matches_hand_calculation() {
        let x = TemplateId(0);
        let a = TemplateId(1);
        let b = TemplateId(2);
        let mut ppm = Ppm::new(16, 8, 8);
        ppm.learn(&[x], a, 1);
        ppm.learn(&[x], a, 2);
        ppm.learn(&[x], b, 3);
        let probability = ppm.probability(&[x], a, 2).probability;
        let expected = 2.0 / 5.0 + 2.0 / 5.0 * (2.5 / 4.5);
        assert!((probability - expected).abs() < 1.0e-12);
    }

    #[test]
    fn follower_pruning_preserves_escape_mass() {
        let x = TemplateId(0);
        let a = TemplateId(1);
        let b = TemplateId(2);
        let mut ppm = Ppm::new(16, 1, 8);
        ppm.learn(&[x], a, 1);
        ppm.learn(&[x], a, 2);
        ppm.learn(&[x], b, 3);
        let state = ppm.context_state(&[x]).unwrap();
        assert_eq!(state.total, 2);
        assert_eq!(state.pruned_count, 1);
        assert!(ppm.probability(&[x], b, 2).probability > 0.0);
    }

    #[test]
    fn multilevel_escape_and_unknown_mass_are_exact() {
        let x = TemplateId(0);
        let y = TemplateId(1);
        let z = TemplateId(2);
        let a = TemplateId(3);
        let b = TemplateId(4);
        let mut ppm = Ppm::new(16, 8, 8);
        ppm.learn(&[x], a, 1);
        ppm.learn(&[y, x], b, 2);

        let a_probability = ppm.probability(&[y, x], a, 2).probability;
        let b_probability = ppm.probability(&[y, x], b, 2).probability;
        let unknown = ppm.unknown_probability(&[y, x], 2);
        assert!((a_probability - 13.0 / 56.0).abs() < 1.0e-12);
        assert!((b_probability - 41.0 / 56.0).abs() < 1.0e-12);
        assert!((unknown - 1.0 / 28.0).abs() < 1.0e-12);
        assert!((a_probability + b_probability + unknown - 1.0).abs() < 1.0e-12);

        let backed_off = ppm.probability(&[z, x], a, 2);
        assert!((backed_off.probability - 13.0 / 28.0).abs() < 1.0e-12);
        assert_eq!(backed_off.backoffs, 1);
    }

    #[test]
    fn follower_pruning_uses_recency_before_identifier() {
        let x = TemplateId(0);
        let older = TemplateId(1);
        let newer = TemplateId(2);
        let mut ppm = Ppm::new(16, 1, 8);
        ppm.learn(&[x], older, 1);
        ppm.learn(&[x], newer, 2);
        let followers = &ppm.context_state(&[x]).unwrap().followers;
        assert!(!followers.contains_key(&older));
        assert!(followers.contains_key(&newer));
    }

    #[test]
    fn removing_the_only_follower_cleans_reverse_context_indexes() {
        let context_member = TemplateId(0);
        let follower = TemplateId(1);
        let mut ppm = Ppm::new(16, 8, 8);
        ppm.learn(&[context_member], follower, 1);
        ppm.remove_template(follower);
        assert_eq!(ppm.context_count(), 0);
        assert!(!ppm.member_contexts.contains_key(&context_member));
        assert!(ppm.context_order.is_empty());
    }

    #[test]
    fn evicted_context_identifiers_are_reused_deterministically() {
        let a = TemplateId(0);
        let b = TemplateId(1);
        let mut ppm = Ppm::new(1, 8, 8);
        ppm.learn(&[a], b, 1);
        let first = ppm.context_id(&[a]).unwrap();
        ppm.learn(&[b], a, 2);
        assert_eq!(ppm.context_id(&[b]), Some(first));
        assert_eq!(ppm.contexts.len(), 1);
    }
}
