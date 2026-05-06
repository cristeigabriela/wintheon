//! Builder that combines multiple [`Source`]s and tags each yielded
//! [`FileEntry`] with a per-source priority weight.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Instant;

use tracing::debug;

use crate::file::{FileEntry, Priority, Result};
use crate::gather::dedup::dedup_decision;
use crate::gather::{
    DesktopSource, MatchIndex, Origin, Source, StartMenuSource, WindowsAppsSource,
};

/// One [`FileEntry`] tagged with its [`Origin`] and the priority weight
/// of the source it came from. Yielded by [`Gatherer::scan`].
#[derive(Debug)]
pub struct WeightedEntry {
    pub entry: Box<dyn FileEntry>,
    pub origin: Origin,
    pub source_priority: Priority,
    index: OnceLock<MatchIndex>,
}

impl WeightedEntry {
    /// Construct a new weighted entry. Most consumers don't call this
    /// directly — [`Gatherer::scan`] builds them as it walks each
    /// source — but it's exposed for tests and custom pipelines.
    #[must_use]
    pub fn new(entry: Box<dyn FileEntry>, origin: Origin, source_priority: Priority) -> Self {
        Self {
            entry,
            origin,
            source_priority,
            index: OnceLock::new(),
        }
    }

    /// Static ranking score: `source_priority × entry.priority()`.
    /// Higher = ranked first.
    pub fn priority_score(&self) -> f32 {
        self.source_priority.0 * self.entry.priority().0
    }

    /// Combined ranking score: [`priority_score`](Self::priority_score)
    /// multiplied by [`MatchIndex::score`](super::MatchIndex::score) for
    /// `query`. Returns `None` when the entry doesn't match.
    ///
    /// Mixed-case `query` is handled — the implementation lowercases
    /// internally only when needed (`Cow`-style: zero allocation when
    /// the input is already lowercase). Empty `query` is a neutral 1.0
    /// match, so this reduces to [`priority_score`](Self::priority_score)
    /// when no filter is active.
    ///
    /// The internal [`MatchIndex`](super::MatchIndex) is built lazily
    /// on the first call and cached for the lifetime of the entry, so
    /// repeated calls (e.g. one per keystroke in a UI) only pay the
    /// version-info read once.
    pub fn score(&self, query: &str) -> Option<f32> {
        let q: Cow<'_, str> = if query.chars().any(char::is_uppercase) {
            Cow::Owned(query.to_lowercase())
        } else {
            Cow::Borrowed(query)
        };
        Some(self.index().score(&q)? * self.priority_score())
    }

    /// Lazily build (or return) the cached [`MatchIndex`].
    fn index(&self) -> &MatchIndex {
        self.index
            .get_or_init(|| MatchIndex::from_entry(&*self.entry))
    }
}

