//! Thin Win32 wrappers for `VS_VERSIONINFO` resources.
//!
//! Surface mirrors [`System.Diagnostics.FileVersionInfo`](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.fileversioninfo?view=net-10.0)
//! — every property .NET exposes for the same `VS_VERSIONINFO` resource
//! lives somewhere on [`FileVersionInfo`], [`FileInformation`] (per
//! translation), or [`FixedFileInfo`] (file-global numeric/flag info).

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::mem;
use std::path::Path;
use std::ptr;

use wincorda::prelude::*;
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FF_DEBUG, VS_FF_INFOINFERRED, VS_FF_PATCHED,
    VS_FF_PRERELEASE, VS_FF_PRIVATEBUILD, VS_FF_SPECIALBUILD, VS_FIXEDFILEINFO, VerQueryValueW,
};
use windows_sys::Win32::System::SystemServices::{LANG_ENGLISH, SUBLANG_ENGLISH_US};

/// Equivalent to the `MAKELANGID` macro from `winnt.h`. Narrows to `u16` —
/// the `windows-sys` language constants are typed as `u32` but real
/// [LANGIDs](https://learn.microsoft.com/en-us/windows/win32/intl/language-identifiers) are 16-bit.
const fn make_langid(primary: u32, sub: u32) -> u16 {
    ((sub << 10) | primary) as u16
}

/// Not defined in [`windows_sys`], but defined [here](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/oe/oe-codepageid-constants).
///
/// "Indicates the Unicode character set, Windows code page 1200."
const CP_UNICODE: u16 = 1200;

/// A `(LANGID, codepage)` pair from the
/// [`\VarFileInfo\Translation`](https://learn.microsoft.com/en-us/windows/win32/menurc/varfileinfo-block) table.
///
/// The type uses C-repr and 1-align.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C, align(1))]
pub struct Translation {
    pub language: u16,
    pub code_page: u16,
}

// Assert layout to match file-version information spec.
const _: () = {
    assert!(mem::size_of::<Translation>() == 4);
    assert!(mem::offset_of!(Translation, language) == 0);
    assert!(mem::offset_of!(Translation, code_page) == 2);
};

impl Default for Translation {
    /// US English (`LANG_ENGLISH | SUBLANG_ENGLISH_US` = `0x0409`) with the
    /// Unicode codepage (`1200`).
    fn default() -> Self {
        Self {
            language: make_langid(LANG_ENGLISH, SUBLANG_ENGLISH_US),
            code_page: CP_UNICODE,
        }
    }
}

impl Translation {
    /// The user's current UI language
    /// ([`GetUserDefaultUILanguage`](https://learn.microsoft.com/en-us/windows/win32/api/winnls/nf-winnls-getuserdefaultuilanguage))
    /// paired with the Unicode codepage.
    pub fn from_system() -> Self {
        Self {
            language: unsafe { GetUserDefaultUILanguage() },
            code_page: CP_UNICODE,
        }
    }
}

/// String fields from a single `\StringFileInfo\<lang-codepage>\` block.
/// Every field is optional — a PE may declare any subset. Field names
/// mirror the [`System.Diagnostics.FileVersionInfo`](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.fileversioninfo?view=net-10.0)
/// surface .NET exposes for the same `VS_VERSIONINFO` resource.
#[derive(Debug, Clone, Default)]
pub struct FileInformation {
    pub comments: Option<String>,
    pub company_name: Option<String>,
    pub file_description: Option<String>,
    pub file_version: Option<String>,
    pub internal_name: Option<String>,
    pub legal_copyright: Option<String>,
    pub legal_trademarks: Option<String>,
    pub original_filename: Option<String>,
    pub private_build: Option<String>,
    pub product_name: Option<String>,
    pub product_version: Option<String>,
    pub special_build: Option<String>,
}

