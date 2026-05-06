//! File icon extraction via Shell APIs and GDI.

use std::ffi::c_void;
use std::io;
use std::mem;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::sync::Mutex;

use tracing::{debug, trace};
use wincorda::prelude::*;
use windows::Win32::Foundation::SIZE;
use windows::Win32::UI::Controls::IImageList;
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SHGetImageList,
    SHIL_EXTRALARGE, SHIL_JUMBO, SIIGBF, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT,
};
use windows::core::{Interface, PCWSTR};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, GetDC, GetDIBits, HBITMAP, ReleaseDC, SelectObject,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows_sys::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON, SHGFI_SYSICONINDEX, SHGetFileInfoW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};

use crate::win::com;
use crate::win::files::{resolve_appexec_link, resolve_shortcut};

/// Side length (px) of the [default](IconSize::Large) extracted icon.
/// Equivalent to `IconSize::Large.pixels()`; kept as a `const` for
/// fixed-size buffer allocations.
pub const ICON_SIZE: u32 = IconSize::Large.pixels();

/// Pixel side length to extract a [`FileIcon`] at.
///
/// The four named variants map to the system image lists Win32 exposes
/// through the simple shell APIs (`SHGetFileInfoW` for `Small`/`Large`,
/// `SHGetImageList(SHIL_*)` for `ExtraLarge`/`Jumbo`).
/// [`Custom`](Self::Custom) goes through `IShellItemImageFactory::GetImage`,
/// which accepts arbitrary pixel sizes at the cost of a slightly heavier
/// shell dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSize {
    /// 16×16 — `SHGFI_SMALLICON`.
    Small,
    /// 32×32 — `SHGFI_LARGEICON`. The library default.
    Large,
    /// 48×48 — `SHIL_EXTRALARGE`.
    ExtraLarge,
    /// 256×256 — `SHIL_JUMBO`.
    Jumbo,
    /// Arbitrary side length, rendered through `IShellItemImageFactory`.
    Custom(u32),
}

impl IconSize {
    /// Side length in pixels.
    #[must_use]
    pub const fn pixels(self) -> u32 {
        match self {
            Self::Small => 16,
            Self::Large => 32,
            Self::ExtraLarge => 48,
            Self::Jumbo => 256,
            Self::Custom(px) => px,
        }
    }
}

/// A Windows file's shell icon.
///
/// At construction, [`new`](Self::new) follows `AppExec` links and `.lnk`
/// shortcuts to record both the original path and the resolved target.
/// [`extract_icon_at`](Self::extract_icon_at) tries the **original** path
/// first (so a `.lnk` returns the shortcut's chosen icon — many `.lnk`s
/// point at generic `Update.exe`-style targets that have no icon of
/// their own) and falls back to the resolved target (so `AppExec` stubs
/// under `WindowsApps`, whose own icon extraction usually fails, still
/// resolve to a renderable icon via the underlying executable).
#[derive(Debug, Clone)]
pub struct FileIcon {
    original: PathBuf,
    resolved: Option<PathBuf>,
}

impl FileIcon {
    /// Build a `FileIcon` from `path`, recording the resolved target if
    /// `path` is an `AppExec` link or `.lnk` shortcut.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        // Gate the AppExec probe on the OS-reported reparse attribute —
        // `resolve_appexec_link` does a `CreateFileW` + `DeviceIoControl`
        // round-trip that we'd otherwise pay on every plain file.
        let resolved = if has_reparse_attribute(&path) {
            resolve_appexec_link(&path)
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        {
            resolve_shortcut(&path)
        } else {
            None
        };
        Self {
            original: path,
            resolved,
        }
    }

    /// Build a `FileIcon` directly from already-known paths, skipping
    /// the auto-resolve in [`new`](Self::new).
    ///
    /// Use this when the caller already has the resolved target cached
    /// — for example, [`Shortcut`](crate::file::Shortcut) caches its
    /// target at construction, so re-walking the `.lnk` inside `new`
    /// would be wasted COM work. Pass `None` for `resolved` when the
    /// `original` path doesn't need following (a regular file, or an
    /// already-resolved reparse target).
    #[must_use]
    pub const fn from_paths(original: PathBuf, resolved: Option<PathBuf>) -> Self {
        Self { original, resolved }
    }

