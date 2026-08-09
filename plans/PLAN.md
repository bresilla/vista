# Vista correction-quality plan

> This is the only plan file. Execute phases in order, run every gate, and update
> the table here. Do not create other plan/index files. Stop on a STOP condition;
> do not improvise.

## Status

- Planned at commit `ac845e4`, 2026-08-09.
- Audited state: branch `feat/rendered-completions`, three commits adding
  `predict_rendered`, `predict_aligned`, `MatchInput`/`score_match`,
  `SimilarityMatcher`, and history-driven token arbitration. 73 integration /
  16 unit / 14 minimal tests green across every feature combination.
- Overall: incomplete. Correction quality is unmeasured, the repair loop runs
  once, and one hardcoded constant still decides misspelling-versus-argument.

| Phase | Work | Priority | Effort | Depends | Status |
|---|---|---:|---:|---|---|
| 1 | Correction evaluation harness | P0 | M | — | TODO |
| 2 | Mine correction pairs from history | P0 | M | 1 | TODO |
| 3 | Iterative repair with safety constraints | P0 | M | 1–2 | TODO |
| 4 | Noisy-channel repair scoring | P1 | L | 1–3 | TODO |
| 5 | Learned substring edit model | P2 | L | 4 | TODO |
| 6 | Symmetric-delete retrieval | P2 | M | 1 | TODO |
| 7 | Pin and verify musl toolchain | P1 | M | — | TODO |
| 8 | Simplify Nix development shell | P2 | M | 7 | TODO |
| 9 | Run real-corpus quality gate | P1 | M | 1–4 | BLOCKED: corpus required |
| 10 | Refresh final documentation | P1 | S | 1–9 | TODO |

Statuses: `TODO`, `IN PROGRESS`, `DONE`, `BLOCKED: reason`, `REJECTED: reason`.

## Non-negotiable repository rules

- Use Makefile targets for all relevant build/test/lint/benchmark/export work.
- Do not use Python to edit or process files.
- No maintained repository file may exceed **600 physical lines**, including
  Rust, tests, examples, Makefile, CI, Markdown, and this plan. Generated/ignored
  `.git/`, `target/`, and `.direnv/` contents are excluded.
- `make loc-check` counts tracked plus nonignored untracked files, prints every
  violation, exits nonzero, and stays in `make verify`.
- Comments describe only what code does; no rationale/changelog comments and no
  comment volume above roughly 20% of implementation.
- Keep one current pre-1.0 path; do not leave forwarding/legacy implementations.
- No dependency, unsafe code, native library, async runtime, network client, or
  predictor-owned storage without explicit maintainer approval after a STOP.
- Never commit application history, secrets, or generated research data.
- Suggested commits are title-only Conventional Commits, unsigned, <=50 chars.
- Run `make verify` after every phase; do not start a dependent phase while red.

## Product boundary

Vista predicts complete previously observed items and repairs an item by
aligning it against them. It never generates unseen text from a model of
language, contains no parser for any input syntax, and holds no authored rules,
templates, dictionaries, or confusion lists. Every decision it makes is taken
from observed history. This plan does not add a second predictor, LLM, database,
service, or application-history reader.

Normalization stays optional. `predict_aligned` must keep working with
`Predictor::new(Config::default())` and no configuration of any kind.

---

## Research basis

Two results decide the ordering of this plan. Both come from published spelling
correction work whose problem shape matches Vista's: correct a short string from
a log of previously seen strings, without a trusted lexicon.

### The task is a noisy channel

`best = argmax P(intended) x P(typed | intended)`, a *source/language model* term
and an *error model* term (Kernighan et al. 1990; Brill & Moore 2000).

Vista already owns a strong source model: variable-order PPM to order 8 with
escape-mass backoff, conditioned on the current stream. It owns no error model at
all. `TYPO_SIMILARITY = 0.5` in `src/engine/alignment.rs` is a boolean standing
in for one.

### The source model dominates; the error model barely matters

Cucerzan & Brill, EMNLP 2004, corrected web queries against query logs with no
trusted lexicon. Ablation on 1044 queries, accuracy in percent, the `Misspelled`
column being recall over 180 misspelled queries:

| Configuration | All | Valid | Misspelled |
|---|---:|---:|---:|
| Full system | 81.8 | 84.8 | **67.2** |
| All edits equal (crippled error model) | 80.4 | 83.3 | **66.1** |
| Unigrams only (crippled source model) | 54.7 | 57.4 | **41.7** |
| 1 iteration only | 80.9 | 88.0 | **47.2** |
| 2 iterations only | 81.3 | 84.4 | 66.7 |
| No lexicon | 70.3 | 72.2 | 61.1 |
| No query log | 77.0 | 82.1 | 52.8 |

