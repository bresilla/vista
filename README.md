# vista

Vista is a deterministic, bounded next-sentence predictor for commands,
workflow steps, actions, and tool calls. It learns from chronological history
and returns concrete items that were observed previously. It does not generate
unseen text and does not use an LLM, neural network, embedding service, or async
runtime.

## Model

Vista has one prediction path:

1. a `Normalizer` maps each raw item to a reusable template;
2. a variable-order PPM-style model learns template sequences up to order 8 by
   default;
3. an integrated recent-history cache adapts to changing behavior;
4. the predicted templates expand into concrete historical surfaces;
5. context, outcomes, and partial input make bounded ranking adjustments.

Vista has no trained neural weights or separate weight file. Its learned state
is the template dictionary, PPM counts, surface statistics, retrieval indexes,
and recent cache stored together in a compiled snapshot. The values in
`Config::weights` are fixed presentation coefficients, not learned parameters.

The default crate build contains the live predictor, recent cache, explanations,
snapshot persistence, and surface retrieval indexes. Disable default features
for the smallest sequence-only core, then enable only what the host needs:
`recent-cache` for short-term adaptation, `snapshot` for persistence,
`explanations` for rendered reasons, `evaluation` for metrics and baselines, and
`research` for external-model export. `surface-indexes` enables caller-context,
token, and character-fragment retrieval. Evaluation and research code never
enters an embedded consumer unless requested.

Long contexts back off automatically when evidence is sparse. A position gap or
`break_stream` resets sequence continuity, so events on opposite sides of a
hidden or removed observation are never joined.

## Basic use

```rust
use vista::{Config, Item, Observation, Position, Predictor, Query, StreamId};

fn event(position: u64, value: &str) -> Observation {
    Observation {
        item: Item::new("command", value),
        stream: StreamId(7),
        position: Position(position),
        timestamp: position as i64,
        context: vec![],
        outcome: vec![],
    }
}

let mut predictor = Predictor::new(Config::default());
predictor.replay([
    event(1, "build the project"),
    event(2, "run the tests"),
    event(3, "build the project"),
]);

let predictions = predictor.predict(&Query::new(StreamId(7), Position(4), 3));
assert_eq!(predictions[0].item.value, "run the tests");
assert!(predictions[0].probability > 0.0);
```

`Prediction::probability` is the next-template probability before presentation
adjustments. `Prediction::score` is the final ranking value. With the
`explanations` feature, `Prediction::explanation` reports the matched context
depth, backoff, recent-cache evidence, and relevant ranking adjustments.
`Prediction::context_depth` is always available without rendered strings.

## Tiny embedding

Use the sequence-only feature set and strict runtime bounds when footprint is
more important than retrieval recall:

```toml
[dependencies]
vista = { path = "../vista", default-features = false }

[profile.release]
opt-level = "z"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
```

```rust
# use vista::{Config, Predictor};
let predictor = Predictor::new(Config::tiny());
# let _ = predictor;
```

Cargo ignores release profiles declared by dependencies, so the final host
package must repeat the profile settings. `panic = "abort"` is a host-wide
choice and should be omitted if the application requires unwinding.

`Config::tiny()` limits retained model state; it does not change linked code
size. The no-default-features build keeps the dictionary, bounded PPM sequence
learning, concrete historical surfaces, probability queries, matching, and
forgetting. It omits recent-cache adaptation, snapshots, explanation strings,
and caller-context/token/fragment retrieval. Features can be added back one at
a time.

On Rust 1.97.1 for x86-64 Linux, `make size` links the minimal live
learn-and-predict example at 449,088 bytes. The same release profile links an
empty Rust example at 285,880 bytes, giving a same-build approximate Vista
increment of 163,208 bytes. These are toolchain and target measurements, not
portable guarantees. `make size-check` enforces a 475,000-byte regression
ceiling, which can be changed with `MAX_EMBEDDED_BYTES=...`; `make size-full`
measures the all-feature build.

