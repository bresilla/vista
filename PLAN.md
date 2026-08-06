# Vista Sentence Prediction Plan

## Status

- **State:** implemented; synthetic validation complete, real-corpus validation
  awaits application-owned history
- **Scope:** next complete sentence/command prediction from chronological history
- **Baseline:** the current uncommitted `0.1.0` working tree; this repository has no
  `HEAD` commit yet
- **Compatibility:** deliberate pre-1.0 API break; do not preserve the optional
  FTRL API or the current fixed-depth transition representation
- **Primary validation command:** `make verify`

This plan replaces the deleted plan completely. It is written so that another
executor can implement it without relying on the conversation that produced it.

## 1. Product definition

Vista predicts the next **complete previously observed item**, especially a
sentence, shell command, action, workflow step, or tool call. It is not a text
generator and it does not synthesize unseen sentences.

The required behavior is:

1. consume a chronological stream of completed observations;
2. normalize each raw item into a reusable predictive template;
3. learn variable-length sequence contexts over those templates;
4. combine long-term sequence probability with a bounded recent-history cache;
5. select and rank concrete historical surface forms of the predicted template;
6. update online after every observation;
7. save and restore the compiled model without replaying the full history;
8. remain deterministic, bounded, explainable, and independent of an async
   runtime or application-owned storage.

For example, an application may normalize these raw commands:

```text
ssh alice@host1
ssh bob@host2
ssh root@host3
```

into one predictive template:

```text
ssh {user}@{host}
```

Vista learns transitions between template identifiers, while retaining the raw
commands as possible completions. Given the template prediction, it returns the
most plausible previously observed surface form for the current context.

## 2. Non-goals

The following are explicitly outside this plan:

- generating a sentence that has never appeared in history;
- an LLM, neural network, embedding model, or external inference service;
- a shell parser embedded in the generic core library;
- reading shell history files directly;
- choosing which observations are private or safe to retain;
- owning file paths, databases, migrations, background services, or async I/O;
- collaborative filtering across multiple users;
- importing CPT+, PBCT, or research code as a runtime dependency;
- maintaining the current FTRL learned-ranker API.

Applications remain responsible for collection, sanitization, stream and
position assignment, retention policy, and atomic replacement of snapshot
files. Vista supplies deterministic model encoding and decoding over `Read` and
`Write` interfaces.

## 3. Evidence behind the design

The implementation must remain grounded in the following primary research and
author-provided code rather than blog posts:

1. Begleiter, El-Yaniv, and Yona compare variable-order Markov predictors and
   find PPM and decomposed CTW strongest overall for sequence log-loss:
   <https://arxiv.org/abs/1107.0051>.
2. Gueniche et al. describe CPT+, which uses compressed complete sequences and
   noise-tolerant similarity matching. It is a useful benchmark challenger but
   retains more sequence information and has more complex prediction behavior:
   <https://www.philippe-fournier-viger.com/spmf/PAKDD2015_Compact_Prediction_tree%2B.pdf>.
   The authors' comparison framework is <https://github.com/tedgueniche/IPredict>.
3. Ghani, Heard, and Sanna Passino apply parsimonious Bayesian context trees to
   categorical sequences including terminal sessions. Their command vocabulary
   is heavily normalized, demonstrating why template reduction is required:
   <https://arxiv.org/abs/2407.19236>. Their code is
   <https://github.com/daniyarghani/pbct>.
4. Cunial, Alanko, and Belazzougui show that variable-order Markov models can be
   represented at multi-gigabyte training scale using compact context indexes:
   <https://academic.oup.com/bioinformatics/article/35/22/4607/5475595>.
   Their implementation is <https://github.com/jnalanko/VOMM>.
5. Davison and Hirsh evaluate chronological online UNIX command prediction and
   show the importance of recent command history and prequential evaluation:
   <https://www.cse.lehigh.edu/~brian/pubs/1997/mltr41/>. The authors' corrected
   results are <https://www.cse.lehigh.edu/~brian/pubs/1997/hci/>.

The production choice is therefore a PPM-style variable-order model rather than
CPT+ or PBCT. CPT+ remains an offline benchmark. PBCT remains a future research
experiment after Vista has real normalized corpora.

## 4. Current state and reasons for replacement

The current library already provides useful invariants that must survive:

- `Observation`, `Query`, `StreamId`, and `Position` express chronological
  input and prevent transitions across position gaps;
- `Predictor::observe`, `predict`, `replay`, `break_stream`, `forget`, and
  `clear` provide a small synchronous API;
