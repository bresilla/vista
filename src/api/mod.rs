mod config;
mod error;
mod feature;
mod item;
mod observation;
mod stream;

pub use config::{Config, Weights};
pub use error::InputError;
pub use feature::Feature;
pub use item::Item;
pub use observation::{Observation, Query};
pub use stream::{Position, StreamId};

#[cfg(feature = "surface-indexes")]
pub(crate) use feature::association_keys;
pub(crate) use item::{SurfaceId, TemplateId};
#[cfg(feature = "snapshot")]
pub(crate) use stream::StreamState;
pub(crate) use stream::StreamTable;
