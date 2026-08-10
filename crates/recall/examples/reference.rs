//! Repairing against a reference corpus as well as personal history.
//!
//! Two predictors, no coupling. The personal one answers `predict` and repairs
//! what the caller has done before. The reference one is built from documented
//! commands the caller may never have run, and only ever repairs. Because
//! repair composes rather than retrieves, the reference can supply a correct
//! spelling for a command that appears in neither corpus.
//!
//!   cargo run -p vista-recall --example reference

use vista_recall::{Config, Item, Observation, Position, Predictor, Query, StreamId};

/// Documented commands, as `tools/tldr-pairs.sh --skeleton` would emit them.
const REFERENCE: [&str; 6] = [
    "git checkout --force",
    "git commit --message",
    "pkill --signal",
    "whereis -bm",
    "systemctl restart",
    "docker compose up --detach",
];

/// What this caller has actually run.
const PERSONAL: [&str; 4] = [
    "cargo build --release",
    "cargo test --all-features",
    "hexe mux float --help",
    "git status",
];

fn build(commands: &[&str], per_stream: usize) -> Predictor {
    let mut predictor = Predictor::new(Config::default());
    let (mut stream, mut position) = (0u64, 0u64);
    for (index, command) in commands.iter().enumerate() {
        if index % per_stream == 0 {
            stream += 1;
            position = 0;
        }
        position += 1;
        let _ = predictor.observe(Observation {
            item: Item::new("command", *command),
            stream: StreamId(stream),
            position: Position(position),
            timestamp: index as i64 + 1,
            context: Vec::new(),
            outcome: Vec::new(),
        });
    }
    predictor
}

fn main() {
    let personal = build(&PERSONAL, 8);
    // order carries no meaning in a reference corpus, so keep the streams short
    let reference = build(&REFERENCE, 1);

    // no stream continuity is needed; repair retrieves and aligns
    let query = Query::new(StreamId(u64::MAX), Position(1), 3);

    for broken in ["git chekout my-branch", "pkil --signal", "wheris hexa"] {
        let item = Item::new("command", broken);
        let mine = personal.predict_aligned(&query, &item);
        let theirs = reference.predict_aligned(&query, &item);

        println!("typed:     {broken}");
        println!(
            "  personal:  {}",
            mine.first()
                .map(|p| p.item.value.as_str())
                .unwrap_or("(nothing)")
        );
        match theirs.first() {
            Some(fixed) => println!(
                "  reference: {}   via {}",
                fixed.item.value, fixed.template.value
            ),
            None => println!("  reference: (nothing)"),
        }
        println!();
    }
}