- candidate generation is bounded;
- partial matching, caller context, outcomes, and explanations exist;
- deterministic ordering uses item identity as the final tie-breaker;
- chronological evaluation and fixed baselines exist;
- `make verify` runs formatting, checks, tests, Clippy, and rustdoc.

The current architecture cannot simply be enlarged to millions of events:

1. `src/transitions.rs` stores only depths one through three and therefore
   cannot learn variable-length dependencies.
2. Complete `Item` strings are cloned into transition, context, token, partial,
   and statistics maps. High-cardinality histories multiply string memory.
3. `ItemTable::refresh_global` sorts the complete retained vocabulary after
   every observation.
4. `src/pruning.rs` repeatedly scans complete maps to find one weakest entry.
5. `Config::max_items` defaults to 4,096, so replaying a large history evicts
   most of its vocabulary instead of compiling it into a durable model.
6. `replay` is the only restoration mechanism. There is no compiled snapshot.
7. Ranking is an additive collection of hand-scaled evidence plus an optional
   FTRL layer. This creates two user-visible learning modes and does not provide
   a calibrated next-item probability.
8. There is no distinction between a normalized template and a concrete raw
   surface form, so argument changes fragment the sequence alphabet.

The implementation must replace these constraints rather than adding another
optional layer beside them.

## 5. Architecture decisions

### 5.1 One predictor

There is one default prediction path. It learns sequence counts, the recent
cache, contextual associations, surface forms, and item statistics. There is no
`learned_ranker: Option<_>` and no separate learned report. Compile-time feature
gates may remove retrieval, persistence, explanation, or cache code from a
size-constrained host, but they do not introduce a second predictor.

In the default build, the recent cache is an internal component of the same
probability model. It is not an opt-in second predictor.

### 5.2 Intern all repeated identities

Introduce compact identifiers:

```rust
pub(crate) struct TemplateId(u32);
pub(crate) struct SurfaceId(u32);
```

Store each string exactly once in a dictionary. Every hot index uses identifiers
and integer counts, never cloned `Item` values. Resolve identifiers back to an
`Item` only when returning a prediction or explanation.

Identifier allocation is chronological and never reused within a model. This
makes online updates, tie-breaking, snapshots, and replay deterministic.

### 5.3 Separate templates from surface forms

Add a public normalization boundary:

```rust
pub struct NormalizedItem {
    pub template: Item,
    pub slots: Vec<Feature>,
}

pub trait Normalizer: Send + Sync {
    fn normalize(&self, item: &Item) -> NormalizedItem;
}
```

Provide `IdentityNormalizer` as the zero-configuration default. It returns the
raw item as its own template and no slots. This preserves generic behavior while
allowing applications to normalize paths, hosts, branches, identifiers, and
other variable arguments.

Do not ship a misleading regex-based shell parser in the core. Add an example
normalizer in `examples/template.rs` showing application-defined replacement of
known argument classes. The example must preserve the original raw item as the
surface returned to the caller.

### 5.4 Variable-order PPM-style probability

Replace `Transitions` with a suffix-context model supporting depths zero through
`max_order`. The initial default is `max_order = 8`; configuration normalization
must clamp it to `1..=32`.

For a context `h`:

```text
N(h) = total observations following h
T(h) = number of distinct followers of h
c(h,x) = count of candidate x following h
```

Use an interpolated escape calculation:

```text
P(x | h) = c(h,x) / (N(h) + T(h))
          + T(h) / (N(h) + T(h)) * P(x | suffix(h))
```

When `h` has no evidence, use the next shorter suffix immediately. The depth-zero
distribution uses Krichevsky-Trofimov smoothing over the retained template
vocabulary plus one unknown bucket:

```text
P0(x) = (count(x) + 0.5) / (total + 0.5 * (vocabulary + 1))
```

All calculations use `f64`. Clamp only the final public score/probability as
needed to avoid zero, infinity, or NaN. Store raw integer counts in the model.

The implementation must expose enough internal information to explain:

- deepest matching context;
- count and total at that context;
- number of backoff steps;
- long-term probability before the recent-cache interpolation.

### 5.5 Integrated recent-history cache

Maintain a bounded cache per stream plus a global fallback cache. The initial
defaults are:

- `recent_cache_items = 256`;
- `recent_cache_weight = 0.20`;
- `recent_cache_half_life = 32` observations.

Cache counts decay by powers of two at half-life boundaries, with deterministic
linear interpolation between boundaries to avoid a general power function in
small binaries. Blend the cache distribution with the long-term PPM
distribution:

```text
Psequence(x) = (1 - rho) * Pppm(x) + rho * Pcache(x)
```

