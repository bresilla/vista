//! Deterministic next-sentence prediction from chronological history.
//!
//! Vista learns variable-order template sequences, adapts through one integrated
//! recent-history cache, and returns concrete surfaces observed previously. The
//! caller owns collection, sanitisation, normalization policy, and storage paths.

#[cfg(any(feature = "recent-cache", feature = "snapshot"))]
mod cache;
mod candidates;
mod config;
#[cfg(any(feature = "snapshot", feature = "surface-indexes"))]
mod context;
mod dictionary;
#[cfg(feature = "evaluation")]
mod evaluation;
mod explanation;
#[cfg(feature = "research")]
mod export;
mod feature;
mod item;
mod matcher;
mod normalizer;
mod observation;
mod ppm;
mod predictor;
#[cfg(feature = "surface-indexes")]
mod pruning;
mod ranking;
#[cfg(feature = "snapshot")]
mod snapshot;
mod statistics;
mod stream;
mod tokenizer;
mod trainer;

pub use config::{Config, Weights};
#[cfg(feature = "evaluation")]
pub use evaluation::{Baseline, Evaluation, EvaluationMetrics, EvaluationReport};
pub use explanation::Explanation;
#[cfg(feature = "research")]
pub use export::{ResearchExport, ResearchExportError};
pub use feature::Feature;
pub use item::Item;
pub use matcher::{CandidateMatcher, ContainsMatcher, ItemMatcher};
pub use normalizer::{IdentityNormalizer, NormalizedItem, Normalizer};
pub use observation::{Observation, Query};
pub use predictor::{ModelStats, Predictor, PredictorBuilder};
pub use ranking::Prediction;
#[cfg(feature = "snapshot")]
pub use snapshot::SnapshotError;
pub use stream::{Position, StreamId};
pub use tokenizer::{Tokenizer, WhitespaceTokenizer};
pub use trainer::Trainer;
