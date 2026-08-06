/// Caller-defined categorical or numeric context.
///
/// The library assigns no meaning to feature names; it only counts how often a
/// feature co-occurs with an item.
#[derive(Clone, Debug, PartialEq)]
pub enum Feature {
    Categorical { name: String, value: String },
    Numeric { name: String, value: f32 },
}

impl Feature {
    pub fn categorical(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Categorical {
            name: name.into(),
            value: value.into(),
        }
    }

    pub fn numeric(name: impl Into<String>, value: f32) -> Self {
        Self::Numeric {
            name: name.into(),
            value,
        }
    }

    #[cfg(any(feature = "evaluation", feature = "surface-indexes"))]
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Categorical { name, value } => format!("c:{name}={value}"),
            Self::Numeric { name, value } => format!("n:{name}={:08x}", value.to_bits()),
        }
    }

    /// Interprets the conventional outcome names as a zero-to-one quality.
    pub(crate) fn quality(&self) -> Option<f32> {
        match self {
            Self::Categorical { name, value }
                if matches!(name.as_str(), "success" | "accepted") =>
            {
                Some(if matches!(value.as_str(), "true" | "yes" | "1") {
                    1.0
                } else {
                    0.0
                })
            }
            Self::Numeric { name, value }
                if matches!(name.as_str(), "score" | "success" | "accepted") =>
            {
                value.is_finite().then(|| value.clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

#[cfg(feature = "surface-indexes")]
pub(crate) fn context_keys(features: &[Feature], limit: usize) -> Vec<String> {
    let mut keyed: Vec<_> = features.iter().take(limit).map(Feature::key).collect();
    keyed.sort();
    keyed.dedup();
    keyed
}

/// Single features plus every unordered pair of them.
#[cfg(feature = "surface-indexes")]
pub(crate) fn association_keys(features: &[Feature], limit: usize) -> Vec<String> {
    let singles = context_keys(features, limit);
    let mut keys: Vec<_> = singles.iter().take(limit).cloned().collect();
    for left in 0..singles.len() {
        for right in left + 1..singles.len() {
            if keys.len() >= limit {
                return keys;
            }
            keys.push(format!("{}&{}", singles[left], singles[right]));
        }
    }
    keys
}