If the applicable cache is empty, set `rho` to zero. Normalize configuration so
`rho` is finite and within `0.0..=0.5`. This cache is part of the default single
predictor and exists to adapt when recent behavior changes without erasing the
long-term model. Explicit `default-features = false` embedding builds may omit
it and use the PPM sequence distribution directly.

### 5.6 Concrete surface selection

Transitions predict `TemplateId`, not `SurfaceId`. Each template owns a bounded
surface table with frequency, recency, context, outcome, and normalized slot
statistics.

Candidate generation first obtains template candidates. It then expands only
the best surfaces for each candidate template. The initial bounds are:

- `max_surface_candidates_per_template = 8`;
- `max_candidate_templates = 128`;
- `max_candidates = 128` concrete returned candidates before final truncation.

Identity normalization produces exactly one template per distinct item, so this
pipeline also works without a custom normalizer.

### 5.7 One final score

The primary term is the calibrated sequence probability:

```text
score(x) = ln(Psequence(template(x)))
         + context_adjustment(x)
         + surface_adjustment(x)
         + outcome_adjustment(x)
         + partial_adjustment(x)
```

Each adjustment must be bounded and documented in `src/ranking.rs`. Context and
surface adjustments use `ln_1p` count ratios rather than unconstrained raw
counts. Partial matching remains a hard filter when the matcher returns `None`.

Remove the token-overlap transition heuristic once template normalization and
the variable-order model are working. Token and partial indexes may still be
used for candidate retrieval, but they must not masquerade as sequence
probability.

Return the following additional fields in `Prediction`:

```rust
pub probability: f64,
pub template: Item,
pub context_depth: usize,
```

`probability` is the sequence probability before non-probabilistic presentation
adjustments. `score` remains the final ranking value. This distinction prevents
evaluation from treating an arbitrary ranking score as a probability.
`context_depth` keeps evaluation independent of rendered explanation text.

### 5.8 Persistence belongs to the model, paths belong to the caller

Add a deterministic, versioned snapshot codec over standard I/O traits:

```rust
impl Predictor {
    pub fn write_snapshot<W: std::io::Write>(&self, writer: W)
        -> Result<(), SnapshotError>;

    pub fn read_snapshot<R: std::io::Read>(
        config: Config,
        normalizer: impl Normalizer + 'static,
        tokenizer: impl Tokenizer + 'static,
        matcher: impl CandidateMatcher + 'static,
        reader: R,
    ) -> Result<Self, SnapshotError>;
}
```

The library must not open, rename, or delete paths. The application can write to
a temporary path, `fsync`, and rename atomically.

Snapshot format version 1 contains:

1. eight-byte Vista magic;
2. format version;
3. feature flags;
4. serialized configuration relevant to model interpretation;
5. template dictionary;
6. surface dictionary and template-to-surface mapping;
7. zero-order counts;
8. variable-order contexts and follower counts;
9. context, outcome, and partial indexes;
10. recent stream state and cache state;
11. observation clock and model statistics;
12. payload checksum.

Use an explicit internal codec rather than serializing private Rust structs
directly. Encode integers with checked lengths and reject sections exceeding the
configured bounds before allocation. Loading corrupted, truncated, oversized,
or unsupported snapshots returns `SnapshotError` and never partially mutates an
existing predictor.

Snapshots created from the same ordered observations and configuration must be
byte-for-byte identical.

## 6. Target module layout

Keep modules focused and use the existing short-file style:

```text
src/
├── cache.rs           recent per-stream and global distributions
├── candidates.rs      bounded template and surface candidate union
├── config.rs          all normalized hard limits and score parameters
├── context.rs         caller feature associations by compact IDs
├── dictionary.rs      template/surface interning and lookup
├── evaluation.rs      chronological metrics and baselines
├── explanation.rs     typed contributions rendered to readable reasons
├── feature.rs         caller-defined features and normalized slots
├── item.rs            Item and compact identifier definitions
├── matcher.rs         partial-input filtering and partial candidate index
├── normalizer.rs      Normalizer, NormalizedItem, IdentityNormalizer
├── observation.rs     Observation and Query
├── ppm.rs             variable-order counts, backoff, and probabilities
├── predictor.rs       orchestration and public state transitions
├── pruning.rs         incremental bounded indexes without full-map scans
├── ranking.rs         final single-path scoring
├── snapshot.rs        deterministic checked snapshot codec
├── statistics.rs      template and surface statistics
├── stream.rs          continuity and recent histories by identifier
└── tokenizer.rs       candidate-retrieval tokenization only
```

