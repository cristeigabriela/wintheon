//! COM initialization helpers.
//!
//! Windows COM APIs (`CoCreateInstance`, shell interfaces like `IShellLinkW`,
//! etc.) require the calling thread to have entered a COM apartment first.
//! Shell APIs in particular expect a [Single-Threaded Apartment](https://learn.microsoft.com/en-us/windows/win32/com/single-threaded-apartments) (STA).
//!
//! [`ensure_sta`] initializes COM as STA exactly once per thread; the
//! matching `CoUninitialize` runs when the thread exits via the
//! thread-local destructor.

use std::ptr;

use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

thread_local! {
    static COM_GUARD: ComGuard = ComGuard::init_sta();
}

struct ComGuard;

impl ComGuard {
    fn init_sta() -> Self {
        // SAFETY: `CoInitializeEx` is callable from any thread. Subsequent
        // calls on the same thread return `S_FALSE` (already initialized);
        // we ignore the HRESULT — best-effort init for the current thread.
        unsafe {
            CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
        }
        Self
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: paired with the `CoInitializeEx` in `init_sta`.
        unsafe {
            CoUninitialize();
        }
    }
}

/// Initialize COM as a Single-Threaded Apartment for the current thread,
/// once. Cheap to call repeatedly — subsequent calls touch a thread-local
/// flag and return immediately.
pub fn ensure_sta() {
    COM_GUARD.with(|_| {});
}