Flattening the error model costs **1.1 points**. Flattening the source model
costs **25.5 points**. Running one iteration instead of the full loop costs
**20.0 points**.

A second evaluation on genuinely reformulated query pairs repeats the ranking:
full 73.1, all-edits-equal 69.9, unigrams-only 43.0, one-iteration-only 45.5.
Removing the trusted lexicon entirely (61.1) beat keeping the lexicon while
discarding the logs (52.8) — direct evidence that observed history outperforms
an authored word list.

**Consequence for Vista.** Iteration is worth roughly twenty times what a learned
error model is worth. Phase 3 before phase 5. Phase 5 carries a STOP because the
evidence predicts it will not pay for itself.

### Why Brill & Moore appear to disagree

Brill & Moore, ACL 2000, report large error-model gains: 1-best accuracy 87.0
with single-character edits, 92.9 at substring window 2, 93.6 at window 3, and
95.0 with position information. Their generic edit `a -> b` over strings gives

`P(s|w) = max over R in Part(w), T in Part(s) of product P(Ti | Ri)`

trained by aligning each pair by single-character minimum edit distance, adding
the N adjacent edits around every non-match, and estimating
`P(a -> b) = count(a -> b) / count(a)`.

Those numbers were measured against a **null language model** assigning equal
probability to all 200,000 dictionary words. With no source model the error model
is the only signal. Vista is in Cucerzan & Brill's regime, not this one. Do not
cite Brill & Moore's headline gains as justification for phase 5.

Window size saturates at 2–3 in both of their tables; anything wider is wasted.

### Free supervision already sits in the observation stream

Cucerzan & Brill built their evaluation set by sampling successive queries from
one user where the unweighted edit distance was at most
`1 + (len(q1) + len(q2)) / 10` — one point change allowed per five characters.
No annotation, no dictionary; the user's own retyping is the label.

Vista records stream, position, and outcome for every observation. A failed item
followed by a positionally adjacent successful item in the same stream is exactly
such a pair. This is phase 2 and it costs nothing to collect.

### Two safety constraints worth importing

- No two *adjacent known* tokens may change in the same iteration. Cucerzan &
  Brill use this to prevent `log wood -> dog food`.
- The first iteration may not change a known token at all, so unknown-token
  errors are resolved before any substitution of a real token.

### Retrieval

SymSpell's symmetric-delete scheme turns every edit type into deletes computed
on both sides. A 5-character term within edit distance 3 needs 25 deletes rather
than enumerating roughly 3,000,000 candidates, and the method is independent of
the language being corrected. Vista's `PartialIndex` is a character 1/2/3-gram
index; this is a retrieval swap, not a quality change.

### Reference list

- Cucerzan & Brill, *Spelling Correction as an Iterative Process that Exploits
  the Collective Knowledge of Web Users*, EMNLP 2004.
- Brill & Moore, *An Improved Error Model for Noisy Channel Spelling
  Correction*, ACL 2000.
- Kernighan, Church & Gale, *A Spelling Correction Program Based on a Noisy
  Channel Model*, COLING 1990.
- Ristad & Yianilos, *Learning String Edit Distance*, IEEE TPAMI 20(5), 1998.
- Huang & Efthimiadis, *Analyzing and Evaluating Query Reformulation Strategies
  in Web Search Logs*, CIKM 2009.
- Wolf Garbe, *SymSpell*, symmetric-delete spelling correction.

---

## Phase 1: Correction evaluation harness

### Goal

Nothing in phases 2–6 can be judged today. `Evaluation` measures next-item
prediction, not repair. Build the measurement before building the improvements.

### Steps

1. Add `src/evaluation/correction.rs` behind the existing `evaluation` feature.
2. Define `CorrectionMetrics`:
   - `suggestions`, `opportunities`
   - `precision` — correct repairs over repairs offered
   - `recall` — correct repairs over pairs needing one
   - `top_1_accuracy`, `top_3_accuracy`
   - `false_positive_rate` — repairs offered where the input was already correct
   - `abstention_rate` — inputs left untouched
   - `mean_iterations`, `mean_latency`
3. `CorrectionEvaluation::run(config, pairs, controls)` replays chronologically:
   observe prior history, then repair the failed item, then compare to the known
   good item. Never let a pair train the model before it is scored.
4. `controls` are already-correct items. Offering a repair for one is a false
   positive. Precision without this is meaningless.
5. Report per-configuration so ablations are directly comparable, mirroring the
   research table above.

### Files

