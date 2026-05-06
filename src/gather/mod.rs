//! Discover [`FileEntry`](crate::file::FileEntry)s from configured
//! [`Source`]s, deduplicate, and rank.
//!
//! The pipeline is:
//!
//! 1. Configure a [`Gatherer`] with the [`Source`]s you want — built-in
//!    [`DesktopSource`] / [`StartMenuSource`] / [`WindowsAppsSource`], or
//!    your own via [`Gatherer::with_source`].
//! 2. Call [`Gatherer::scan`] to stream [`WeightedEntry`]s lazily.
//!    Duplicates are dropped on the fly using the same heuristic
//!    [`DedupByRealpath`] exposes as a standalone combinator.
//! 3. Rank with one of:
//!    - [`WeightedEntry::priority_score`] — source × entry priority alone.
//!    - [`WeightedEntry::score`] — priority × query relevance, using
//!      a per-entry [`MatchIndex`] cache.
//!    - [`WeightedEntryIteratorExt`] adds `.score(query)` and
//!      `.sorted_by_score(query)` to any iterator whose items can be
//!      viewed as a [`WeightedEntry`] — owned, borrowed, or user wrappers
//!      via an `AsRef<WeightedEntry>` impl.
//!
//! # Static ranking
//!
//! ```rust,no_run
//! use wintheon::file::Priority;
//! use wintheon::gather::Gatherer;
//!
//! let gatherer = Gatherer::new()
//!     .with_desktop(Priority(1.0))
//!     .with_start_menu(Priority(1.5))
//!     .with_windows_apps(Priority(2.0));
//!
//! let mut entries: Vec<_> = gatherer.scan().filter_map(|r| r.ok()).collect();
//! entries.sort_by(|a, b| b.priority_score().total_cmp(&a.priority_score()));
//! for w in entries.iter().take(20) {
//!     println!("{} ({})", w.entry.display_name(), w.origin);
//! }
//! ```
//!
//! # Query-based ranking
//!
//! [`WeightedEntryIteratorExt::sorted_by_score`] gathers every surviving
//! entry and returns a `Vec<item>` sorted highest-score-first in a
//! single call:
//!
//! ```rust,no_run
//! use wintheon::file::Priority;
//! use wintheon::gather::{Gatherer, WeightedEntryIteratorExt};
//!
//! let gatherer = Gatherer::new()
//!     .with_desktop(Priority(1.0))
//!     .with_start_menu(Priority(1.5))
//!     .with_windows_apps(Priority(2.0));
//!
//! let ranked = gatherer
//!     .scan()
//!     .filter_map(|r| r.ok())
//!     .sorted_by_score("chrome");
//!
//! for w in ranked.iter().take(20) {
//!     println!("{}", w.entry.display_name());
//! }
//! ```
//!
//! ## Common patterns on `ranked`
//!
//! Once you have a sorted `Vec`, the usual slice/iterator methods cover
//! most needs:
//!
//! ```rust,no_run
//! # use wintheon::file::Priority;
//! # use wintheon::gather::{Gatherer, WeightedEntryIteratorExt};
//! # let gatherer = Gatherer::new().with_desktop(Priority(1.0));
//! let ranked = gatherer.scan().filter_map(|r| r.ok()).sorted_by_score("chrome");
//!
//! // Best match (or `None` if nothing matched).
//! if let Some(best) = ranked.first() {
//!     println!("best: {}", best.entry.display_name());
//! }
//!
//! // Top N.
//! let top10 = &ranked[..ranked.len().min(10)];
//!
//! // Empty-state UX hook.
//! if ranked.is_empty() {
//!     println!("no matches");
//! }
//! ```
//!
//! Use [`WeightedEntryIteratorExt::score`] directly when you need the
//! scores alongside the items (e.g. to filter by score tier, or to
//! print debug rankings) — it's lazy and pairs with `take`, `filter`,
//! etc. before collecting.
//!
//! ## Custom wrappers via `AsRef<WeightedEntry>`
//!
//! The trait works on any iterator whose items can be viewed as a
//! [`WeightedEntry`] — implement [`AsRef<WeightedEntry>`] for your own
//! wrapper type and you get `.score()` / `.sorted_by_score()` on
//! iterators of it for free, without unwrapping or rebuilding.
//!
//! ```rust,no_run
//! # use std::sync::OnceLock;
//! use wintheon::gather::{WeightedEntry, WeightedEntryIteratorExt};
//!
//! /// A launcher row with its scan result plus per-row UI caches.
//! struct LauncherEntry {
//!     weighted: WeightedEntry,
//!     icon: OnceLock<Vec<u8>>,
//! }
//!
//! impl AsRef<WeightedEntry> for LauncherEntry {
//!     fn as_ref(&self) -> &WeightedEntry {
//!         &self.weighted
//!     }
//! }
//!
//! fn render(entries: &[LauncherEntry], query: &str) {
//!     for entry in entries.iter().sorted_by_score(query) {
//!         println!("{}", entry.weighted.entry.display_name());
//!         let _ = &entry.icon; // …reuse cached UI state…
//!     }
//! }
//! ```
//!
//! For a long-running UI re-ranking on every keystroke, this is the
//! pattern: keep [`WeightedEntry`]s (or wrappers around them) in a
//! stable `Vec`, call `entries.iter().sorted_by_score(query)` per frame.
//! The entries' built-in [`MatchIndex`] caches make subsequent queries
//! cheap; only the rendering ever touches the heavy I/O.

mod dedup;
mod gatherer;
mod origin;
mod relevance;
mod sort;
mod source;
mod sources;

pub use dedup::DedupByRealpath;
pub use gatherer::{Gatherer, WeightedEntry};
pub use origin::Origin;
pub use relevance::MatchIndex;
pub use sort::WeightedEntryIteratorExt;
pub use source::{FileEntries, Source};
pub use sources::{DesktopSource, StartMenuSource, WindowsAppsSource};
