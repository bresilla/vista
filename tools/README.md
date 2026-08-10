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