Delete `src/learning.rs` after its public API and tests are removed. Delete
`src/transitions.rs` after `src/ppm.rs` passes the replacement characterization
tests. Do not keep deprecated aliases or dual implementations in the final
tree.

## 7. Configuration contract

Replace the current configuration with names that match the new model:

```rust
pub struct Config {
    pub max_templates: usize,
    pub max_surfaces: usize,
    pub max_streams: usize,
    pub max_order: usize,
    pub max_contexts: usize,
    pub max_followers_per_context: usize,
    pub max_context_associations: usize,
    pub max_tokens: usize,
    pub max_partial_chars_per_item: usize,
    pub max_partial_associations: usize,
    pub max_candidate_templates: usize,
    pub max_surface_candidates_per_template: usize,
    pub max_candidates: usize,
    pub recent_cache_items: usize,
    pub recent_cache_weight: f64,
    pub recent_cache_half_life: u64,
    pub weights: Weights,
}
```

Initial defaults:

| Field | Default |
|---|---:|
| `max_templates` | 16,384 |
| `max_surfaces` | 32,768 |
| `max_streams` | 256 |
| `max_order` | 8 |
| `max_contexts` | 262,144 |
| `max_followers_per_context` | 64 |
| `max_context_associations` | 65,536 |
| `max_tokens` | 32,768 |
| `max_partial_chars_per_item` | 512 |
| `max_partial_associations` | 65,536 |
| `max_candidate_templates` | 128 |
| `max_surface_candidates_per_template` | 8 |
| `max_candidates` | 128 |
| `recent_cache_items` | 256 |
| `recent_cache_weight` | 0.20 |
| `recent_cache_half_life` | 32 |

These are safe library defaults, not a promise that every million-line corpus
fits without configuration. The evaluation tool must print observed unique
templates, surfaces, contexts, follower associations, and estimated bytes so an
application can choose informed limits.

Remove these obsolete fields:

- `max_items`;
- `max_sequence_depth`;
- `max_transitions_per_state`;
- `feature_hash_size`;
- `weights.learned`;
- `learned_ranker`.

Every numeric setting must be normalized or rejected consistently. Memory caps
must never become unbounded because of zero, overflow, NaN, or infinity.

## 8. Scalability rules

### 8.1 No work proportional to vocabulary on each observation

The update path must not sort or scan all templates, surfaces, contexts, or
associations. Replace `refresh_global` and full-map pruning with incrementally
maintained ordered sets or heaps using generation counters.

Required complexity targets, excluding normalizer/tokenizer work:

| Operation | Target |
|---|---|
| intern existing item | amortized `O(1)` or `O(log V)` |
| learn one observation | `O(max_order * log C)` |
| prune one bounded follower map | `O(max_followers_per_context)` |
| candidate generation | `O(max_order * followers + configured indexes)` |
| final ranking | `O(max_candidates log max_candidates)` |
| snapshot write/read | linear in retained model size |

### 8.2 Determinism despite efficient maps

Runtime indexes may use `HashMap`, but hash iteration order must never affect:

- identifier allocation;
- eviction choice;
- candidate truncation;
- ranking ties;
- explanation ordering;
- snapshot bytes.

Collect and sort by stable identifier or explicit deterministic key at every
observable boundary. Keep the final item identity tie-breaker.

### 8.3 Pruning must preserve valid sequence boundaries

Evicting a surface removes it from retrieval indexes. Evicting the final surface
of a template removes that template as a follower and invalidates every stream
history containing it. It must never splice the predecessor and successor into
a transition that did not occur.

Context eviction is based on a deterministic tuple such as:

```text
(total evidence, last update clock, context identifier)
```

Follower eviction is based on:

```text
(count, last update clock, follower identifier)
```

Document whether counts lost through pruning are included in the escape mass.
For format version 1, retain a per-context `pruned_count` and include it in the
escape mass so pruning does not make a sparse context falsely overconfident.

### 8.4 Bulk ingestion

Keep `replay` for convenience, but add a streaming trainer facade:

```rust
pub struct Trainer { /* same bounded mutable model */ }

impl Trainer {
    pub fn observe(&mut self, observation: Observation);
    pub fn finish(self) -> Predictor;
}
```

`Trainer` must not collect observations. A caller can feed a million-line file
one record at a time. `Predictor::replay` delegates to this same learning path so
bulk and live behavior cannot drift.

## 9. Evaluation design

### 9.1 Chronological evaluation only

Retain the predict-before-learn order:

```text
predict observation n
score prediction against observation n
learn observation n
```

