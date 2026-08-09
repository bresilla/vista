# Vista hardening and completion plan

> This is the only plan file. Execute phases in order, run every gate, and update
> the table here. Do not create other plan/index files. Stop on a STOP condition;
> do not improvise.

## Status

- Planned at commit `a8d51a6`, 2026-08-09.
- Audited state: clean `main`, one dependency-free deterministic predictor, no
  legacy FTRL/fixed-transition path, native `make verify` passing.
- Overall: phases 1–8 are complete; real-corpus acceptance is blocked on the
  operator inputs listed in phase 9, and phase 10 remains dependent on it.

| Phase | Work | Priority | Effort | Depends | Status |
|---|---|---:|---:|---|---|
| 1 | Modularize source and enforce 600 LOC | P0 | L | — | DONE |
| 2 | Define stream/cache isolation | P0 | S | 1 | DONE |
| 3 | Bound retained and snapshot bytes | P0 | L | 1–2 | DONE |
| 4 | Produce valid research exports | P1 | S | 1 | DONE |
| 5 | Report snapshot evaluation failures | P1 | M | 3 | DONE |
| 6 | Intern PPM context identities | P1 | L | 3 | DONE |
| 7 | Pin and verify musl toolchain | P1 | M | 1 | DONE |
| 8 | Simplify Nix development shell | P2 | M | 7 | DONE |
| 9 | Run real-corpus quality gate | P1 | M | 2–6 | BLOCKED: corpus required |
| 10 | Refresh final documentation | P1 | S | 1–9 | BLOCKED: phase 9 |

Statuses: `TODO`, `IN PROGRESS`, `DONE`, `BLOCKED: reason`, `REJECTED: reason`.

## Non-negotiable repository rules

- Use Makefile targets for all relevant build/test/lint/benchmark/export work.
- Do not use Python to edit or process files.
- No maintained repository file may exceed **600 physical lines**, including
  Rust, tests, examples, Makefile, CI, Markdown, and this plan. Generated/ignored
  `.git/`, `target/`, and `.direnv/` contents are excluded.
- Add `make loc-check`, include it in `make verify`, and count tracked plus
  nonignored untracked files. The target prints every violation and exits nonzero.
- Comments describe only what code does; no rationale/changelog comments and no
  comment volume above roughly 20% of implementation.
- Keep one current pre-1.0 path; do not leave forwarding/legacy implementations.
- No dependency, unsafe code, native library, async runtime, network client, or
  predictor-owned storage without explicit maintainer approval after a STOP.
- Never commit application history, secrets, or generated research data.
- Suggested commits are title-only Conventional Commits, unsigned, <=50 chars.
- Run `make verify` after every phase; do not start a dependent phase while red.

## Product boundary

Vista predicts complete previously observed items. It never generates unseen
text. One variable-order PPM path, integrated recent cache, normalization,
surface selection, and bounded retrieval indexes work together. Learned state is
the compiled snapshot, not neural weights. This plan does not add a second
predictor, LLM, database, service, or application-history reader.

## Audited baseline

Passed: `make verify`, `make run`, synthetic/fixture evaluation, research export,
`make bench-tiny`, `make bench-million`, and `make size-full`.

| Rust 1.94.0 x86-64 Linux measurement | Value |
|---|---:|
| all-feature tests | 60 |
| empty / minimal / Vista increment | 299,664 / 463,504 / 163,840 B |
| all-feature example | 553,616 B |
| tiny heap estimate | 52,504 B |
| million ingest / p95 | 63,154 events/s / 88 us |
| million heap / snapshot / load | 481,175 B / 202,370 B / 1 ms |

The host and Nix shell currently lack the musl Rust standard library. An earlier
configured environment built musl; reproducible provisioning remains open.

---

## Phase 1: Modularize source and enforce 600 LOC

### Decision: folders, not multiple crates

Keep one Cargo package/library and organize it into domain folders. Do **not**
add `[workspace]` or `crates/` now.

Evidence: Vista is 4,924 library LOC across 23 files, exposes one API, has no
dependencies, and `Predictor`/snapshot deeply share crate-private state. Feature
gates already remove evaluation/research from tiny builds, and fat LTO handles
unused code. Multiple crates would force cross-crate APIs, feature forwarding,
manifests, and compatibility work without reducing binary size.

Reconsider crates only if a component needs independent publishing/versioning,
evaluation/research gains isolated dependencies, a committed `no_std` core is
required, another project consumes a lower-level component directly, or measured
compile/team ownership proves a benefit.

### Target tree

