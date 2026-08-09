# vista

Vista is a deterministic, bounded predictor for ordered items — commands,
workflow steps, actions, tool calls. It learns from chronological history and
returns concrete items that were observed before.

No LLM, no neural network, no embeddings, no async runtime, no dependencies.

## What it does

**Predicts what comes next.** A variable-order PPM model learns sequences up to
order 8, blended with a recent-history cache, adjusted by caller context,
outcomes, and partial input.

**Repairs what you got wrong.** Pass in a failed item and Vista rebuilds it from
the structure of items history already contains:

```text
you typed:  apt install ripgrep
history:    sudo apt install fd
result:     sudo apt install ripgrep
```

No rules, no templates, no dictionary. Shared tokens are structure, tokens only
history has are the repair, and differing tokens are decided by what you have
actually typed before.

## Use

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

To repair instead of predict, call `predict_aligned(&query, &failed_item)`.

## Properties

- **Bounded.** Every collection has a hard limit in `Config`. `Config::tiny()`
  is a strict low-memory preset.
- **Deterministic.** Identical inputs give identical output, down to tie-breaks.
- **Transactional.** A rejected observation leaves the model byte-identical.
- **Caller-owned.** You own persistence, sanitization, retention, and consent.
  Vista holds only derived state and reads and writes streams, never paths.
- **Gap-aware.** A missing position breaks continuity rather than joining
  unrelated neighbours.

`StreamId` separates sequence continuity, not privacy — use separate predictors
for separate users or tenants.

## Features

Default build: live prediction, repair, recent cache, explanations, snapshots,
and retrieval indexes. Turn defaults off for the smallest sequence-only core and
add back `recent-cache`, `snapshot`, `explanations`, `surface-indexes`,
`evaluation`, or `research` as needed.

```toml
vista = { path = "../vista", default-features = false }
```

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
