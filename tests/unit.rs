//! Unit tests — small isolated checks of individual `wintheon` items.
//!
//! Anything that scans the file system or pokes at real installed apps
//! lives in `tests/integration.rs`.

use std::path::PathBuf;

use wintheon::file::{
    FileEntry, FileIcon, Priority, RegularFile, ReparsePoint, Shortcut, Translation,
};
use wintheon::gather::{
    DedupByRealpath, DesktopSource, MatchIndex, Origin, Source, StartMenuSource, WeightedEntry,
    WeightedEntryIteratorExt, WindowsAppsSource,
};

#[test]
fn translation_default_is_us_english_unicode() {
    let t = Translation::default();
    assert_eq!(t.language, 0x0409, "LANG_ENGLISH | SUBLANG_ENGLISH_US");
    assert_eq!(t.code_page, 1200, "Unicode codepage");
}

#[test]
fn translation_from_system_returns_a_valid_langid() {
    let t = Translation::from_system();
    assert_ne!(
        t.language, 0,
        "GetUserDefaultUILanguage should return a real LANGID"
    );
    assert_eq!(t.code_page, 1200);
}

#[test]
fn regular_file_round_trips_its_constructor_path() {
    let path = PathBuf::from(r"C:\example.txt");
    let entry = RegularFile::new(path.clone());
    assert_eq!(entry.path(), path.as_path());
    assert!(entry.link_path().is_none());
}

#[test]
fn shortcut_returns_none_for_nonexistent_path() {
    let nope = PathBuf::from(r"C:\definitely\does\not\exist\nope.lnk");
    assert!(Shortcut::new(nope).is_none());
}

#[test]
fn reparse_point_returns_none_for_a_regular_file() {
    // `cmd.exe` ships on every Windows install and is *not* a reparse point.
    let regular = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    assert!(ReparsePoint::new(regular).is_none());
}

#[test]
fn file_icon_falls_back_to_original_for_unresolvable_path() {
    // cmd.exe isn't an AppExec link and isn't a `.lnk`, so the resolved
    // path should match the original.
    let path = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    let icon = FileIcon::new(path.clone());
    assert_eq!(icon.path(), path.as_path());
}

#[test]
fn source_origins_match() {
    assert_eq!(DesktopSource::new().origin(), Origin::Desktop);
    assert_eq!(StartMenuSource::new().origin(), Origin::StartMenu);
    assert_eq!(WindowsAppsSource::new().origin(), Origin::WindowsApps);
}

#[test]
fn sources_implement_default() {
    let _: DesktopSource = Default::default();
    let _: StartMenuSource = Default::default();
    let _: WindowsAppsSource = Default::default();
}

#[test]
fn match_index_tier_ladder_for_synthetic_paths() {
    // Nonexistent paths -> `version_info` errors out and `RegularFile`
    // falls back to file_stem for `display_name`. That makes these
    // tier-ladder assertions robust to whatever the host has installed.
    let notepad = RegularFile::new(PathBuf::from(r"C:\fake\notepad.exe"));
    let idx = MatchIndex::from_entry(&notepad);
    // display_lc = "notepad", path stem = "notepad"
    assert_eq!(idx.score("notepad"), Some(10.0), "exact match");
    assert_eq!(idx.score("note"), Some(8.0), "starts_with");
    assert_eq!(idx.score("pad"), Some(1.0), "mid-word in display");
    assert_eq!(idx.score(""), Some(1.0), "empty needle is neutral");
    assert_eq!(idx.score("zzz"), None, "no match");
}

#[test]
fn match_index_word_boundary_outranks_mid_word_substring() {
    // The motivating case: query "chrome" must rank "Google Chrome"
    // (display contains at word boundary, tier 3 = 3.0) above
    // "iCloudChrome" (mid-word substring, tier 5 = 1.0).
    let google = RegularFile::new(PathBuf::from(r"C:\fake\Google Chrome.exe"));
    let icloud = RegularFile::new(PathBuf::from(r"C:\fake\iCloudChrome.exe"));
    let g = MatchIndex::from_entry(&google);
    let i = MatchIndex::from_entry(&icloud);
    let g_score = g.score("chrome").expect("google should match");
    let i_score = i.score("chrome").expect("icloud should still match");
    assert!(g_score > i_score, "{g_score} should outrank {i_score}");
    assert_eq!(g_score, 3.0);
    assert_eq!(i_score, 1.0);
}

