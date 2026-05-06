//! Win32 implementations behind the [`crate::file`] surface — icon
//! extraction (Shell + GDI), shortcut resolution (`IShellLinkW`),
//! `AppExec` reparse-point following (`FSCTL_GET_REPARSE_POINT`), and
//! `VS_VERSIONINFO` parsing.

mod icon;
mod reparse;
mod shortcut;
mod version_info;

pub use icon::{FileIcon, ICON_SIZE, IconSize};
pub use reparse::{AppExecLink, resolve_appexec_link};
pub use shortcut::resolve_shortcut;
pub use version_info::{FileInformation, FileVersionInfo, FixedFileInfo, Translation};
