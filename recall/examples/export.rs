use std::io::BufRead;
use std::path::Path;

use vista_recall::{Item, Observation, Position, ResearchExport, StreamId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let history = std::env::args().nth(1).expect("history path");
    let output = std::env::args().nth(2).expect("output directory");
    let file = std::fs::File::open(history)?;
    let observations = std::io::BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.expect("read history line");
            (!line.trim().is_empty()).then(|| Observation {
                item: Item::new("sentence", line),
                stream: StreamId(1),
                position: Position(index as u64 + 1),
                timestamp: index as i64,
                context: Vec::new(),
                outcome: Vec::new(),
            })
        });
    let export = ResearchExport::from_observations(observations)?;
    std::fs::create_dir_all(&output)?;
    export.write_spmf(std::fs::File::create(
        Path::new(&output).join("sequences.spmf"),
    )?)?;
    export.write_plain(std::fs::File::create(
        Path::new(&output).join("sequences.txt"),
    )?)?;
    export.write_dictionary(std::fs::File::create(
        Path::new(&output).join("dictionary.tsv"),
    )?)?;
    Ok(())
}
