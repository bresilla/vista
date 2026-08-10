use std::cmp::Ordering;

use crate::api::Item;
#[cfg(feature = "explanations")]
use crate::engine::{Explanation, Reason, Reasons};

const MAX_LOG_RATIO: f64 = 8.0;

/// A concrete historical completion and its next-template probability.
#[derive(Clone, Debug, PartialEq)]
pub struct Prediction {
    pub item: Item,
    pub template: Item,
    pub probability: f64,
    pub score: f64,
    pub context_depth: usize,
    /// Repair passes that produced `item`; zero outside `predict_aligned`.
    pub repair_iterations: usize,
    #[cfg(feature = "explanations")]
    pub explanation: Explanation,
}

impl Prediction {
    pub(crate) fn cmp_rank(a: &Self, b: &Self) -> Ordering {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.item.cmp(&b.item))
    }
}

pub(crate) struct RankInput {
    pub(crate) item: Item,
    pub(crate) template: Item,
    pub(crate) probability: f64,
    #[cfg(feature = "explanations")]
    pub(crate) long_term_probability: f64,
    pub(crate) context: f64,
    pub(crate) surface: f64,
    pub(crate) outcome: f64,
    pub(crate) partial: f64,
    pub(crate) deepest: usize,
    #[cfg(feature = "explanations")]
    pub(crate) backoffs: usize,
    #[cfg(feature = "explanations")]
    pub(crate) count: u64,
    #[cfg(feature = "explanations")]
    pub(crate) total: u64,
    #[cfg(feature = "explanations")]
    pub(crate) cache_probability: Option<f64>,
}

pub(crate) fn rank(input: RankInput, weights: &crate::Weights) -> Prediction {
    // Context and surface use capped ln(1 + ratio); outcome and partial use [0, 1].
    let context = input.context.ln_1p().min(MAX_LOG_RATIO) * weights.context;
    let surface = input.surface.ln_1p().min(MAX_LOG_RATIO) * weights.surface;
    let outcome = input.outcome.clamp(0.0, 1.0) * weights.outcome;
    let partial = input.partial.clamp(0.0, 1.0) * weights.partial;
    let score =
        input.probability.max(f64::MIN_POSITIVE).ln() + context + surface + outcome + partial;
    #[cfg(feature = "explanations")]
    let explanation = {
        let mut reasons = Reasons::default();
        if input.deepest > 0 {
            reasons.push(Reason::Sequence {
                probability: input.long_term_probability,
                depth: input.deepest,
                count: input.count,
                total: input.total,
            });
        } else {
            reasons.push(Reason::Global {
                probability: input.long_term_probability,
            });
        }
        if input.backoffs > 0 {
            reasons.push(Reason::Backoff {
                steps: input.backoffs,
            });
        }
        if let Some(cache) = input.cache_probability {
            reasons.push(Reason::Cache { probability: cache });
        }
        if input.context > 0.0 {
            reasons.push(Reason::Context {
                adjustment: context,
            });
        }
        if input.surface > 0.0 {
            reasons.push(Reason::Surface {
                adjustment: surface,
            });
        }
        if input.outcome > 0.0 {
            reasons.push(Reason::Outcome {
                adjustment: outcome,
            });
        }
        if input.partial > 0.0 {
            reasons.push(Reason::Partial {
                adjustment: partial,
            });
        }
        reasons.finish(7)
    };
    Prediction {
        item: input.item,
        template: input.template,
        probability: input.probability,
        score,
        context_depth: input.deepest,
        repair_iterations: 0,
        #[cfg(feature = "explanations")]
        explanation,
    }
}
