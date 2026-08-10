use crate::{
    CandidateMatcher, Config, InputError, Normalizer, Observation, Predictor, PredictorBuilder,
    Tokenizer,
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

    pub fn observe(&mut self, observation: Observation) -> Result<(), InputError> {
        self.predictor.observe(observation)
    }

    pub fn replay<I>(&mut self, observations: I) -> Result<(), InputError>
    where
        I: IntoIterator<Item = Observation>,
    {
        self.predictor.replay(observations)
    }

    pub fn finish(self) -> Predictor {
        self.predictor
    }
}
