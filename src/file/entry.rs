//! The general structure of a file entry.

use std::collections::HashSet;
use std::fmt::Debug;
use std::path::Path;
use std::sync::OnceLock;

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

/// A discovered filesystem entry: a [`RegularFile`](super::RegularFile),
/// [`Shortcut`](super::Shortcut), [`ReparsePoint`](super::ReparsePoint),
/// or any custom implementation a downstream crate provides.
///
/// Implementors expose the entry's resolved target [`path`](Self::path),
/// optional `.lnk`/reparse [`link_path`](Self::link_path), shell
/// [`icon`](Self::icon), `VS_VERSIONINFO` resource via
/// [`version_info`](Self::version_info), and a heuristic ranking
/// [`priority`](Self::priority).
///
/// Several methods come with default implementations covering the common
/// cases ([`extension`](Self::extension), [`is_executable`](Self::is_executable),
/// [`display_name`](Self::display_name)); override them when an entry
/// kind has a smarter answer.
///
/// `FileEntry: Debug + Send + Sync`, so trait objects ship safely across
/// threads and dump cleanly via `dbg!`.
pub trait FileEntry: Debug + Send + Sync {
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

    /// The extension of the entry's display path (the [`link_path`](Self::link_path)
    /// when present, otherwise the [`path`](Self::path)), as a UTF-8
    /// `&str`. `None` for paths without an extension or whose extension
    /// isn't valid UTF-8.
    fn extension(&self) -> Option<&str> {
        self.link_path()
            .unwrap_or_else(|| self.path())
            .extension()
            .and_then(|s| s.to_str())
    }

    /// Whether the entry's extension is registered as a directly-runnable
    /// executable in the `%PATHEXT%` env var (`.exe`, `.bat`, `.cmd`,
    /// `.com`, `.msi`, etc.). Used by ranking heuristics to prefer
    /// "things you'd actually run" over inert files in the same folder.
    fn is_executable(&self) -> bool {
        self.extension()
            .is_some_and(|e| executable_extensions().contains(e.to_ascii_lowercase().as_str()))
    }

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

/// Lowercased set of extensions parsed from the `%PATHEXT%` env var,
/// stored without the leading `.`. Cached for the process lifetime.
fn executable_extensions() -> &'static HashSet<String> {
    static EXTS: OnceLock<HashSet<String>> = OnceLock::new();
    EXTS.get_or_init(|| {
        std::env::var("PATHEXT")
            .unwrap_or_default()
            .split(';')
            .filter_map(|raw| {
                let cleaned = raw.trim().trim_start_matches('.').to_ascii_lowercase();
                (!cleaned.is_empty()).then_some(cleaned)
            })
            .collect()
    })
}
