//! The general structure of a file entry.

use std::path::Path;

use super::{FileIcon, FileVersionInfo, Result};

/// Heuristic ordering weight for an entry. Higher = more likely to surface.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Priority(pub f32);

impl Default for Priority {
    /// Neutral weight (`1.0`).
    fn default() -> Self {
        Self(1.0)
    }
}

pub trait FileEntry: Send + Sync {
    /// The full path to the underlying file (resolved through any link).
    fn path(&self) -> &Path;

    /// The path the entry was reached *through* — the `.lnk` for a
    /// [shortcut](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/16cb4ca1-9339-4d0c-a68d-bf1d6cc0f943)
    /// or the [reparse point](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/reparse-points)
    /// itself. `None` when [`path`](Self::path) is the original location.
    fn link_path(&self) -> Option<&Path>;

    /// The icon of the file (not of the shortcut or reparse point).
    fn icon(&self) -> Result<FileIcon>;

    /// The version information of the file. The returned
    /// [`FileVersionInfo`] carries every translation the PE declares;
    /// pick one with [`FileVersionInfo::for_translation`],
    /// [`FileVersionInfo::english`], or [`FileVersionInfo::system`].
    fn version_info(&self) -> Result<FileVersionInfo>;

    /// Heuristic [priority](Priority) used for ranking.
    fn priority(&self) -> Priority;

    /// A user-friendly display label for the entry.
    ///
    /// Built-in semantics:
    /// - [`Shortcut`](crate::file::Shortcut): the `.lnk` filename stem
    ///   (the user-chosen label).
    /// - [`ReparsePoint`](crate::file::ReparsePoint): the target's
    ///   `file_description` from English version info when present,
    ///   otherwise the stub's filename stem.
    /// - [`RegularFile`](crate::file::RegularFile): for `.exe` files,
    ///   `file_description` if present otherwise the file stem; for any
    ///   other extension, the full filename including extension.
    ///
    /// Default: the path's full file name. May do I/O when an impl reads
    /// version info — callers that render frequently should cache the
    /// result.
    fn display_name(&self) -> String {
        self.path()
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}
