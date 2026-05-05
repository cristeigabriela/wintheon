//! A discoverable origin of file entries.

use crate::file::{FileEntry, Result};

use super::Origin;

/// A lazy stream of scanned file entries. Each item is a [`Result`] so a
/// single bad entry doesn't abort the whole scan.
pub type FileEntries<'a> = Box<dyn Iterator<Item = Result<Box<dyn FileEntry>>> + Send + 'a>;

pub trait Source: Send + Sync {
    /// The [origin](Origin) this source represents.
    fn origin(&self) -> Origin;

    /// Lazily enumerate file entries from this source.
    fn scan(&self) -> FileEntries<'_>;
}
