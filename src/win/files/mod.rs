mod icon;
mod reparse;
mod shortcut;
mod version_info;

pub use icon::{FileIcon, ICON_SIZE};
pub use reparse::{AppExecLink, resolve_appexec_link};
pub use shortcut::resolve_shortcut;
pub use version_info::{FileInformation, FileVersionInfo, FixedFileInfo, Translation};