Never randomize observations. Sort by timestamp, stream, and position only when
the caller explicitly uses the convenience evaluator. Add a streaming evaluator
that assumes already ordered input and does not collect or sort the corpus.

Position gaps and `break_stream` reset sequence context in evaluation exactly as
they do in production.

### 9.2 Baselines and challengers

The in-tree evaluator must compare:

1. most frequent;
2. most recent;
3. context frequency;
4. fixed order 1;
5. fixed order 3;
6. fixed order 5;
7. longest-context-only order 8;
8. the new interpolated variable-order model;
9. the same model with template normalization disabled.

CPT+, IPredict PPM/AKOM, and PBCT are external research challengers. Add a
documented integer-sequence export format under `tools/README.md`, but do not
make Java, Python, R, or C++ part of `make verify`.

### 9.3 Required metrics

Extend `EvaluationMetrics` with:

- top-1, top-3, top-5, and top-10 accuracy;
- mean reciprocal rank;
- candidate recall;
- coverage, defined as observations receiving at least one candidate;
- mean negative log-likelihood and perplexity;
- cold-start accuracy and log-loss;
- per-stream macro accuracy in addition to global micro accuracy;
- mean and maximum retained context depth used;
- prediction and update latency distribution, at least p50, p95, and p99;
- exact retained structure counts and estimated heap bytes;
- snapshot byte length and snapshot load time;
- normalization reduction ratio: surfaces divided by templates.

Do not calculate log-loss from `Prediction::score`. Use the actual sequence
probability, including the unknown bucket when the true template is not retained.

### 9.4 Keystroke-savings simulation

Add a separate completion metric that simulates prefixes of the actual surface
string. For each observation, find the shortest prefix at which the actual item
is ranked first and report:

```text
saved = item_character_count - prefix_character_count - acceptance_cost
```

Use an acceptance cost of one character by default and count Unicode scalar
values consistently. Report total and mean saved characters. Keep this metric
separate from next-item accuracy because it exercises the partial-input index.

## 10. Implementation phases

Each phase ends with `make verify`. Do not begin the next phase while the current
phase has failing tests. Suggested commits are title-only Conventional Commit
messages and contain no signature.

### Phase 0 — Lock down current behavior

**Status:** complete

**Files:** `tests/predictor.rs`, `src/evaluation.rs`, new `tests/fixtures/` data.

1. Add characterization tests for stream gaps, deterministic ties, forgetting,
   bounded candidates, partial filtering, replay/live equivalence, and current
   fixed-order behavior.
2. Add small synthetic corpora covering repeated workflows, sparse long
   contexts, argument variation, changing recent behavior, and multiple streams.
3. Record current baseline metrics in test assertions using relationships rather
   than fragile wall-clock values.
4. Verify: `make verify`.

**Commit:** `test: lock prediction behavior`

### Phase 1 — Intern templates and surfaces

**Status:** complete

**Files:** new `src/dictionary.rs`, `src/item.rs`, `src/statistics.rs`,
`src/stream.rs`, `src/context.rs`, `src/tokenizer.rs`, `src/matcher.rs`,
`src/predictor.rs`, `src/lib.rs`, tests.

1. Add compact checked identifiers and bidirectional dictionaries.
2. Convert all private hot structures from `Item` keys to IDs.
3. Preserve public `Item`, `Observation`, `Query`, and `Prediction` values.
4. Replace full-vocabulary global refresh with incremental frequent/recent/
   outcome indexes.
5. Keep byte-for-byte deterministic public ordering for identical inputs.
6. Add tests proving strings are stored once logically, identifier exhaustion is
   handled without overflow, and eviction removes every reverse association.
7. Verify: `make verify`.

**Commit:** `refactor: intern prediction items`

### Phase 2 — Add template normalization

**Status:** complete

**Files:** new `src/normalizer.rs`, `src/predictor.rs`, `src/config.rs`,
`src/lib.rs`, new `examples/template.rs`, tests, `README.md`.

1. Add `Normalizer`, `NormalizedItem`, and `IdentityNormalizer`.
2. Add constructor or builder support for normalizer, tokenizer, and matcher
   without an explosion of `with_*` combinations.
3. Learn template-to-surface and slot statistics.
4. Ensure predictions return raw surfaces plus their normalized templates.
5. Ensure forgetting a raw surface retains its template only when another
   surface still belongs to it.
6. Add tests for many surfaces sharing a template, context-sensitive surface
   selection, and identity-normalizer equivalence.
7. Verify: `make verify`.

**Commit:** `feat: normalize prediction templates`

### Phase 3 — Implement variable-order probability

**Status:** complete

