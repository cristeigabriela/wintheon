//! A [shortcut](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/16cb4ca1-9339-4d0c-a68d-bf1d6cc0f943) (`.lnk`) entry.

use std::path::{Path, PathBuf};

use crate::file::{FileEntry, FileIcon, FileVersionInfo, Priority, Result};
use crate::win::resolve_shortcut;

pub struct Shortcut {
    link_path: PathBuf,
    target_path: PathBuf,
}

impl Shortcut {
    /// Resolve `link_path` (a `.lnk` file) to its target via `IShellLinkW`,
    /// returning `None` if the file isn't a valid shortcut or its target
    /// can't be read.
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
        // Do not use `target_path`, as, for example, applications
        // like Discord have a `Discord.lnk`, which has an icon, but
        // they point to files like `Update.exe`, which does not have an
        // icon in turn.
        Ok(FileIcon::new(self.link_path.clone()))
    }

    fn version_info(&self) -> Result<FileVersionInfo> {
        // Version info lives in the target executable, not in the `.lnk`.
        Ok(FileVersionInfo::load(&self.target_path)?)
    }

    fn priority(&self) -> Priority {
        // User-curated entries weigh slightly higher than raw files; tune
        // as the ranking model evolves.
        Priority(1.5)
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