/// Generate `pub fn $field(&self) -> Option<&str>` accessors that borrow
/// without cloning. Field names are reused — Rust keeps fields and methods
/// in separate namespaces (`obj.field` vs `obj.field()`).
macro_rules! string_getters {
    ($($field:ident),* $(,)?) => {
        $(
            pub fn $field(&self) -> Option<&str> {
                self.$field.as_deref()
            }
        )*
    };
}

impl FileInformation {
    string_getters!(
        comments,
        company_name,
        file_description,
        file_version,
        internal_name,
        legal_copyright,
        legal_trademarks,
        original_filename,
        private_build,
        product_name,
        product_version,
        special_build,
    );

    /// Like [`product_name`](Self::product_name) but skips the generic
    /// `"Microsoft® Windows® Operating System"` string that ships on
    /// most bundled system executables (notepad, cmd, mspaint, …) — too
    /// generic to surface as a display label.
    pub fn meaningful_product_name(&self) -> Option<&str> {
        let raw = self.product_name()?;
        if raw == "Microsoft\u{ae} Windows\u{ae} Operating System" {
            return None;
        }
        Some(raw)
    }
}

/// File-global numeric and flag fields from the root `VS_FIXEDFILEINFO`
/// record. Surface mirrors what's documented in the Win32
/// [VERSIONINFO resource](https://learn.microsoft.com/en-us/windows/win32/menurc/versioninfo-resource):
/// the four-part file/product version, the `IS_*`/`IsInfoInferred` flag
/// breakdown, and the OS/type/subtype/date metadata that callers can
/// match against the windows-sys `VOS_*` / `VFT_*` / `VFT2_*` constants.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedFileInfo {
    /// Binary version of `VS_FIXEDFILEINFO` itself (`dwStrucVersion`).
    /// Encoded high-word.low-word; `0x0001_0000` for v1.0 (the only one
    /// in the wild).
    pub struct_version: u32,

    pub file_major_part: u16,
    pub file_minor_part: u16,
    pub file_build_part: u16,
    pub file_private_part: u16,

    pub product_major_part: u16,
    pub product_minor_part: u16,
    pub product_build_part: u16,
    pub product_private_part: u16,

    /// Bitmask of which `dwFileFlags` bits the writer considered valid.
    /// `is_*` flags below are already masked through this.
    pub file_flags_mask: u32,

    pub is_debug: bool,
    pub is_patched: bool,
    pub is_pre_release: bool,
    pub is_private_build: bool,
    pub is_special_build: bool,
    /// `VS_FF_INFOINFERRED` — the resource was synthesized rather than
    /// pulled from a real `VS_VERSIONINFO` block.
    pub is_info_inferred: bool,

    /// `VOS_*` constant from `windows_sys::Win32::Storage::FileSystem`.
    pub file_os: u32,
    /// `VFT_*` constant.
    pub file_type: u32,
    /// `VFT2_*` constant. Interpretation depends on [`file_type`](Self::file_type).
    pub file_subtype: i32,
    /// 64-bit binary file date (`dwFileDateMS` << 32 | `dwFileDateLS`).
    /// Almost always zero in real files; here for completeness.
    pub file_date: u64,
}

/// All translations of a PE file's `VS_VERSIONINFO` resource plus the
/// file-global [`FixedFileInfo`].
pub struct FileVersionInfo {
    by_translation: HashMap<Translation, FileInformation>,
    fixed: Option<FixedFileInfo>,
}

impl FileVersionInfo {
    /// Read and parse every `\StringFileInfo\<lang-codepage>\` block plus
    /// the root `VS_FIXEDFILEINFO`. Wraps `GetFileVersionInfoSizeW` +
    /// `GetFileVersionInfoW` + `VerQueryValueW`.
    pub fn load(path: &Path) -> io::Result<Self> {
        let path_w = wide(path.to_string_lossy());
        let buffer = read_block(&path_w)?;
        let by_translation = read_translations(&buffer)
            .into_iter()
            .map(|t| (t, read_strings(&buffer, t)))
            .collect();
        let fixed = read_fixed(&buffer);
        Ok(Self {
            by_translation,
            fixed,
        })
    }

