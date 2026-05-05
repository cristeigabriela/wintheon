//! Builder that combines multiple [`Source`]s and tags each yielded
//! [`FileEntry`] with a per-source priority weight.

use std::collections::HashSet;

use crate::file::{FileEntry, Priority, Result};
use crate::gather::dedup::dedup_decision;
use crate::gather::{
    DesktopSource, Origin, Source, StartMenuSource, WindowsAppsSource,
};

/// One [`FileEntry`] tagged with its [`Origin`] and the priority weight
/// of the source it came from. Yielded by [`Gatherer::scan`].
pub struct WeightedEntry {
    pub entry: Box<dyn FileEntry>,
    pub origin: Origin,
    pub source_priority: Priority,
}

/// Builder that aggregates [`Source`]s and exposes a single
/// **deduplicated-by-default** stream of [`WeightedEntry`]s.
///
/// Dedup uses the same `.lnk`-aware heuristic as
/// [`DedupByRealpath`](super::DedupByRealpath) and can be turned off via
/// [`with_dedup(false)`](Self::with_dedup) when callers want the raw
/// stream (e.g. for diagnostics).
///
/// ```rs,ignore
/// use wintheon::file::Priority;
/// use wintheon::gather::Gatherer;
///
/// let gatherer = Gatherer::new()
///     .with_start_menu(Priority(1.5))
///     .with_desktop(Priority(1.0))
///     .with_windows_apps(Priority(1.0))
///     .with_source(my_custom_source, Priority(2.0));
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
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            dedup: true,
        }
    }

    /// Add the user + public Desktop with the given source weight.
    pub fn with_desktop(self, priority: Priority) -> Self {
        self.with_source(DesktopSource::new(), priority)
    }

    /// Add the per-user + system Start Menu Programs with the given source weight.
    pub fn with_start_menu(self, priority: Priority) -> Self {
        self.with_source(StartMenuSource::new(), priority)
    }

    /// Add the WindowsApps AppExec stub directory with the given source weight.
    pub fn with_windows_apps(self, priority: Priority) -> Self {
        self.with_source(WindowsAppsSource::new(), priority)
    }

    /// Add an arbitrary [`Source`] with the given weight. The extension
    /// point for user-defined sources.
    pub fn with_source<S>(mut self, source: S, priority: Priority) -> Self
    where
        S: Source + 'static,
    {
        self.sources.push((Box::new(source), priority));
        self
    }

    /// Toggle realpath-based deduplication. **On by default.**
    pub fn with_dedup(mut self, dedup: bool) -> Self {
        self.dedup = dedup;
        self
    }

    /// Stream every entry from the configured sources. By default,
    /// duplicates are dropped using the same two-axis heuristic as
    /// [`DedupByRealpath`](super::DedupByRealpath). `Err` items pass
    /// through.
    pub fn scan(&self) -> impl Iterator<Item = Result<WeightedEntry>> + '_ {
        let mut seen_targets: HashSet<String> = HashSet::new();
        let mut seen_aliased_lnks: HashSet<(String, String)> = HashSet::new();
        let dedup = self.dedup;
        self.sources
            .iter()
            .flat_map(|(src, prio)| {
                let prio = *prio;
                let origin = src.origin();
                // `Origin::Custom` carries a `Cow<str>` so the enum is no
                // longer `Copy`; clone per-yield instead of moving.
                src.scan().map(move |r| (r, prio, origin.clone()))
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
                        })
                    })
                }
                Err(err) => Some(Err(err)),
            })
    }
}