#[test]
fn weighted_entry_priority_score_multiplies_source_and_entry() {
    // `.txt` isn't in `%PATHEXT%` so `RegularFile::priority` returns 1.0;
    // multiplied by source 2.5 we expect exactly 2.5.
    let entry: Box<dyn FileEntry> = Box::new(RegularFile::new(PathBuf::from(r"C:\example.txt")));
    let weighted = WeightedEntry::new(entry, Origin::Desktop, Priority(2.5));
    assert_eq!(weighted.priority_score(), 2.5);
}

#[test]
fn weighted_entry_score_auto_lowercases_query() {
    // Mixed-case input should produce the same score as the lowercased
    // version — the Cow-based check inside `score` must allocate when
    // needed without leaking the casing distinction to the caller.
    let entry: Box<dyn FileEntry> =
        Box::new(RegularFile::new(PathBuf::from(r"C:\fake\notepad.exe")));
    let weighted = WeightedEntry::new(entry, Origin::Desktop, Priority(1.0));
    let lower = weighted.score("notepad");
    let upper = weighted.score("NOTEPAD");
    let mixed = weighted.score("Notepad");
    assert!(lower.is_some());
    assert_eq!(lower, upper);
    assert_eq!(lower, mixed);
}

#[test]
fn weighted_entry_score_empty_query_returns_priority_score() {
    // Empty needle is a neutral 1.0 multiplier, so `score("")` should
    // collapse to `priority_score()`.
    let entry: Box<dyn FileEntry> =
        Box::new(RegularFile::new(PathBuf::from(r"C:\fake\notepad.exe")));
    let weighted = WeightedEntry::new(entry, Origin::Desktop, Priority(1.5));
    assert_eq!(weighted.score(""), Some(weighted.priority_score()));
}

#[test]
fn sorted_by_score_filters_misses_and_orders_descending() {
    // Two entries match "note" at tier 2 (starts_with). Different
    // priorities resolve the tie, so the higher-priority one wins.
    // Scores aren't returned (sorted_by_score drops them after sorting),
    // so we verify the ordering by inspecting the resulting items.
    let entries = [
        WeightedEntry::new(
            Box::new(RegularFile::new(PathBuf::from(r"C:\fake\notepad.exe"))),
            Origin::Desktop,
            Priority(1.0),
        ),
        WeightedEntry::new(
            Box::new(RegularFile::new(PathBuf::from(
                r"C:\fake\note taking app.exe",
            ))),
            Origin::Desktop,
            Priority(2.0),
        ),
        WeightedEntry::new(
            Box::new(RegularFile::new(PathBuf::from(r"C:\fake\unrelated.exe"))),
            Origin::Desktop,
            Priority(1.0),
        ),
    ];

    let ranked = entries.iter().sorted_by_score("note");

    assert_eq!(ranked.len(), 2, "unrelated.exe should be filtered out");
    let first = ranked[0]
        .entry
        .path()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        first, "note taking app",
        "higher priority should sort first"
    );
}

#[test]
fn dedup_by_realpath_drops_duplicate_targets() {
    // Two `RegularFile` entries pointing at the same path should
    // collapse to one through `DedupByRealpath`. The lowercased path
    // string is the dedup key, so casing differences also fold.
    let stream: Vec<wintheon::file::Result<Box<dyn FileEntry>>> = vec![
        Ok(Box::new(RegularFile::new(PathBuf::from(
            r"C:\fake\App.exe",
        )))),
        Ok(Box::new(RegularFile::new(PathBuf::from(
            r"C:\FAKE\APP.EXE",
        )))),
        Ok(Box::new(RegularFile::new(PathBuf::from(
            r"C:\fake\Other.exe",
        )))),
    ];
    let kept: Vec<_> = DedupByRealpath::new(stream.into_iter())
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(
        kept.len(),
        2,
        "case-insensitive dedup should collapse the two App.exe entries"
    );
}
