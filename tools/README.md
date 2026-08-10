# Research comparison export

`ResearchExport` converts chronological observations to deterministic integer
sequences. Use the same observations, stream gaps, split point, and normalizer
for every external model.

- `write_spmf` emits the SPMF/IPredict format: `id -1 ... -2`.
- `write_plain` emits one whitespace-separated sequence per line for PBCT or
  custom PPM tools.
- `write_dictionary` emits `id`, escaped namespace, and escaped template as
  tab-separated text. Backslash, tab, carriage return, and newline use
  `\\`, `\t`, `\r`, and `\n` escapes.
- A position gap ends the current sequence. Streams are never joined.
- IDs are positive integers assigned by first chronological appearance starting
  at one. SPMF `-1` and `-2` remain itemset and sequence terminators.

Create all three files from sanitized one-sentence-per-line history:

```sh
make research-export HISTORY=/path/to/history OUTPUT=target/research
```

For application normalization or multiple streams, call
`ResearchExport::with_normalizer` from the application so the export uses the
same symbols and boundaries as production.

## Official challenger setup

IPredict's official Makefile compiles and launches its Java controller:

```sh
git clone https://github.com/tedgueniche/IPredict.git
cd IPredict
make
make run
```

The stock controller uses its bundled datasets and k-fold evaluation. Do not
compare those numbers with Vista. Add `sequences.spmf` as one dataset and change
`MainController` to one chronological train/test split before running CPT+,
first-order PPM, and AKOM. Record the exact commit and split with the result.

PBCT's official repository exposes a library and reference notebook rather than
a benchmark CLI:

```sh
git clone https://github.com/daniyarghani/pbct.git
cd pbct
python3 -m venv .venv
.venv/bin/pip install -e lib/
.venv/bin/pip install jupyter
.venv/bin/jupyter nbconvert --to notebook --execute notebooks/test_pbct.ipynb \
  --output test_pbct.executed.ipynb
```

Adapt the notebook to read `sequences.txt`, preserving the chronological split.
PBCT preprocessing and posterior settings must be recorded with the result.

The space-efficient VOMM reference is a scalability implementation, not a
drop-in accuracy harness. Its official optimized build is:

```sh
git clone --recursive https://github.com/jnalanko/VOMM.git
cd VOMM
make optimized
```

External research repositories are intentionally not dependencies or CI jobs.
Report top-k accuracy and log-loss only when every tool uses the identical
chronological split and normalized dictionary. Never compare shuffled results.

# tldr page extraction

`tldr-pairs.sh` reads a local tealdeer/tldr page cache and emits tab-separated
`description<TAB>command` lines. Commands keep their `{{slot}}` placeholders,
which already match the template form a `Normalizer` produces, and each carries
a natural-language description — the pairing an intent-to-item index needs.

```sh
tldr --update                       # populate the cache first
tools/tldr-pairs.sh > pairs.tsv     # defaults to the common and linux platforms
tools/tldr-pairs.sh ~/.cache/tealdeer/tldr-pages/pages.en common
```

Pages are CC-BY-4.0 from the tldr-pages project. Anything shipped from this
output must carry that attribution. The extract is not committed here; it is
regenerated from whatever cache the user already has.

## Measured: reference repair

Skeletons make a usable repair corpus. Held-out test on a real 308,000-command
history: train on the first two thirds, then damage one character in 3,000
commands the personal model had **never seen**.

| Repairing from | Recovered |
|---|---:|
| personal history alone | 0.010 |
| **tldr skeletons alone** | **0.106** |
| either | 0.114 |

Only 0.03% of those targets appear in tldr at all, so the gain is not retrieval.
Repair composes: the reference supplies a correct spelling and the caller's own
arguments survive, producing commands present in neither corpus.

```
typed:  hexe mux float --hlp
  ->    hexe mux float --help
  via:  crane index filter --help
```

`hexe` is not documented anywhere; an unrelated page supplied `--help`.

Keep the corpora in separate `Predictor` instances. A reference predictor must
never answer `predict`, or 29,000 commands the caller has never run become
suggestions. See `crates/recall/examples/reference.rs`.

## Measured: structural retrieval

Repair works by structural analogy, not word lookup. `crane index filter --help`
fixes `hexe mux float --hlp` because the two are arranged alike, sharing no
vocabulary at all.

Retrieval was the whole bottleneck: of the repairs that failed, 87.5% failed
because the correct word never reached a candidate, while alignment and ranking
together failed 0.3%. Two obvious fixes both lost to the baseline — near
spellings alone evict the structurally similar candidates composition needs,
and shape alone returns 22,000 candidates. Their intersection is narrow, 18
candidates on average, and reaches 44% of the corrections.

| Repairing from | before | after |
|---|---:|---:|
| all failures | 0.137 | **0.206** |
| item run before | 0.184 | **0.398** |
| item never run before | 0.118 | **0.131** |

Prediction is unaffected: structural retrieval runs only on the repair path,
and top-1 and latency are unchanged.
