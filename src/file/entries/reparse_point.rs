//! A [reparse point](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/reparse-points) entry.

use std::path::{Path, PathBuf};

use crate::file::{FileEntry, FileIcon, FileVersionInfo, Priority, Result};
use crate::win::resolve_appexec_link;

pub struct ReparsePoint {
    link_path: PathBuf,
    target_path: PathBuf,
}

impl ReparsePoint {
    /// Resolve `link_path` for an App Execution Alias, returning `None`
    /// if the file isn't a resolvable reparse or any of the underlying I/O fails.
    pub fn new(link_path: PathBuf) -> Option<Self> {
        let target_path = resolve_appexec_link(&link_path)?;
        Some(Self {
            link_path,
            target_path,
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
        // Use the resolved target, instead of the app execution alias,
        // as that has no icon.
        Ok(FileIcon::new(self.target_path.clone()))
    }

    fn version_info(&self) -> Result<FileVersionInfo> {
        // Version info lives in the resolved target, not in the reparse stub.
        Ok(FileVersionInfo::load(&self.target_path)?)
    }

    fn priority(&self) -> Priority {
        // Baseline weight; tune as the ranking model evolves.
        Priority(1.0)
    }

    fn display_name(&self) -> String {
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
