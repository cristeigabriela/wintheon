//! A [reparse point](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/reparse-points) entry.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::file::{FileEntry, FileIcon, FileVersionInfo, Priority, Result};
use crate::win::resolve_appexec_link;

/// An App Execution Alias reparse point and its resolved target.
///
/// Microsoft Store apps install command-line shims under
/// `%LOCALAPPDATA%\Microsoft\WindowsApps` as reparse points with the
/// `IO_REPARSE_TAG_APPEXECLINK` tag. [`new`](Self::new) follows the
/// reparse with `FSCTL_GET_REPARSE_POINT` and parses the body to recover
/// the real executable path; returns `None` for files that aren't
/// `AppExec` aliases or whose ioctl fails. tyranid's [Overview of
/// Windows Execution Aliases](https://www.tiraniddo.dev/2019/09/overview-of-windows-execution-aliases.html)
/// addresses the details of this mechanism in detail.
///
/// [`path`](FileEntry::path) returns the resolved target,
/// [`link_path`](FileEntry::link_path) the stub. [`icon`](FileEntry::icon)
/// uses the resolved target since the stub itself has no extractable
/// icon. [`display_name`](FileEntry::display_name) prefers the target's
/// version-info product name (skipping the generic "Microsoft Windows
/// Operating System"), falling back to the file stem; the result is
/// cached after the first call.
#[derive(Debug)]
pub struct ReparsePoint {
    link_path: PathBuf,
    target_path: PathBuf,
    display_name: OnceLock<String>,
}

impl ReparsePoint {
    /// Resolve `link_path` for an App Execution Alias, returning `None`
    /// if the file isn't a resolvable reparse or any of the underlying I/O fails.
    #[must_use]
    pub fn new(link_path: PathBuf) -> Option<Self> {
        let target_path = resolve_appexec_link(&link_path)?;
        Some(Self {
            link_path,
            target_path,
            display_name: OnceLock::new(),
        })
    }
}

impl FileEntry for ReparsePoint {
    fn path(&self) -> &Path {
        &self.target_path
    }

    fn link_path(&self) -> Option<&Path> {
        Some(&self.link_path)
    }

    fn icon(&self) -> Result<FileIcon> {
        // Use the resolved target directly — the AppExec stub itself
        // returns a generic icon. `from_paths` skips the auto-resolve
        // step (the target is a real exe, not a reparse / .lnk).
        Ok(FileIcon::from_paths(self.target_path.clone(), None))
    }

    fn version_info(&self) -> Result<FileVersionInfo> {
        // Version info lives in the resolved target, not in the reparse stub.
        Ok(FileVersionInfo::load(&self.target_path)?)
    }

    fn priority(&self) -> Priority {
        Priority(1.0)
    }

    fn display_name(&self) -> String {
        self.display_name
            .get_or_init(|| self.compute_display_name())
            .clone()
    }
}

impl ReparsePoint {
    fn compute_display_name(&self) -> String {
        self.version_info()
            .ok()
            .and_then(|info| {
                info.english()
                    .and_then(|fi| fi.meaningful_product_name().map(String::from))
            })
            .or_else(|| {
                self.target_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_default()
    }
}
