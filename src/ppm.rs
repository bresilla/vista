use std::collections::{BTreeMap, BTreeSet};

use crate::item::TemplateId;

const MIN_PROBABILITY: f64 = 1.0e-300;

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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProbabilityTrace {
    pub(crate) probability: f64,
    pub(crate) deepest: usize,
    pub(crate) backoffs: usize,
    pub(crate) count: u64,
    pub(crate) total: u64,
}

pub(crate) struct Ppm {
    pub(crate) contexts: BTreeMap<Vec<TemplateId>, ContextState>,
    pub(crate) zero: BTreeMap<TemplateId, u64>,
    pub(crate) zero_total: u64,
    pub(crate) max_contexts: usize,
    pub(crate) max_followers: usize,
    pub(crate) max_order: usize,
    context_order: BTreeSet<(u64, u64, Vec<TemplateId>)>,
    member_contexts: BTreeMap<TemplateId, BTreeSet<Vec<TemplateId>>>,
    follower_contexts: BTreeMap<TemplateId, BTreeSet<Vec<TemplateId>>>,
}

impl Ppm {
    pub(crate) fn new(max_contexts: usize, max_followers: usize, max_order: usize) -> Self {
        Self {
            contexts: BTreeMap::new(),
            zero: BTreeMap::new(),
            zero_total: 0,
            max_contexts,
            max_followers,
            max_order,
            context_order: BTreeSet::new(),
            member_contexts: BTreeMap::new(),
            follower_contexts: BTreeMap::new(),
        }
    }

    pub(crate) fn learn(&mut self, history: &[TemplateId], next: TemplateId, clock: u64) {
        *self.zero.entry(next).or_default() =
            self.zero.get(&next).copied().unwrap_or(0).saturating_add(1);
        self.zero_total = self.zero_total.saturating_add(1);
        for depth in 1..=history.len().min(self.max_order) {
            let context = history[history.len() - depth..].to_vec();
            if !self.contexts.contains_key(&context) && self.contexts.len() >= self.max_contexts {
                self.evict_context();
            }
            let is_new_context = !self.contexts.contains_key(&context);
            if is_new_context {
                for member in context.iter().copied().collect::<BTreeSet<_>>() {
                    self.member_contexts
                        .entry(member)
                        .or_default()
                        .insert(context.clone());
                }
            }
            let state = self.contexts.entry(context.clone()).or_default();
            self.context_order
                .remove(&(state.evidence(), state.last_seen, context.clone()));
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
                    .insert(context.clone());
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

    fn evict_context(&mut self) {
        if let Some((_, _, context)) = self.context_order.pop_first() {
            self.remove_context(&context);
        }
    }

    fn remove_context(&mut self, context: &[TemplateId]) -> Option<ContextState> {
        let state = self.contexts.remove(context)?;
        self.context_order
            .remove(&(state.evidence(), state.last_seen, context.to_vec()));
        for member in context.iter().copied().collect::<BTreeSet<_>>() {
            if let Some(contexts) = self.member_contexts.get_mut(&member) {
                contexts.remove(context);
                if contexts.is_empty() {
                    self.member_contexts.remove(&member);
                }
            }
        }
        for follower in state.followers.keys() {
            if let Some(contexts) = self.follower_contexts.get_mut(follower) {
                contexts.remove(context);
                if contexts.is_empty() {
                    self.follower_contexts.remove(follower);
                }
            }
        }
        Some(state)
    }

    pub(crate) fn candidates(&self, history: &[TemplateId], limit: usize) -> Vec<TemplateId> {
        let mut weighted = BTreeMap::<TemplateId, (usize, u64)>::new();
        for depth in (1..=history.len().min(self.max_order)).rev() {
            let Some(state) = self.contexts.get(&history[history.len() - depth..]) else {
                continue;
            };
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
        for depth in 1..=history.len().min(self.max_order) {
            let Some(state) = self.contexts.get(&history[history.len() - depth..]) else {
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
        let denominator = self.zero_total as f64 + 0.5 * (vocabulary as f64 + 1.0);
        let mut probability = if denominator > 0.0 {
            0.5 / denominator
        } else {
            1.0 / (vocabulary.max(1) + 1) as f64
        };
        for depth in 1..=history.len().min(self.max_order) {
            let Some(state) = self.contexts.get(&history[history.len() - depth..]) else {
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
            self.remove_context(&context);
        }
        let follower_contexts = self.follower_contexts.remove(&template).unwrap_or_default();
        for context in follower_contexts {
            let remove_context = if let Some(state) = self.contexts.get_mut(&context) {
                self.context_order
                    .remove(&(state.evidence(), state.last_seen, context.clone()));
                if let Some(follower) = state.followers.remove(&template) {
                    state.total = state.total.saturating_sub(follower.count);
                }
                let empty = state.followers.is_empty();
                if !empty {
                    self.context_order
                        .insert((state.evidence(), state.last_seen, context.clone()));
                }
                empty
            } else {
                false
            };
            if remove_context {
                self.remove_context(&context);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.contexts.clear();
        self.zero.clear();
        self.context_order.clear();
        self.member_contexts.clear();
        self.follower_contexts.clear();
        self.zero_total = 0;
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
        let context_order = contexts
            .iter()
            .map(|(context, state)| (state.evidence(), state.last_seen, context.clone()))
            .collect();
        let mut member_contexts = BTreeMap::<TemplateId, BTreeSet<Vec<TemplateId>>>::new();
        let mut follower_contexts = BTreeMap::<TemplateId, BTreeSet<Vec<TemplateId>>>::new();
        for (context, state) in &contexts {
            for member in context.iter().copied().collect::<BTreeSet<_>>() {
                member_contexts
                    .entry(member)
                    .or_default()
                    .insert(context.clone());
            }
            for follower in state.followers.keys() {
                follower_contexts
                    .entry(*follower)
                    .or_default()
                    .insert(context.clone());
            }
        }
        Self {
            contexts,
            zero,
            zero_total,
            max_contexts,
            max_followers,
            max_order,
            context_order,
            member_contexts,
            follower_contexts,
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
        let state = ppm.contexts.get(&vec![x]).unwrap();
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
        let followers = &ppm.contexts.get(&vec![x]).unwrap().followers;
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
        assert!(ppm.contexts.is_empty());
        assert!(!ppm.member_contexts.contains_key(&context_member));
        assert!(ppm.context_order.is_empty());
    }
}