```text
src/
├── lib.rs
├── api/       {mod,config,feature,item,observation,stream}.rs
├── adapters/  {mod,matcher,normalizer,tokenizer}.rs
├── model/     {mod,cache,dictionary,ppm,statistics}.rs
├── engine/
│   ├── mod.rs, candidates.rs, context.rs, explanation.rs, pruning.rs, ranking.rs
│   ├── predictor/{mod,builder,observe,predict,maintenance}.rs
│   └── trainer.rs
├── snapshot/  {mod,codec,read,write,validate}.rs
├── evaluation/{mod,accumulator,baseline,runner}.rs
└── research/  mod.rs
```

Map flat files by domain:

- `config feature item observation stream` -> `api/`.
- `matcher normalizer tokenizer` -> `adapters/`.
- `cache dictionary ppm statistics` -> `model/`.
- `candidates context explanation pruning ranking trainer` -> `engine/`.
- `predictor.rs` -> the `engine/predictor/` files by responsibility.
- `snapshot.rs` -> `snapshot/` codec/read/write/validation files.
- `evaluation.rs` -> `evaluation/` accumulator/baseline/runner files.
- `export.rs` -> `research/mod.rs` unless it approaches 600 LOC later.

Folder `mod.rs` files apply feature gates and re-export public API with `pub use`
and internals only with `pub(crate) use`. `src/lib.rs` keeps crate docs, domain
declarations, feature gates, and the exact existing root public exports. Do not
make internal state public to ease imports. Only `src/lib.rs` may remain directly
under `src/`.

### Current LOC violations and required splits

| File | Audit LOC | Required split |
|---|---:|---|
| `tests/predictor.rs` | 1,247 | shared `tests/support/mod.rs` plus sequence, normalization, snapshot, evaluation, research integration tests |
| `plans/PLAN.md` | 1,177 before this revision | keep this replacement <=600 LOC |
| `src/snapshot.rs` | 953 | `snapshot/{mod,codec,read,write,validate}.rs` |
| `src/evaluation.rs` | 644 | `evaluation/{mod,accumulator,baseline,runner}.rs` |
| `src/predictor.rs` | 614 | `engine/predictor/{mod,builder,observe,predict,maintenance}.rs` |

Split tests by behavior, not arbitrary line ranges:

- `tests/sequence.rs`: sequence, streams, cache, probability, trainer, bounds.
- `tests/normalization.rs`: templates, slots, surfaces, matcher/tokenizer, ranking.
- `tests/snapshot.rs`: persistence, corruption, compatibility, failure atomicity.
- `tests/evaluation.rs`: metrics, chronology, Unicode completion, stress quality.
- `tests/research.rs`: deterministic/gapped/interleaved exports.
- `tests/support/mod.rs`: only shared item/observation/query helpers.

Add explicit `[[test]] required-features` entries for snapshot/evaluation/research
targets where needed. Do not duplicate helpers across test crates.

### Snapshot-key hazard

Built-in adapter keys default to `type_name::<Self>()`; moving modules would
silently change version-1 keys. Before moving, override built-ins with the live
old values and test them:

- `vista::normalizer::IdentityNormalizer`;
- `vista::tokenizer::WhitespaceTokenizer`;
- `vista::matcher::ContainsMatcher`.

Probe current values first. If they differ, lock the probed values. Leave custom
adapter defaults unchanged. Equivalent pre/post histories must keep identical
snapshot bytes. Phase 3, not this move, owns snapshot version 2.

### `make loc-check` contract

Add `MAX_FILE_LOC ?= 600`. Use Git's tracked/nonignored file list with NUL-safe
paths, count physical lines, print `lines path` for each excess file, and fail if
any exists. Exclude only ignored/generated directories, never hand-maintained
file types. Add `.PHONY`, help text, and `loc-check` to `verify`.

### Steps

1. Capture pre-move adapter keys, snapshot bytes, `make size`, `make size-full`,
   feature checks, and test results under ignored `target/` output.
2. Add `loc-check`; confirm it fails and lists the five audited violations.
3. Add stable built-in adapter keys/tests; run `make verify` before moves.
4. Create folders, move files with history, and remove all flat originals.
5. Split predictor/snapshot/evaluation/tests by the responsibilities above.
6. Update imports and narrow domain re-exports; avoid broad `use crate::*` inside
   implementation modules.
7. Preserve every current feature combination and root public import.
8. Compare adapter keys, snapshot bytes, minimal/full sizes, and behavior.
9. Update README architecture description; do not expose private module paths as
   supported API.
10. Run structural/LOC/full gates.

### Gates

