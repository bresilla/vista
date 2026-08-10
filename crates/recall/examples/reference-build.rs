//! Build a reusable reference snapshot from documented commands.
//!
//! Run once. The snapshot it writes is the whole "database": load it at
//! startup in milliseconds instead of replaying the corpus every time.
//!
//!   tools/tldr-pairs.sh --skeleton > skeletons.tsv
//!   cargo run -p vista-recall --release --features snapshot \
//!       --example reference-build -- skeletons.tsv reference.vista

use std::io::BufRead;
use vista_recall::{
    Config, ContainsMatcher, IdentityNormalizer, Item, Observation, Position, Predictor, Query,
    StreamId, WhitespaceTokenizer,
};

/// A reference corpus is larger than a personal history and must not evict.
/// The caller has to use exactly this config to load the snapshot again.
fn reference_config() -> Config {
    Config {
        max_templates: 131_072,
        max_surfaces: 131_072,
        max_contexts: 524_288,
        ..Config::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().unwrap_or_else(|| "skeletons.tsv".into());
    let output = args.next().unwrap_or_else(|| "reference.vista".into());

    let commands: Vec<String> = std::io::BufReader::new(std::fs::File::open(&input)?)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| line.split('\t').nth(1).map(str::to_string))
        .filter(|command| !command.is_empty())
        .collect();
    println!("read {} commands from {input}", commands.len());

    let started = std::time::Instant::now();
    let mut predictor = Predictor::new(reference_config());
    let (mut stream, mut position) = (0_u64, 0_u64);
    for (index, command) in commands.iter().enumerate() {
        // documentation order carries no workflow meaning, so isolate each entry
        stream += 1;
        position = position.max(1);
        let _ = predictor.observe(Observation {
            item: Item::new("command", command.clone()),
            stream: StreamId(stream),
            position: Position(position),
            timestamp: index as i64 + 1,
            context: Vec::new(),
            outcome: Vec::new(),
        });
    }
    let built = started.elapsed();
    let stats = predictor.stats();
    println!(
        "built in {built:?}: {} surfaces, {} tokens, {} retained string bytes",
        stats.surfaces, stats.tokens, stats.retained_string_bytes
    );

    let mut bytes = Vec::new();
    predictor.write_snapshot(&mut bytes)?;
    std::fs::write(&output, &bytes)?;
    println!("wrote {output}: {} bytes", bytes.len());

    let started = std::time::Instant::now();
    let restored = Predictor::read_snapshot(
        reference_config(),
        IdentityNormalizer,
        WhitespaceTokenizer,
        ContainsMatcher,
        std::io::Cursor::new(bytes),
    )?;
    println!("reloaded in {:?}", started.elapsed());

    let query = Query::new(StreamId(u64::MAX), Position(1), 1);
    for broken in ["git chekout my-branch", "systemct restart nginx"] {
        let fixed = restored.predict_aligned(&query, &Item::new("command", broken));
        println!(
            "  {broken}  ->  {}",
            fixed
                .first()
                .map(|p| p.item.value.as_str())
                .unwrap_or("(nothing)")
        );
    }
    Ok(())
}
