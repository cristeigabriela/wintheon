//! A discoverable origin of file entries.

use std::fmt::Debug;

use crate::file::{FileEntry, Result};

use super::Origin;

/// A lazy stream of scanned file entries. Each item is a [`Result`] so a
/// single bad entry doesn't abort the whole scan.
pub type FileEntries<'a> = Box<dyn Iterator<Item = Result<Box<dyn FileEntry>>> + Send + 'a>;

/// A discoverable origin of [`FileEntry`]s.
///
/// Implementors enumerate one logical location — a folder, a registry
/// tree, a custom database — as a lazy stream. `Err` items inside the
/// stream surface a single bad entry without aborting the rest of the
/// scan.
///
/// Built-in implementations:
/// [`DesktopSource`](super::DesktopSource),
/// [`StartMenuSource`](super::StartMenuSource), and
/// [`WindowsAppsSource`](super::WindowsAppsSource). Custom sources plug
/// into a [`Gatherer`](super::Gatherer) via
/// [`Gatherer::with_source`](super::Gatherer::with_source) and report
/// themselves through [`Origin::Custom`].
pub trait Source: Debug + Send + Sync {
    /// The [`Origin`] this source represents — used to tag every yielded
    /// [`WeightedEntry`](super::WeightedEntry).
    fn origin(&self) -> Origin;

    /// Lazily enumerate file entries from this source. The returned
    /// iterator borrows `self`, so the source must outlive the iterator.
    fn scan(&self) -> FileEntries<'_>;
}