**Files:** new `src/ppm.rs`, `src/stream.rs`, `src/candidates.rs`,
`src/ranking.rs`, `src/config.rs`, `src/predictor.rs`, tests.

1. Implement suffix contexts up to `max_order` using template IDs.
2. Store totals, distinct follower counts, follower counts, pruned mass, and
   last-update clocks.
3. Implement the specified escape recursion and KT base distribution.
4. Generate candidates from every available suffix, with longer contexts
   visited first but without excluding useful backed-off candidates.
5. Replace the current logarithmic transition-count heuristic with log sequence
   probability.
6. Add exact hand-calculated probability tests, including unseen followers,
   missing contexts, pruning, gaps, and orders greater than three.
7. Add tests that all reported probabilities are finite, within `0..=1`, and
   sum to at most one over the retained vocabulary plus unknown mass within a
   documented floating-point tolerance.
8. Verify: `make verify`.

**Commit:** `feat: add variable order prediction`

After this phase passes, delete `src/transitions.rs` in the same commit or a
follow-up `refactor: remove fixed transitions` commit.

### Phase 4 — Integrate the adaptive cache and ranking

**Status:** complete

**Files:** new `src/cache.rs`, `src/ranking.rs`, `src/candidates.rs`,
`src/explanation.rs`, `src/config.rs`, tests.

1. Implement bounded recency-decayed per-stream and global cache distributions.
2. Blend cache and PPM probabilities using the configured `rho`.
3. Convert remaining context, outcome, surface, and partial contributions to
   bounded adjustments.
4. Produce typed internal explanation contributions before rendering strings.
5. Remove token-transition scoring; retain token retrieval only where it
   improves candidate recall.
6. Add a regime-change corpus proving adaptation improves recent accuracy while
   old long-term patterns remain recoverable.
7. Verify: `make verify`.

**Commit:** `feat: blend recent sentence cache`

### Phase 5 — Remove the optional FTRL layer

**Status:** complete

**Files:** `src/learning.rs`, `src/config.rs`, `src/predictor.rs`,
`src/evaluation.rs`, `src/lib.rs`, tests, `README.md`.

1. Remove `Ftrl`, `FtrlConfig`, `FeatureSpace`, `learned_ranker`, learned score
   weight, hard-negative training, and learned evaluation report.
2. Delete tests whose only purpose is FTRL configuration or feature-array size.
3. Replace them with tests proving the single predictor learns immediately from
   both replayed and live history.
4. Do not leave deprecated exports or inactive feature flags.
5. Verify: `make verify`.

**Commit:** `refactor: remove optional ranker`

### Phase 6 — Add deterministic snapshots

**Status:** complete

**Files:** new `src/snapshot.rs`, all state-owning modules, `src/lib.rs`, tests,
`README.md`.

1. Define snapshot version 1 and checked section codecs.
2. Serialize dictionaries and indexes in canonical ID order.
3. Check magic, version, lengths, arithmetic, configured limits, duplicate IDs,
   dangling references, invalid probabilities, and checksum during load.
4. Restore stream continuity and observation clock.
5. Add round-trip equivalence tests comparing stats, predictions, explanations,
   and subsequent online learning.
6. Add deterministic-byte, truncation, bit-flip, oversized-length, unsupported-
   version, and trailing-data tests.
7. Add a test ensuring a load failure leaves an existing predictor unchanged.
8. Verify: `make verify`.

**Commit:** `feat: persist prediction snapshots`

### Phase 7 — Replace full scans and add streaming training

**Status:** complete

**Files:** `src/pruning.rs`, `src/statistics.rs`, `src/ppm.rs`,
`src/predictor.rs`, new `src/trainer.rs`, tests and benchmarks.

1. Replace repeated global scans with incremental bounded indexes.
2. Use exact incrementally updated ordered sets for eviction; this avoids lazy
   heap entries entirely, so stale generations cannot evict strengthened state.
3. Add `Trainer` and make replay delegate to the same observation logic.
4. Add a 50,000-observation CI stress test with configured bounds and no
   absolute sub-second assertion.
5. Add an ignored one-million-observation benchmark target invoked through
   `make bench-million`; it must report ingestion throughput, prediction
   percentiles, structure counts, estimated memory, snapshot size, and load
   time.
6. Inspect the `Makefile` target before running it and keep generated benchmark
   artifacts under ignored `target/` paths.
7. Verify: `make verify` and manually run `make bench-million` before declaring
   the phase complete.

**Commit:** `perf: scale history ingestion`

### Phase 8 — Expand evaluation and research adapters

**Status:** complete

