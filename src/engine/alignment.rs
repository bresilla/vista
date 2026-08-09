const MAX_TOKENS: usize = 64;
const MAX_TOKEN_CHARS: usize = 64;
const TYPO_SIMILARITY: f64 = 0.5;

/// Rebuilds `candidate`'s structure around `source`'s own arguments.
///
/// Tokens shared by both are structure, tokens only the candidate has are the
/// repair, and tokens that differ are resolved by how much they resemble each
/// other: near-identical tokens are one word misspelled, unrelated tokens are
/// caller arguments that must survive. Nothing here is configured or authored;
/// the split falls out of the two strings.
pub(crate) fn repair(source: &str, candidate: &str) -> Option<String> {
    let source: Vec<&str> = source.split_whitespace().collect();
    let candidate: Vec<&str> = candidate.split_whitespace().collect();
    if source.is_empty() || candidate.is_empty() {
        return None;
    }
    if source.len() > MAX_TOKENS || candidate.len() > MAX_TOKENS {
        return None;
    }

    let mut repaired = Vec::new();
    let (mut consumed_source, mut consumed_candidate) = (0, 0);
    let ends = [(source.len(), candidate.len())];
    for (at_source, at_candidate) in common_subsequence(&source, &candidate)
        .into_iter()
        .chain(ends)
    {
        resolve(
            &source[consumed_source..at_source],
            &candidate[consumed_candidate..at_candidate],
            &mut repaired,
        );
        if at_source < source.len() {
            repaired.push(source[at_source]);
        }
        consumed_source = at_source + 1;
        consumed_candidate = at_candidate + 1;
    }
    Some(repaired.join(" "))
}

fn resolve<'a>(source: &[&'a str], candidate: &[&'a str], repaired: &mut Vec<&'a str>) {
    if source.is_empty() {
        repaired.extend_from_slice(candidate);
        return;
    }
    if candidate.is_empty() || source.len() != candidate.len() {
        repaired.extend_from_slice(source);
        return;
    }
    for (typed, observed) in source.iter().zip(candidate) {
        repaired.push(if similarity(typed, observed) >= TYPO_SIMILARITY {
            observed
        } else {
            typed
        });
    }
}

/// Indices of the longest common token subsequence, as `(source, candidate)`.
fn common_subsequence(source: &[&str], candidate: &[&str]) -> Vec<(usize, usize)> {
    let mut lengths = vec![vec![0_usize; candidate.len() + 1]; source.len() + 1];
    for left in (0..source.len()).rev() {
        for right in (0..candidate.len()).rev() {
            lengths[left][right] = if source[left] == candidate[right] {
                lengths[left + 1][right + 1] + 1
            } else {
                lengths[left + 1][right].max(lengths[left][right + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut left, mut right) = (0, 0);
    while left < source.len() && right < candidate.len() {
        if source[left] == candidate[right] {
            pairs.push((left, right));
            left += 1;
            right += 1;
        } else if lengths[left + 1][right] >= lengths[left][right + 1] {
            left += 1;
        } else {
            right += 1;
        }
    }
    pairs
}

/// Edit distance over bounded tokens, scaled to zero-to-one.
fn similarity(left: &str, right: &str) -> f64 {
    let left: Vec<char> = left.chars().take(MAX_TOKEN_CHARS).collect();
    let right: Vec<char> = right.chars().take(MAX_TOKEN_CHARS).collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
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
        std::mem::swap(&mut previous, &mut current);
    }
    1.0 - previous[right.len()] as f64 / left.len().max(right.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_tokens_are_kept_and_arguments_survive() {
        let repaired = repair("apt install ripgrep", "sudo apt install fd");
        assert_eq!(repaired.as_deref(), Some("sudo apt install ripgrep"));
    }

    #[test]
    fn a_misspelling_is_corrected_while_a_new_argument_is_preserved() {
        let repaired = repair("git chekout feature", "git checkout main");
        assert_eq!(repaired.as_deref(), Some("git checkout feature"));
    }

    #[test]
    fn unrelated_candidates_leave_the_source_untouched() {
        let repaired = repair("apt install ripgrep", "cargo build --release");
        assert_eq!(repaired.as_deref(), Some("apt install ripgrep"));
    }

    #[test]
    fn oversized_and_empty_inputs_are_rejected() {
        let long = "x ".repeat(MAX_TOKENS + 1);
        assert_eq!(repair(&long, "ls"), None);
        assert_eq!(repair("ls", "   "), None);
    }
}
