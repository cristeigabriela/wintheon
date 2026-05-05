//! Reparse-point utilities — App Execution Aliases (`AppExecLink`), etc.

use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;

use wincorda::prelude::*;
use windows_sys::Wdk::Storage::FileSystem::REPARSE_DATA_BUFFER;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, MAXIMUM_REPARSE_DATA_BUFFER_SIZE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_APPEXECLINK;

#[derive(Debug, Clone)]
pub struct AppExecLink {
    pub package_name: String,
    pub package_entrypoint: String,
    pub real_path: PathBuf,
}

/// Follow an AppExecutionAlias reparse point to the real path.
///
/// Because of this being an AppExecLink, we can't just open a handle,
/// follow reparse and call `GetFinalPathNameByHandleW`, because `CreateFileW`
/// will fail.
/// 
/// For more info: https://www.tiraniddo.dev/2019/09/overview-of-windows-execution-aliases.html
pub fn resolve_appexec_link(path: &Path) -> Option<PathBuf> {
    let path_w = NullTerminated::<WCHAR>::from(path.to_string_lossy().into_owned());

    // SAFETY: open without following the reparse so we get a handle to the
    // stub itself; `FILE_FLAG_OPEN_REPARSE_POINT` bypasses the "reparse,
    // refuse plain reads" check on the file system.
    //
    // "Normal reparse point processing will not occur; CreateFile will attempt to
    //  open the reparse point. When a file is opened, a file handle is returned, whether
    //  or not the filter that controls the reparse point is operational."
    let handle = unsafe {
        CreateFileW(
            path_w.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    // SAFETY: `Vec<u8>` is guaranteed to be 1-byte-aligned, so it is safe to
    // pass `as_mut_ptr` around, as long as capacity is respected.
    let mut buf: Vec<u8> = vec![0; MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize]; // will usually be smaller, may truncate
    let mut returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_GET_REPARSE_POINT,
            ptr::null(),
            0,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    // SAFETY: handle is closed regardless of ioctl result.
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 {
        return None;
    }

    // Get the real path from the appexec reparse point buffer.
    Some(parse_appexec_buffer(&buf[..returned as usize])?.real_path)
}

/// Read a [`REPARSE_DATA_BUFFER`] for an [`IO_REPARSE_TAG_APPEXECLINK`]
/// for reparse information.
///
/// Example:
/// ```rs
/// [
///     // AppX manifest package name
///     "Microsoft.WindowsNotepad_8wekyb3d8bbwe",
///     // AppX manifest package entrypoint
///     "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App",
///     // True path of reparse point
///     "C:\\Program Files\\WindowsApps\\Microsoft.WindowsNotepad_11.2512.29.0_x64__8wekyb3d8bbwe\\Notepad\\Notepad.exe",
///     "0",
/// ]
/// ```
fn parse_appexec_buffer(buffer: &[u8]) -> Option<AppExecLink> {
    if buffer.len() < mem::size_of::<REPARSE_DATA_BUFFER>() {
        return None;
    }

    // SAFETY: buffer is large enough to be read as a `REPARSE_DATA_BUFFER`.
    let reparse = unsafe { &*(buffer.as_ptr() as *const REPARSE_DATA_BUFFER) };
    if reparse.ReparseTag != IO_REPARSE_TAG_APPEXECLINK {
        return None;
    }

    // The body sits in `Anonymous.GenericReparseBuffer.DataBuffer` and starts
    // with a `u32` version field; four UTF-16 strings follow.
    //
    // SAFETY: union projection is valid because the tag matched APPEXECLINK;
    // the strings live in the same allocation as the header.
    let strings_ptr = unsafe {
        reparse
            .Anonymous
            .GenericReparseBuffer
            .DataBuffer
            .as_ptr()
            .add(mem::size_of::<u32>()) as *const WCHAR
    };

    let strings: Vec<String> = MultiBuffer::try_from(strings_ptr)
        .ok()?
        .into_iter()
        .collect();
    // Exactly 4 entries: package family name, entry point, target exe, app type.
    let [package_name, package_entrypoint, real_path, _app_type] =
        <[String; 4]>::try_from(strings).ok()?;

    Some(AppExecLink {
        package_name,
        package_entrypoint,
        real_path: PathBuf::from(real_path),
    })
}