`make bench-tiny` feeds 100,000 observations through the sequence-only tiny
configuration. The current synthetic run retains 64 templates, 64 surfaces,
192 contexts, and 192 followers with a 52,504-byte model heap estimate. The
estimate describes retained structures, not allocator RSS, and real data can
produce a different mix up to the configured hard bounds.

## Template normalization

The default `IdentityNormalizer` treats each raw item as its own template. For
high-cardinality history, provide a domain normalizer through `Predictor::builder`:

```rust
# use vista::{Config, Feature, Item, NormalizedItem, Normalizer, Predictor};
struct Commands;

impl Normalizer for Commands {
    fn normalize(&self, item: &Item) -> NormalizedItem {
        if let Some(target) = item.value.strip_prefix("ssh ") {
            NormalizedItem {
                template: Item::new(item.namespace.clone(), "ssh {target}"),
                slots: vec![Feature::categorical("target", target)],
            }
        } else {
            NormalizedItem { template: item.clone(), slots: vec![] }
        }
    }
}

let predictor = Predictor::builder(Config::default())
    .normalizer(Commands)
    .build();
# let _ = predictor;
```

Vista learns transitions between templates while retaining raw surfaces such as
`ssh alice@host1` and `ssh bob@host2`. See `make run EXAMPLE=template`.

The library deliberately does not contain a generic shell parser. Applications
know which paths, hosts, branches, identifiers, and secrets are safe to retain
or replace. Vista retains at most 1,024 normalized slots per surface so a
normalizer cannot create an unbounded snapshot section.

## Million-line histories

Use `Trainer` to ingest observations one at a time. It never retains the source
observation collection:

```rust
# use vista::{Config, Trainer};
let mut trainer = Trainer::new(Config::default());
// trainer.observe(observation);
let predictor = trainer.finish();
# let _ = predictor;
```

Use `Trainer::from_builder(Predictor::builder(...).normalizer(...))` to stream
with application adapters.

Custom `Tokenizer` implementations should implement both `tokens` for observed
items and `query_tokens` for partial input so retrieval uses the same token
scheme in both directions.

Hot indexes use compact template and surface identifiers instead of cloning
complete strings into every context. All collections are bounded by `Config`.
Prediction consults only bounded suffix, cache, context, and candidate indexes;
it does not scan the original history.

Run the release-mode one-million-event ingestion and snapshot check with:

```sh
make bench-million
```

The target prints ingestion throughput, prediction percentiles, retained model
counts, estimated heap bytes, snapshot bytes, and snapshot load time. These are
measurements for the current machine, not fixed performance promises.

## Snapshots

Enable the `snapshot` Cargo feature for this API.

Compiled snapshots avoid replaying the full history after restart:

```rust
# use std::io::Cursor;
# use vista::{Config, ContainsMatcher, IdentityNormalizer, Predictor, WhitespaceTokenizer};
# let predictor = Predictor::new(Config::default());
let mut bytes = Vec::new();
predictor.write_snapshot(&mut bytes)?;

let restored = Predictor::read_snapshot(
    Config::default(),
    IdentityNormalizer,
    WhitespaceTokenizer,
    ContainsMatcher,
    Cursor::new(bytes),
)?;
# let _ = restored;
# Ok::<(), vista::SnapshotError>(())
```

Loading is linear in the retained snapshot structures, not in the number of
source history lines. Consequently, a million-line history that normalizes into
a compact model can load quickly without rereading those million lines. Disk
I/O is additional to the snapshot decoding time printed by the in-memory
benchmark.

The format has a magic value, version, feature flags, normalized configuration,
configuration fingerprint, checked section lengths, reference validation, and
checksum. Identical state produces identical bytes. Corrupt, truncated,
oversized, incompatible, or trailing data is rejected before a predictor is
returned.

Version 1 is the only supported format. A different version, unknown feature
bit, adapter key, or normalized configuration is rejected rather than guessed.

Snapshots also record each adapter's `snapshot_key`. Stateful normalizers,
tokenizers, or matchers must override that method so the key changes whenever
configuration that affects behavior changes. Loading with a different key is
rejected. Snapshot loading normalizes every retained surface again and rejects
any template or slot mismatch, so normalization must be deterministic.

