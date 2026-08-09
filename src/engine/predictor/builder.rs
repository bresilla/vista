use crate::adapters::{
    CandidateMatcher, ContainsMatcher, IdentityNormalizer, Normalizer, Tokenizer,
    WhitespaceTokenizer,
};
use crate::api::Config;

use super::Predictor;

pub struct PredictorBuilder<N = IdentityNormalizer, T = WhitespaceTokenizer, M = ContainsMatcher> {
    config: Config,
    normalizer: N,
    tokenizer: T,
    matcher: M,
}

impl PredictorBuilder {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            normalizer: IdentityNormalizer,
            tokenizer: WhitespaceTokenizer,
            matcher: ContainsMatcher,
        }
    }
}

impl<N, T, M> PredictorBuilder<N, T, M> {
    pub fn normalizer<N2>(self, normalizer: N2) -> PredictorBuilder<N2, T, M> {
        PredictorBuilder {
            config: self.config,
            normalizer,
            tokenizer: self.tokenizer,
            matcher: self.matcher,
        }
    }

    pub fn tokenizer<T2>(self, tokenizer: T2) -> PredictorBuilder<N, T2, M> {
        PredictorBuilder {
            config: self.config,
            normalizer: self.normalizer,
            tokenizer,
            matcher: self.matcher,
        }
    }

    pub fn matcher<M2>(self, matcher: M2) -> PredictorBuilder<N, T, M2> {
        PredictorBuilder {
            config: self.config,
            normalizer: self.normalizer,
            tokenizer: self.tokenizer,
            matcher,
        }
    }
}

impl<N, T, M> PredictorBuilder<N, T, M>
where
    N: Normalizer + 'static,
    T: Tokenizer + 'static,
    M: CandidateMatcher + 'static,
{
    pub fn build(self) -> Predictor {
        Predictor::with_components(self.config, self.normalizer, self.tokenizer, self.matcher)
    }
}
