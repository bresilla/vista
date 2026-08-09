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

Ingestion is fallible: `observe` and `replay` return `Result<(), InputError>`.
Vista validates raw items, normalizer output, features, tokens, and derived
index keys before changing the model. Rejected observations leave predictions,
statistics, clocks, and snapshot bytes unchanged.

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

`StreamId` is a sequence-continuity boundary, not a tenant or privacy boundary.
Direct transitions never cross streams, while aggregate PPM evidence and the
global recent-cache fallback may inform predictions in another stream. Gaps and
`break_stream` discard only the affected stream's private continuity. Hosts
must use separate `Predictor` instances for separate users, tenants, or privacy
domains.

## Architecture

Vista stays one Cargo library so its feature gates, private model state, and
root API remain coordinated without cross-crate plumbing. The implementation is
split into internal domains: `api` holds public data types, `adapters` holds
normalization and matching interfaces, `model` owns retained state, `engine`
runs learning and prediction, and the feature-gated `snapshot`, `evaluation`,
and `research` domains provide optional tooling. These paths are implementation
details; consumers should import the public types re-exported from `vista`.

Every maintained repository file is limited to 600 physical lines. `make
loc-check` enforces the limit for tracked and nonignored untracked files, and is
part of `make verify`.

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
])?;

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

On Rust 1.97.1 for `x86_64-unknown-linux-gnu`, `make size` links the minimal
live learn-and-predict example at 447,280 bytes. The same release profile links
an empty Rust example at 285,864 bytes, giving a same-build approximate Vista
increment of 161,416 bytes. The all-feature example is 552,056 bytes. These are
toolchain and target measurements, not portable guarantees. `make size-check`
enforces a 475,000-byte native regression ceiling, which can be changed with
`MAX_EMBEDDED_BYTES=...`; `make size-full` measures the all-feature build.

For `x86_64-unknown-linux-musl`, the corresponding empty, minimal, incremental,
and all-feature measurements are 373,216, 541,152, 167,936, and 647,648 bytes.
Use `make check-musl` and `make size-musl`. Vista itself has no native
dependencies, but a consuming application's dependencies and build scripts
determine whether its final executable remains fully statically linked.

`make bench-tiny` feeds 100,000 observations through the sequence-only tiny
configuration. The current synthetic run retains 64 templates, 64 surfaces,
192 contexts, and 192 followers with a 75,544-byte model heap estimate. The
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

## Repairing an item from history

`predict` returns surfaces exactly as they were observed, so a predicted shape
arrives carrying whichever arguments history retained. `predict_aligned` instead
rebuilds each candidate's structure around the arguments of the item you pass
in, using nothing but the two strings:

```rust
# use vista::{Config, Item, Predictor, Position, Query, StreamId};
# let predictor = Predictor::new(Config::default());
let failed = Item::new("command", "apt install ripgrep");
let query = Query::new(StreamId(7), Position(4), 3);
for prediction in predictor.predict_aligned(&query, &failed) {
    // history holds `sudo apt install fd`, so the repair is
    // `sudo apt install ripgrep`, never `sudo apt install fd`
    println!("{}", prediction.item.value);
}
```

Tokens shared by both sides are structure, tokens only the candidate carries are
the repair, and differing tokens are resolved by how much they resemble each
other: near-identical tokens are one word misspelled, unrelated tokens are your
arguments. `git chekout feature` against `git checkout main` yields
`git checkout feature` — the misspelling corrected, the argument kept.

Nothing is configured or authored. There is no normalizer, no template, no rule,
and no threshold to tune; the split between structure and argument comes out of
the alignment. Candidates that share no structure repair to the source unchanged
and are dropped, so alignment is also its own filter and the candidate matcher is
not applied on this path. `Prediction::template` still identifies which history
matched, and `Prediction::probability` still orders repairs by what usually
follows in this stream.

## Rendered completions

When an application does supply a normalizer, `predict_rendered` ranks templates
instead, returns each one once, and refills it from the slots of the item being
completed:

```rust
# use vista::{Config, Item, Predictor, Position, Query, SimilarityMatcher, StreamId};
# let predictor = Predictor::builder(Config::default())
#     .matcher(SimilarityMatcher::default())
#     .build();
let failed = Item::new("command", "apt install ripgrep");
let query = Query::new(StreamId(7), Position(4), 3);
for prediction in predictor.predict_rendered(&query, &failed) {
    // template `sudo apt install {pkg}` renders as `sudo apt install ripgrep`,
    // not as the historical surface `sudo apt install fd`
    println!("{}", prediction.item.value);
}
```