**Files:** `src/evaluation.rs`, new `tools/README.md`, new `examples/evaluate.rs`,
tests, `Makefile`.

1. Add streaming chronological evaluation and all metrics from section 9.
2. Add fixed-order and normalization-disabled baselines.
3. Add the completion/keystroke simulation.
4. Add deterministic integer-sequence export for IPredict/CPT+ and PBCT
   experiments, including stream separators and a template dictionary.
5. Document exact external commands separately; do not download tools or add
   them to CI.
6. Add `make evaluate EXAMPLE=<fixture>` for the local evaluation example.
7. Verify: `make verify`.

**Commit:** `feat: benchmark sentence prediction`

### Phase 9 — Documentation and final release gate

**Status:** complete except the application-owned real-corpus gate

**Files:** `README.md`, crate documentation in `src/lib.rs`, examples, `PLAN.md`.

1. Rewrite the README around one predictor, template normalization, online
   learning, snapshots, million-line ingestion, and caller responsibilities.
2. Clearly state that Vista predicts historical surfaces and does not generate
   unseen sentences.
3. Document snapshot compatibility and safe atomic persistence by callers.
4. Document every configuration bound and its memory/accuracy tradeoff.
5. Run the complete release gate in section 12.
6. Mark every completed phase and record benchmark results in section 13.

**Commit:** `docs: document sentence predictor`

## 11. Detailed test matrix

### Probability correctness

- exact depth-zero KT calculation;
- exact one-level and multi-level escape calculation;
- deeper context disambiguates equal lower-order counts;
- sparse deeper context backs off rather than returning no prediction;
- pruned follower mass increases escape probability;
- unknown template receives nonzero base probability;
- probability never becomes NaN, infinite, negative, or greater than one.

### Normalization and surfaces

- identity normalization matches raw-item behavior;
- multiple surfaces share one transition template;
- a predicted template expands to the correct context-specific surface;
- deleting one surface does not delete surviving template history;
- deleting the final surface invalidates affected histories;
- slots containing Unicode, empty strings, or duplicate keys remain
  deterministic.

### Streams and privacy

- separate streams never share transitions or recent cache state;
- position gaps reset both PPM history and per-stream cache continuity;
- explicit breaks reset the same state;
- forgetting does not bridge neighboring events;
- sanitized replay cannot recover a removed item from snapshots or indexes;
- failed snapshot loading does not expose partial state.

### Bounds and performance

- every configured collection remains within its cap under adversarial unique
  input;
- identifier conversion checks `u32` exhaustion;
- length multiplication and allocation checks reject overflow;
- candidate count remains bounded across all sources;
- updates do not call code that scans the complete vocabulary;
- 50,000-event CI stress test completes without fragile hardware-specific time
  assertions;
- one-million-event benchmark records actual time and memory rather than using
  an unverified claim in documentation.

### Persistence

- empty and populated round trips;
- deterministic bytes from identical histories;
- prediction equivalence before and after restore;
- continued learning equivalence after restore;
- unsupported version;
- corrupt checksum;
- truncated section;
- oversized declared count;
- duplicate or dangling identifier;
- configuration incompatibility;
- trailing bytes policy is explicit and tested.

### Evaluation

- chronological predict-before-learn ordering;
- no transition across gaps;
- macro and micro metrics differ correctly for imbalanced streams;
- log-loss uses probability, not ranking score;
- candidate recall is never below top-k accuracy;
- Unicode prefix lengths in keystroke simulation;
- external export is deterministic and includes stream separators.

## 12. Final acceptance gates

All of the following must pass before the plan is considered implemented:

```sh
make fmt-check
make check
make test
make check-all
make test-all
make clippy
make rustdoc
make verify
make run
make bench-million
```

Functional acceptance:

- one public prediction path exists;
- no FTRL or optional learned-ranker API remains;
- contexts deeper than three affect predictions;
- sparse contexts back off to useful shorter contexts;
- recent regime changes adapt through the integrated cache;
- normalized templates generalize across raw argument variations;
- predictions still return concrete historical `Item` values;
- unseen sentences are never synthesized;
- snapshot restore avoids full observation replay;
- live learning after restore matches uninterrupted learning;
- identical histories produce identical rankings and snapshot bytes;
- forgetting and gaps never create false transitions.

Scalability acceptance on the one-million-event benchmark:

- ingestion is streaming and does not retain the source observations;
- every structure count respects configuration;
- prediction work is bounded by model order and candidate limits, not history
  length;
