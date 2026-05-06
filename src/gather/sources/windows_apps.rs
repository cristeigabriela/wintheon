//! Source that scans `%LOCALAPPDATA%\Microsoft\WindowsApps` (the `AppExec`
//! stub directory used by the Microsoft Store and command-line shims).

use std::path::PathBuf;

use crate::gather::{FileEntries, Origin, Source};

use super::read_folder;

/// [`Source`] over `%LOCALAPPDATA%\Microsoft\WindowsApps` — the `AppExec`
/// stub directory that the Microsoft Store and command-line shims
/// populate. Almost every entry here is a reparse point that
/// [`ReparsePoint`](crate::file::ReparsePoint) follows to the real
/// executable.
#[derive(Debug)]
pub struct WindowsAppsSource;

impl WindowsAppsSource {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// `%LOCALAPPDATA%\Microsoft\WindowsApps` — every entry here is
    /// expected to be an `AppExec` reparse stub.
    fn folders(&self) -> Vec<PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Microsoft").join("WindowsApps"))
            .into_iter()
            .collect()
    }
}

impl Default for WindowsAppsSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for WindowsAppsSource {
    fn origin(&self) -> Origin {
        Origin::WindowsApps
    }

    fn scan(&self) -> FileEntries<'_> {
        let stream = self.folders().into_iter().flat_map(read_folder);
        Box::new(stream)
    }
}
