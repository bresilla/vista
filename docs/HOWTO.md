# Vista in detail

Everything the crate does, and how to drive it. The README is the summary; this
is the reference.

## Mental model

Vista watches an ordered stream of things you did, and answers questions about
what comes next. It has no model of language, no neural weights, no parser for
whatever syntax your items happen to use, and no authored rules.

`predict` returns only items it has actually observed. `predict_aligned` is the
exception worth knowing: it *composes*, so it can return a string that was never
observed. Repairing `apt install ripgrep` against an observed
`sudo apt install fd` yields `sudo apt install ripgrep`, which appears nowhere
in history. Every token in it does, and the structure holding them does, but the
result is new. Nothing is generated from a model of language; it is recombined
from what you supplied.

Two levels of identity underpin everything:

```
raw item              "sudo apt install ripgrep"
   │ Normalizer
   ▼
template + slots      "sudo apt install {pkg}"  +  {pkg: ripgrep}
   │
   ├─ TemplateId(u32) ─── the sequence model learns over these
   └─ SurfaceId(u32)  ─── the concrete strings handed back to you
```

The sequence model never sees a string. That is what keeps it small while the
surface store holds the real text. Without a normalizer every item is its own
template, and everything still works.

Learned state is the template dictionary, PPM counts, surface statistics,
retrieval indexes, the recent cache, and the mined correction log. All of it is
derived from observations you supplied and all of it is bounded.

---

## Core types

### `Item`

A namespaced string. The namespace separates incompatible kinds so that
`action / deploy` and `sentence / deploy` never collide.

```rust
use vista_recall::Item;
let item = Item::new("command", "cargo build --release");
```

### `StreamId` and `Position`

A stream is one independent sequence — a shell session, a workflow run, a
document. Positions are consecutive integers within a stream.

**A position gap is meaningful.** Observing position 12 then 14 tells Vista that
something happened at 13 that it was not shown, so it does not learn a
transition across the gap. Consume a position for every event boundary even when
you exclude the event itself.

`StreamId` is a **sequence-continuity boundary, not a privacy boundary.** Direct
transitions never cross streams, but aggregate PPM evidence and the global
recent-cache fallback can inform predictions in another stream. Use separate
`Predictor` instances for separate users, tenants, or privacy domains.

### `Feature`

Caller-defined context or outcome, categorical (`Feature::categorical("cwd",
"/srv/app")`) or numeric (`Feature::numeric("battery", 0.4)`). Vista assigns no
meaning to the names; it only counts co-occurrence.

Outcome features named `success`, `accepted`, or `score` are read as a
zero-to-one quality and feed both ranking and correction mining. Everything else
is treated as opaque context.

### `Observation`

One completed, learnable event.

```rust
use vista_recall::{Feature, Item, Observation, Position, StreamId};
let observation = Observation {
    item: Item::new("command", "cargo build --release"),
    stream: StreamId(7),
    position: Position(12),
    timestamp: 1_700_000_000,
    context: vec![Feature::categorical("cwd", "/srv/app")],
    outcome: vec![Feature::categorical("success", "true")],
};
```

Events that were hidden, cancelled, or rejected should not become observations —
but their positions should still be consumed, so the next visible observation
carries a detectable gap.

### `Query`

`Query::new(stream, position, limit)`, with an optional `partial` string and
`context` features.

Sequence evidence applies only when `position` directly follows the stream's
last observed position. Otherwise the query still works, ranked by context,
frequency, and recency alone.

---

## Feeding it history

Ingestion is fallible. `observe` and `replay` return `Result<(), InputError>`.

```rust
# use vista_recall::{Config, Predictor, Observation};
# let mut predictor = Predictor::new(Config::default());
# let observations: Vec<Observation> = vec![];
predictor.replay(observations)?;
# Ok::<(), vista_recall::InputError>(())
```

Vista validates the raw item, normalizer output, slot count, features, tokens,
and every derived index key **before** touching the model. A rejected
observation leaves predictions, statistics, clocks, and snapshot bytes
completely unchanged. `replay` additionally checkpoints the whole model and
restores it if any observation in the batch fails, so a batch is all-or-nothing.