`src/evaluation/correction.rs`, `src/evaluation/mod.rs`, `tests/predictor_cases/correction.rs`.

### Gate

`make verify`. A fixture pair set under `tests/fixtures/` with at least 40 pairs
and 40 controls, committed as synthetic data only. Record the phase-1 baseline
numbers for today's single-pass repair in the status table.

### STOP

If measured baseline recall already exceeds 0.90 on the fixture set, stop and
report; the fixture is too easy and phases 3–5 cannot be judged by it.

---

## Phase 2: Mine correction pairs from history

### Goal

Collect `(failed, corrected)` pairs from the observation stream with no
annotation, no dictionary, and no rules. These are both training data for phase 4
and test data for phase 1.

### Steps

1. Add `src/model/corrections.rs` holding a bounded `CorrectionLog`.
2. During `observe`, a pair is recorded when all hold:
   - same `StreamId`;
   - positions are consecutive, so continuity was never broken;
   - the earlier observation carried a failing outcome and the later one a
     succeeding outcome, by the existing `Feature::quality` reading;
   - unweighted character edit distance is at most
     `1 + (len(a) + len(b)) / 10`, the Cucerzan & Brill gate.
3. Bound it with `Config::max_correction_pairs` (default 4096, tiny preset 32),
   evicting least-recently-observed. Count its strings in
   `retained_string_bytes` so the existing byte budget still holds.
4. Expose `Predictor::corrections()` returning the retained pairs, and include
   the count in `ModelStats`.
5. Serialize into the snapshot as a new section; bump `VERSION` to 3 and reject
   version 2 files rather than silently migrating.

### Files

`src/model/corrections.rs`, `src/model/mod.rs`, `src/api/config.rs`,
`src/engine/predictor/{observe,maintenance}.rs`, `src/snapshot/*`,
`tests/predictor_cases/corrections.rs`.

### Gate

`make verify`. Tests must cover: a gap between the two positions records nothing;
a cross-stream pair records nothing; a success following a success records
nothing; an edit distance above the gate records nothing; the log respects its
bound; and a snapshot round-trip preserves the log exactly.

### STOP

Do not infer failure from anything except the outcome features the caller
supplied. If a fixture needs Vista to guess that an item failed, stop.

---

## Phase 3: Iterative repair with safety constraints

### Goal

The single largest measured win available: 47.2 to 67.2 recall in the reference
ablation. Today `predict_aligned` repairs once.

### Steps

1. In `src/engine/predictor/predict.rs`, loop `predict_aligned` over its own
   output until the repaired value stops changing or `max_repair_iterations` is
   reached. Default 3, tiny preset 1.
2. Iteration 1 may only rewrite tokens absent from the observed vocabulary.
   Later iterations may rewrite known tokens.
3. Never rewrite two adjacent known tokens within one iteration.
4. Track the iteration count on `Prediction` behind the `explanations` feature so
   the harness can report `mean_iterations`.
5. Guarantee termination: a repair that revisits any earlier value in the chain
   ends the loop for that candidate.

### Files

`src/engine/alignment.rs`, `src/engine/predictor/predict.rs`,
`src/api/config.rs`, `tests/predictor_cases/alignment.rs`.

### Gate

`make verify`, plus phase-1 metrics showing recall above the phase-1 baseline
with false-positive rate no worse than baseline + 0.02. Add a test asserting a
two-edit repair that single-pass alignment cannot reach, and a test asserting
that two adjacent known tokens are never rewritten together.

### STOP

If iteration raises recall but pushes the false-positive rate more than 0.05
above baseline, stop and report both numbers; do not tune the constraints to hide
it.

---

## Phase 4: Noisy-channel repair scoring

### Goal

Delete `TYPO_SIMILARITY`, the last authored constant in the repair path, and
replace the boolean with a probability so ranking can trade a likelier command
against a larger edit.

### Steps

1. Score each repair as
   `ln P(candidate | history) + channel_weight * ln P(typed | candidate)`.
   The first term is the existing PPM probability, already computed.
2. Estimate `P(typed | candidate)` from the phase-2 log: the observed rate at
   which a token was retyped as another token. Back off, for pairs never seen, to
   a length-normalized edit distance converted to a log-probability.
3. Decide misspelling-versus-argument by comparing that probability against the
   probability of the typed token being intended, rather than against a constant.
4. Add `Config::channel_weight` (default 1.0) and keep it inside the existing
   `normalise` clamp.
5. Abstention becomes a probability floor, not a similarity floor.

### Files

`src/engine/alignment.rs`, `src/engine/ranking.rs`, `src/model/corrections.rs`,
`src/api/config.rs`.