    /// Look up the entry for `translation`.
    ///
    /// Tries looking up the provided `translation` in the translations set first.
    /// If that fails, in order, it goes for:
    ///
    /// - System default, if available;
    /// - English, if available;
    /// - The first available translation, if any.
    ///
    /// # Arguments
    ///
    /// * `translation`: The language to seek out the file version information for.
    ///   Combine with [`Translation::default`] for English or
    ///   [`Translation::from_system`] for the system UI language.
    pub fn for_translation(&self, translation: Translation) -> Option<&FileInformation> {
        self.by_translation
            .get(&translation)
            .or_else(|| self.by_translation.get(&Translation::from_system())) // fallback on system default
            .or_else(|| self.by_translation.get(&Translation::default())) // fallback on english
            .or_else(|| self.by_translation.values().next()) // fallback on first option
    }

    /// Shortcut for [`for_translation`](Self::for_translation) with
    /// [`Translation::default`] (US English).
    pub fn english(&self) -> Option<&FileInformation> {
        self.for_translation(Translation::default())
    }

    /// Shortcut for [`for_translation`](Self::for_translation) with
    /// [`Translation::from_system`] (the user's UI language).
    pub fn system(&self) -> Option<&FileInformation> {
        self.for_translation(Translation::from_system())
    }

    /// All loaded translations.
    pub fn all(&self) -> &HashMap<Translation, FileInformation> {
        &self.by_translation
    }

    /// File-global numeric/flag info from the root `VS_FIXEDFILEINFO`.
    /// `None` when the resource omitted it or `VerQueryValueW` failed.
    pub fn fixed(&self) -> Option<&FixedFileInfo> {
        self.fixed.as_ref()
    }
}

/// Wrap any string-ish value as a null-terminated wide string.
fn wide<'a>(s: impl Into<Cow<'a, str>>) -> NullTerminated<'static, WCHAR> {
    NullTerminated::from(s.into())
}