`Trainer` is the streaming façade for building a model without holding the
source collection: `Trainer::new(config)`, then `observe`, then `finish()`. Use
`Trainer::from_builder` to stream with application adapters.

Call `break_stream(stream)` when you know continuity was interrupted but have no
observation to record.

---

## Asking it questions

| Method | Returns | Use for |
|---|---|---|
| `predict` | Ranked observed items | What comes next; autocomplete |
| `predict_aligned` | Repairs of an item you pass in | Fixing a failed or mistyped item |
| `predict_rendered` | Predicted templates refilled with your slots | Completion when you have a normalizer |
| `probability_of` | `f64` | Scoring one specific candidate |

Every `Prediction` carries:

- `item` — the concrete result
- `template` — which shape it came from
- `probability` — next-template probability, before presentation adjustments
- `score` — the final ranking value
- `context_depth` — how many history steps actually matched
- `repair_iterations` — repair passes, zero outside `predict_aligned`
- `explanation` — with the `explanations` feature

`probability` and `score` are different on purpose. `probability` is a real
probability you can threshold or compare. `score` is `ln(probability)` plus
bounded presentation adjustments and is only meaningful as an ordering.

---

## Adapters

Four traits let the application supply domain knowledge. All are optional; the
defaults work with zero configuration. Supply them through
`Predictor::builder(config).normalizer(..).tokenizer(..).matcher(..).build()`.

### `Normalizer`

Maps a raw item to a reusable template plus slots, and back again.

```rust
# use vista_recall::{Feature, Item, NormalizedItem, Normalizer};
struct Commands;

impl Normalizer for Commands {
    fn normalize(&self, item: &Item) -> NormalizedItem {
        if let Some(target) = item.value.strip_prefix("ssh ") {
            return NormalizedItem {
                template: Item::new(item.namespace.clone(), "ssh {target}"),
                slots: vec![Feature::categorical("target", target)],
            };
        }
        NormalizedItem { template: item.clone(), slots: vec![] }
    }

    fn render(&self, template: &Item, slots: &[Feature]) -> Option<Item> {
        let mut value = template.value.clone();
        for slot in slots {
            if let Feature::Categorical { name, value: filled } = slot {
                value = value.replace(&format!("{{{name}}}"), filled);
            }
        }
        (!value.contains('{')).then(|| Item::new(template.namespace.clone(), value))
    }
}
```

`render` is the inverse and powers `predict_rendered`. Returning `None` rejects
a template — the default does exactly that whenever slots exist but no inverse
is implemented, so a partial normalizer can never emit a half-filled template.

Vista deliberately ships no parser for any syntax. Only the application knows
which paths, hosts, branches, and secrets are safe to retain. At most 1,024
slots are retained per surface.

### `Tokenizer`

Supplies retrieval tokens. Implement both `tokens` (for observed items) and
`query_tokens` (for partial input) so retrieval uses one scheme in both
directions.

### `CandidateMatcher`

Scores partial input against a candidate. `None` **excludes** the candidate, so
the matcher is a filter as well as a score.

Override `score_match` to receive `MatchInput`, which carries both the source
template and the candidate template, and compare shapes rather than concrete
arguments. The default ignores templates and defers to `score`.

`ContainsMatcher` (default) requires substring containment, so it rejects typos.
`SimilarityMatcher` compares character trigrams, prefers templates when
available, and returns `None` below its threshold.

The reason templates matter here: concrete arguments dominate similarity between
otherwise identical items.

```text
"apt install ripgrep" vs "sudo apt install fd"      → rejected
"apt install {pkg}"   vs "sudo apt install {pkg}"   → accepted
```

### `ItemMatcher`

Selects items for `forget`. Any `Fn(&Item) -> bool` implements it.

---

## How prediction works

Four stages, all bounded.

**1. Resolve history.** The stream's recent template IDs, but only if the
query's position directly follows the last observed one.

**2. Generate candidates.** Templates are proposed by the PPM model, the recent
cache, and global popularity, merged by rank-weighted evidence. Those templates
expand into concrete surfaces, joined by surfaces retrieved from the caller-context
index, the token index, and the character-fragment index when partial input is
present. The merged set is truncated to `max_candidates`. Prediction never scans
the vocabulary.