- a snapshot is produced and successfully restored;
- pre- and post-restore predictions are identical;
- measured throughput, p50/p95/p99 prediction latency, estimated heap bytes,
  snapshot bytes, and load time are recorded below.

Quality acceptance on at least one real application-owned chronological corpus:

- variable-order top-5 accuracy is not worse than fixed-order-3;
- variable-order mean log-loss is better than most-frequent and fixed-order-1;
- template normalization improves either candidate recall or memory without a
  material top-5 regression;
- integrated cache improves the recent regime-change slice;
- candidate recall is at least 99% when evaluated with the chosen production
  limits, or the lower measured value is explicitly accepted and documented.

Do not promise numerical accuracy, latency, or memory targets in the README
until these measurements exist.

### 12.1 Release-gate record

On 2026-08-06, `make verify` passed formatting, default and minimal feature
checks, 60 tests (ten unit and fifty integration), all-feature checks/tests, Clippy with
warnings denied, and rustdoc with warnings denied. `make run`, `make evaluate`,
`make evaluate EXAMPLE=tests/fixtures/workflow.txt`, `make research-export`, and
`make bench-million` also passed. Legacy FTRL and fixed-transition names are
absent from source, tests, examples, README, tools, and the Makefile.
Final hardening covers eviction-time history invalidation, synchronized stream
cache eviction, reverse association cleanup without full-index scans,
recency-decayed cache retrieval, bounded normalized slots, and normalizer
revalidation during snapshot loading.

The application-owned real-corpus quality gate remains intentionally open. The
library does not read private history by itself; completing that gate requires a
sanitized chronological file supplied explicitly through `HISTORY=/path`.

### 12.2 Embedded-size record

The default behavior remains complete, while compile-time features remove code
that an embedded host does not use. `Config::tiny()` adds strict retained-state
bounds. The release profile uses size optimization, fat LTO, one codegen unit,
abort-on-panic, and symbol stripping. The consuming root package must repeat
these profile settings because Cargo ignores dependency profiles.

On Rust 1.97.1 for x86-64 Linux, the no-default-feature live learn/predict
example is 449,088 bytes. An empty Rust example built with the same profile is
285,880 bytes, leaving an approximate Vista increment of 163,208 bytes. The
all-feature learn/predict example is 541,368 bytes. `make size-check` enforces a
475,000-byte minimal-example ceiling. A 100,000-observation `make bench-tiny`
run retained 64 templates, 64 surfaces, 192 contexts, and 192 followers with a
52,504-byte model heap estimate.

## 13. Benchmark record

Fill this table with actual results. Never substitute estimates for measurements.

| Corpus | Events | Templates | Surfaces | Top-1 | Top-5 | MRR | Log-loss | Candidate recall | Ingest events/s | p95 predict | Heap estimate | Snapshot | Load time |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| synthetic million | 1,000,000 | 64 | 64 | N/A | N/A | N/A | N/A | N/A | 66,726 | 85 us | 481,175 B | 202,370 B | 1 ms |
| synthetic workflow | 2,000 | 4 | 4 | 0.9975 | 0.9980 | 0.9976 | 0.004237 | 0.9980 | N/A | 16 us | 26,624 B | 14,730 B | 129 us |
| real chronological | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |

Synthetic measurements above were recorded in release mode on 2026-08-06. The
million-event run also verified every configured structure bound and identical
predictions before and after snapshot restore.

Also record external challenger results when run:

| Corpus | Vista VOMM | CPT+ | IPredict PPM | AKOM | PBCT | Notes |
|---|---:|---:|---:|---:|---:|---|
| real chronological | TBD | TBD | TBD | TBD | TBD | Same split and normalized symbols required |

## 14. Stop conditions

The executor must stop and report instead of improvising if any of these occur:

1. a normalizer cannot deterministically reproduce its template during snapshot
   restore;
2. exact snapshot validation requires application-specific parser state that is
   not represented in the public normalizer contract;
3. the PPM probability test does not conserve probability after pruning;
4. million-event memory is dominated by duplicated strings after ID interning;
5. the update path still requires a full-vocabulary or full-context scan;
6. normalization improves compression but materially reduces real-corpus top-5
   accuracy;
7. a proposed dependency introduces native build requirements or an async
   runtime;
8. external research comparisons cannot use the identical chronological split;
9. `make verify` rewrites tracked files or disagrees with CI behavior;
10. snapshot format changes are proposed after version 1 fixtures exist without
    an explicit versioning decision.

## 15. Completion definition

This plan is complete only when all phases are implemented, all acceptance gates
pass, the benchmark record contains real measurements, and the README describes
the verified behavior. Passing unit tests alone is not sufficient.
