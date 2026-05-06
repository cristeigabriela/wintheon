//! Win32 / COM building blocks the higher-level [`crate::file`] surface
//! sits on top of.
//!
//! Most consumers should use the [`crate::file`] re-exports — these
//! items are exposed primarily so callers can drive the underlying
//! Win32 routines directly when the higher-level abstractions don't fit.
//! The two free functions here, [`resolve_shortcut`] and
//! [`resolve_appexec_link`], are the same routines the
//! [`Shortcut`](crate::file::Shortcut) and
//! [`ReparsePoint`](crate::file::ReparsePoint) constructors call into.
//!
//! [`com`] carries the per-thread STA initialization helper required
//! before any shell-COM call.

pub mod com;
mod files;

pub use files::{
    AppExecLink, FileIcon, FileInformation, FileVersionInfo, FixedFileInfo, ICON_SIZE, IconSize,
    Translation, resolve_appexec_link, resolve_shortcut,
};