Vista reads and writes streams, not paths. The caller owns file permissions,
temporary-file creation, flushing, `fsync`, and atomic rename.

## Configuration

| Setting | Default | Tiny | Purpose |
|---|---:|---:|---|
| `max_templates` | 16,384 | 64 | Retained normalized sequence symbols |
| `max_surfaces` | 32,768 | 128 | Retained concrete historical items |
| `max_streams` | 256 | 4 | Independent live sequence histories |
| `max_order` | 8 | 3 | Longest learned suffix context, clamped to 1–32 |
| `max_contexts` | 262,144 | 256 | Retained variable-order states |
| `max_followers_per_context` | 64 | 4 | Followers retained per state |
| `max_context_associations` | 65,536 | 128 | Caller context-to-surface associations |
| `max_tokens` | 32,768 | 64 | Retrieval token vocabulary |
| `max_partial_chars_per_item` | 512 | 64 | Surface characters indexed for partial input |
| `max_partial_associations` | 65,536 | 128 | Character-fragment associations |
| `max_candidate_templates` | 128 | 12 | Templates considered per query |
| `max_surface_candidates_per_template` | 8 | 3 | Surfaces expanded per template |
| `max_candidates` | 128 | 12 | Concrete candidates ranked per query |
| `recent_cache_items` | 256 | 16 | Recent items retained per cache |
| `recent_cache_weight` | 0.20 | 0.20 | PPM/cache interpolation, clamped to 0–0.5 |
| `recent_cache_half_life` | 32 | 16 | Observation-age decay |
| `weights.context` | 0.35 | 0.35 | Bounded context-ratio presentation adjustment |
| `weights.surface` | 0.20 | 0.20 | Bounded surface frequency/recency adjustment |
| `weights.outcome` | 0.15 | 0.15 | Bounded observed-outcome adjustment |
| `weights.partial` | 0.50 | 0.50 | Bounded partial-match adjustment |

Small bounds reduce memory but can reduce candidate recall. Use `Evaluation` on
real chronological history before choosing production limits. Zero count limits
normalize to one, `max_order` normalizes to 1–32, identifier counts cannot exceed
`u32::MAX`, and non-finite weights revert to defaults. Presentation weights are
clamped to -10–10; the recent-cache weight is clamped to 0–0.5.
Recent-cache decay is exact at half-life boundaries and linearly interpolated
between them to avoid linking a general power function in small binaries.

## Evaluation and research comparisons

`Evaluation::run_ordered` performs prequential evaluation: predict, score, then
learn. It reports top-1/3/5/10 accuracy, MRR, candidate recall, coverage,
log-loss, perplexity, cold-start accuracy/log-loss, macro stream accuracy,
context depth, prediction/update latency percentiles, memory, snapshot size/load
time, normalization ratio, and simulated completion keystroke savings.
Latency percentiles use a fixed 65-bucket logarithmic histogram, so ordered
evaluation retains bounded metric state instead of one duration per event.

```sh
make evaluate
make evaluate EXAMPLE=tests/fixtures/workflow.txt
make evaluate HISTORY=/path/to/one-sentence-per-line.txt
```

Blank lines are skipped while retaining their positions as sequence gaps. The
library does not parse shell-specific timestamp prefixes; sanitize and convert
application history before evaluating it.

`ResearchExport` writes deterministic integer sequences for external CPT+,
IPredict PPM/AKOM, and PBCT comparisons. See `tools/README.md`. External research
projects are not downloaded by the build and are not runtime dependencies.

## Retention and privacy

Vista owns only derived in-memory model state. The caller owns persistence,
sanitization, retention, and consent. For durable forgetting:

1. remove the observations from application storage;
2. preserve their position gaps;
3. call `forget` for immediate in-memory removal;
4. create a new snapshot, or replay the sanitized source history when complete
   derived-state reconstruction is required.

Removing or evicting an item clears affected live histories instead of joining
its neighbors.

## Development

```sh
make build
make run
make test
make verify
make evaluate
make bench-million
```
