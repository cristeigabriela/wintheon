//! Iterator extension that scores [`WeightedEntry`] streams against a
//! query, attaching combined `priority × relevance` scores and filtering
//! non-matches.
//!
//! Generic over `Item: AsRef<WeightedEntry>`, so the same trait covers:
//!
//! - owned iterators yielding [`WeightedEntry`] (via the reflexive
//!   `AsRef<WeightedEntry>` impl on the type itself);
//! - borrowed iterators yielding `&WeightedEntry` (via std's blanket
//!   `impl<T: AsRef<U>> AsRef<U> for &T`);
//! - any user wrapper that implements `AsRef<WeightedEntry>` — e.g. a
//!   launcher's `LauncherEntry { weighted: WeightedEntry, … }` simply
//!   adds `impl AsRef<WeightedEntry> for LauncherEntry { … }` and
//!   gets `.score(q)` / `.sorted_by_score(q)` for free.
//!
//! Two methods on the trait:
//!
//! - [`score`](WeightedEntryIteratorExt::score) — lazy, returns
//!   `Iterator<Item = (T, f32)>` filtered to matches. Compose with
//!   `take`, `filter`, etc. before collecting.
//! - [`sorted_by_score`](WeightedEntryIteratorExt::sorted_by_score) —
//!   eager, returns `Vec<T>` sorted highest-score-first (scores dropped
//!   after sorting). The one-liner for "give me the ranked list."
//!
//! ```no_run
//! use wintheon::file::Priority;
//! use wintheon::gather::{Gatherer, WeightedEntryIteratorExt};
//!
//! let gatherer = Gatherer::new().with_start_menu(Priority(1.5));
//! let top = gatherer
//!     .scan()
//!     .filter_map(|r| r.ok())
//!     .sorted_by_score("chrome");
//! for w in top.iter().take(10) {
//!     println!("{}", w.entry.display_name());
//! }
//! ```

use std::borrow::Cow;

use crate::gather::WeightedEntry;

/// Adds `.score(query)` and `.sorted_by_score(query)` to any iterator
/// whose items can be viewed as a [`WeightedEntry`]. Bring the trait
/// into scope (`use wintheon::gather::WeightedEntryIteratorExt;`) to
/// use the methods.
pub trait WeightedEntryIteratorExt: Iterator + Sized
where
    Self::Item: AsRef<WeightedEntry>,
{
    /// Filter out non-matches and pair each survivor with its combined
    /// `priority × relevance` score. Lazy — wrap in your own sort,
    /// `take`, etc.
    ///
    /// `query` is lowercased once at the start (zero-allocation when
    /// it's already lowercase, single allocation when it isn't), then
    /// passed straight through to each entry's score loop without any
    /// per-entry work. Empty `query` is a neutral 1.0 match, so the
    /// unfiltered list rolls back to pure priority order.
    fn score(self, query: &str) -> impl Iterator<Item = (Self::Item, f32)> {
        let q: Cow<'_, str> = if query.chars().any(char::is_uppercase) {
            Cow::Owned(query.to_lowercase())
        } else {
            Cow::Borrowed(query)
        };
        self.filter_map(move |item| {
            let s = item.as_ref().score(&q)?;
            Some((item, s))
        })
    }

    /// Eager version of [`score`](Self::score) — collects, sorts
    /// descending by score, and returns just the items (the scores are
    /// dropped after sorting).
    ///
    /// Use [`score`](Self::score) directly if you need the scores
    /// alongside the items (for tier-based filtering, diagnostic
    /// printing, etc.).
    fn sorted_by_score(self, query: &str) -> Vec<Self::Item> {
        let mut v: Vec<_> = self.score(query).collect();
        v.sort_by(|(_, a), (_, b)| b.total_cmp(a));
        v.into_iter().map(|(item, _)| item).collect()
    }
}

impl<I> WeightedEntryIteratorExt for I
where
    I: Iterator + Sized,
    I::Item: AsRef<WeightedEntry>,
{
}
