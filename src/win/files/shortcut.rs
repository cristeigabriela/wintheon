//! Windows shortcut (`.lnk`) resolution via COM and `IShellLinkW`.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::ptr;

use tracing::{debug, trace};
use wincorda::prelude::*;
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{Interface, PCWSTR};

use crate::win::com;

/// Resolve a Windows shortcut (`.lnk`) to its underlying target path.
///
/// Returns `None` if the file isn't a valid shortcut, the target can't be
/// resolved, or any of the underlying COM calls fail.
///
/// Uses `CLSID_ShellLink` to instantiate `IShellLinkW`, loads the file with
/// `IPersistFile::Load`, then reads the target via `IShellLinkW::GetPath`.
pub fn resolve_shortcut(path: &Path) -> Option<PathBuf> {
    com::ensure_sta();

    let path_w = NullTerminated::<WCHAR>::from(path.to_string_lossy());

    // SAFETY: COM is initialized for this thread (STA) by `ensure_sta` above.
    // Each call below is `unsafe` only because the windows-crate marks COM
    // calls as such; we propagate failures by short-circuiting with `?` and
    // log the underlying HRESULT via `inspect_err` before discarding.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .inspect_err(|e| debug!(path = %path.display(), error = %e, "CoCreateInstance(ShellLink) failed"))
            .ok()?;
        let persist: IPersistFile = link
            .cast()
            .inspect_err(|e| debug!(path = %path.display(), error = %e, "QueryInterface(IPersistFile) failed"))
            .ok()?;
        persist
            .Load(PCWSTR(path_w.as_ptr()), STGM_READ)
            .inspect_err(
                |e| debug!(path = %path.display(), error = %e, "IPersistFile::Load failed"),
            )
            .ok()?;

        let mut buf =
            NullTerminated::<WCHAR>::zeroed(NonZeroUsize::new(MAX_PATH as usize).unwrap());
        link.GetPath(buf.as_mut_slice(), ptr::null_mut(), 0)
            .inspect_err(
                |e| debug!(path = %path.display(), error = %e, "IShellLinkW::GetPath failed"),
            )
            .ok()?;
        let target = PathBuf::from(buf.to_string());
        trace!(link = %path.display(), target = %target.display(), "resolved shortcut");
        Some(target)
    }
}
