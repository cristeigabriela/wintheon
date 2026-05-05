//! Where a [`FileEntry`](crate::file::FileEntry) was discovered.

use core::fmt;
use std::borrow::Cow;

/// Where a discovered entry came from.
///
/// Built-in variants cover the sources `wintheon` ships out of the box.
/// User-defined [`Source`](super::Source) implementations should return
/// [`Origin::Custom`] with a human-readable label.
///
/// Marked `#[non_exhaustive]` so adding more built-in variants in a
/// future release isn't a breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Origin {
    Desktop,
    StartMenu,
    WindowsApps,
    /// User-defined source label. Use `Cow::Borrowed("…")` for static
    /// names (the common case) or `Cow::Owned(…)` for runtime ones.
    Custom(Cow<'static, str>),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Desktop => f.write_str("Desktop"),
            Origin::StartMenu => f.write_str("Start Menu"),
            Origin::WindowsApps => f.write_str("Apps"),
            Origin::Custom(label) => f.write_str(label),
        }
    }
}
