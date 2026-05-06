//! Source that scans the user and public Desktop folders.

use std::path::PathBuf;

use crate::gather::{FileEntries, Origin, Source};

use super::read_folder;

/// [`Source`] over the per-user (`%USERPROFILE%\Desktop`) and shared
/// (`%PUBLIC%\Desktop`) Desktop folders. Non-recursive — Desktop is a
/// flat folder by convention.
#[derive(Debug)]
pub struct DesktopSource;

impl DesktopSource {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Per-user (`%USERPROFILE%\Desktop`) and shared (`%PUBLIC%\Desktop`)
    /// Desktop folders.
    fn folders(&self) -> Vec<PathBuf> {
        let user = std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Desktop"));
        let public = std::env::var_os("PUBLIC").map(|p| PathBuf::from(p).join("Desktop"));
        [user, public].into_iter().flatten().collect()
    }
}

impl Default for DesktopSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for DesktopSource {
    fn origin(&self) -> Origin {
        Origin::Desktop
    }

    fn scan(&self) -> FileEntries<'_> {
        let stream = self.folders().into_iter().flat_map(read_folder);
        Box::new(stream)
    }
}
