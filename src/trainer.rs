use crate::{
    CandidateMatcher, Config, Normalizer, Observation, Predictor, PredictorBuilder, Tokenizer,
};

/// Streaming construction facade that never retains source observations.
pub struct Trainer {
    predictor: Predictor,
}

impl Trainer {
    pub fn new(config: Config) -> Self {
        Self {
            predictor: Predictor::new(config),
        }
    }

    pub fn from_builder<N, T, M>(builder: PredictorBuilder<N, T, M>) -> Self
    where
        N: Normalizer + 'static,
        T: Tokenizer + 'static,
        M: CandidateMatcher + 'static,
    {
        Self {
            predictor: builder.build(),
        }
    }

    pub fn observe(&mut self, observation: Observation) {
        self.predictor.observe(observation);
    }

    pub fn finish(self) -> Predictor {
        self.predictor
    }
}
