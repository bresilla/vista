use std::fmt;

/// Rejection of an observation before retained model state changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputError {
    StringTooLong {
        field: &'static str,
        bytes: usize,
        limit: usize,
    },
    TooManySlots {
        count: usize,
        limit: usize,
    },
    RetainedStringBytesExceeded {
        bytes: usize,
        limit: usize,
    },
    InconsistentNormalization,
    IdentifierExhausted(&'static str),
}

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StringTooLong {
                field,
                bytes,
                limit,
            } => write!(formatter, "{field} is {bytes} bytes; limit is {limit}"),
            Self::TooManySlots { count, limit } => {
                write!(
                    formatter,
                    "normalized item has {count} slots; limit is {limit}"
                )
            }
            Self::RetainedStringBytesExceeded { bytes, limit } => {
                write!(
                    formatter,
                    "retained strings require {bytes} bytes; limit is {limit}"
                )
            }
            Self::InconsistentNormalization => {
                formatter.write_str("raw item normalized to a different retained template")
            }
            Self::IdentifierExhausted(section) => {
                write!(formatter, "{section} identifier space is exhausted")
            }
        }
    }
}

impl std::error::Error for InputError {}
