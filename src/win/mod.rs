pub mod com;
mod files;

pub use files::{
    AppExecLink, FileIcon, FileInformation, FileVersionInfo, FixedFileInfo, ICON_SIZE,
    Translation, resolve_appexec_link, resolve_shortcut,
};
