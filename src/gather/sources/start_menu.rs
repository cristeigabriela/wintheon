//! Source that scans the per-user and system-wide Start Menu Programs folders.

use std::path::PathBuf;

use crate::gather::{FileEntries, Origin, Source};

use super::read_folder_recursive;

pub struct StartMenuSource;

impl StartMenuSource {
    pub fn new() -> Self {
        Self
    }

    /// Per-user (`%APPDATA%\Microsoft\Windows\Start Menu\Programs`) and
    /// system-wide (`%ProgramData%\Microsoft\Windows\Start Menu\Programs`)
    /// Start Menu Programs roots.
    ///
    /// Note: this is a non-recursive scan of the immediate folder; Start
    /// Menu is conventionally nested (e.g. `…\Windows PowerShell\…`), so a
    /// recursive walker can be layered on top later.
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
