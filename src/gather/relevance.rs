//! Query-relevance scoring for [`FileEntry`]s.
//!
//! Build a [`MatchIndex`] once per entry; query it repeatedly with a
//! lowercased needle. The score is intended to be multiplied against
//! [`WeightedEntry::priority_score`](super::WeightedEntry::priority_score)
//! for combined source × entry × relevance ranking — see the
//! [`gather`](super) module overview for the full pipeline.
//!
//! ```no_run
//! use std::path::PathBuf;
//! use wintheon::file::RegularFile;
//! use wintheon::gather::MatchIndex;
//!
//! let entry = RegularFile::new(PathBuf::from(r"C:\Windows\System32\notepad.exe"));
//! let index = MatchIndex::from_entry(&entry);
//! assert!(index.score("notepad").is_some());
//! assert!(index.score("zzz").is_none());
//! ```

use crate::file::FileEntry;

/// Lowercased corpus extracted from a [`FileEntry`] for query-time
/// relevance scoring.
///
/// Built once via [`from_entry`](Self::from_entry) — which reads
/// version-info, so it isn't free — then queried repeatedly via
/// [`score`](Self::score). The struct is `Send + Sync`, so it composes
/// with `OnceLock<MatchIndex>` (or any other interior-mutability cache)
/// for lazy population on the consumer side.
#[derive(Debug, Clone)]
pub struct MatchIndex {
    display_lc: String,
    path_stem_lc: String,
    corpus: String,
}

impl MatchIndex {
    /// Build the index. Reads `entry.display_name()`, the path's file
    /// stem, the link path's file stem (when present), and English-locale
    /// version-info fields (description, company, product name, original
    /// filename, file version) — all lowercased.
    pub fn from_entry(entry: &dyn FileEntry) -> Self {
        let display_lc = entry.display_name().to_lowercase();
        let path_stem_lc = entry
            .path()
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let link_stem_lc = entry
            .link_path()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let mut pieces: Vec<String> = Vec::with_capacity(8);
        push_unique(&mut pieces, display_lc.clone());
        push_unique(&mut pieces, path_stem_lc.clone());
        push_unique(&mut pieces, link_stem_lc);

        if let Ok(info) = entry.version_info()
            && let Some(fi) = info.english()
        {
            for s in [
                fi.file_description(),
                fi.company_name(),
                fi.product_name(),
                fi.original_filename(),
                fi.file_version(),
            ]
            .into_iter()
            .flatten()
            {
                push_unique(&mut pieces, s.to_lowercase());
            }
        }

        let corpus = pieces.join("\n");
        Self {
            display_lc,
            path_stem_lc,
            corpus,
        }
    }

    /// Score the index against a lowercased `needle`. Tier ladder:
    ///
    /// | tier | match site                                   | multiplier |
    /// |------|----------------------------------------------|------------|
    /// | 1    | display name == needle                       | 10.0       |
    /// | 2    | display name starts with needle              |  8.0       |
    /// | 3    | display contains needle at a word boundary   |  3.0       |
    /// | 4    | path stem contains needle at a word boundary |  2.0       |
    /// | 5    | display contains needle anywhere (mid-word)  |  1.0       |
    /// | 6    | needle anywhere in version-info corpus       |  0.5       |
    ///
    /// Word-boundary tiers prevent accidental substring matches inside
    /// run-on names (e.g. `"chrome"` inside `"iCloudChrome"`) from
    /// outranking real-word matches like `"Google Chrome"`. A "word
    /// boundary" here is the start of the string or any non-alphanumeric
    /// character.
    ///
    /// Tier ratios are tuned so any tier-step jump (≥ 1.5×) exceeds
    /// realistic per-source priority spreads, keeping relevance
    /// dominant over source weight.
    ///
    /// Empty `needle` is treated as a neutral 1.0 match so the unfiltered
    /// list rolls back to pure priority order.
    #[must_use]
    pub fn score(&self, needle: &str) -> Option<f32> {
        if needle.is_empty() {
            return Some(1.0);
        }
        if self.display_lc == needle {
            return Some(10.0);
        }
        if self.display_lc.starts_with(needle) {
            return Some(8.0);
        }
        if contains_at_word_boundary(&self.display_lc, needle) {
            return Some(3.0);
        }
        if contains_at_word_boundary(&self.path_stem_lc, needle) {
            return Some(2.0);
        }
        if self.display_lc.contains(needle) {
            return Some(1.0);
        }
        if self.corpus.contains(needle) {
            return Some(0.5);
        }
        None
    }
}

/// Append `s` to `pieces` if non-empty and not already the most recent
/// entry (most duplication is adjacent — `display_name` is often the
/// same as the path stem).
fn push_unique(pieces: &mut Vec<String>, s: String) {
    if s.is_empty() {
        return;
    }
    if pieces.last().is_some_and(|prev| prev == &s) {
        return;
    }
    pieces.push(s);
}

/// `true` iff `needle` occurs in `haystack` starting at a word boundary —
/// the start of the string or right after a non-alphanumeric char (space,
/// `-`, `_`, `.`, etc.). Both arguments are expected to be lowercased;
/// the comparison is byte-exact.
fn contains_at_word_boundary(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut prev: Option<char> = None;
    for (i, c) in haystack.char_indices() {
        if haystack[i..].starts_with(needle) && prev.is_none_or(|p| !p.is_alphanumeric()) {
            return true;
        }
        prev = Some(c);
    }
    false
}
