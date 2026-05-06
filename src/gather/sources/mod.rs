//! Built-in [`Source`](super::Source) implementations covering the
//! locations a typical Windows launcher cares about — Desktop, Start
//! Menu, and the Microsoft Store `WindowsApps` stub directory.

mod desktop;
mod start_menu;
mod windows_apps;

pub use desktop::DesktopSource;
pub use start_menu::StartMenuSource;
pub use windows_apps::WindowsAppsSource;

use std::fs;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::PathBuf;

use tracing::trace;
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

use crate::file::{FileEntry, RegularFile, ReparsePoint, Result, Shortcut};
use crate::gather::FileEntries;

/// Walk `folder`'s immediate children and yield each non-directory entry
/// through [`classify`]. Subfolders are skipped (not descended into and
/// not yielded). `read_dir` failures surface as `Err` items.
pub(super) fn read_folder(folder: PathBuf) -> FileEntries<'static> {
    Box::new(FolderWalker::new(folder, false).map(classify))
}

/// Like [`read_folder`] but recurses into subdirectories (depth-first).
/// Symlinked/junctioned folders are skipped to avoid traversal loops.
pub(super) fn read_folder_recursive(folder: PathBuf) -> FileEntries<'static> {
    Box::new(FolderWalker::new(folder, true).map(classify))
}

/// Map a directory entry to the appropriate [`FileEntry`] impl.
///
/// Tries `.lnk` shortcut resolution first, then `AppExec` reparse-point
/// resolution; falls back to a plain [`RegularFile`].
pub(super) fn classify(entry: io::Result<fs::DirEntry>) -> Result<Box<dyn FileEntry>> {
    let entry = entry?;
    let path = entry.path();

    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        && let Some(shortcut) = Shortcut::new(path.clone())
    {
        trace!(path = %path.display(), kind = "Shortcut", "classified entry");
        return Ok(Box::new(shortcut));
    }

    // Only probe for a reparse target if the OS actually marked the file
    // as one — otherwise `ReparsePoint::new` would do a `CreateFileW` +
    // `DeviceIoControl` round-trip per file just to fail.
    let is_reparse = entry
        .metadata()
        .ok()
        .is_some_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0);
    if is_reparse && let Some(reparse) = ReparsePoint::new(path.clone()) {
        trace!(path = %path.display(), kind = "ReparsePoint", "classified entry");
        return Ok(Box::new(reparse));
    }

    trace!(path = %path.display(), kind = "RegularFile", "classified entry");
    Ok(Box::new(RegularFile::new(path)))
}

/// Directory walker — yields only files, optionally descends into
/// subdirectories, and always skips symlinks/junctions.
struct FolderWalker {
    stack: Vec<fs::ReadDir>,
    pending: Option<io::Error>,
    recursive: bool,
}

impl FolderWalker {
    fn new(root: PathBuf, recursive: bool) -> Self {
        match fs::read_dir(&root) {
            Ok(rd) => Self {
                stack: vec![rd],
                pending: None,
                recursive,
            },
            Err(err) => Self {
                stack: Vec::new(),
                pending: Some(err),
                recursive,
            },
        }
    }
}

impl Iterator for FolderWalker {
    type Item = io::Result<fs::DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(err) = self.pending.take() {
            return Some(Err(err));
        }
        loop {
            let read_dir = self.stack.last_mut()?;
            match read_dir.next() {
                None => {
                    self.stack.pop();
                }
                Some(Err(err)) => return Some(Err(err)),
                Some(Ok(entry)) => {
                    let file_type = match entry.file_type() {
                        Ok(t) => t,
                        Err(err) => return Some(Err(err)),
                    };
                    // Skip symlinked/junctioned dirs to avoid traversal loops.
                    // AppExec reparses report `is_symlink() == false`
                    // because their tag is `IO_REPARSE_TAG_APPEXECLINK`,
                    // not `IO_REPARSE_TAG_SYMLINK`, so they fall through
                    // to the file branch as expected.
                    if file_type.is_symlink() {
                        trace!(path = %entry.path().display(), "skipping symlink/junction");
                        continue;
                    }
                    if file_type.is_dir() {
                        if self.recursive {
                            trace!(path = %entry.path().display(), "descending into subdirectory");
                            match fs::read_dir(entry.path()) {
                                Ok(rd) => self.stack.push(rd),
                                Err(err) => return Some(Err(err)),
                            }
                        }
                        // Non-recursive: skip the directory entirely.
                        continue;
                    }
                    return Some(Ok(entry));
                }
            }
        }
    }
}