    /// The path the icon nominally represents — the resolved target when
    /// the input was an `AppExec` / `.lnk`, otherwise the original.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.resolved.as_deref().unwrap_or(&self.original)
    }

    /// Extract the icon at the [default](IconSize::Large) size as raw
    /// RGBA bytes — `32×32`, top-down, 4 bytes per pixel.
    #[must_use]
    pub fn extract_icon(&self) -> Option<Vec<u8>> {
        self.extract_icon_at(IconSize::Large)
    }

    /// Extract the icon at the given [`IconSize`] as raw RGBA bytes —
    /// `pixels × pixels`, top-down, 4 bytes per pixel.
    pub fn extract_icon_at(&self, size: IconSize) -> Option<Vec<u8>> {
        if let Some(rgba) = extract_at(&self.original, size) {
            return Some(rgba);
        }
        let rgba = self.resolved.as_deref().and_then(|p| extract_at(p, size));
        if rgba.is_none() {
            debug!(
                original = %self.original.display(),
                resolved = ?self.resolved.as_deref().map(|p| p.display().to_string()),
                size_px = size.pixels(),
                "icon extraction failed for both original and resolved paths",
            );
        }
        rgba
    }

    /// Extract the [default](IconSize::Large) icon and encode it as a
    /// PNG byte stream.
    #[must_use]
    pub fn extract_icon_as_png(&self) -> Option<Vec<u8>> {
        self.extract_icon_as_png_at(IconSize::Large)
    }

    /// Extract the icon at the given [`IconSize`] and encode it as a
    /// PNG byte stream.
    #[must_use]
    pub fn extract_icon_as_png_at(&self, size: IconSize) -> Option<Vec<u8>> {
        let rgba = self.extract_icon_at(size)?;
        let px = size.pixels();
        let mut png_buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_buf, px, px);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(&rgba).ok()?;
        }
        Some(png_buf)
    }
}

