//! Iterator combinator for dropping duplicate [`FileEntry`] results.

use std::collections::HashSet;
use std::path::Path;

use tracing::trace;

use crate::file::{FileEntry, Result};

/// Wraps a stream of [`FileEntry`] results and drops duplicates.
///
/// Two-axis dedup:
///
/// - **Trivial entries** (plain files, reparse stubs, or `.lnk` shortcuts
///   whose filename matches the target's filename — e.g. `chrome.lnk →
///   chrome.exe`) are deduplicated by their resolved target path. So
///   `chrome.lnk` and a separate `chrome.exe` both representing the same
///   binary collapse to one entry.
/// - **Aliased `.lnk`s** (filename differs from the target's filename —
///   e.g. `My PowerShell Launcher.lnk → powershell.exe`) are kept across
///   different filenames pointing at the same target, but two `.lnk`s
///   with the *same filename* and *same target* (e.g. the same
///   `Registry Finder.lnk` appearing on both the Desktop and the Start
///   Menu) collapse to one entry.
///
/// `Err` items pass straight through.
pub struct DedupByRealpath<I> {
    inner: I,
    seen_targets: HashSet<String>,
    seen_aliased_lnks: HashSet<(String, String)>,
}

impl<I> DedupByRealpath<I> {
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            seen_targets: HashSet::new(),
            seen_aliased_lnks: HashSet::new(),
        }
    }
}

impl<I> Iterator for DedupByRealpath<I>
where
    I: Iterator<Item = Result<Box<dyn FileEntry>>>,
{
    type Item = Result<Box<dyn FileEntry>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.inner.next()?;
            let Ok(entry) = &item else {
                return Some(item);
            };

            if dedup_decision(
                &mut self.seen_targets,
                &mut self.seen_aliased_lnks,
                entry.path(),
                entry.link_path(),
            ) {
                return Some(item);
            }
        }
    }
}

/// Shared heuristic between [`DedupByRealpath`] and the [`Gatherer`](super::Gatherer).
/// Returns `true` to keep the entry; updates the appropriate `seen` set.
///
/// Keys are the lowercased path strings. Windows file paths are
/// case-insensitive at the file system layer, and the various Win32
/// surfaces (`IShellLinkW::GetPath`, `fs::read_dir`, etc.) return the
/// same file with inconsistent case.
pub fn dedup_decision(
    seen_targets: &mut HashSet<String>,
    seen_aliased_lnks: &mut HashSet<(String, String)>,
    target: &Path,
    link_path: Option<&Path>,
) -> bool {
    let target_key = target.to_string_lossy().to_lowercase();

    let lnk_stem = link_path
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        })
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().to_lowercase());

    let target_stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase());

    let is_aliased_lnk = matches!(
        (&lnk_stem, &target_stem),
        (Some(l), Some(t)) if l != t
    );

    let kept = if is_aliased_lnk {
        // Aliased shortcut. Dedup by (target, link filename) so two
        // copies of the same `.lnk` from different sources collapse,
        // but distinct aliases (e.g. x64 vs x86 dev prompts) survive.
        seen_aliased_lnks.insert((target_key.clone(), lnk_stem.clone().unwrap()))
    } else {
        // Trivial shortcut or plain file/reparse. Ddedup by target.
        seen_targets.insert(target_key.clone())
    };
    trace!(
        target = %target_key,
        link = ?lnk_stem,
        aliased = is_aliased_lnk,
        kept,
        "dedup decision",
    );
    kept
}