/// Reflexive `AsRef` so `WeightedEntry` itself satisfies the
/// `Item: AsRef<WeightedEntry>` bound on
/// [`WeightedEntryIteratorExt`](super::WeightedEntryIteratorExt).
/// Combined with the std blanket `impl<T: AsRef<U>> AsRef<U> for &T`,
/// any iterator over `WeightedEntry`, `&WeightedEntry`, or any user
/// wrapper that impls `AsRef<WeightedEntry>` works with the extension
/// trait.
impl AsRef<Self> for WeightedEntry {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// Builder that aggregates [`Source`]s and exposes a single
/// **deduplicated-by-default** stream of [`WeightedEntry`]s.
///
/// Dedup uses the same `.lnk`-aware heuristic as
/// [`DedupByRealpath`](super::DedupByRealpath) and can be turned off via
/// [`with_dedup(false)`](Self::with_dedup) when callers want the raw
/// stream (e.g. for diagnostics).
///
/// ```rust,no_run
/// use wintheon::file::Priority;
/// use wintheon::gather::Gatherer;
///
/// let gatherer = Gatherer::new()
///     .with_start_menu(Priority(1.5))
///     .with_desktop(Priority(1.0))
///     .with_windows_apps(Priority(2.0));
///
/// for result in gatherer.scan() {
///     let weighted = result.unwrap();
///     println!(
///         "{:?} @ {}",
///         weighted.entry.path(),
///         weighted.source_priority.0,
///     );
/// }
/// ```
///
/// User-defined sources plug in via [`with_source`](Self::with_source).
#[derive(Debug)]
pub struct Gatherer {
    sources: Vec<(Box<dyn Source>, Priority)>,
    dedup: bool,
}

impl Default for Gatherer {
    fn default() -> Self {
        Self::new()
    }
}

impl Gatherer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            dedup: true,
        }
    }

    /// Add the user + public Desktop with the given source weight.
    #[must_use]
    pub fn with_desktop(self, priority: Priority) -> Self {
        self.with_source(DesktopSource::new(), priority)
    }

    /// Add the per-user + system Start Menu Programs with the given source weight.
    #[must_use]
    pub fn with_start_menu(self, priority: Priority) -> Self {
        self.with_source(StartMenuSource::new(), priority)
    }

    /// Add the `WindowsApps` `AppExec` stub directory with the given source weight.
    #[must_use]
    pub fn with_windows_apps(self, priority: Priority) -> Self {
        self.with_source(WindowsAppsSource::new(), priority)
    }

    /// Add an arbitrary [`Source`] with the given weight. The extension
    /// point for user-defined sources.
    #[must_use]
    pub fn with_source<S>(mut self, source: S, priority: Priority) -> Self
    where
        S: Source + 'static,
    {
        self.sources.push((Box::new(source), priority));
        self
    }

    /// Toggle realpath-based deduplication. **On by default.**
    #[must_use]
    pub const fn with_dedup(mut self, dedup: bool) -> Self {
        self.dedup = dedup;
        self
    }

    /// Stream every entry from the configured sources. By default,
    /// duplicates are dropped using the same two-axis heuristic as
    /// [`DedupByRealpath`](super::DedupByRealpath). `Err` items pass
    /// through.
    pub fn scan(&self) -> impl Iterator<Item = Result<WeightedEntry>> + '_ {
        debug!(
            sources = self.sources.len(),
            dedup = self.dedup,
            "Gatherer::scan starting",
        );
        let mut seen_targets: HashSet<String> = HashSet::new();
        let mut seen_aliased_lnks: HashSet<(String, String)> = HashSet::new();
        let dedup = self.dedup;
        self.sources
            .iter()
            .flat_map(|(src, prio)| {
                let prio = *prio;
                let origin = src.origin();
                debug!(origin = %origin, "scanning source");
                let traced = TracedSourceIter::new(src.scan(), origin.clone());
                // `Origin::Custom` carries a `Cow<str>` so the enum is no
                // longer `Copy`; clone per-yield instead of moving.
                traced.map(move |r| (r, prio, origin.clone()))
            })
            .filter_map(move |(result, prio, origin)| match result {
                Ok(entry) => {
                    let keep = !dedup
                        || dedup_decision(
                            &mut seen_targets,
                            &mut seen_aliased_lnks,
                            entry.path(),
                            entry.link_path(),
                        );
                    keep.then(|| {
                        Ok(WeightedEntry {
                            entry,
                            origin,
                            source_priority: prio,
                            index: OnceLock::new(),
                        })
                    })
                }
                Err(err) => Some(Err(err)),
            })
    }
}

/// Wraps a [`Source`]'s iterator so that on `Drop` (i.e. once iteration
/// finishes or the consumer abandons it) we emit a single completion
/// event with the entry count and elapsed wall time. Errors are counted
/// separately so a flood of failed reads is visible without grepping.
struct TracedSourceIter<I> {
    inner: I,
    origin: Origin,
    started: Instant,
    ok: u64,
    err: u64,
}

impl<I> TracedSourceIter<I> {
    fn new(inner: I, origin: Origin) -> Self {
        Self {
            inner,
            origin,
            started: Instant::now(),
            ok: 0,
            err: 0,
        }
    }
}

impl<I> Iterator for TracedSourceIter<I>
where
    I: Iterator<Item = Result<Box<dyn FileEntry>>>,
{
    type Item = Result<Box<dyn FileEntry>>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.inner.next()?;
        match &item {
            Ok(_) => self.ok += 1,
            Err(_) => self.err += 1,
        }
        Some(item)
    }
}

impl<I> Drop for TracedSourceIter<I> {
    fn drop(&mut self) {
        debug!(
            origin = %self.origin,
            ok = self.ok,
            err = self.err,
            elapsed_ms = self.started.elapsed().as_millis() as u64,
            "source scan finished",
        );
    }
}