/// Read version info block for `path_w` using [`GetFileVersionInfoW`].
///
/// If succesful, returns a "pointer to a buffer that receives the file-version
/// information".
fn read_block(path_w: &NullTerminated<'static, WCHAR>) -> io::Result<Vec<u8>> {
    // NOTE: `lpdwHandle` would always be set to 0.
    let size = unsafe { GetFileVersionInfoSizeW(path_w.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `Vec<u8>` is guaranteed to be 1-byte aligned, and thus passing
    // a pointer to the internal buffer is safe, as long as capacity is respected.
    let mut buffer = vec![0u8; size as usize];

    // NOTE: `dwHandle` is ignored.
    let ok =
        unsafe { GetFileVersionInfoW(path_w.as_ptr(), 0, size, buffer.as_mut_ptr() as *mut _) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(buffer)
}

/// Read the [`\VarFileInfo\Translation`](https://learn.microsoft.com/en-us/windows/win32/api/winver/nf-winver-verqueryvaluea)
/// resource from the file-version information buffer, obtaining all the available translations.
fn read_translations(buffer: &[u8]) -> Vec<Translation> {
    // The sub-block is a version-information value present in the file-version information buffer.
    let sub_block = wide("\\VarFileInfo\\Translation");

    let mut data: *mut c_void = ptr::null_mut();
    let mut len: u32 = 0;
    let ok = unsafe {
        VerQueryValueW(
            buffer.as_ptr() as *const c_void,
            sub_block.as_ptr(),
            &mut data,
            &mut len,
        )
    };
    if ok == 0 || data.is_null() || len == 0 {
        return Vec::new();
    }

    // Read the result into a vector of `Translation`s.
    let count = (len as usize) / mem::size_of::<Translation>();
    let pairs = unsafe { std::slice::from_raw_parts(data as *const Translation, count) };
    pairs.to_vec()
}

/// Read the file-version information `buffer` for version-information constants
/// within a `translation`.
///
/// # Arguments
///
/// * `buffer` - File-version information buffer.
/// * `translation` - Provided translation to grab the information from.
fn read_strings(buffer: &[u8], translation: Translation) -> FileInformation {
    let prefix = format!(
        "\\StringFileInfo\\{:04x}{:04x}\\",
        translation.language, translation.code_page,
    );
    let q = |name: &str| read_string(buffer, &prefix, name);
    FileInformation {
        comments: q("Comments"),
        company_name: q("CompanyName"),
        file_description: q("FileDescription"),
        file_version: q("FileVersion"),
        internal_name: q("InternalName"),
        legal_copyright: q("LegalCopyright"),
        legal_trademarks: q("LegalTrademarks"),
        original_filename: q("OriginalFilename"),
        private_build: q("PrivateBuild"),
        product_name: q("ProductName"),
        product_version: q("ProductVersion"),
        special_build: q("SpecialBuild"),
    }
}

/// Read a string entry in the file-version information `buffer`.
fn read_string(buffer: &[u8], prefix: &str, name: &str) -> Option<String> {
    let path = wide(format!("{prefix}{name}"));
    let mut data: *mut c_void = ptr::null_mut();
    let mut len: u32 = 0;
    let ok = unsafe {
        VerQueryValueW(
            buffer.as_ptr() as *const c_void,
            path.as_ptr(),
            &mut data,
            &mut len,
        )
    };
    if ok == 0 || data.is_null() || len == 0 {
        return None;
    }

    let nt = NullTerminated::<WCHAR>::try_from(data as *const WCHAR).ok()?;
    Some(String::from(nt))
}

/// Read the root `VS_FIXEDFILEINFO` from the file-version information
/// buffer via `VerQueryValueW(L"\\")` and decode it into [`FixedFileInfo`].
fn read_fixed(buffer: &[u8]) -> Option<FixedFileInfo> {
    let sub_block = wide("\\");
    let mut data: *mut c_void = ptr::null_mut();
    let mut len: u32 = 0;
    let ok = unsafe {
        VerQueryValueW(
            buffer.as_ptr() as *const c_void,
            sub_block.as_ptr(),
            &mut data,
            &mut len,
        )
    };
    if ok == 0 || data.is_null() || (len as usize) < mem::size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }

    // SAFETY: VerQueryValueW points us at a properly-aligned
    // `VS_FIXEDFILEINFO` of the size we just verified.
    let v: &VS_FIXEDFILEINFO = unsafe { &*(data as *const VS_FIXEDFILEINFO) };

    let masked = v.dwFileFlags & v.dwFileFlagsMask;
    Some(FixedFileInfo {
        struct_version: v.dwStrucVersion,

        file_major_part: hi_word(v.dwFileVersionMS),
        file_minor_part: lo_word(v.dwFileVersionMS),
        file_build_part: hi_word(v.dwFileVersionLS),
        file_private_part: lo_word(v.dwFileVersionLS),

        product_major_part: hi_word(v.dwProductVersionMS),
        product_minor_part: lo_word(v.dwProductVersionMS),
        product_build_part: hi_word(v.dwProductVersionLS),
        product_private_part: lo_word(v.dwProductVersionLS),

        file_flags_mask: v.dwFileFlagsMask,
        is_debug: masked & VS_FF_DEBUG != 0,
        is_patched: masked & VS_FF_PATCHED != 0,
        is_pre_release: masked & VS_FF_PRERELEASE != 0,
        is_private_build: masked & VS_FF_PRIVATEBUILD != 0,
        is_special_build: masked & VS_FF_SPECIALBUILD != 0,
        is_info_inferred: masked & VS_FF_INFOINFERRED != 0,

        file_os: v.dwFileOS,
        file_type: v.dwFileType,
        file_subtype: v.dwFileSubtype as i32,
        file_date: ((v.dwFileDateMS as u64) << 32) | (v.dwFileDateLS as u64),
    })
}

const fn hi_word(d: u32) -> u16 {
    (d >> 16) as u16
}

const fn lo_word(d: u32) -> u16 {
    (d & 0xFFFF) as u16
}