```sh
make loc-check
make check-minimal
make test-minimal
make test
make check-all
make clippy
make rustdoc
make size
make size-full
make verify
find src -maxdepth 1 -type f -name '*.rs' -print
git diff --find-renames --stat
```

Expected: only `src/lib.rs` from the `find`; no maintained file >600 LOC; Git
recognizes moves; public examples/tests need no import changes; keys/snapshots are
identical. Minimal Vista increment and full example may grow no more than the
lower of 1% or 4 KiB.

### Done / STOP

Done when one crate remains, all files satisfy 600 LOC, root API/features and
snapshots are unchanged, no forwarding modules remain, size stays within limit,
and all gates pass.

STOP if internal state must become public, boundaries create an unsolved cycle,
keys/snapshots/behavior change, size exceeds the limit, or a real independent
crate boundary emerges.

Suggested commit: `refactor: organize source modules`

---

## Phase 2: Define stream/cache isolation

Current code uses stream-local evidence then global fallback
(`src/cache.rs:114-131`), and tests intentionally expose global evidence to an
unseen stream. Adopt this contract unless explicitly rejected: `StreamId` is a
continuity boundary, not a tenant boundary; streams never form direct transitions;
gaps/breaks reset private continuity; global aggregate evidence may cross streams;
separate privacy domains require separate predictors.

Steps: document the contract, preserve global-fallback coverage, add an
interleaved-stream no-cross-transition test, and preserve gap/break/eviction
tests. Run `make test && make verify`.

Done when continuity and privacy are unambiguous and separately tested. STOP if
strict tenant isolation is chosen; that requires partitioning all global state.

Suggested commit: `docs: define stream cache isolation`

---

## Phase 3: Bound retained and snapshot bytes

Items/features accept unlimited strings; slot count alone is bounded; observation
is infallible; snapshot only caps one string at write/read and has no total budget.
A live model can therefore become unsnapshotable.

Add normalized config limits:

| Limit | Default | Tiny |
|---|---:|---:|
| `max_string_bytes` | 65,536 | 1,024 |
| `max_retained_string_bytes` | 67,108,864 | 65,536 |
| `max_snapshot_bytes` | 134,217,728 | 1,048,576 |

Add typed `InputError`; make predictor/trainer observe and replay return `Result`.
Validate raw, normalized, and tokenized data before any mutation. Charge every
owned string, release on eviction/forgetting, expose logical retained bytes in
stats, and reject without changing clock/stats/predictions/snapshot bytes. Never
truncate or silently drop data.

Use checked cumulative snapshot reader/writer budgets. Bump format to version 2,
fingerprint new limits, and explicitly reject version 1. Test oversized raw and
derived strings, total budgets, transactional rejection, overflow, corrupt data,
failed-load atomicity, and that every accepted model serializes.

Gates: `make check-all`, `make test`, `make test-minimal`, `make verify`, tiny and
million benchmarks. STOP if validation cannot precede mutation, v1 migration is
required, ownership is undefined, or data would be truncated.

Suggested commit: `fix: bound predictor input bytes`

---

## Phase 4: Produce valid research exports

SPMF currently emits ID zero, but its documented items are positive integers;
dictionary TSV is ambiguous for tabs/newlines. Assign IDs from one consistently
in SPMF/plain/dictionary, preserve `-1`/`-2`, check overflow, and escape `\\`, tab,
carriage return, and newline deterministically. Update tests/docs.

Gates:
`make test`; `make research-export HISTORY=tests/fixtures/workflow.txt OUTPUT=target/research-check`; `make verify`.
Require positive SPMF items, matching dictionary IDs, escaped fields, preserved
gap/order behavior, and no external dependency.

Suggested commit: `fix: emit valid SPMF identifiers`

---

## Phase 5: Report snapshot evaluation failures

Evaluation currently turns snapshot write/read failure into success-looking zero
metrics. Replace scalars with typed `Success { bytes, load_time }`,
`Failed { stage, error }`, and `NotMeasured`. Preserve other metrics on failure;
never include application contents in errors. Test success/write/read/not-measured
using phase 3 budgets. Print explicit status in the evaluation example.

Gates: `make test`, `make evaluate`, `make verify`. STOP if phase 3 lacks
deterministic failure or the API permits contradictory success/error state.

Suggested commit: `fix: report snapshot metric failures`

---

## Phase 6: Intern PPM context identities

PPM clones each context vector into primary, eviction, member, and follower
indexes while heap estimation omits many clones. Add checked `ContextId(u32)`:
one map owns each context vector, state is keyed by ID, and eviction/member/
follower indexes store IDs. Reuse IDs deterministically or test exhaustion.

