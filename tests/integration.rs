//! Integration tests — exercise `wintheon` end-to-end against real OS files
//! and Win32 surfaces (notepad, cmd, the WindowsApps AppExec stubs).

use std::path::{Path, PathBuf};

use wildmatch::WildMatch;
use wincorda::prelude::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{Interface, PCWSTR};
use wintheon::file::{FileEntry, FileIcon, FileVersionInfo, ICON_SIZE, RegularFile};
use wintheon::gather::{Source, WindowsAppsSource};
use wintheon::win::{com, resolve_appexec_link, resolve_shortcut};

#[test]
fn loads_version_info_from_notepad() {
    let path = Path::new(r"C:\Windows\System32\notepad.exe");
    let info =
        FileVersionInfo::load(path).expect("notepad.exe should have a VS_VERSIONINFO resource");

    assert!(
        !info.all().is_empty(),
        "expected at least one translation in notepad's version info",
    );

    let copyright = info
        .english()
        .and_then(|i| i.legal_copyright())
        .expect("missing english legal_copyright");
    assert!(
        copyright.to_lowercase().contains("microsoft corporation"),
        "wrong legal copyright field",
    );
}

#[test]
fn resolves_appexec_link_for_windowsapps_notepad() {
    let local = std::env::var("LOCALAPPDATA").expect("couldn't get local appdata");
    let stub = format!(r"{local}\Microsoft\WindowsApps\notepad.exe");

    let real =
        resolve_appexec_link(Path::new(&stub)).expect("couldn't resolve appexec link");

    // The resolved package directory is suffixed with version + arch + signature
    // (e.g. `Microsoft.WindowsNotepad_11.2512.29.0_x64__8wekyb3d8bbwe`); match
    // the moving parts with `*` so the test survives store updates.
    let program_files = std::env::var("PROGRAMFILES").expect("couldn't get program files");
    let pattern = WildMatch::new(&format!(
        r"{program_files}\WindowsApps\Microsoft.WindowsNotepad_*\Notepad\Notepad.exe"
    ));
    let real_str = real.to_string_lossy();
    assert!(
        pattern.matches(&real_str),
        "real_path {real_str:?} didn't match pattern {pattern:?}",
    );
}

#[test]
fn resolve_shortcut_round_trips_a_freshly_created_lnk() {
    com::ensure_sta();

    let target = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    let lnk_path = std::env::temp_dir()
        .join(format!("wintheon_shortcut_{}.lnk", std::process::id()));

    let target_w = NullTerminated::<WCHAR>::from(target.to_string_lossy());
    let lnk_path_w = NullTerminated::<WCHAR>::from(lnk_path.to_string_lossy());

    // SAFETY: COM initialized as STA on this thread by `ensure_sta`.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .expect("CoCreateInstance(ShellLink) failed");
        link.SetPath(PCWSTR(target_w.as_ptr()))
            .expect("IShellLinkW::SetPath failed");
        let persist: IPersistFile = link.cast().expect("QueryInterface(IPersistFile) failed");
        persist
            .Save(PCWSTR(lnk_path_w.as_ptr()), false)
            .expect("IPersistFile::Save failed");
    }

    let resolved = resolve_shortcut(&lnk_path).expect("resolve_shortcut returned None");
    // Cleanup before the assertion so a failing assert doesn't leak the file.
    let _ = std::fs::remove_file(&lnk_path);
    assert_eq!(resolved, target);
}

#[test]
fn extracts_cmd_icon_as_rgba_and_png() {
    let icon = FileIcon::new(PathBuf::from(r"C:\Windows\System32\cmd.exe"));

    let rgba = icon.extract_icon().expect("extract_icon returned None");
    assert_eq!(
        rgba.len(),
        (ICON_SIZE * ICON_SIZE * 4) as usize,
        "expected ICON_SIZE×ICON_SIZE×4 RGBA bytes",
    );
    assert!(
        rgba.chunks_exact(4).any(|p| p[3] != 0),
        "icon was fully transparent",
    );

    let png = icon
        .extract_icon_as_png()
        .expect("extract_icon_as_png returned None");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "missing PNG magic");
    assert_eq!(&png[12..16], b"IHDR", "missing IHDR chunk");
}

#[test]
fn extracts_icon_through_appexec_resolution() {
    let local = std::env::var("LOCALAPPDATA").expect("couldn't get local appdata");
    let stub = format!(r"{local}\Microsoft\WindowsApps\notepad.exe");

    let icon = FileIcon::new(PathBuf::from(&stub));
    let pattern =
        WildMatch::new(r"*\WindowsApps\Microsoft.WindowsNotepad_*\Notepad\Notepad.exe");
    assert!(
        pattern.matches(&icon.path().to_string_lossy()),
        "FileIcon::new didn't follow the AppExec link",
    );

    let png = icon
        .extract_icon_as_png()
        .expect("extract_icon_as_png returned None");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn file_entry_icon_method_extracts_a_png() {
    let entry = RegularFile::new(PathBuf::from(r"C:\Windows\System32\cmd.exe"));
    let icon = entry.icon().expect("FileEntry::icon");
    let png = icon
        .extract_icon_as_png()
        .expect("extract_icon_as_png returned None");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn file_entry_version_info_has_english_translation() {
    let entry = RegularFile::new(PathBuf::from(r"C:\Windows\System32\notepad.exe"));
    let info = entry.version_info().expect("FileEntry::version_info");
    assert!(
        info.english().is_some(),
        "notepad should expose an English translation",
    );
}

#[test]
fn windows_apps_source_yields_appexec_entries() {
    let entries: Vec<_> = WindowsAppsSource::new()
        .scan()
        .filter_map(|r| r.ok())
        .collect();

    // Standard Win10/11 installs ship at least the `notepad.exe` AppExec
    // stub under WindowsApps; if it's gone something stranger is up.
    assert!(!entries.is_empty(), "WindowsApps scan returned no entries");

    let reparse_count = entries.iter().filter(|e| e.link_path().is_some()).count();
    assert!(
        reparse_count > 0,
        "expected at least one reparse-point entry, got {reparse_count}",
    );
}
