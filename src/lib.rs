//! Deterministic next-sentence prediction from chronological history.
//!
//! Vista learns variable-order template sequences, adapts through one integrated
//! recent-history cache, and returns concrete surfaces observed previously. The
//! caller owns collection, sanitisation, normalization policy, and storage paths.
//!
//! A [`StreamId`] separates sequence continuity, not privacy or tenancy. Direct
//! transitions never cross streams, and gaps or [`Predictor::break_stream`]
//! reset a stream's private continuity. Aggregate model and recent-cache
//! evidence may still be shared across streams. Use separate predictors for
//! separate privacy domains.

mod adapters;
mod api;
mod engine;
#[cfg(feature = "evaluation")]
mod evaluation;
mod model;
#[cfg(feature = "research")]
mod research;
#[cfg(feature = "snapshot")]
mod snapshot;

pub use adapters::{
    CandidateMatcher, ContainsMatcher, IdentityNormalizer, ItemMatcher, MatchInput, NormalizedItem,
    Normalizer, SimilarityMatcher, Tokenizer, WhitespaceTokenizer,
};
pub use api::{Config, Feature, InputError, Item, Observation, Position, Query, StreamId, Weights};
pub use engine::{Explanation, ModelStats, Prediction, Predictor, PredictorBuilder, Trainer};
#[cfg(feature = "evaluation")]
pub use evaluation::{
    Baseline, CorrectionAttempt, CorrectionEvaluation, CorrectionMetrics, CorrectionReport,
    Evaluation, EvaluationMetrics, EvaluationReport, SnapshotMeasurement, SnapshotStage,
};
pub use model::CorrectionPair;
#[cfg(feature = "research")]
pub use research::{ResearchExport, ResearchExportError};
#[cfg(feature = "snapshot")]
pub use snapshot::SnapshotError;
