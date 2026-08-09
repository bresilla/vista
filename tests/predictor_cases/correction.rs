use super::*;

use vista::{CorrectionAttempt, CorrectionEvaluation, CorrectionMetrics};

const COMMANDS: [&str; 10] = [
    "cargo build --release",
    "cargo test --all-features",
    "git checkout main",
    "git commit --amend",
    "sudo apt install ripgrep",
    "docker compose up --detach",
    "kubectl get pods --all-namespaces",
    "systemctl restart nginx",
    "rsync --archive backup remote",
    "journalctl --unit sshd",
];

/// Drops one character from a word long enough to survive the loss.
fn typo(value: &str, seed: usize) -> String {
    let words: Vec<&str> = value.split_whitespace().collect();
    let index = seed % words.len();
    let word = words[index];
    let mut damaged: Vec<char> = word.chars().collect();
    if damaged.len() > 4 {
        damaged.remove(2 + seed % 2);
    }
    let damaged: String = damaged.into_iter().collect();
    words
        .iter()
        .enumerate()
        .map(|(at, part)| if at == index { &damaged } else { *part })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Damages two neighbouring words, which one pass cannot repair because
/// adjacent tokens never both change together.
fn double_typo(value: &str, seed: usize) -> Option<String> {
    let words: Vec<&str> = value.split_whitespace().collect();
    let at = (0..words.len().saturating_sub(1))
        .find(|index| words[*index].len() > 4 && words[index + 1].len() > 4)?;
    let damaged: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            if index != at && index != at + 1 {
                return (*word).to_owned();
            }
            let mut characters: Vec<char> = word.chars().collect();
            characters.remove(2 + seed % 2);
            characters.into_iter().collect()
        })
        .collect();
    Some(damaged.join(" "))
}

fn history() -> Vec<Observation> {
    let mut observations = Vec::new();
    for round in 0..8_u64 {
        for (index, command) in COMMANDS.iter().enumerate() {
            let position = round * COMMANDS.len() as u64 + index as u64 + 1;
            observations.push(Observation {
                item: Item::new("command", *command),
                stream: StreamId(1),
                position: Position(position),
                timestamp: position as i64,
                context: Vec::new(),
                outcome: vec![Feature::categorical("success", "true")],
            });
        }
    }
    observations
}

/// Forty repairs and forty controls, all synthetic and deterministic.
fn attempts() -> Vec<CorrectionAttempt> {
    let base = 8 * COMMANDS.len() as u64 + 1;
    let mut attempts = Vec::new();
    for seed in 0..40_usize {
        let command = COMMANDS[seed % COMMANDS.len()];
        // Every third opportunity damages two neighbouring words, which one
        // pass cannot repair because adjacent tokens never both change.
        let damaged = match seed % 3 == 2 {
            true => double_typo(command, seed).unwrap_or_else(|| typo(command, seed)),
            false => typo(command, seed),
        };
        attempts.push(CorrectionAttempt::repair(
            StreamId(1),
            Position(base + seed as u64),
            Item::new("command", damaged),
            Item::new("command", command),
        ));
    }
    for seed in 0..40_usize {
        attempts.push(CorrectionAttempt::control(
            StreamId(1),
            Position(base + 100 + seed as u64),
            Item::new("command", COMMANDS[seed % COMMANDS.len()]),
        ));
    }
    attempts
}

fn measure(config: Config) -> CorrectionMetrics {
    CorrectionEvaluation::run(config, history(), attempts()).metrics
}

#[test]
fn the_harness_scores_repairs_and_controls_separately() {
    let metrics = measure(Config::default());
    assert_eq!(metrics.opportunities, 40);
    assert_eq!(metrics.controls, 40);
    assert!(metrics.recall > 0.0, "no repair was ever correct");
    assert!(
        metrics.recall < 0.90,
        "fixture is too easy to judge phases 3-5"
    );
    assert!(metrics.top_3_accuracy >= metrics.top_1_accuracy);
}

#[test]
fn a_control_is_never_repaired_into_something_else() {
    let metrics = measure(Config::default());
    assert_eq!(
        metrics.false_positive_rate, 0.0,
        "already-correct commands must be left alone"
    );
}

#[test]
fn iteration_reaches_repairs_one_pass_cannot() {
    let single = measure(Config {
        max_repair_iterations: 1,
        ..Config::default()
    });
    let iterated = measure(Config {
        max_repair_iterations: 3,
        ..Config::default()
    });
    assert!(
        iterated.recall > single.recall + 0.10,
        "iteration gained {:.3}, expected a two-edit repair to need it",
        iterated.recall - single.recall
    );
    assert!(iterated.precision >= single.precision);
    assert!(iterated.false_positive_rate <= single.false_positive_rate + 0.02);
    assert!(iterated.mean_iterations > single.mean_iterations);
}

#[test]
fn repairs_converge_by_the_second_pass() {
    let two = measure(Config {
        max_repair_iterations: 2,
        ..Config::default()
    });
    let three = measure(Config {
        max_repair_iterations: 3,
        ..Config::default()
    });
    assert_eq!(two.recall, three.recall);
    assert_eq!(two.mean_iterations, three.mean_iterations);
}

#[test]
fn a_weakened_channel_costs_precision() {
    let weakened = measure(Config {
        channel_weight: 0.5,
        ..Config::default()
    });
    let full = measure(Config::default());
    assert!(full.precision > weakened.precision);
    assert!(full.recall > weakened.recall);
}

#[test]
fn the_channel_weight_is_tunable_without_breaking_the_harness() {
    for weight in [0.5, 1.0, 2.0] {
        let metrics = measure(Config {
            channel_weight: weight,
            ..Config::default()
        });
        assert_eq!(metrics.opportunities, 40);
        assert!(metrics.precision.is_finite());
        assert!(metrics.recall.is_finite());
    }
}

#[test]
fn retypings_are_mined_from_failures_that_precede_a_success() {
    let mut predictor = Predictor::new(Config::default());
    let mut position = 0;
    for _ in 0..3 {
        position += 1;
        predictor
            .observe(Observation {
                item: Item::new("command", "git chekout main"),
                stream: StreamId(1),
                position: Position(position),
                timestamp: position as i64,
                context: Vec::new(),
                outcome: vec![Feature::categorical("success", "false")],
            })
            .unwrap();
        position += 1;
        predictor
            .observe(Observation {
                item: Item::new("command", "git checkout main"),
                stream: StreamId(1),
                position: Position(position),
                timestamp: position as i64,
                context: Vec::new(),
                outcome: vec![Feature::categorical("success", "true")],
            })
            .unwrap();
    }
    let mined = predictor.corrections();
    assert_eq!(mined.len(), 1);
    assert_eq!(mined[0].0.typed.value, "git chekout main");
    assert_eq!(mined[0].0.corrected.value, "git checkout main");
    assert_eq!(mined[0].1, 3);
    assert_eq!(predictor.stats().correction_pairs, 1);
}
