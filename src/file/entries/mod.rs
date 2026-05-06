//! Concrete [`FileEntry`](super::FileEntry) implementations.
//!
//! Each type wraps a path plus the metadata needed to serve the trait.
//! Sources in [`crate::gather`] construct them as they walk the
//! filesystem, but they're usable standalone:
//!
//! ```no_run
//! use std::path::PathBuf;
//! use wintheon::file::{FileEntry, RegularFile};
//!
//! let entry = RegularFile::new(PathBuf::from(r"C:\Windows\System32\notepad.exe"));
//! println!("{}", entry.display_name());
//! ```

mod regular_file;
mod reparse_point;
mod shortcut;

pub use regular_file::RegularFile;
pub use reparse_point::ReparsePoint;
pub use shortcut::Shortcut;
