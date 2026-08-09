use super::*;

fn shell(stream: u64, position: u64, value: &str) -> Observation {
    Observation {
        item: Item::new("command", value),
        stream: StreamId(stream),
        position: Position(position),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

/// No normalizer, no matcher, no configuration: history is the only input.
fn history_predictor(history: &[&str]) -> Predictor {
    let mut predictor = Predictor::new(Config::default());
    predictor
        .replay(
            history
                .iter()
                .enumerate()
                .map(|(index, value)| shell(1, index as u64 + 1, value)),
        )
        .unwrap();
    predictor
}

#[test]
fn a_missing_token_is_restored_around_your_own_argument() {
    let predictor = history_predictor(&[
        "sudo apt install fd",
        "sudo apt install jq",
        "sudo apt install bat",
    ]);
    let failed = Item::new("command", "apt install ripgrep");

    let aligned = predictor.predict_aligned(&query(1, 4, 3), &failed);

    assert_eq!(aligned[0].item.value, "sudo apt install ripgrep");
    assert!(
        !values(&aligned).iter().any(|value| value.contains("fd")),
        "a historical argument leaked into the repair"
    );
}

#[test]
fn a_typo_is_fixed_while_a_new_argument_is_kept() {
    let predictor = history_predictor(&["git checkout main", "git checkout develop", "git status"]);
    let failed = Item::new("command", "git chekout feature");

    let aligned = predictor.predict_aligned(&query(1, 4, 3), &failed);

    assert_eq!(aligned[0].item.value, "git checkout feature");
}

#[test]
fn the_default_matcher_does_not_block_a_repair() {
    let predictor = history_predictor(&["cargo build --release", "cargo test --all-features"]);
    let failed = Item::new("command", "cargo biuld --release");

    let aligned = predictor.predict_aligned(&query(1, 3, 3), &failed);

    assert!(
        aligned
            .iter()
            .any(|prediction| prediction.item.value == "cargo build --release"),
        "alignment must not inherit the substring gate, got {:?}",
        values(&aligned)
    );
}

#[test]
fn plain_prediction_still_honours_the_matcher() {
    let predictor = history_predictor(&["cargo build --release", "cargo test --all-features"]);
    let mut typo = query(1, 3, 5);
    typo.partial = Some("cargo biuld --release".into());

    assert!(predictor.predict(&typo).is_empty());
}

#[test]
fn repairs_are_distinct_and_never_echo_the_source() {
    let predictor = history_predictor(&[
        "sudo apt install fd",
        "sudo apt install jq",
        "sudo apt install bat",
        "apt install hexyl",
    ]);
    let failed = Item::new("command", "apt install ripgrep");

    let aligned = predictor.predict_aligned(&query(1, 5, 8), &failed);
    let repairs = values(&aligned);

    assert!(!repairs.contains(&failed.value.as_str()));
    let mut unique = repairs.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(repairs.len(), unique.len());
}

#[test]
fn history_decides_what_counts_as_a_misspelling() {
    let unseen = history_predictor(&["git checkout main", "git checkout main"]);
    let typo = Item::new("command", "git checkout maim");
    assert_eq!(
        unseen.predict_aligned(&query(1, 3, 3), &typo)[0].item.value,
        "git checkout main",
        "an unrecognised token close to an observed one is a misspelling"
    );

    let seen = history_predictor(&["git checkout main", "git checkout maim"]);
    assert!(
        seen.predict_aligned(&query(1, 3, 3), &typo)
            .iter()
            .all(|prediction| prediction.item.value != "git checkout main"),
        "a token history has produced is never rewritten"
    );
}

#[test]
fn an_empty_history_suggests_nothing() {
    let predictor = Predictor::new(Config::default());
    let failed = Item::new("command", "apt install ripgrep");
    assert!(
        predictor
            .predict_aligned(&query(1, 1, 5), &failed)
            .is_empty()
    );
}

#[test]
fn sequence_context_orders_the_repairs() {
    let predictor = history_predictor(&[
        "git pull",
        "cargo build --release",
        "git pull",
        "cargo build --release",
        "git pull",
    ]);
    let failed = Item::new("command", "cargo biuld --release");

    let aligned = predictor.predict_aligned(&query(1, 6, 3), &failed);
    assert_eq!(aligned[0].item.value, "cargo build --release");
}
