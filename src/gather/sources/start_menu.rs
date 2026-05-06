//! Source that scans the per-user and system-wide Start Menu Programs folders.

use std::path::PathBuf;

use crate::gather::{FileEntries, Origin, Source};

use super::read_folder_recursive;

/// [`Source`] over the per-user
/// (`%APPDATA%\Microsoft\Windows\Start Menu\Programs`) and system
/// (`%ProgramData%\Microsoft\Windows\Start Menu\Programs`) Start Menu
/// `Programs` directories.
#[derive(Debug)]
pub struct StartMenuSource;

impl StartMenuSource {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Per-user (`%APPDATA%\Microsoft\Windows\Start Menu\Programs`) and
    /// system-wide (`%ProgramData%\Microsoft\Windows\Start Menu\Programs`)
    /// Start Menu Programs roots.
    ///
    /// Walked recursively by [`scan`](Self::scan) since Start Menu is conventionally
    /// nested (e.g. `…\Windows PowerShell\…`).
    fn folders(&self) -> Vec<PathBuf> {
        let user = std::env::var_os("APPDATA").map(|p| {
            PathBuf::from(p)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
        });
        let system = std::env::var_os("ProgramData").map(|p| {
            PathBuf::from(p)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
        });
        [user, system].into_iter().flatten().collect()
    }
}

impl Default for StartMenuSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for StartMenuSource {
    fn origin(&self) -> Origin {
        Origin::StartMenu
    }

    fn scan(&self) -> FileEntries<'_> {
        let stream = self.folders().into_iter().flat_map(read_folder_recursive);
        Box::new(stream)
    }
}
