use std::collections::BTreeMap;

/// Drops the lowest counts until `limit` entries remain. Ties break by key.
pub(crate) fn prune_counts<K: Clone + Ord>(counts: &mut BTreeMap<K, u64>, limit: usize) {
    while counts.len() > limit {
        let Some(key) = weakest(counts) else { break };
        counts.remove(&key);
    }
}

pub(crate) fn prune_counts_removed<K: Clone + Ord>(
    counts: &mut BTreeMap<K, u64>,
    limit: usize,
) -> Vec<K> {
    let mut removed = Vec::new();
    while counts.len() > limit {
        let Some(key) = weakest(counts) else { break };
        counts.remove(&key);
        removed.push(key);
    }
    removed
}

fn weakest<K: Clone + Ord>(counts: &BTreeMap<K, u64>) -> Option<K> {
    counts
        .iter()
        .min_by_key(|(key, count)| (**count, *key))
        .map(|(key, _)| key.clone())
}
