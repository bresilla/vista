#[cfg(any(feature = "recent-cache", feature = "snapshot"))]
mod cache;
mod corrections;
mod dictionary;
mod ppm;
mod statistics;

#[cfg(any(feature = "recent-cache", feature = "snapshot"))]
pub(crate) use cache::RecentCache;
pub(crate) use corrections::CorrectionLog;
pub use corrections::CorrectionPair;
pub(crate) use dictionary::{Dictionary, Stats};
#[cfg(feature = "snapshot")]
pub(crate) use dictionary::{SurfaceRecord, TemplateRecord};
#[cfg(feature = "snapshot")]
pub(crate) use ppm::{ContextState, FollowerState};
pub(crate) use ppm::{Ppm, PpmHistory};
#[cfg(feature = "surface-indexes")]
pub(crate) use statistics::context_ratio;
pub(crate) use statistics::surface_ratio;
