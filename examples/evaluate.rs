use std::io::BufRead;

use vista::{
    Baseline, Config, Evaluation, Item, Observation, Position, SnapshotMeasurement, StreamId,
};

fn observation(position: u64, value: &str) -> Observation {
    Observation {
        item: Item::new("command", value),
        stream: StreamId(1),
        position: Position(position),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

fn synthetic() -> Vec<Observation> {
    (1..=2_000)
        .map(|position| {
            observation(
                position,
                match position % 4 {
                    0 => "git push",
                    1 => "git status",
                    2 => "git add .",
                    _ => "git commit",
                },
            )
        })
        .collect()
}

fn history(path: &str) -> impl Iterator<Item = Observation> {
    let file = std::fs::File::open(path).expect("open history");
    std::io::BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.expect("read history line");
            (!line.trim().is_empty()).then(|| observation(index as u64 + 1, &line))
        })
}

fn main() {
    let path = std::env::args().nth(1);
    let report = match path.as_deref() {
        Some(path) => Evaluation::run_ordered(Config::default(), history(path)),
        None => Evaluation::run_ordered(Config::default(), synthetic()),
    };
    let fixed1 = &report.baselines[&Baseline::FixedOrder1];
    let fixed3 = &report.baselines[&Baseline::FixedOrder3];
    let frequent = &report.baselines[&Baseline::MostFrequent];
    let metrics = &report.variable_order;
    println!("corpus={}", path.as_deref().unwrap_or("synthetic"));
    println!("observations={}", metrics.observations);
    println!("predictions={}", metrics.predictions);
    println!("top1={:.4}", metrics.top_1_accuracy);
    println!("top3={:.4}", metrics.top_3_accuracy);
    println!("top5={:.4}", metrics.top_5_accuracy);
    println!("top10={:.4}", metrics.top_10_accuracy);
    println!("mrr={:.4}", metrics.mean_reciprocal_rank);
    println!("log_loss={:.6}", metrics.mean_log_loss);
    println!("perplexity={:.6}", metrics.perplexity);
    println!("cold_start_top1={:.4}", metrics.cold_start_accuracy);
    println!("cold_start_log_loss={:.6}", metrics.cold_start_log_loss);
    println!("candidate_recall={:.4}", metrics.candidate_recall);
    println!("coverage={:.4}", metrics.coverage);
    println!("macro_stream_top1={:.4}", metrics.macro_stream_accuracy);
    println!("mean_context_depth={:.4}", metrics.mean_context_depth);
    println!("max_context_depth={}", metrics.max_context_depth);
    println!("templates={}", metrics.templates);
    println!("surfaces={}", metrics.surfaces);
    println!("streams={}", metrics.streams);
    println!("contexts={}", metrics.contexts);
    println!("followers={}", metrics.followers);
    println!("zero_order_entries={}", metrics.zero_order_entries);
    println!("cache_entries={}", metrics.cache_entries);
    println!("stream_history_entries={}", metrics.stream_history_entries);
    println!("context_associations={}", metrics.context_associations);
    println!("tokens={}", metrics.tokens);
    println!("token_associations={}", metrics.token_associations);
    println!("partial_associations={}", metrics.partial_associations);
    println!("heap_bytes={}", metrics.estimated_heap_bytes);
    match &metrics.snapshot {
        SnapshotMeasurement::Success { bytes, load_time } => {
            println!("snapshot_status=success");
            println!("snapshot_bytes={bytes}");
            println!("snapshot_load_us={}", load_time.as_micros());
        }
        SnapshotMeasurement::Failed { stage, error } => {
            println!("snapshot_status=failed");
            println!("snapshot_stage={stage:?}");
            println!("snapshot_error={error}");
        }
        SnapshotMeasurement::NotMeasured => println!("snapshot_status=not-measured"),
    }
    println!(
        "mean_predict_us={}",
        metrics.mean_prediction_latency.as_micros()
    );
    println!("mean_update_us={}", metrics.mean_update_latency.as_micros());
    println!(
        "p50_predict_us={}",
        metrics.p50_prediction_latency.as_micros()
    );
    println!(
        "p95_predict_us={}",
        metrics.p95_prediction_latency.as_micros()
    );
    println!(
        "p99_predict_us={}",
        metrics.p99_prediction_latency.as_micros()
    );
    println!("p50_update_us={}", metrics.p50_update_latency.as_micros());
    println!("p95_update_us={}", metrics.p95_update_latency.as_micros());
    println!("p99_update_us={}", metrics.p99_update_latency.as_micros());
    println!("normalization_ratio={:.4}", metrics.normalization_ratio);
    println!("saved_characters={}", metrics.completion_saved_characters);
    println!("mean_saved_characters={:.4}", metrics.mean_saved_characters);
    println!("fixed3_top5={:.4}", fixed3.top_5_accuracy);
    println!("fixed1_log_loss={:.6}", fixed1.mean_log_loss);
    println!("frequent_log_loss={:.6}", frequent.mean_log_loss);
}
