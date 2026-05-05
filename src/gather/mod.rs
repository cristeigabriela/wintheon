mod dedup;
mod gatherer;
mod origin;
mod source;
mod sources;

pub use dedup::DedupByRealpath;
pub use gatherer::{Gatherer, WeightedEntry};
pub use origin::Origin;
pub use source::{FileEntries, Source};
pub use sources::{DesktopSource, StartMenuSource, WindowsAppsSource};
