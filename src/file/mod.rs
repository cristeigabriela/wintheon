//! Filesystem entry types and their metadata.
//!
//! [`FileEntry`] is the central trait every discovered item implements,
//! with three built-in shapes:
//!
//! - [`RegularFile`]: a plain file (`.exe`, `.txt`, anything in `%PATHEXT%`).
//! - [`Shortcut`]: a `.lnk` resolved through `IShellLinkW` to its target.
//! - [`ReparsePoint`]: an App Execution Alias (Microsoft Store stub)
//!   followed to the underlying executable.
//!
//! Per-entry helpers expose the shell icon ([`FileIcon`], sized via
//! [`IconSize`]) and the full `VS_VERSIONINFO` resource
//! ([`FileVersionInfo`], mirroring `System.Diagnostics.FileVersionInfo`).

mod entries;
mod entry;
mod error;
mod icon;
mod version_info;

pub use entries::{RegularFile, ReparsePoint, Shortcut};
pub use entry::{FileEntry, Priority};
pub use error::{Error, Result};
pub use icon::{FileIcon, ICON_SIZE, IconSize};
pub use version_info::{FileInformation, FileVersionInfo, FixedFileInfo, Translation};