**3. Score.** The sequence model is variable-order PPM to `max_order` (default 8)
with escape-mass backoff. Longer matching contexts win when they have support;
otherwise it backs off. Pruned followers keep their mass in `pruned_count` so
escape probabilities stay correct after eviction. With the `recent-cache`
feature the result is interpolated with a half-life-decayed recent model at
`recent_cache_weight`.

**4. Rank.**

```text
score = ln(probability)
      + context  · weights.context      // capped ln(1 + ratio)
      + surface  · weights.surface      // capped ln(1 + ratio)
      + outcome  · weights.outcome      // clamped 0–1
      + partial  · weights.partial      // clamped 0–1
```

Ties break by item identity, so identical inputs give identical output.

---

## How repair works

`predict_aligned` fixes an item using only the items history already contains.
No normalizer, no templates, no rules, no threshold.

```rust
# use vista_recall::{Config, Item, Position, Predictor, Query, StreamId};
# let predictor = Predictor::new(Config::default());
let failed = Item::new("command", "apt install ripgrep");
let query = Query::new(StreamId(7), Position(4), 3);
for repair in predictor.predict_aligned(&query, &failed) {
    println!("{}", repair.item.value);   // "sudo apt install ripgrep"
}
```

### Token alignment

Each ranked candidate is aligned against your item by longest common
subsequence:

```
you typed:   apt install ripgrep
history:     sudo apt install fd
             ────  ───────────  ──
             insert   shared    differ
result:      sudo apt install ripgrep
```

- **Shared** tokens are structure; keep them.
- **Only in the candidate** — that is the repair (`sudo`).
- **Differing** — decided by the channel, below.

### The channel

Differing tokens are a noisy-channel decision: the probability that the token
you typed was intended, against the probability that it was retyped as the
observed one.

- A token **history has produced before** is certain, so it is never rewritten.
- Otherwise Vista prefers the **observed rate** at which that retyping actually
  happened, taken from the mined correction log.
- Only where no such retyping was ever observed does it fall back to character
  resemblance, scaled so half-shared characters sit exactly at the
  unknown-token floor.

`Config::channel_weight` scales the channel term.

### Iteration

Repair loops to a fixpoint, bounded by `max_repair_iterations` and terminated by
any revisited value. **Adjacent tokens never both change in one pass**, which
forces a second pass rather than a cascade of rewrites.

On the synthetic fixture in `tests/predictor_cases/correction.rs`, iterating is
worth 22.5 points of recall and 31 points of precision over a single pass, and
results converge by the second pass.

### Abstention

A candidate sharing no structure repairs your item to itself, which is not a
repair and is dropped. So an item that needs no fixing yields nothing, and the
candidate matcher is deliberately not applied on this path — alignment is its
own filter.

### Mined corrections

Retypings are collected from the observations themselves. A failed item directly
followed in the same stream by a similar successful one is recorded as a
`CorrectionPair`. Similarity is gated at one point change per five characters.

```rust
# use vista_recall::{Config, Predictor};
# let predictor = Predictor::new(Config::default());
for (pair, count) in predictor.corrections() {
    println!("{} -> {} ({count}x)", pair.typed.value, pair.corrected.value);
}
```

Nothing is annotated and no dictionary is consulted. Your own retyping is the
only supervision, which requires that you supply outcome features.

---

## Configuration