/// `true` when the OS marks `path` with `FILE_ATTRIBUTE_REPARSE_POINT`.
/// Uses `symlink_metadata` so `AppExec` stubs (which `metadata` can't follow)
/// are reported correctly.
fn has_reparse_attribute(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

/// Process-wide lock around shell-icon extraction. The shell extensions
/// invoked by `SHGFI_ICON` / `SHGetImageList` / `IShellItemImageFactory`
/// aren't reliably thread-safe; without serialization, concurrent callers
/// occasionally observe spurious failures even for valid paths.
static SHELL_LOCK: Mutex<()> = Mutex::new(());

/// Dispatch an extraction to the appropriate Win32 surface based on
/// `size`, then render the resulting GDI object to RGBA.
fn extract_at(path: &Path, size: IconSize) -> Option<Vec<u8>> {
    // The mutex guards `()`, so a poisoned lock carries no corrupted state —
    // recover the guard rather than swallowing the call.
    let _guard = SHELL_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    // Shell APIs require COM initialized as STA on the calling thread.
    com::ensure_sta();

    let px = size.pixels();
    let path_w = NullTerminated::<WCHAR>::from(path.to_string_lossy());

    // SAFETY: each helper documents its own preconditions.
    unsafe {
        match size {
            IconSize::Small | IconSize::Large => extract_via_sh_get_file_info(&path_w, size, path),
            IconSize::ExtraLarge | IconSize::Jumbo => {
                extract_via_image_list(&path_w, size, path, px)
            }
            IconSize::Custom(_) => extract_via_shell_item(&path_w, path, px),
        }
    }
}

/// `SHGetFileInfoW(SHGFI_ICON | SHGFI_SMALLICON|SHGFI_LARGEICON)` path —
/// the simplest route, returns an `HICON` directly.
///
/// # Safety
/// `path_w` must be null-terminated; called under `SHELL_LOCK` and
/// `com::ensure_sta`.
unsafe fn extract_via_sh_get_file_info(
    path_w: &NullTerminated<'_, WCHAR>,
    size: IconSize,
    path: &Path,
) -> Option<Vec<u8>> {
    let flag = if matches!(size, IconSize::Small) {
        SHGFI_SMALLICON
    } else {
        SHGFI_LARGEICON
    };
    // SAFETY: zeroed `SHFILEINFOW` is a valid input; pointer is valid.
    unsafe {
        let mut info: SHFILEINFOW = mem::zeroed();
        let cb = mem::size_of::<SHFILEINFOW>() as u32;
        let r = SHGetFileInfoW(path_w.as_ptr(), 0, &raw mut info, cb, SHGFI_ICON | flag);
        if r == 0 || info.hIcon.is_null() {
            trace!(
                path = %path.display(),
                returned = r,
                hicon_null = info.hIcon.is_null(),
                "SHGetFileInfoW returned no icon",
            );
            return None;
        }
        let result = render_icon_rgba(info.hIcon, size.pixels());
        DestroyIcon(info.hIcon);
        if result.is_none() {
            debug!(path = %path.display(), "GDI render of HICON to RGBA failed");
        }
        result
    }
}

/// `SHGetFileInfoW(SHGFI_SYSICONINDEX)` + `SHGetImageList(SHIL_*)` +
/// `IImageList::GetIcon` — required for the larger sizes Win32 only
/// exposes through the system image list.
///
/// # Safety
/// `path_w` must be null-terminated; called under `SHELL_LOCK` and
/// `com::ensure_sta`.
unsafe fn extract_via_image_list(
    path_w: &NullTerminated<'_, WCHAR>,
    size: IconSize,
    path: &Path,
    px: u32,
) -> Option<Vec<u8>> {
    // SAFETY: zeroed `SHFILEINFOW` is a valid input; pointer is valid.
    let icon_index = unsafe {
        let mut info: SHFILEINFOW = mem::zeroed();
        let cb = mem::size_of::<SHFILEINFOW>() as u32;
        let r = SHGetFileInfoW(path_w.as_ptr(), 0, &raw mut info, cb, SHGFI_SYSICONINDEX);
        if r == 0 {
            debug!(
                path = %path.display(),
                error = %io::Error::last_os_error(),
                "SHGetFileInfoW(SYSICONINDEX) failed",
            );
            return None;
        }
        info.iIcon
    };

    let shil = match size {
        IconSize::ExtraLarge => SHIL_EXTRALARGE,
        IconSize::Jumbo => SHIL_JUMBO,
        _ => unreachable!("extract_via_image_list called with non-image-list size"),
    };
    // SAFETY: STA initialized; `shil` is a documented constant.
    let image_list: IImageList = unsafe { SHGetImageList(shil as i32) }
        .inspect_err(|e| debug!(path = %path.display(), error = %e, "SHGetImageList failed"))
        .ok()?;
    // ILD_NORMAL == 0; render the icon with no draw modifiers.
    let hicon_typed = unsafe { image_list.GetIcon(icon_index, 0) }
        .inspect_err(|e| debug!(path = %path.display(), error = %e, "IImageList::GetIcon failed"))
        .ok()?;

    // `windows::HICON` is `pub struct HICON(pub *mut c_void)`; `windows-sys`
    // `HICON` is the same raw pointer type aliased — unwrap the field.
    let hicon: HICON = hicon_typed.0;
    // SAFETY: `hicon` is a fresh handle owned by us; render then destroy.
    let result = unsafe { render_icon_rgba(hicon, px) };
    unsafe { DestroyIcon(hicon) };
    if result.is_none() {
        debug!(path = %path.display(), "GDI render of HICON to RGBA failed");
    }
    result
}

/// `IShellItemImageFactory::GetImage` — the modern, arbitrary-size path.
/// Returns an `HBITMAP` (not an `HICON`) that we read pixels from via
/// `GetDIBits`.
///
/// # Safety
/// `path_w` must be null-terminated; called under `SHELL_LOCK` and
/// `com::ensure_sta`.
unsafe fn extract_via_shell_item(
    path_w: &NullTerminated<'_, WCHAR>,
    path: &Path,
    px: u32,
) -> Option<Vec<u8>> {
    // SAFETY: pointer is null-terminated; `bhid` = None uses default.
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(path_w.as_ptr()), None) }
        .inspect_err(
            |e| debug!(path = %path.display(), error = %e, "SHCreateItemFromParsingName failed"),
        )
        .ok()?;
    let factory: IShellItemImageFactory = item
        .cast()
        .inspect_err(|e| debug!(path = %path.display(), error = %e, "QueryInterface(IShellItemImageFactory) failed"))
        .ok()?;
    // SIIGBF_ICONONLY skips thumbnail generation — we want the icon only,
    // matching the behavior of the SHGetFileInfoW / SHGetImageList paths.
    let flags = SIIGBF(SIIGBF_RESIZETOFIT.0 | SIIGBF_ICONONLY.0);
    let bitmap = unsafe {
        factory.GetImage(
            SIZE {
                cx: px as i32,
                cy: px as i32,
            },
            flags,
        )
    }
    .inspect_err(
        |e| debug!(path = %path.display(), error = %e, "IShellItemImageFactory::GetImage failed"),
    )
    .ok()?;

    let hbitmap: HBITMAP = bitmap.0 as HBITMAP;
    // SAFETY: `hbitmap` is owned by us; release with `DeleteObject` after read.
    let result = unsafe { render_bitmap_rgba(hbitmap, px) };
    unsafe { DeleteObject(hbitmap.cast()) };
    if result.is_none() {
        debug!(path = %path.display(), "GDI read of HBITMAP failed");
    }
    result
}

