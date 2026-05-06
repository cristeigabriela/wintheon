//! Errors surfaced by [`FileEntry`](super::FileEntry) operations.

use core::result;
use std::io;

use thiserror::Error;

/// The library's error type.
///
/// Surfaces failures from the underlying Win32/COM/IO routines. Marked
/// `#[non_exhaustive]` so adding new variants for future fallible
/// operations isn't a breaking change — downstream `match`es must
/// include a `_` arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// An underlying I/O failure. Typically wraps a Win32 error pulled
    /// via [`io::Error::last_os_error`] from the version-info / shell
    /// resolution paths.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Convenience [`Result`](core::result::Result) alias parameterized by
/// the library's [`Error`](enum@Error).
pub type Result<T> = result::Result<T, Error>;
