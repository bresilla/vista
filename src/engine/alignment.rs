const MAX_TOKENS: usize = 64;
const MAX_TOKEN_CHARS: usize = 64;
const TYPO_SIMILARITY: f64 = 0.5;

/// Rebuilds `candidate`'s structure around `source`'s own arguments.
///
/// Tokens shared by both are structure and tokens only the candidate has are
/// the repair. Tokens that differ are decided by `known`, which reports whether
/// history has ever produced that token: a token history recognises is one the
/// caller meant, and only an unrecognised token is judged on how closely it
/// resembles the observed one. The split between structure and argument comes
/// out of the alignment, not from any description of the input's syntax.
pub(crate) fn repair(
    source: &str,
    candidate: &str,
    known: &dyn Fn(&str) -> bool,
) -> Option<String> {
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
            known,
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

fn resolve<'a>(
    source: &[&'a str],
    candidate: &[&'a str],
    known: &dyn Fn(&str) -> bool,
    repaired: &mut Vec<&'a str>,
) {
    if source.is_empty() {
        repaired.extend_from_slice(candidate);
        return;
    }
    if candidate.is_empty() || source.len() != candidate.len() {
        repaired.extend_from_slice(source);
        return;
    }
    for (typed, observed) in source.iter().zip(candidate) {
        let misspelled = !known(typed) && similarity(typed, observed) >= TYPO_SIMILARITY;
        repaired.push(if misspelled { observed } else { typed });
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

    fn nothing_known(_: &str) -> bool {
        false
    }

    fn vocabulary(tokens: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |token| tokens.contains(&token)
    }

    #[test]
    fn inserted_tokens_are_kept_and_arguments_survive() {
        let repaired = repair("apt install ripgrep", "sudo apt install fd", &nothing_known);
        assert_eq!(repaired.as_deref(), Some("sudo apt install ripgrep"));
    }

    #[test]
    fn a_misspelling_is_corrected_while_a_new_argument_is_preserved() {
        let repaired = repair("git chekout feature", "git checkout main", &nothing_known);
        assert_eq!(repaired.as_deref(), Some("git checkout feature"));
    }

    #[test]
    fn unrelated_candidates_leave_the_source_untouched() {
        let repaired = repair(
            "apt install ripgrep",
            "cargo build --release",
            &nothing_known,
        );
        assert_eq!(repaired.as_deref(), Some("apt install ripgrep"));
    }

    #[test]
    fn a_token_history_recognises_is_never_treated_as_a_misspelling() {
        let seen = vocabulary(&["git", "checkout", "main", "maim"]);
        assert_eq!(
            repair("git checkout maim", "git checkout main", &seen).as_deref(),
            Some("git checkout maim"),
        );
        assert_eq!(
            repair("git checkout maim", "git checkout main", &nothing_known).as_deref(),
            Some("git checkout main"),
        );
    }

    #[test]
    fn oversized_and_empty_inputs_are_rejected() {
        let long = "x ".repeat(MAX_TOKENS + 1);
        assert_eq!(repair(&long, "ls", &nothing_known), None);
        assert_eq!(repair("ls", "   ", &nothing_known), None);
    }
}
