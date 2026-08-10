/// The strongest deterministic reasons behind one prediction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Explanation {
    pub reasons: Vec<String>,
}

#[cfg(feature = "explanations")]
pub(crate) enum Reason {
    Sequence {
        probability: f64,
        depth: usize,
        count: u64,
        total: u64,
    },
    Global {
        probability: f64,
    },
    Backoff {
        steps: usize,
    },
    Cache {
        probability: f64,
    },
    Context {
        adjustment: f64,
    },
    Surface {
        adjustment: f64,
    },
    Outcome {
        adjustment: f64,
    },
    Partial {
        adjustment: f64,
    },
}

#[cfg(feature = "explanations")]
impl Reason {
    fn contribution(&self) -> f64 {
        match self {
            Self::Sequence { probability, .. }
            | Self::Global { probability }
            | Self::Cache { probability } => *probability,
            Self::Context { adjustment }
            | Self::Surface { adjustment }
            | Self::Outcome { adjustment }
            | Self::Partial { adjustment } => *adjustment,
            Self::Backoff { .. } => 0.0,
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Sequence {
                probability,
                depth,
                count,
                total,
            } => format!(
                "matched sequence depth {depth} with {count}/{total} observations; long-term probability {probability:.6}"
            ),
            Self::Global { probability } => format!(
                "used the global sentence distribution; long-term probability {probability:.6}"
            ),
            Self::Backoff { steps } => format!("backed off {steps} context level(s)"),
            Self::Cache { probability } => {
                format!("recent-cache probability {probability:.6}")
            }
            Self::Context { adjustment } => {
                format!("associated with the current context; adjustment {adjustment:.6}")
            }
            Self::Surface { adjustment } => {
                format!("preferred historical surface; adjustment {adjustment:.6}")
            }
            Self::Outcome { adjustment } => {
                format!("has positive observed outcomes; adjustment {adjustment:.6}")
            }
            Self::Partial { adjustment } => {
                format!("matches the partial input; adjustment {adjustment:.6}")
            }
        }
    }
}

#[cfg(feature = "explanations")]
#[derive(Default)]
pub(crate) struct Reasons {
    entries: Vec<Reason>,
}

#[cfg(feature = "explanations")]
impl Reasons {
    pub(crate) fn push(&mut self, reason: Reason) {
        self.entries.push(reason);
    }

    pub(crate) fn finish(mut self, limit: usize) -> Explanation {
        self.entries.sort_by(|a, b| {
            b.contribution()
                .total_cmp(&a.contribution())
                .then_with(|| a.render().cmp(&b.render()))
        });
        Explanation {
            reasons: self
                .entries
                .into_iter()
                .take(limit)
                .map(|reason| reason.render())
                .collect(),
        }
    }
}
