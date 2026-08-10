use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::io::{Read, Write};

mod codec;
mod read;
mod validate;
mod write;

use codec::*;
use validate::*;

use crate::adapters::{
    CandidateMatcher, MAX_SLOTS_PER_ITEM, Normalizer, PartialIndex, TokenIndex, Tokenizer,
};
use crate::api::{Config, StreamId, StreamState, StreamTable, SurfaceId, TemplateId};
use crate::engine::{ContextIndex, Predictor};
use crate::model::{
    ContextState, CorrectionLog, CorrectionPair, Dictionary, FollowerState, Ppm, SurfaceRecord,
    TemplateRecord,
};

const MAGIC: &[u8; 8] = b"VISTA\0\r\n";
const VERSION: u32 = 3;
const FEATURE_FLAGS: u64 =
    (cfg!(feature = "surface-indexes") as u64) | ((cfg!(feature = "recent-cache") as u64) << 1);
const CONFIG_WORDS: usize = 26;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug)]
pub enum SnapshotError {
    Io(std::io::Error),
    InvalidMagic,
    UnsupportedVersion(u32),
    UnsupportedFeatures(u64),
    IncompatibleConfig,
    Corrupt(&'static str),
    LimitExceeded(&'static str),
    ChecksumMismatch,
    TrailingData,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "snapshot I/O failed: {error}"),
            Self::InvalidMagic => formatter.write_str("invalid Vista snapshot magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported Vista snapshot version {version}")
            }
            Self::UnsupportedFeatures(features) => {
                write!(
                    formatter,
                    "unsupported Vista snapshot features {features:#x}"
                )
            }
            Self::IncompatibleConfig => formatter.write_str("snapshot configuration mismatch"),
            Self::Corrupt(section) => write!(formatter, "corrupt Vista snapshot {section}"),
            Self::LimitExceeded(section) => {
                write!(formatter, "Vista snapshot exceeds configured {section}")
            }
            Self::ChecksumMismatch => formatter.write_str("Vista snapshot checksum mismatch"),
            Self::TrailingData => formatter.write_str("Vista snapshot contains trailing data"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<std::io::Error> for SnapshotError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