Implement `Normalizer::render` to enable this. The default accepts a template
only when the source produced no slots, so a normalizer without an inverse
never emits a half-filled template. Rendering that leaves a slot unfilled should
return `None`, which drops that template from the results.

Matching also changes. `CandidateMatcher::score_match` receives both the source
template and the candidate template through `MatchInput`, because concrete
arguments dominate similarity between otherwise identical commands:

```text
"apt install ripgrep" vs "sudo apt install fd"      → rejected
"apt install {pkg}"   vs "sudo apt install {pkg}"   → accepted
```

The default `score_match` ignores templates and defers to `score`, so existing
matchers are unaffected. `SimilarityMatcher` compares character trigrams,
preferring templates when they are available, and returns `None` below its
threshold so that no plausible completion means no suggestion.

## Million-line histories

Use `Trainer` to ingest observations one at a time. It never retains the source
observation collection:

```rust
# use vista::{Config, Trainer};
let mut trainer = Trainer::new(Config::default());
// trainer.observe(observation)?;
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

PPM contexts own one template-ID vector each. Eviction, membership, follower,
and lookup indexes reference checked context IDs instead of cloning those
vectors. Snapshots serialize logical contexts in deterministic order and never
persist the transient IDs.

`ModelStats::retained_string_bytes` reports the logical bytes owned by retained
items, templates, slots, and retrieval-index keys. Eviction, forgetting, and
clearing release those charges.

Run the release-mode one-million-event ingestion and snapshot check with:

```sh
make bench-million
make bench-memory
```

The target prints ingestion throughput, prediction percentiles, retained model
counts, estimated heap bytes, snapshot bytes, and snapshot load time. These are
measurements for the current machine, not fixed performance promises.
`make bench-memory` additionally records peak RSS with `/usr/bin/time -v` in
`target/bench-memory.txt`; it is an optional local diagnostic, not part of
`make verify`.

The 2026-08-09 synthetic million-event run ingested 48,470 events/s with a
60-us prediction p95, retained an estimated 686,999 heap bytes and 14,063
logical string bytes, wrote a 202,394-byte snapshot, restored it in 2 ms, and
peaked at 5,768 KiB RSS. These values were measured from the worktree based on
commit `a8d51a6` and must be remeasured for a consumer's corpus and hardware.

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

Version 2 is the only supported format. It fingerprints all byte limits and
enforces cumulative snapshot bytes while reading and writing. Version 1 and any
other version, unknown feature bit, adapter key, or normalized configuration is
rejected rather than guessed.

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
| `max_string_bytes` | 65,536 | 1,024 | Maximum bytes in one retained input or derived string |
| `max_retained_string_bytes` | 67,108,864 | 65,536 | Total logical bytes in retained strings |
| `max_snapshot_bytes` | 134,217,728 | 1,048,576 | Total encoded snapshot bytes |
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
Snapshot measurement is typed as `Success`, `Failed`, or `NotMeasured`; a
serialization failure is never reported as a zero-byte successful result.

```sh
make evaluate
make evaluate EXAMPLE=tests/fixtures/workflow.txt
make evaluate HISTORY=/path/to/one-sentence-per-line.txt
```

Blank lines are skipped while retaining their positions as sequence gaps. The
library does not parse shell-specific timestamp prefixes; sanitize and convert
application history before evaluating it.

`ResearchExport` writes deterministic integer sequences for external CPT+,
IPredict PPM/AKOM, and PBCT comparisons. IDs start at one, SPMF uses `-1` and
`-2` sentinels, and dictionary fields escape backslash, tab, carriage return,
and newline. See `tools/README.md`. External research projects are not
downloaded by the build and are not runtime dependencies.

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

The repository pins the official Rust 1.97.1 toolchain, rustfmt, Clippy,
rust-analyzer, and the `x86_64-unknown-linux-musl` standard library in
`rust-toolchain.toml`. CI and the Nix shell use the same compiler version.

```sh
make build
make run
make test
make verify
make evaluate
make bench-million
make check-musl
make size-musl
```

The minimized Nix shell contains only the pinned Rust toolchain and command-line
tools used by the Makefile:

```sh
nix flake show
nix develop -c make verify
nix develop -c make check-musl
```

The measurements above use Rust 1.97.1, dated 2026-08-09, for the GNU and musl
x86-64 Linux targets. Synthetic and fixture evaluations pass, but they are not
a production-quality acceptance result. That gate requires a sanitized
chronological corpus, the production limits and normalizer, a predefined recent
slice, and approval to report aggregates.