| Setting | Default | Tiny | Purpose |
|---|---:|---:|---|
| `max_string_bytes` | 65,536 | 1,024 | Bytes in one retained or derived string |
| `max_retained_string_bytes` | 67,108,864 | 65,536 | Total logical retained string bytes |
| `max_snapshot_bytes` | 134,217,728 | 1,048,576 | Total encoded snapshot bytes |
| `max_templates` | 16,384 | 64 | Retained normalized sequence symbols |
| `max_surfaces` | 32,768 | 128 | Retained concrete historical items |
| `max_streams` | 256 | 4 | Independent live sequence histories |
| `max_order` | 8 | 3 | Longest learned context, clamped 1–32 |
| `max_contexts` | 262,144 | 256 | Retained variable-order states |
| `max_followers_per_context` | 64 | 4 | Followers retained per state |
| `max_context_associations` | 65,536 | 128 | Caller-context to surface associations |
| `max_tokens` | 32,768 | 64 | Retrieval token vocabulary |
| `max_partial_chars_per_item` | 512 | 64 | Characters indexed for partial input |
| `max_partial_associations` | 65,536 | 128 | Character-fragment associations |
| `max_candidate_templates` | 128 | 12 | Templates considered per query |
| `max_surface_candidates_per_template` | 8 | 3 | Surfaces expanded per template |
| `max_candidates` | 128 | 12 | Concrete candidates ranked per query |
| `recent_cache_items` | 256 | 16 | Recent items retained per cache |
| `recent_cache_weight` | 0.20 | 0.20 | PPM/cache interpolation, clamped 0–0.5 |
| `recent_cache_half_life` | 32 | 16 | Observation-age decay |
| `max_repair_iterations` | 3 | 1 | Repair passes, clamped 1–8 |
| `max_correction_pairs` | 4,096 | 32 | Retained mined retypings |
| `channel_weight` | 1.0 | 1.0 | Repair channel scale, clamped 0–10 |
| `weights.context` | 0.35 | 0.35 | Context-ratio adjustment |
| `weights.surface` | 0.20 | 0.20 | Surface frequency/recency adjustment |
| `weights.outcome` | 0.15 | 0.15 | Observed-outcome adjustment |
| `weights.partial` | 0.50 | 0.50 | Partial-match adjustment |

`Config::tiny()` is a strict low-memory preset. It limits retained state; it does
not change linked code size.

Normalization is applied on construction: zero counts become one, `max_order`
clamps to 1–32, identifier counts cannot exceed `u32::MAX`, non-finite weights
revert to defaults, presentation weights clamp to -10–10, and the recent-cache
weight clamps to 0–0.5.

Small bounds cut memory but can cut candidate recall. Measure with `Evaluation`
on real chronological history before choosing production limits.

---

## Snapshots

Requires the `snapshot` feature.

```rust
# use std::io::Cursor;
# use vista_recall::{Config, ContainsMatcher, IdentityNormalizer, Predictor, WhitespaceTokenizer};
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
# Ok::<(), vista_recall::SnapshotError>(())
```

Loading is linear in retained structures, not in source history lines, so a
million-line history that normalizes into a compact model loads quickly.

The format carries a magic value, version, feature flags, normalized
configuration, a configuration fingerprint, checked section lengths, reference
validation, and a checksum. Identical state produces identical bytes. Corrupt,
truncated, oversized, incompatible, or trailing data is rejected before a
predictor is returned.

**Version 3 is the only supported format.** Earlier versions, unknown feature
bits, mismatched adapter keys, and differing normalized configuration are
rejected rather than guessed. Version 3 added the mined correction log.

Snapshots record each adapter's `snapshot_key`. A stateful normalizer,
tokenizer, or matcher must override that method so the key changes whenever
behavior-affecting configuration changes. Loading re-normalizes every retained
surface and rejects any template or slot mismatch, so normalization must be
deterministic.

Vista reads and writes streams, never paths. The caller owns file permissions,
temporary files, flushing, `fsync`, and atomic rename.

---

## Evaluation

Requires the `evaluation` feature.

### Next-item prediction

`Evaluation::run_ordered` is prequential: predict, score, then learn. It reports
top-1/3/5/10 accuracy, MRR, candidate recall, coverage, log-loss, perplexity,
cold-start accuracy and log-loss, macro stream accuracy, context depth,
prediction and update latency percentiles, memory, snapshot size and load time,
normalization ratio, and simulated keystroke savings.

Latency percentiles use a fixed 65-bucket logarithmic histogram, so evaluation
keeps bounded state rather than one duration per event. Snapshot measurement is
typed `Success`, `Failed`, or `NotMeasured`; a serialization failure is never
reported as a zero-byte success.

```sh
make evaluate
make evaluate HISTORY=/path/to/one-item-per-line.txt
```

Blank lines are skipped but keep their positions as gaps. Vista does not parse
shell timestamp prefixes — sanitize and convert before evaluating.

