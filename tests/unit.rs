//! Unit tests — small isolated checks of individual `wintheon` items.
//!
//! Anything that scans the file system or pokes at real installed apps
//! lives in `tests/integration.rs`.

use std::path::PathBuf;

use wintheon::file::{FileEntry, FileIcon, RegularFile, ReparsePoint, Shortcut, Translation};
use wintheon::gather::{DesktopSource, Origin, Source, StartMenuSource, WindowsAppsSource};

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
