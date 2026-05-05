//! A regular file entry (e.g. `.exe`, `.bat`, `.txt`).

use std::path::{Path, PathBuf};

use crate::file::{FileEntry, FileIcon, FileVersionInfo, Priority, Result};

pub struct RegularFile {
    path: PathBuf,
}

impl RegularFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl FileEntry for RegularFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn link_path(&self) -> Option<&Path> {
        None
    }

    fn icon(&self) -> Result<FileIcon> {
        Ok(FileIcon::new(self.path.clone()))
    }

    fn version_info(&self) -> Result<FileVersionInfo> {
        Ok(FileVersionInfo::load(&self.path)?)
    }

    fn priority(&self) -> Priority {
        // Baseline weight; tune as the ranking model evolves.
        Priority(1.0)
    }

    fn display_name(&self) -> String {
        let is_exe = self
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));

        if is_exe {
            self.version_info()
                .ok()
                .and_then(|info| {
                    info.english()
                        .and_then(|fi| fi.meaningful_product_name().map(String::from))
                })
                .or_else(|| {
                    self.path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_default()
        } else {
            // Non-exe: keep the extension so `readme.txt` reads as
            // "readme.txt" rather than "readme".
            self.path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        }
    }
}