### Repair

`CorrectionEvaluation::run(config, observations, attempts)` replays
chronologically and scores each attempt strictly before the observation at that
position is learned.

`CorrectionAttempt::repair(stream, position, typed, intended)` is an opportunity;
`CorrectionAttempt::control(stream, position, typed)` is an item that needed no
repair. Controls are what make precision meaningful — offering a repair for one
is a false positive.

Reported: `precision`, `recall`, `top_1_accuracy`, `top_3_accuracy`,
`false_positive_rate`, `abstention_rate`, `mean_iterations`, `mean_latency`.

### External comparison

`ResearchExport` writes deterministic integer sequences for external CPT+,
IPredict PPM/AKOM, and PBCT comparisons. IDs start at one, SPMF uses `-1` and
`-2` sentinels, and dictionary fields escape backslash, tab, carriage return, and
newline. See `tools/README.md`. Those projects are not downloaded by the build
and are not runtime dependencies.

---

## Cargo features

| Feature | Default | Provides |
|---|:---:|---|
| `explanations` | yes | Rendered `Prediction::explanation` strings |
| `recent-cache` | yes | Short-term adaptation to changing behavior |
| `snapshot` | yes | Binary persistence |
| `surface-indexes` | yes | Caller-context, token, and fragment retrieval |
| `evaluation` | no | Metrics and baselines (implies `snapshot`) |
| `research` | no | External-model export |

`default-features = false` gives the smallest sequence-only core: dictionary,
bounded PPM learning, concrete historical surfaces, probability queries,
matching, repair, and forgetting. Evaluation and research code never enters a
consumer that did not ask for it.

Without `surface-indexes` there is no observed vocabulary, so every differing
token in a repair falls back to the resemblance test.

---

## Memory and bounds

Every collection is bounded by `Config`. Eviction prefers entries that are old,
rare, or unsupported.

Hot indexes use compact `u32` template and surface identifiers rather than
cloning strings. PPM contexts own one template-ID vector each; eviction,
membership, follower, and lookup indexes reference checked context IDs instead
of copying those vectors. Snapshots serialize logical contexts in deterministic
order and never persist transient IDs.

`ModelStats::retained_string_bytes` reports logical bytes owned by retained
items, templates, slots, retrieval-index keys, and mined corrections. Eviction,
forgetting, and clearing release those charges.
`ModelStats::estimated_heap_bytes` estimates retained structures — it is not
allocator RSS.

```sh
make bench-million    # ingestion throughput, latency percentiles, snapshot
make bench-tiny       # sequence-only tiny-preset footprint
make bench-memory     # adds peak RSS via /usr/bin/time -v
make size             # linked size of the minimal example
```

Benchmarks measure the current machine and corpus. They are not portable
guarantees, and they are not part of `make verify` except for `size-check`.

---

## Retention and privacy

Vista owns only derived in-memory state. The caller owns persistence,
sanitization, retention, and consent.

Sanitize before observing. History of any kind can contain secrets — tokens in
arguments, credentials in environment assignments. The `Normalizer` is the right
place to strip them, or refuse to observe the item at all and let the position
gap break continuity.

For durable forgetting:

1. remove the observations from application storage;
2. preserve their position gaps;
3. call `forget` for immediate in-memory removal;
4. create a new snapshot, or replay the sanitized history when complete
   reconstruction is required.

Removing or evicting an item clears affected live histories rather than joining
its neighbours.

Remember that `StreamId` is not a privacy boundary. Separate domains need
separate `Predictor` instances.

---

## Development

```sh
make build      make test       make verify
make run        make clippy     make loc-check
make evaluate   make size       make bench-million
```

`make verify` is the full gate: line limits, formatting, checks and tests across
feature combinations, clippy with warnings denied, rustdoc, and a binary-size
ceiling.

Repository rules: Makefile targets for all build and test work, no maintained
file over 600 physical lines, comments describing only what code does, one
current pre-1.0 path with no legacy forwarding, and no new dependency, unsafe
code, async runtime, or predictor-owned storage without explicit approval.

README code blocks are **not** compiled as doctests. Check them by hand or wire
them into a compiled example before trusting them.
