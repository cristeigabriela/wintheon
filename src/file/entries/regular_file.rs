//! A regular file entry (e.g. `.exe`, `.bat`, `.txt`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::file::{FileEntry, FileIcon, FileVersionInfo, Priority, Result};

/// A plain filesystem entry — anything that isn't a `.lnk` shortcut or
/// a reparse point.
///
/// [`display_name`](FileEntry::display_name) is computed from the path
/// extension: `.exe` falls back to the version-info product name (filtered
/// to skip the generic "Microsoft Windows Operating System"), `.url`
/// drops its extension like a shortcut would, anything else surfaces as
/// the full filename. The result is cached after the first call so
/// repeated calls don't re-read the PE.
///
/// Files whose extension is in `%PATHEXT%` (`.exe`, `.cmd`, `.bat`, …)
/// receive a slight [`Priority`] boost so they rank above inert
/// neighbors like `.txt` in the same folder.
#[derive(Debug)]
pub struct RegularFile {
    path: PathBuf,
    display_name: OnceLock<String>,
}

impl RegularFile {
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self {
            path,
            display_name: OnceLock::new(),
        }
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
        // Boost files the shell would actually run (anything in `%PATHEXT%`)
        // above inert files like `.txt` / `.md` sitting in the same folder.
        if self.is_executable() {
            Priority(1.25)
        } else {
            Priority(1.0)
        }
    }

    fn display_name(&self) -> String {
        self.display_name
            .get_or_init(|| self.compute_display_name())
            .clone()
    }
}

impl RegularFile {
    fn compute_display_name(&self) -> String {
        let ext = self.extension();

        // `.exe` — prefer the version-info product name, fall back to the
        // file stem.
        if ext.is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
            return self
                .version_info()
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
                .unwrap_or_default();
        }

        // `.url` is shortcut-like (internet shortcut) — drop the extension
        // the same way we strip `.lnk` for `Shortcut`. Any other extension
        // stays so plain files like `readme.txt` read as `"readme.txt"`.
        let strip_extension = ext.is_some_and(|e| e.eq_ignore_ascii_case("url"));
        let part = if strip_extension {
            self.path.file_stem()
        } else {
            self.path.file_name()
        };
        part.map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}