/// Render an `HICON` into a 32-bit top-down DIB and return its pixels as RGBA.
///
/// # Safety
/// `hicon` must be a valid icon handle. Ownership of `hicon` is unaffected;
/// the caller is responsible for `DestroyIcon`.
unsafe fn render_icon_rgba(hicon: HICON, px: u32) -> Option<Vec<u8>> {
    let size = px as i32;

    // SAFETY: caller upholds `hicon` validity; every Win32 call below has
    // its preconditions checked (non-null handles, properly initialized
    // structs) and is balanced by a matching release/destroy on every path.
    unsafe {
        let screen_dc = GetDC(ptr::null_mut());
        if screen_dc.is_null() {
            debug!(error = %io::Error::last_os_error(), "GetDC(NULL) failed");
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            debug!(error = %io::Error::last_os_error(), "CreateCompatibleDC failed");
            ReleaseDC(ptr::null_mut(), screen_dc);
            return None;
        }

        // Top-down 32-bit BGRA DIB section.
        let mut bmi: BITMAPINFO = mem::zeroed();
        bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -size; // negative => rows are top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut pixels: *mut c_void = ptr::null_mut();
        let dib = CreateDIBSection(
            mem_dc,
            &raw const bmi,
            DIB_RGB_COLORS,
            &raw mut pixels,
            ptr::null_mut(),
            0,
        );
        if dib.is_null() || pixels.is_null() {
            debug!(error = %io::Error::last_os_error(), "CreateDIBSection failed");
            DeleteDC(mem_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);
            return None;
        }

        let prev = SelectObject(mem_dc, dib);
        let drew = DrawIconEx(
            mem_dc,
            0,
            0,
            hicon,
            size,
            size,
            0,
            ptr::null_mut(),
            DI_NORMAL,
        );
        if drew == 0 {
            debug!(error = %io::Error::last_os_error(), "DrawIconEx failed");
        }

        let rgba = if drew != 0 {
            GdiFlush();
            let n = (px * px) as usize;
            let bgra = slice::from_raw_parts(pixels as *const u8, n * 4);
            Some(bgra_to_rgba(bgra))
        } else {
            None
        };

        SelectObject(mem_dc, prev);
        DeleteObject(dib);
        DeleteDC(mem_dc);
        ReleaseDC(ptr::null_mut(), screen_dc);

        rgba
    }
}

/// Read pixels out of an `HBITMAP` produced by `IShellItemImageFactory::GetImage`
/// and return them as RGBA. `px` must match the bitmap's actual side length.
///
/// # Safety
/// `hbitmap` must be a valid GDI bitmap handle. Ownership is unaffected.
unsafe fn render_bitmap_rgba(hbitmap: HBITMAP, px: u32) -> Option<Vec<u8>> {
    // SAFETY: every Win32 call below checks its return; DC is released
    // on every exit path.
    unsafe {
        let screen_dc = GetDC(ptr::null_mut());
        if screen_dc.is_null() {
            debug!(error = %io::Error::last_os_error(), "GetDC(NULL) failed");
            return None;
        }

        let mut bmi: BITMAPINFO = mem::zeroed();
        bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = px as i32;
        bmi.bmiHeader.biHeight = -(px as i32); // top-down rows
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let n = (px * px) as usize;
        let mut bgra = vec![0u8; n * 4];
        let copied = GetDIBits(
            screen_dc,
            hbitmap,
            0,
            px,
            bgra.as_mut_ptr().cast(),
            &raw mut bmi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(ptr::null_mut(), screen_dc);
        if copied == 0 {
            debug!(error = %io::Error::last_os_error(), "GetDIBits failed");
            return None;
        }
        Some(bgra_to_rgba(&bgra))
    }
}

/// Swap the red and blue channels — Win32 returns BGRA from its DIB
/// sections, callers expect RGBA.
fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for chunk in bgra.chunks_exact(4) {
        rgba.push(chunk[2]);
        rgba.push(chunk[1]);
        rgba.push(chunk[0]);
        rgba.push(chunk[3]);
    }
    rgba
}
