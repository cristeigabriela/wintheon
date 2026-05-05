//! File icon extraction via Shell APIs and GDI.

use std::ffi::c_void;
use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::sync::Mutex;

use wincorda::prelude::*;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC, SelectObject,
};
use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
use windows_sys::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};

use crate::win::com;
use crate::win::files::{resolve_appexec_link, resolve_shortcut};

/// Side length (px) of the extracted icon. Matches `SHGFI_LARGEICON`
/// at 100% DPI.
pub const ICON_SIZE: u32 = 32;

/// A Windows file's shell icon.
///
/// At construction, [`new`](Self::new) follows AppExec links and `.lnk`
/// shortcuts to record both the original path and the resolved target.
/// [`extract_icon`](Self::extract_icon) tries the resolved target first
/// (so a `.lnk` to `cmd.exe` returns `cmd.exe`'s icon) and falls back to
/// the original (so AppExec stubs under `WindowsApps`, whose target the
/// shell can't read directly, still resolve to a renderable icon via the
/// stub itself).
#[derive(Debug, Clone)]
pub struct FileIcon {
    original: PathBuf,
    resolved: Option<PathBuf>,
}

impl FileIcon {
    /// Build a `FileIcon` from `path`, recording the resolved target if
    /// `path` is an AppExec link or `.lnk` shortcut.
    pub fn new(path: PathBuf) -> Self {
        let resolved = resolve_appexec_link(&path).or_else(|| {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
            {
                resolve_shortcut(&path)
            } else {
                None
            }
        });
        Self {
            original: path,
            resolved,
        }
    }

    /// The path the icon nominally represents — the resolved target when
    /// the input was an AppExec / `.lnk`, otherwise the original.
    pub fn path(&self) -> &Path {
        self.resolved.as_deref().unwrap_or(&self.original)
    }

    /// Extract the icon as raw RGBA bytes — `ICON_SIZE * ICON_SIZE`, top-down,
    /// 4 bytes per pixel (`ICON_SIZE * ICON_SIZE * 4` bytes total).
    pub fn extract_icon(&self) -> Option<Vec<u8>> {
        // If there is a shortcut, try to obtain the icon from it first.
        if let Some(rgba) = extract_icon_at(&self.original) {
            return Some(rgba);
        }

        // Obtain the icon from the real path.
        self.resolved.as_deref().and_then(extract_icon_at)
    }

    /// Extract the icon and encode it as a PNG byte stream.
    pub fn extract_icon_as_png(&self) -> Option<Vec<u8>> {
        let rgba = self.extract_icon()?;
        let mut png_buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_buf, ICON_SIZE, ICON_SIZE);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(&rgba).ok()?;
        }
        Some(png_buf)
    }
}

/// Process-wide lock around `SHGetFileInfoW` + GDI rendering. The shell
/// extensions invoked by `SHGFI_ICON` aren't reliably thread-safe; without
/// this, concurrent callers occasionally observe `SHGetFileInfoW` returning
/// 0 or a null `hIcon` even for valid paths.
static SHELL_LOCK: Mutex<()> = Mutex::new(());

/// Run `SHGetFileInfoW` on `path` and render the returned `HICON` to RGBA.
fn extract_icon_at(path: &Path) -> Option<Vec<u8>> {
    // Serialize concurrent shell-icon extractions; see `SHELL_LOCK`.
    let _guard = SHELL_LOCK.lock().ok()?;

    // `SHGetFileInfoW` (and the shell extensions it dispatches to for
    // `SHGFI_ICON`) requires COM initialized as STA on the calling thread.
    com::ensure_sta();

    let path_w = NullTerminated::<WCHAR>::from(path.to_string_lossy());

    // SAFETY: `path_w` is null-terminated; `info` is fully zeroed before use.
    unsafe {
        let mut info: SHFILEINFOW = mem::zeroed();
        let cb = mem::size_of::<SHFILEINFOW>() as u32;
        let r = SHGetFileInfoW(
            path_w.as_ptr(),
            0,
            &mut info,
            cb,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if r == 0 || info.hIcon.is_null() {
            return None;
        }
        let hicon = info.hIcon;

        let result = render_icon_rgba(hicon);
        DestroyIcon(hicon);
        result
    }
}

/// Render an `HICON` into a 32-bit top-down DIB and return its pixels as RGBA.
///
/// # Safety
/// `hicon` must be a valid icon handle. Ownership of `hicon` is unaffected;
/// the caller is responsible for `DestroyIcon`.
unsafe fn render_icon_rgba(hicon: HICON) -> Option<Vec<u8>> {
    let size = ICON_SIZE as i32;

    // SAFETY: caller upholds `hicon` validity; every Win32 call below has
    // its preconditions checked (non-null handles, properly initialized
    // structs) and is balanced by a matching release/destroy on every path.
    unsafe {
        let screen_dc = GetDC(ptr::null_mut());
        if screen_dc.is_null() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
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
            &bmi,
            DIB_RGB_COLORS,
            &mut pixels,
            ptr::null_mut(),
            0,
        );
        if dib.is_null() || pixels.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);
            return None;
        }

        let prev = SelectObject(mem_dc, dib);
        let drew = DrawIconEx(mem_dc, 0, 0, hicon, size, size, 0, ptr::null_mut(), DI_NORMAL);

        let rgba = if drew != 0 {
            GdiFlush();
            let n = (ICON_SIZE * ICON_SIZE) as usize;
            let bgra = slice::from_raw_parts(pixels as *const u8, n * 4);
            let mut rgba = Vec::with_capacity(n * 4);
            for px in bgra.chunks_exact(4) {
                rgba.push(px[2]);
                rgba.push(px[1]);
                rgba.push(px[0]);
                rgba.push(px[3]);
            }
            Some(rgba)
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
