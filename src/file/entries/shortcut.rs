//! A [shortcut](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/16cb4ca1-9339-4d0c-a68d-bf1d6cc0f943) (`.lnk`) entry.

use std::path::{Path, PathBuf};

use crate::file::{FileEntry, FileIcon, FileVersionInfo, Priority, Result};
use crate::win::resolve_shortcut;

/// A Windows shortcut (`.lnk`) and its resolved target.
///
/// Construct via [`new`](Self::new), which loads the `.lnk` through
/// `IShellLinkW` + `IPersistFile` and reads the target path; returns
/// `None` if the file isn't a valid shortcut. [`path`](FileEntry::path)
/// returns the resolved target, [`link_path`](FileEntry::link_path)
/// returns the `.lnk` itself.
///
/// [`icon`](FileEntry::icon) prefers the shortcut's chosen icon (the
/// `.lnk` path is used) over the target's, since many `.lnk`s point at
/// generic launchers (`Update.exe`, etc.) without their own icon.
/// [`display_name`](FileEntry::display_name) returns the `.lnk` filename
/// stem — the user-chosen label.
#[derive(Debug)]
pub struct Shortcut {
    link_path: PathBuf,
    target_path: PathBuf,
}

impl Shortcut {
    /// Resolve `link_path` (a `.lnk` file) to its target via `IShellLinkW`,
    /// returning `None` if the file isn't a valid shortcut or its target
    /// can't be read.
    #[must_use]
    pub fn new(link_path: PathBuf) -> Option<Self> {
        let target_path = resolve_shortcut(&link_path)?;
        Some(Self {
            link_path,
            target_path,
        })
    }
}

impl FileEntry for Shortcut {
    fn path(&self) -> &Path {
        &self.target_path
    }

    fn link_path(&self) -> Option<&Path> {
        Some(&self.link_path)
    }

    fn icon(&self) -> Result<FileIcon> {
        // Pass both paths explicitly so `FileIcon::new` doesn't re-walk
        // the `.lnk` (the resolve already happened in `Shortcut::new`)
        // or re-compute the reparse point target.
        // Don't drop `target_path` either — apps like Discord have a
        // `Discord.lnk` with a real icon pointing at `Update.exe`,
        // which has no icon of its own; the resolved target is the
        // fallback when the `.lnk` itself fails to render.
        Ok(FileIcon::from_paths(
            self.link_path.clone(),
            Some(self.target_path.clone()),
        ))
    }

    fn version_info(&self) -> Result<FileVersionInfo> {
        // Version info lives in the target executable, not in the `.lnk`.
        Ok(FileVersionInfo::load(&self.target_path)?)
    }

    fn priority(&self) -> Priority {
        Priority(1.0)
    }

    fn display_name(&self) -> String {
        // The `.lnk` filename is the user-chosen label; the target's
        // metadata would describe the underlying executable, not the
        // shortcut's purpose.
        self.link_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}
