# vista

Two peer crates for predicting and repairing ordered items — commands, workflow
steps, actions, tool calls.

```
crates/
├── recall/   vista-recall    what you have done      deterministic, zero deps
└── invent/   vista-invent    what you could do       semantic, has a model
```

Neither depends on the other. Use either alone, or compose both in your own
crate — the interface between them is plain text and caller-owned identifiers,
so no type crosses the boundary.

| | vista-recall | vista-invent |
|---|---|---|
| Knows | your history | meaning, beyond your history |
| Holds | dictionary, sequence model, indexes | vectors and a model |
| Depends on | nothing | an embedding model |
| Answers | what comes next; what you meant | what else could have been meant |
| Determinism | exact, tie-breaks included | deterministic embeddings |
| Status | **working** | **stub — not implemented** |

---

## vista-recall

Learns from chronological history. `predict` returns concrete items observed
before; `predict_aligned` recombines them, so a repair may be a string that was
never observed even though every part of it was. No LLM, no neural network, no
embeddings, no async runtime, no dependencies.

**Predicts what comes next.** A variable-order PPM model learns sequences,
blended with a recent-history cache, adjusted by caller context, outcomes, and
partial input.

**Repairs what you got wrong.** Pass in a failed item and it rebuilds it from
the structure of items history already contains:

```text
you typed:  apt install ripgrep
history:    sudo apt install fd
result:     sudo apt install ripgrep
```

No rules, no templates, no dictionary. Shared tokens are structure, tokens only
history has are the repair, and differing tokens are decided by what you have
actually typed before.

```rust
use vista_recall::{Config, Item, Observation, Position, Predictor, Query, StreamId};

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

To repair instead of predict, call `predict_aligned(&query, &failed_item)`.

### Properties

- **Bounded.** Every collection has a hard limit in `Config`. `Config::tiny()`
  is a strict low-memory preset.
- **Deterministic.** Identical inputs give identical output, down to tie-breaks.
- **Transactional.** A rejected observation leaves the model byte-identical.
- **Caller-owned.** You own persistence, sanitization, retention, and consent.
  It holds only derived state, and reads and writes streams, never paths.
- **Gap-aware.** A missing position breaks continuity rather than joining
  unrelated neighbours.

`StreamId` separates sequence continuity, not privacy — use separate predictors
for separate users or tenants.

### Features

Default build: live prediction, repair, recent cache, explanations, snapshots,
and retrieval indexes. Turn defaults off for the smallest sequence-only core and
add back `recent-cache`, `snapshot`, `explanations`, `surface-indexes`,
`evaluation`, or `research` as needed.

```toml
vista-recall = { path = "../vista/crates/recall", default-features = false }
```

---

## vista-invent

**Not implemented.** A stub crate reserving the other half of the problem.

`vista-recall` can only return what it has seen. Measured on a real 308,000-line
shell history, the intended next command never reached the candidate set **26.8%
of the time** — because it had never been run before. No amount of ranking
recovers that; it needs knowledge from outside your history.

The intended shape is an embedding index keyed by caller identifiers, so it
holds vectors and a model but never your history:

```text
insert(id, text)        embed and retain a vector
remove(id)              drop it
search(text, limit)     nearest identifiers by cosine similarity
```

That buys intent-to-item matching that literal matching cannot reach — asking
for `upload my changes` and being handed `git push origin main`.

---

## Measured

Prequential evaluation over 20,000 real shell commands, 3,585 distinct, each
predicted before being learned:

| Predictor | top-1 |
|---|---:|
| Repeat last command | 0.076 |
| Most frequent overall | 0.112 |
| Best successor (bigram) | 0.170 |
| **vista-recall** | **0.189** |
| Answer present in candidate set | 0.732 |

Ranking sits near the ceiling for this class of model: every configuration knob
moves top-1 by less than 0.01, and surprise-based re-ranking makes it strictly
worse. The remaining headroom is the 26.8% that retrieval never surfaces.

These are measurements from one machine and one corpus, not portable claims.

## Commands

```sh
make build
make test
make verify
make evaluate
```

## Documentation

**[docs/HOWTO.md](docs/HOWTO.md)** covers everything: core types, adapters, how
prediction and repair actually work, the full configuration table, snapshots,
evaluation, memory bounds, and privacy.