### Gate

`make verify`. Phase-1 metrics at least matching phase 3 on recall while
improving precision. `rg TYPO_SIMILARITY src/` returns nothing. A test must show
a likelier command with a larger edit outranking a rarer command with a smaller
edit — the case the boolean cannot express.

### STOP

If the channel term cannot beat phase 3 on any weight in `{0.5, 1.0, 2.0}`,
record the numbers and mark this phase `REJECTED`, keeping phase 3 behaviour.

---

## Phase 5: Learned substring edit model

### Goal

Brill & Moore's `a -> b` substring edits estimated from the phase-2 pairs.

**The research predicts this is worth about one point.** It exists in the plan so
the decision is measured rather than assumed. Do not start it before phase 4 is
`DONE` and do not start it if phase 4 was `REJECTED`.

### Steps

1. Align each phase-2 pair by single-character minimum edit distance.
2. For every non-match, add the adjacent edits within a window of 2. Window 3 may
   be tried once; both reference tables saturate by 3.
3. Estimate `P(a -> b) = count(a -> b) / count(a)`, bounded by
   `Config::max_edit_rules`.
4. Feed it as the `P(typed | candidate)` estimator from phase 4, behind a new
   `edit-model` feature so the minimal build never links it.

### Gate

`make verify`, plus phase-1 metrics.

### STOP

If it does not beat phase 4 by at least 2 points of recall at equal precision,
mark `REJECTED`, delete the code, and record the measurement. A one-point gain
does not justify a new feature flag, a snapshot section, and a training pass.

---

## Phase 6: Symmetric-delete retrieval

### Goal

Cheaper bounded-edit candidate generation than the current 1/2/3-gram index.
Retrieval speed only; correction quality must not move.

### Steps

1. Add a symmetric-delete index beside `PartialIndex` under `surface-indexes`.
2. Precompute deletes of retained surfaces to `Config::max_delete_distance`
   (default 2), and generate deletes of the query at lookup.
3. Keep the existing index until benchmarks justify removing it.

### Gate

`make verify`, `make bench-million`. Candidate recall unchanged within 0.01,
p95 prediction latency improved, heap growth within the configured bounds.

### STOP

If the delete index costs more retained bytes than it saves in latency at the
tiny preset, mark `REJECTED`.

---

## Phase 7: Pin and verify musl toolchain

Carried forward, unstarted, and now blocking every gate.

`Cargo.toml` declares `rust-version = "1.97.1"`. The host toolchain is 1.94.0 and
no `rust-toolchain.toml` exists, so every cargo invocation is refused by the
manifest gate before compilation. Phases 1–6 were planned but cannot be verified
by `make verify` until this is fixed.

### Steps

1. Commit a `rust-toolchain.toml` pinning the channel used by the flake, with the
   `x86_64-unknown-linux-musl` target and `clippy`/`rustfmt`.
2. Reconcile `rust-version` with the pinned channel; lower it if the pin cannot
   be raised.
3. Confirm `make check-musl` and `make size-musl` run.

### Gate

`make verify` runs end to end with no `--ignore-rust-version` override.

---

## Phase 8: Simplify Nix development shell

Carried forward, unstarted. Depends on phase 7.

Reduce the flake to one toolchain derivation providing the musl target, and drop
any separate rustc/cargo/rustfmt/clippy inputs that can disagree with it.

### Gate

A clean `nix develop` provides the pinned toolchain and `make verify` passes
inside it.

---

## Phase 9: Run real-corpus quality gate

**BLOCKED: corpus required.**

Synthetic fixtures cannot establish that repair helps real users. This phase
needs a real chronological history with genuine failures and retypings.

### When unblocked

1. Import the history through the caller-side sanitiser, never committing it.
2. Report phase-1 metrics for: single-pass repair, iterative repair, and the
   noisy-channel scorer.
3. Record top-1 accuracy, precision, false-positive rate, and p95 latency.

### STOP

Do not substitute a public corpus of English prose. The error distribution of
typed commands is not the error distribution of prose, and the measurement would
not transfer.

---

## Phase 10: Refresh final documentation

Depends on 1–9.

1. Rewrite the README repair section against measured numbers, replacing every
   qualitative claim with a metric from phase 1.
2. Record the ablation Vista actually measured next to the published one, so a
   future reader can see whether the research transferred.
3. Refresh the `Audited baseline` table with the pinned toolchain.
4. Delete this plan's completed phases and leave only open work.

### Gate

`make verify`, and every README snippet compiled — the README is not doctested,
so each block must be checked by hand or wired into a compiled example.