Serialize logical vectors/state, not transient IDs; rebuild reverse indexes on
load. Count owned capacities/associations in memory estimates. Add optional
dependency-free `make bench-memory` using a system RSS tool outside `verify`.
Record before/after estimate, RSS, snapshot, throughput, and p95 under `target/`.

Gates: `make test`, tiny/million/memory benchmarks, `make verify`. STOP if IDs
change probability/order/snapshot semantics, require full-context update scans,
unsafe instrumentation, or another snapshot format change.

Suggested commit: `perf: intern PPM context identities`

---

## Phase 7: Pin and verify musl toolchain

CI floats stable, Nix differs, musl is absent, and size paths hardcode native
`target/release`. Pin one exact official stable edition-2024 compiler everywhere;
include rustfmt, Clippy, rust-analyzer where appropriate, and
`x86_64-unknown-linux-musl`.

Make release paths target-aware. Add `check-musl` reusing existing targets and
`size-musl` reporting empty/minimal/increment/full sizes. Add target-specific CI
coverage/cache and document that consumer dependencies determine final static
linkage.

Gates: `make verify`, `make size`, `make check-musl`, `make size-musl`. STOP if
the compiler fails current code, musl needs native dependencies, or another
triple is required without selection.

Suggested commit: `ci: pin and verify musl builds`

---

## Phase 8: Simplify Nix development shell

Remove unrelated nixGL/Nvidia/Vulkan/X11/Wayland/ALSA/udev/WGPU/clang/mold setup.
Keep only the pinned Rust/musl toolchain and packages directly mapped to Makefile
commands (Git, coreutils/binutils, optional RSS tool, and release tool if used).
Remove display/Nvidia/impure logic from `.envrc`; keep the flake pure and pinned.

Gates: `nix flake show`, `nix develop -c make verify`,
`nix develop -c make check-musl`. STOP if checked-in code truly needs a removed
native library or the chosen Nix toolchain cannot provide musl.

Suggested commit: `build: minimize the Nix dev shell`

---

## Phase 9: Run real-corpus quality gate

Remain blocked until the operator supplies sanitized chronological `HISTORY`,
production config/limits, production normalizer or identity confirmation, a
predefined recent-regime slice, and confirmation that aggregates are safe.

Freeze one split/config before evaluation. Compare production variable-order,
fixed orders 3/1, most frequent, cache-off, identity normalization, and production
candidate limits. Report top-k, MRR, log-loss/perplexity, recall/coverage,
normalization, retained bytes/counts, latency, snapshot status/size/load, and
completion savings. Never tune on the evaluation partition.

Acceptance: top-5 >= fixed-3; log-loss better than frequent/fixed-1;
normalization improves recall or memory without predeclared material top-5 loss;
cache improves the predefined recent slice; candidate recall >=99%. A miss needs
explicit acceptance, not moved thresholds.

Gates: `make verify`, `make evaluate HISTORY="$HISTORY"`, optional approved
research export under `target/`, `make bench-million`, clean Git state. STOP on
missing/unsafe data, unequal splits, snapshot failure, or unaccepted threshold.

Suggested commit for generic evaluator changes: `test: add real corpus release gate`

---

## Phase 10: Refresh final documentation

Verify phases 1–9 against done criteria, then remeasure on the pinned toolchain.
Record commit/date/compiler/targets, architecture, 600-LOC gate, stream contract,
byte limits/API errors, snapshot version, export grammar, native/musl commands,
sizes, heap/RSS, benchmarks, real aggregates, and accepted deviations. Never copy
old numbers or promise portable performance.

Final commands:

```sh
make loc-check
make verify
make run
make evaluate
make evaluate EXAMPLE=tests/fixtures/workflow.txt
make research-export HISTORY=tests/fixtures/workflow.txt OUTPUT=target/research-check
make bench-tiny
make bench-million
make size
make size-full
make check-musl
make size-musl
```

Complete only when every command passes, every maintained file is <=600 LOC,
real thresholds pass or deviations are accepted, docs match measurements, and
generated/private data is untracked.

Suggested commit: `docs: refresh predictor release record`

## Global STOP conditions

Stop if stream isolation is undecided; byte rejection cannot be transactional;
snapshot v2 needs unapproved migration; context IDs change semantics; musl needs
new runtime/native dependencies; real inputs are absent/unsafe; a threshold fails
without acceptance; `make verify` rewrites tracked files; any maintained file
exceeds 600 LOC; or work escapes a phase's declared scope.
