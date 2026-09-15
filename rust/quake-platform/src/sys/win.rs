//! `Quake/sys_sdl_win.c`: the Win32 arm. Wide-path file access, the
//! registry / known-folder store lookups (from Ironwail), the dedicated
//! server console, DbgHelp stack traces.
//!
//! `Sys_Error` here never runs `Host_Shutdown` (the C arm did not either),
//! so [`error_core`] cannot raise and only keeps the status-returning
//! shape of the Unix arm.

use core::ffi::{c_char, c_int, c_void, CStr};
use core::mem::{offset_of, size_of, zeroed, MaybeUninit};
use core::ptr::{self, addr_of_mut};
use std::ffi::CString;
use std::io::Write as _;

use quake_c_sys::sys as c;
use quake_c_sys::{
    fileattribs_t_FA_DIRECTORY, findfile_t, qfileofs_t, steamgame_t, FILE, MAX_OSPATH,
};
use windows_sys::core::{w, GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GENERIC_READ, HANDLE,
    INVALID_HANDLE_VALUE, MAX_PATH, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
};
use windows_sys::Win32::Globalization::{MultiByteToWideChar, WideCharToMultiByte, CP_UTF8};
use windows_sys::Win32::Media::timeBeginPeriod;
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, FindClose, FindFirstFileW, FindNextFileW, GetFileAttributesW,
    ReadFile, WriteFile, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING, WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use windows_sys::Win32::System::Console::{
    AllocConsole, FreeConsole, GetNumberOfConsoleInputEvents, GetStdHandle, ReadConsoleInputA,
    INPUT_RECORD, KEY_EVENT, SHIFT_PRESSED, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    CheckRemoteDebuggerPresent, DebugBreak, OutputDebugStringA, RtlCaptureStackBackTrace,
    SymFromAddr, SymGetLineFromAddr64, SymInitialize, SymSetOptions, IMAGEHLP_LINE64,
    IMAGE_NT_HEADERS64, SYMBOL_INFO, SYMOPT_DEFERRED_LOADS, SYMOPT_FAIL_CRITICAL_ERRORS,
    SYMOPT_LOAD_LINES, SYMOPT_NO_PROMPTS, SYMOPT_UNDNAME,
};
use windows_sys::Win32::System::Environment::GetCurrentDirectoryA;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_SZ,
};
use windows_sys::Win32::System::SystemServices::IMAGE_DOS_HEADER;
use windows_sys::Win32::System::Threading::{
    EnterCriticalSection, GetCurrentProcess, GetCurrentThreadId, InitializeCriticalSection,
    LeaveCriticalSection, OpenThread, SetThreadAffinityMask, CRITICAL_SECTION,
    THREAD_QUERY_INFORMATION, THREAD_SET_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ToAscii, VkKeyScanA};
use windows_sys::Win32::UI::Shell::Common::ITEMIDLIST;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_ProgramData, FOLDERID_SavedGames, SHGetKnownFolderPath, SHOpenFolderAndSelectItems,
    SHParseDisplayName,
};

use super::{copy_into, mem_strdup, sys_error, sys_printf, ERRORTXT1, ERRORTXT2, MAX_STACK_FRAMES};

/// `common.h` -- `FS_ENT_NONE` / `FS_ENT_FILE` / `FS_ENT_DIRECTORY`.
const FS_ENT_NONE: c_int = 0;
const FS_ENT_FILE: c_int = 1;
const FS_ENT_DIRECTORY: c_int = 2;

const MAX_PATH_CH: usize = MAX_PATH as usize;

/// `static HANDLE hinput, houtput;`
static mut HINPUT: HANDLE = ptr::null_mut();
static mut HOUTPUT: HANDLE = ptr::null_mut();
/// `static char cwd[1024];`
static mut CWD: [c_char; 1024] = [0; 1024];
/// `static bool win32_DbgHelp_init_success = false;`
static mut DBGHELP_INIT_SUCCESS: bool = false;
/// `static CRITICAL_SECTION win32_DbgHelp_lock;` -- written by
/// `InitializeCriticalSection` in [`init`] and never touched before it
/// (`Sys_StackTrace` bails on `DBGHELP_INIT_SUCCESS` first).
static mut DBGHELP_LOCK: MaybeUninit<CRITICAL_SECTION> = MaybeUninit::uninit();
/// `static intptr_t win32_Dwarf_offset = 0;`
static mut DWARF_OFFSET: isize = 0;

/// `bool Sys_IsInDebugger`'s `isDedicated` reads and writes.
unsafe fn is_dedicated() -> bool {
    // SAFETY: caller contract -- main thread or a worker reading a flag
    // `main` set before the threads existed.
    unsafe { quake_c_sys::isDedicated }
}

/// `errno` on the MSVC CRT.
pub(super) fn errno() -> c_int {
    // SAFETY: `_errno` returns the calling thread's errno slot.
    unsafe { *c::_errno() }
}

/// C: `int Sys_fseek (FILE *file, qfileofs_t ofs, int origin)`
///
/// # Safety
/// `file` is an open stream.
pub unsafe fn fseek(file: *mut FILE, ofs: qfileofs_t, origin: c_int) -> c_int {
    // SAFETY: caller contract.
    unsafe { c::_fseeki64(file, ofs, origin) }
}

/// C: `qfileofs_t Sys_ftell (FILE *file)`
///
/// # Safety
/// `file` is an open stream.
pub unsafe fn ftell(file: *mut FILE) -> qfileofs_t {
    // SAFETY: caller contract.
    unsafe { c::_ftelli64(file) }
}

/// `static void UTF8ToWideString (const char *src, wchar_t *dst, size_t maxchars)`
///
/// # Safety
/// `src` is a NUL-terminated string.
unsafe fn utf8_to_wide(src: *const c_char, dst: &mut [u16]) {
    // SAFETY: caller contract; `dst` is sized by the slice.
    let n = unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            0,
            src.cast(),
            -1,
            dst.as_mut_ptr(),
            dst.len() as c_int,
        )
    };
    if n == 0 {
        // SAFETY: plain thread-local read.
        sys_error(&format!("MultiByteToWideChar failed: {}", unsafe {
            GetLastError()
        }));
    }
}

/// `static void WideStringToUTF8 (const wchar_t *src, char *dst, size_t maxbytes)`
///
/// # Safety
/// `src` is a NUL-terminated wide string, `dst` writable for `maxbytes`.
unsafe fn wide_to_utf8(src: *const u16, dst: *mut c_char, maxbytes: usize) {
    // SAFETY: caller contract.
    let n = unsafe {
        WideCharToMultiByte(
            CP_UTF8,
            0,
            src,
            -1,
            dst.cast(),
            maxbytes as c_int,
            ptr::null(),
            ptr::null_mut(),
        )
    };
    if n == 0 {
        // SAFETY: plain thread-local read.
        sys_error(&format!("WideCharToMultiByte failed: {}", unsafe {
            GetLastError()
        }));
    }
}

/// A NUL-terminated `[u16]` buffer up to (not including) its terminator.
fn wide_str(buf: &[u16]) -> &[u16] {
    let len = buf.iter().position(|&w| w == 0).unwrap_or(buf.len());
    &buf[..len]
}

/// `%s` of a C string: the bytes, lossily decoded for a Rust `format!`.
///
/// # Safety
/// `s` is a NUL-terminated string.
unsafe fn lossy(s: *const c_char) -> String {
    // SAFETY: caller contract.
    unsafe { CStr::from_ptr(s).to_string_lossy().into_owned() }
}

/// C: `int Sys_FileType (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string.
pub unsafe fn file_type(path: *const c_char) -> c_int {
    let mut wpath = [0u16; MAX_PATH_CH];

    // SAFETY: caller contract.
    let result = unsafe {
        utf8_to_wide(path, &mut wpath);
        GetFileAttributesW(wpath.as_ptr())
    };

    if result == INVALID_FILE_ATTRIBUTES {
        return FS_ENT_NONE;
    }
    if result & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return FS_ENT_DIRECTORY;
    }

    FS_ENT_FILE
}

/// C: `FILE *Sys_fopen (const char *path, const char *mode)`
///
/// # Safety
/// `path` and `mode` are NUL-terminated strings.
pub unsafe fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    let mut wpath = [0u16; MAX_PATH_CH];
    let mut wmode = [0u16; 8];

    // SAFETY: caller contract.
    let mode_bytes = unsafe { CStr::from_ptr(mode).to_bytes() };
    for (i, &ch) in mode_bytes.iter().enumerate() {
        if i == wmode.len() - 1 {
            sys_error(&format!(
                "Sys_fopen: invalid mode \"{}\"",
                String::from_utf8_lossy(mode_bytes)
            ));
        }
        wmode[i] = ch as c_char as u16;
    }

    // SAFETY: caller contract; every index below stays inside `wpath`
    // because the conversion NUL-terminated it.
    unsafe {
        utf8_to_wide(path, &mut wpath);

        if wpath[0] != 0 && mode_bytes.contains(&b'w') {
            // create directory structure
            let mut i = 1;
            while wpath[i] != 0 {
                if wpath[i] != u16::from(b'\\') && wpath[i] != u16::from(b'/') {
                    i += 1;
                    continue;
                }

                // keep the trailing slash
                let wc = wpath[i + 1];
                wpath[i + 1] = 0;

                let attr = GetFileAttributesW(wpath.as_ptr());
                if attr != INVALID_FILE_ATTRIBUTES && attr & FILE_ATTRIBUTE_DIRECTORY == 0 {
                    return ptr::null_mut();
                }

                if attr == INVALID_FILE_ATTRIBUTES
                    && CreateDirectoryW(wpath.as_ptr(), ptr::null()) == 0
                {
                    let err = GetLastError();
                    if err != ERROR_ALREADY_EXISTS {
                        return ptr::null_mut();
                    }
                }

                wpath[i + 1] = wc;
                i += 1;
            }
        }

        c::_wfopen(wpath.as_ptr(), wmode.as_ptr())
    }
}

/*
==============================================================================
STORE INSTALL LOCATIONS (from Ironwail)
==============================================================================
*/

/// `static qboolean Sys_GetRegistryString (HKEY root, const wchar_t *dir, const wchar_t *keyname, char *out, size_t maxchars)`
///
/// # Safety
/// `dir`/`keyname` are NUL-terminated wide strings, `out` writable for
/// `maxchars` bytes.
unsafe fn registry_string(
    root: HKEY,
    dir: PCWSTR,
    keyname: PCWSTR,
    out: *mut c_char,
    maxchars: usize,
) -> bool {
    let mut wpath = [0u16; MAX_PATH_CH + 1];
    let mut key: HKEY = ptr::null_mut();
    let mut size: u32 = 0;
    let mut ty: u32 = 0;

    if maxchars == 0 {
        return false;
    }

    // SAFETY: caller contract; the second query is bounded by the size the
    // first one reported, which was checked against `wpath` less its NUL.
    unsafe {
        *out = 0;

        if RegOpenKeyExW(root, dir, 0, KEY_READ, &mut key) != ERROR_SUCCESS {
            return false;
        }

        // Note: string might not contain a terminating null character
        // https://docs.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regqueryvalueexw#remarks

        let err = RegQueryValueExW(
            key,
            keyname,
            ptr::null(),
            &mut ty,
            ptr::null_mut(),
            &mut size,
        );
        if err != ERROR_SUCCESS
            || ty != REG_SZ
            || size as usize > size_of::<[u16; MAX_PATH_CH + 1]>() - size_of::<u16>()
        {
            RegCloseKey(key);
            return false;
        }

        let err = RegQueryValueExW(
            key,
            keyname,
            ptr::null(),
            &mut ty,
            wpath.as_mut_ptr().cast(),
            &mut size,
        );
        RegCloseKey(key);
        if err != ERROR_SUCCESS || ty != REG_SZ {
            return false;
        }

        wpath[size as usize / size_of::<u16>()] = 0;

        if WideCharToMultiByte(
            CP_UTF8,
            0,
            wpath.as_ptr(),
            -1,
            out.cast(),
            maxchars as c_int,
            ptr::null(),
            ptr::null_mut(),
        ) != 0
        {
            return true;
        }
        *out = 0;
    }
    false
}

/// C: `qboolean Sys_GetSteamDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
pub unsafe fn get_steam_dir(path: *mut c_char, pathsize: usize) -> bool {
    // SAFETY: caller contract.
    unsafe {
        registry_string(
            HKEY_CURRENT_USER,
            w!("Software\\Valve\\Steam"),
            w!("SteamPath"),
            path,
            pathsize,
        )
    }
}

/// `static qboolean Sys_StripTrailingSlashes (char *path)`
///
/// # Safety
/// `path` is a NUL-terminated, writable string.
unsafe fn strip_trailing_slashes(path: *mut c_char) -> bool {
    // SAFETY: caller contract.
    unsafe {
        let mut i = CStr::from_ptr(path).to_bytes().len();
        while i > 0 && (*path.add(i - 1) == b'\\' as c_char || *path.add(i - 1) == b'/' as c_char) {
            i -= 1;
            *path.add(i) = 0;
        }
        i > 0
    }
}

/// C: `qboolean Sys_GetGOGQuakeDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
pub unsafe fn get_gog_quake_dir(path: *mut c_char, pathsize: usize) -> bool {
    // SAFETY: caller contract.
    unsafe {
        if !registry_string(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Wow6432Node\\GOG.com\\Games\\1435828198"),
            w!("path"),
            path,
            pathsize,
        ) {
            return false;
        }

        strip_trailing_slashes(path)
    }
}

/// C: `qboolean Sys_GetGOGQuakeEnhancedDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
pub unsafe fn get_gog_quake_enhanced_dir(path: *mut c_char, pathsize: usize) -> bool {
    // SAFETY: caller contract.
    unsafe {
        if !registry_string(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Wow6432Node\\GOG.com\\Games\\1739637082"),
            w!("path"),
            path,
            pathsize,
        ) {
            return false;
        }

        strip_trailing_slashes(path)
    }
}

/// `FAILED (hr)`.
fn failed(hr: HRESULT) -> bool {
    hr < 0
}

// https://github.com/libsdl-org/SDL/blob/120c76c84bbce4c1bfed4e9eb74e10678bd83120/src/core/windows/SDL_windows.c#L88-L99
/// `static HRESULT Sys_InitCOM (void)`
fn init_com() -> HRESULT {
    // SAFETY: plain COM calls with no pointers involved.
    let mut hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if hr == RPC_E_CHANGED_MODE {
        // SAFETY: as above.
        hr = unsafe { CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) };
    }

    /* S_FALSE means success, but someone else already initialized. */
    /* You still need to call CoUninitialize in this case! */
    if hr == S_FALSE {
        return S_OK;
    }

    hr
}

/// `static qboolean Sys_GetKnownFolder (const KNOWNFOLDERID *base, const char *subdir, char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
unsafe fn known_folder(base: &GUID, subdir: &CStr, path: *mut c_char, pathsize: usize) -> bool {
    let mut wpath: *mut u16 = ptr::null_mut();

    if failed(init_com()) {
        return false;
    }

    // SAFETY: caller contract; the shell-owned wide path is released after
    // the copy.
    unsafe {
        let hr = SHGetKnownFolderPath(base, 0, ptr::null_mut(), &mut wpath);
        if failed(hr) {
            CoUninitialize();
            return false;
        }

        let ret = WideCharToMultiByte(
            CP_UTF8,
            0,
            wpath,
            -1,
            path.cast(),
            pathsize as c_int,
            ptr::null(),
            ptr::null_mut(),
        ) != 0;
        CoTaskMemFree(wpath.cast());
        CoUninitialize();

        ret && c::q_strlcat(path, subdir.as_ptr(), pathsize) < pathsize
    }
}

/// C: `qboolean Sys_Explore (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string. Main thread.
pub unsafe fn explore(path: *const c_char) -> bool {
    let mut wpath = [0u16; MAX_PATH_CH];
    let mut file: *mut ITEMIDLIST = ptr::null_mut();
    let mut folder: *mut ITEMIDLIST = ptr::null_mut();
    let mut sfgaof: u32 = 0;
    let mut result = false;

    const BACKSLASH: u16 = b'\\' as u16;
    const SLASH: u16 = b'/' as u16;
    const DOT: u16 = b'.' as u16;

    // SAFETY: caller contract.
    unsafe {
        if file_type(path) == FS_ENT_NONE {
            sys_printf(&format!("Sys_Explore: '{}' not found.\n", lossy(path)));
            return false;
        }

        utf8_to_wide(path, &mut wpath);
    }

    // Canonicalize path (replace forward slashes with backslashes, handle "." and "..")
    let mut src: isize = 0;
    let mut dst: isize = 0;
    let mut slash: isize = -1;
    while wpath[src as usize] != 0 {
        if wpath[src as usize] == SLASH {
            wpath[src as usize] = BACKSLASH;
        }

        if wpath[src as usize] == BACKSLASH {
            if slash != -1 {
                // Handle "\..\" by going up a level
                if src == slash + 3
                    && wpath[slash as usize + 1] == DOT
                    && wpath[slash as usize + 2] == DOT
                {
                    // We've already written "\..", and dst is now pointing one character past that.
                    // Rewind dst by 4 (the character before the '\') and look for the previous '\'.
                    dst -= 4;
                    while dst >= 0 {
                        if wpath[dst as usize] == BACKSLASH {
                            break;
                        }
                        dst -= 1;
                    }
                    if dst < 0 {
                        // SAFETY: caller contract.
                        sys_printf(&format!("Sys_Explore: malformed path '{}'.\n", unsafe {
                            lossy(path)
                        }));
                        return false;
                    }
                }
                // Ignore "\.\"
                else if src == slash + 2 && wpath[slash as usize + 1] == DOT {
                    dst -= 2;
                }
            }
            slash = src;
        }

        wpath[dst as usize] = wpath[src as usize];
        dst += 1;
        src += 1;
    }
    wpath[dst as usize] = 0;

    // If the cleaned up path is of a different length, we need to find the new index of the last slash character.
    if src != dst {
        src = 0;
        slash = -1;
        while wpath[src as usize] != 0 {
            if wpath[src as usize] == BACKSLASH {
                slash = src;
            }
            src += 1;
        }
    }

    if slash == -1 {
        // SAFETY: caller contract.
        sys_printf(&format!("Sys_Explore: no slash in '{}'.\n", unsafe {
            lossy(path)
        }));
        return false;
    }
    let slash = slash as usize;

    let hr = init_com();
    if failed(hr) {
        sys_printf(&format!(
            "Sys_Explore: failed to initialize COM (0x{:08x}).\n",
            hr as u32
        ));
        return false;
    }

    // SAFETY: `wpath` stays NUL-terminated (the terminator at `dst` is
    // never overwritten); the ID lists are shell-owned and released below.
    unsafe {
        wpath[slash] = 0;
        let hr = SHParseDisplayName(wpath.as_ptr(), ptr::null_mut(), &mut folder, 0, &mut sfgaof);
        if failed(hr) {
            sys_printf(&format!(
                "Sys_Explore: SHParseDisplayName failed (0x{:08x}) for '{}'.\n",
                hr as u32,
                String::from_utf16_lossy(wide_str(&wpath))
            ));
            CoUninitialize();
            return false;
        }

        wpath[slash] = BACKSLASH;
        let hr = SHParseDisplayName(wpath.as_ptr(), ptr::null_mut(), &mut file, 0, &mut sfgaof);
        if failed(hr) {
            sys_printf(&format!(
                "Sys_Explore: SHParseDisplayName failed (0x{:08x}) for '{}'.\n",
                hr as u32,
                String::from_utf16_lossy(wide_str(&wpath))
            ));
            CoTaskMemFree(folder.cast());
            CoUninitialize();
            return false;
        }

        let hr = SHOpenFolderAndSelectItems(
            folder,
            1,
            ptr::addr_of!(file).cast::<*const ITEMIDLIST>(),
            0,
        );
        if !failed(hr) {
            result = true;
        } else {
            sys_printf(&format!(
                "Sys_Explore: SHOpenFolderAndSelectItems failed (0x{:08x}) for '{}'.\n",
                hr as u32,
                String::from_utf16_lossy(wide_str(&wpath))
            ));
        }

        CoTaskMemFree(file.cast());
        CoTaskMemFree(folder.cast());
        CoUninitialize();
    }

    result
}

/// C: `qboolean Sys_GetSteamAPILibraryPath (char *path, size_t pathsize, const steamgame_t *game)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes; `game` is a valid entry.
pub unsafe fn get_steam_api_library_path(
    path: *mut c_char,
    pathsize: usize,
    game: *const steamgame_t,
) -> bool {
    #[cfg(target_pointer_width = "64")]
    {
        let mut installdir = [0 as c_char; MAX_OSPATH];
        // SAFETY: caller contract; `installdir` is NUL-terminated by
        // `Steam_ResolvePath` on success.
        unsafe {
            if !c::Steam_ResolvePath(installdir.as_mut_ptr(), installdir.len(), game) {
                return false;
            }
            let mut s = CStr::from_ptr(installdir.as_ptr()).to_bytes().to_vec();
            s.extend_from_slice(b"/rerelease/steam_api64.dll");
            copy_into(path, pathsize, &s)
        }
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        let _ = (path, pathsize, game);
        false
    }
}

/// C: `qboolean Sys_GetNightdiveUserDir (char *path, size_t pathsize, const char *steamlibrary)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
pub unsafe fn get_nightdive_user_dir(
    path: *mut c_char,
    pathsize: usize,
    _steamlibrary: *const c_char,
) -> bool {
    // same location for Steam and GOG on Windows
    // SAFETY: caller contract.
    unsafe {
        known_folder(
            &FOLDERID_SavedGames,
            c"\\Nightdive Studios\\Quake",
            path,
            pathsize,
        )
    }
}

/// C: `qboolean Sys_GetEGSManifestDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
pub unsafe fn get_egs_manifest_dir(path: *mut c_char, pathsize: usize) -> bool {
    // SAFETY: caller contract.
    unsafe {
        known_folder(
            &FOLDERID_ProgramData,
            c"\\Epic\\EpicGamesLauncher\\Data\\Manifests",
            path,
            pathsize,
        )
    }
}

/// C: `const char *Sys_GetEGSLauncherData (void)` -- a `Mem_Alloc`ed UTF-8
/// buffer the caller `Mem_Free`s, or null.
///
/// # Safety
/// Main thread.
pub unsafe fn get_egs_launcher_data() -> *const c_char {
    let mut path = [0 as c_char; MAX_OSPATH];

    // SAFETY: caller contract; every buffer below is sized by the file
    // length that was just measured, and freed on each failure path.
    unsafe {
        if !known_folder(
            &FOLDERID_ProgramData,
            c"\\Epic\\UnrealEngineLauncher\\LauncherInstalled.dat",
            path.as_mut_ptr(),
            path.len(),
        ) {
            return ptr::null();
        }

        let file = fopen(path.as_ptr(), c"rb".as_ptr());
        if file.is_null() {
            return ptr::null();
        }

        c::_fseeki64(file, 0, super::handles::SEEK_END);
        let filesize = c::_ftelli64(file);
        c::_fseeki64(file, 0, super::handles::SEEK_SET);

        if !(2..=(1 << 30)).contains(&filesize) {
            c::fclose(file);
            return ptr::null();
        }

        let size = filesize as c_int;
        let mut buf = c::Mem_Alloc(size as usize + 1).cast::<c_char>();
        if buf.is_null() {
            c::fclose(file);
            return ptr::null();
        }

        if c::fread(buf.cast(), size as usize, 1, file) != 1 {
            c::Mem_Free(buf.cast());
            c::fclose(file);
            return ptr::null();
        }
        *buf.add(size as usize) = 0;

        c::fclose(file);

        // Convert to UTF-8 if needed
        if *buf.cast::<u8>() == 0xff && *buf.cast::<u8>().add(1) == 0xfe
        // UTF-16 little-endian byte order mark
        {
            let wide = buf.add(2).cast::<u16>();
            let wide_len = size / 2 - 1;

            let size8 = WideCharToMultiByte(
                CP_UTF8,
                0,
                wide,
                wide_len,
                ptr::null_mut(),
                0,
                ptr::null(),
                ptr::null_mut(),
            );
            if size8 <= 0 {
                c::Mem_Free(buf.cast());
                return ptr::null();
            }

            let buf8 = c::Mem_Alloc(size8 as usize + 1).cast::<c_char>();
            if buf8.is_null() {
                c::Mem_Free(buf.cast());
                return ptr::null();
            }

            if WideCharToMultiByte(
                CP_UTF8,
                0,
                wide,
                wide_len,
                buf8.cast(),
                size8,
                ptr::null(),
                ptr::null_mut(),
            ) != size8
            {
                c::Mem_Free(buf8.cast());
                c::Mem_Free(buf.cast());
                return ptr::null();
            }
            *buf8.add(size8 as usize) = 0;

            c::Mem_Free(buf.cast());
            buf = buf8;
        }

        buf
    }
}

/*
==============================================================================
DIRECTORY ENUMERATION (from Ironwail)
==============================================================================
*/

/// `winfindfile_t`.
#[repr(C)]
struct WinFindFile {
    base: findfile_t,
    data: WIN32_FIND_DATAW,
    handle: HANDLE,
}

/// `static void Sys_FillFindData (winfindfile_t *find)`
///
/// # Safety
/// `find` is a live entry.
unsafe fn fill_find_data(find: *mut WinFindFile) {
    // SAFETY: caller contract; `cFileName` is NUL-terminated by the API.
    unsafe {
        wide_to_utf8(
            (*find).data.cFileName.as_ptr(),
            (*find).base.name.as_mut_ptr(),
            (*find).base.name.len(),
        );
        (*find).base.attribs = 0;
        if (*find).data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            (*find).base.attribs |= fileattribs_t_FA_DIRECTORY;
        }
    }
}

/// C: `findfile_t *Sys_FindFirst (const char *dir, const char *ext)`
///
/// # Safety
/// `dir` is a NUL-terminated string; `ext` NUL-terminated or null.
pub unsafe fn find_first(dir: *const c_char, ext: *const c_char) -> *mut findfile_t {
    let mut pattern = [0 as c_char; MAX_OSPATH];
    let mut wpattern = [0u16; MAX_PATH_CH];
    // SAFETY: zero is a valid `WIN32_FIND_DATAW`.
    let mut data: WIN32_FIND_DATAW = unsafe { zeroed() };

    // SAFETY: caller contract.
    unsafe {
        let ext = if ext.is_null() {
            &b"*"[..]
        } else {
            let ext = CStr::from_ptr(ext).to_bytes();
            ext.strip_prefix(b".").unwrap_or(ext)
        };
        let mut s = CStr::from_ptr(dir).to_bytes().to_vec();
        s.extend_from_slice(b"/*.");
        s.extend_from_slice(ext);
        copy_into(pattern.as_mut_ptr(), pattern.len(), &s);

        utf8_to_wide(pattern.as_ptr(), &mut wpattern);
        let handle = FindFirstFileW(wpattern.as_ptr(), &mut data);

        if handle == INVALID_HANDLE_VALUE {
            return ptr::null_mut();
        }

        let ret = c::Mem_Alloc(size_of::<WinFindFile>()).cast::<WinFindFile>();
        if ret.is_null() {
            sys_error("Sys_FindFirst: out of memory");
        }
        (*ret).handle = handle;
        (*ret).data = data;
        fill_find_data(ret);

        ret.cast()
    }
}

/// C: `findfile_t *Sys_FindNext (findfile_t *find)`
///
/// # Safety
/// `find` came from [`find_first`] and has not been closed.
pub unsafe fn find_next(find: *mut findfile_t) -> *mut findfile_t {
    let wfind = find.cast::<WinFindFile>();
    // SAFETY: caller contract -- `findfile_t` is the first member.
    unsafe {
        if FindNextFileW((*wfind).handle, addr_of_mut!((*wfind).data)) == 0 {
            find_close(find);
            return ptr::null_mut();
        }
        fill_find_data(wfind);
    }
    find
}

/// C: `void Sys_FindClose (findfile_t *find)`
///
/// # Safety
/// `find` came from [`find_first`] and has not been closed, or is null.
pub unsafe fn find_close(find: *mut findfile_t) {
    if !find.is_null() {
        let wfind = find.cast::<WinFindFile>();
        // SAFETY: caller contract.
        unsafe {
            FindClose((*wfind).handle);
            c::Mem_Free(wfind.cast());
        }
    }
}

/// `static void Sys_GetBasedir (char *argv0, char *dst, size_t dstsize)`
///
/// # Safety
/// `dst` is writable for `dstsize` bytes.
unsafe fn get_basedir(dst: *mut c_char, dstsize: usize) {
    // SAFETY: caller contract.
    unsafe {
        let rc = GetCurrentDirectoryA(dstsize as u32, dst.cast()) as usize;
        if rc == 0 || rc > dstsize {
            sys_error("Couldn't determine current directory");
        }

        let mut tmp = dst;
        while *tmp != 0 {
            tmp = tmp.add(1);
        }
        while *tmp == 0 && tmp != dst {
            tmp = tmp.sub(1);
            if tmp != dst && (*tmp == b'/' as c_char || *tmp == b'\\' as c_char) {
                *tmp = 0;
            }
        }
    }
}

/// `static void Sys_SetTimerResolution (void)`
fn set_timer_resolution() {
    /* Set OS timer resolution to 1ms.
       Works around buffer underruns with directsound and SDL2, but also
       will make Sleep()/SDL_Dleay() accurate to 1ms which should help framerate
       stability.
    */
    // SAFETY: plain winmm call.
    unsafe { timeBeginPeriod(1) };
}

/// C: `void Sys_Init (void)`
///
/// # Safety
/// Once, from `main`, after `host_parms` is set.
pub unsafe fn init() {
    // SAFETY: caller contract -- single-threaded init; the statics are
    // only written here.
    unsafe {
        super::handles::file_init();

        set_timer_resolution();

        let cwd = addr_of_mut!(CWD).cast::<c_char>();
        cwd.write_bytes(0, 1024);
        get_basedir(cwd, 1024);
        (*quake_c_sys::host::host_parms).basedir = cwd;

        /* userdirs not really necessary for windows guys.
         * can be done if necessary, though... */
        (*quake_c_sys::host::host_parms).userdir = (*quake_c_sys::host::host_parms).basedir; /* code elsewhere relies on this ! */

        if is_dedicated() {
            if AllocConsole() == 0 {
                quake_c_sys::isDedicated = false; /* so that we have a graphical error dialog */
                sys_error("Couldn't create dedicated server console");
            }

            HINPUT = GetStdHandle(STD_INPUT_HANDLE);
            HOUTPUT = GetStdHandle(STD_OUTPUT_HANDLE);
        }

        super::common::init_counter_freq();

        // DbgHelp one-time initialization:
        let process = GetCurrentProcess();

        SymSetOptions(
            SYMOPT_LOAD_LINES
                | SYMOPT_UNDNAME
                | SYMOPT_FAIL_CRITICAL_ERRORS
                | SYMOPT_NO_PROMPTS
                | SYMOPT_DEFERRED_LOADS,
        );

        if SymInitialize(process, ptr::null(), 1) != 0 {
            DBGHELP_INIT_SUCCESS = true;
        }

        InitializeCriticalSection(addr_of_mut!(DBGHELP_LOCK).cast());

        // MSYS2 DWARF debug info is only usable if the stack addresses are offseted
        // by win32_Dwarf_offset
        // We need to look for the executable to look for the original ImageBase in the binary
        // ifself BEFORE it got patched in the loaded image...
        // this is the address we would get by : objdump -p vkqr-engine.exe | grep ImageBase
        let mut path = [0u16; MAX_OSPATH];

        if GetModuleFileNameW(ptr::null_mut(), path.as_mut_ptr(), MAX_OSPATH as u32) != 0 {
            let self_executable = CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            );

            if self_executable != INVALID_HANDLE_VALUE {
                let mut pe_header = [0u8; 4096];
                let mut read: u32 = 0;
                if ReadFile(
                    self_executable,
                    pe_header.as_mut_ptr(),
                    pe_header.len() as u32,
                    &mut read,
                    ptr::null_mut(),
                ) != 0
                    && read as usize >= size_of::<IMAGE_DOS_HEADER>()
                {
                    // the headers are read unaligned out of the byte buffer;
                    // the NT header offset is bounds-checked against what
                    // was read (C trusted it)
                    let e_lfanew = ptr::read_unaligned(
                        pe_header
                            .as_ptr()
                            .add(offset_of!(IMAGE_DOS_HEADER, e_lfanew))
                            .cast::<i32>(),
                    );
                    if e_lfanew >= 0
                        && e_lfanew as usize + size_of::<IMAGE_NT_HEADERS64>() <= read as usize
                    {
                        let nt = pe_header
                            .as_ptr()
                            .add(e_lfanew as usize)
                            .cast::<IMAGE_NT_HEADERS64>();
                        let image_base =
                            ptr::read_unaligned(ptr::addr_of!((*nt).OptionalHeader.ImageBase))
                                as usize;

                        let load_base = GetModuleHandleW(ptr::null()) as usize;

                        if image_base >= load_base {
                            DWARF_OFFSET = (image_base - load_base) as isize;
                        } else {
                            DWARF_OFFSET = -((load_base - image_base) as isize);
                        }
                    }
                }
                CloseHandle(self_executable);
            }
        }
    }
}

/// C: `void Sys_mkdir (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string.
pub unsafe fn mkdir(path: *const c_char) {
    let mut wpath = [0u16; MAX_PATH_CH];

    // SAFETY: caller contract.
    unsafe {
        utf8_to_wide(path, &mut wpath);
        if CreateDirectoryW(wpath.as_ptr(), ptr::null()) != 0 {
            return;
        }
        if GetLastError() != ERROR_ALREADY_EXISTS {
            sys_error(&format!("Unable to create directory {}", lossy(path)));
        }
    }
}

/// `WriteFile (houtput, buf, len, &dummy, NULL)`.
///
/// # Safety
/// After `Sys_Init` with `isDedicated` (the handle is set), or from a
/// worker thread of such a process.
unsafe fn console_write(bytes: &[u8]) {
    let mut dummy: u32 = 0;
    // SAFETY: caller contract; `bytes` is readable for its length.
    unsafe {
        WriteFile(
            HOUTPUT,
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut dummy,
            ptr::null_mut(),
        );
    }
}

/// C: `void Sys_Error (const char *error, ...)` after the C wrapper
/// formatted `text` (a `Mem_Alloc`ed buffer this function frees).
///
/// # Safety
/// `text` is a `Mem_Alloc`ed NUL-terminated string.
pub unsafe fn error_core(text: *mut c_char) -> c_int {
    // SAFETY: caller contract.
    unsafe {
        let full = super::error_prologue(text);
        let full_c = CString::new(full).unwrap_or_default();
        let worker = c::Tasks_IsWorker();

        if worker || is_dedicated() {
            console_write(ERRORTXT1.to_bytes());
        }
        /* SDL will put these into its own stderr log,
        so print to stderr even in graphical mode. */
        c::fputs(ERRORTXT1.as_ptr(), c::stdout_stream());
        c::fputs(ERRORTXT2.as_ptr(), c::stdout_stream());

        let mut with_newlines = full_c.as_bytes().to_vec();
        with_newlines.extend_from_slice(b"\n\n");
        print_text(CString::new(with_newlines).unwrap_or_default().as_ptr());

        if !worker && !is_dedicated() && !is_in_debugger() {
            crate::pl::error_dialog(full_c.as_ptr());
        } else {
            console_write(ERRORTXT2.to_bytes());
            console_write(full_c.as_bytes());
            console_write(b"\r\n");
            super::sleep(3000); /* show the console 3 more seconds */
        }

        c::Mem_Free(text.cast());

        c::exit(1)
    }
}

/// C: `void Sys_Printf (const char *fmt, ...)` after formatting.
///
/// # Safety
/// `text` is a NUL-terminated string.
pub unsafe fn print_text(text: *const c_char) {
    // SAFETY: caller contract.
    unsafe {
        if c::Tasks_IsWorker() || is_dedicated() {
            console_write(CStr::from_ptr(text).to_bytes());
        } else {
            /* SDL will put these into its own stdout log,
            so print to stdout even in graphical mode. */
            c::fputs(text, c::stdout_stream());
            OutputDebugStringA(text.cast());
        }
    }
}

/// C: `void Sys_Quit (void)` -- the status-returning core: a non-zero
/// status is the `Host_Shutdown` raise for the C wrapper to re-raise.
///
/// # Safety
/// Main thread.
pub unsafe fn quit_core() -> c_int {
    // SAFETY: caller contract.
    unsafe {
        let r = c::SysGlue_HostShutdown();
        if r != 0 {
            return r;
        }

        if is_dedicated() {
            FreeConsole();
        }

        c::exit(0)
    }
}

/// C: `const char *Sys_ConsoleInput (void)`
///
/// # Safety
/// Main thread of a dedicated server (after `Sys_Init` set the handles).
pub unsafe fn console_input() -> *const c_char {
    static mut CON_TEXT: [c_char; 256] = [0; 256];
    static mut TEXTLEN: usize = 0;
    static mut CONSOLE_UNQUERYABLE: bool = false;

    // SAFETY: caller contract -- main thread only, so the statics are not
    // shared; `rec` is a single record the API fills.
    unsafe {
        if CONSOLE_UNQUERYABLE {
            return ptr::null();
        }

        let mut rec: INPUT_RECORD = zeroed();
        let mut numread: u32 = 0;
        let mut numevents: u32 = 0;

        loop {
            if GetNumberOfConsoleInputEvents(HINPUT, &mut numevents) == 0 {
                /* Some automated/sandboxed launch environments (the ones the
                Phase 7 soak cells run under) hand the process a console handle
                AllocConsole() accepts but this call cannot query. That is not
                worth aborting over -- it only means this process has no
                readable console input -- so degrade to "no input, ever"
                instead of Sys_Error, and say so once rather than silently. */
                CONSOLE_UNQUERYABLE = true;
                c::Con_SafePrintf(
                    c"Sys_ConsoleInput: console input unavailable (GetNumberOfConsoleInputEvents failed); disabling console input\n"
                        .as_ptr(),
                );
                return ptr::null();
            }

            if numevents == 0 {
                break;
            }

            if ReadConsoleInputA(HINPUT, &mut rec, 1, &mut numread) == 0 {
                sys_error("Error reading console input");
            }

            if numread != 1 {
                sys_error("Couldn't read console input");
            }

            if u32::from(rec.EventType) == KEY_EVENT {
                let key = rec.Event.KeyEvent;
                if key.bKeyDown == 1 {
                    let mut ch = c_int::from(key.uChar.AsciiChar);
                    if ch != 0 && key.dwControlKeyState & SHIFT_PRESSED != 0 {
                        let mut keyboard = [0u8; 256];
                        let mut output: u16 = 0;
                        keyboard[SHIFT_PRESSED as usize] = 0x80;
                        if ToAscii(
                            VkKeyScanA(ch as i8) as i32 as u32,
                            0,
                            keyboard.as_ptr(),
                            &mut output,
                            0,
                        ) == 1
                        {
                            ch = c_int::from(output);
                        }
                    }

                    match ch {
                        0x0d => {
                            console_write(b"\r\n");

                            if TEXTLEN != 0 {
                                CON_TEXT[TEXTLEN] = 0;
                                TEXTLEN = 0;
                                return addr_of_mut!(CON_TEXT).cast();
                            }
                        }

                        0x08 => {
                            console_write(b"\x08 \x08");
                            TEXTLEN = TEXTLEN.saturating_sub(1);
                        }

                        _ => {
                            if ch >= c_int::from(b' ') {
                                console_write(&ch.to_ne_bytes()[..1]);
                                CON_TEXT[TEXTLEN] = ch as c_char;
                                TEXTLEN = (TEXTLEN + 1) & 0xff;
                            }
                        }
                    }
                }
            }
        }
    }
    ptr::null()
}

/// C: `bool Sys_PinCurrentThread (int core_index)`
///
/// # Safety
/// Any thread.
pub unsafe fn pin_current_thread(core_index: c_int) -> bool {
    // valid for both MSVC and MINGW
    //  Open the thread with necessary access rights
    // SAFETY: plain Win32 calls on the calling thread's own handle.
    unsafe {
        let thread_id = GetCurrentThreadId();
        let thread_access = OpenThread(
            THREAD_SET_INFORMATION | THREAD_QUERY_INFORMATION,
            0,
            thread_id,
        );
        if thread_access.is_null() {
            return false;
        }

        // Define the processor affinity mask, fold beyond DWORD_PTR bit size...
        // should allow setting to 64 different cores on 64 bits, should be enough for anybody....
        let mask: usize = 1usize << (core_index as usize % usize::BITS as usize);

        // Set the thread affinity
        let prev_affinity_mask = SetThreadAffinityMask(thread_access, mask);
        if prev_affinity_mask == 0 {
            CloseHandle(thread_access);
            return false;
        }

        // Close the thread handle
        CloseHandle(thread_access);
    }

    true
}

/// `SYMBOL_INFO` with the `MAX_OSPATH + 1` name bytes that overlay its
/// trailing `Name[1]` in the C `symbol_buffer`.
#[repr(C)]
struct SymbolBuffer {
    info: SYMBOL_INFO,
    _name: [u8; MAX_OSPATH + 1],
}

/// `FIND_LAST_DIRSEP`: the file name after the last `/` or `\`.
fn short_file_name(file_name: &[u8]) -> &[u8] {
    match file_name.iter().rposition(|&b| b == b'/' || b == b'\\') {
        Some(i) => &file_name[i + 1..],
        None => file_name,
    }
}

/// C: `const char *Sys_StackTrace (void)` -- always a `Mem_Alloc`ed buffer
/// here (the C arm handed back a string literal for the "not available"
/// case, which its callers then `Mem_Free`d).
///
/// # Safety
/// Any thread, after `Sys_Init`.
pub unsafe fn stack_trace() -> *const c_char {
    // SAFETY: `DBGHELP_INIT_SUCCESS` is only written by `Sys_Init`.
    if !unsafe { DBGHELP_INIT_SUCCESS } {
        return mem_strdup(b"[Not available.]\n");
    }

    let mut output_buffer: Vec<u8> = Vec::new();

    // SAFETY: caller contract -- the lock was initialised by `Sys_Init`
    // (checked through `DBGHELP_INIT_SUCCESS` above); `symbol` and `line`
    // are sized for the DbgHelp contract.
    unsafe {
        let process = GetCurrentProcess();

        let mut stack = [ptr::null_mut::<c_void>(); MAX_STACK_FRAMES];

        let nb_frames = usize::from(RtlCaptureStackBackTrace(
            0,
            MAX_STACK_FRAMES as u32,
            stack.as_mut_ptr(),
            ptr::null_mut(),
        ));

        // DbgHelp Sym* has internal state that must be protected
        EnterCriticalSection(addr_of_mut!(DBGHELP_LOCK).cast());

        for (frame_index, &frame) in stack.iter().enumerate().take(nb_frames) {
            let addr = frame as u64;

            // + 1 for null termination
            // buffer overlays SYMBOL_INFO + Name string
            let mut symbol_buffer = MaybeUninit::<SymbolBuffer>::zeroed();
            let symbol = symbol_buffer.as_mut_ptr().cast::<SYMBOL_INFO>();

            (*symbol).SizeOfStruct = size_of::<SYMBOL_INFO>() as u32;
            (*symbol).MaxNameLen = MAX_OSPATH as u32;

            let pdb_symbol_available = SymFromAddr(process, addr, ptr::null_mut(), symbol) != 0;

            let symbol_name: &[u8] = if pdb_symbol_available {
                CStr::from_ptr(symbol.cast::<c_char>().add(offset_of!(SYMBOL_INFO, Name)))
                    .to_bytes()
            } else {
                b"[no symbols]"
            };

            let mut line: IMAGEHLP_LINE64 = zeroed();
            let mut displacement: u32 = 0;
            line.SizeOfStruct = size_of::<IMAGEHLP_LINE64>() as u32;

            let pdb_file_and_line_available =
                SymGetLineFromAddr64(process, addr, &mut displacement, &mut line) != 0;

            // 1. All information:
            if pdb_file_and_line_available && pdb_symbol_available {
                // we only want the short file name, not the full path:
                let file_name = CStr::from_ptr(line.FileName.cast()).to_bytes();

                let _ = write!(output_buffer, "{frame_index:<2}: ");
                output_buffer.extend_from_slice(symbol_name);
                output_buffer.extend_from_slice(b" - ");
                output_buffer.extend_from_slice(short_file_name(file_name));
                let _ = writeln!(output_buffer, ":{}", line.LineNumber as c_int);
            }
            // 2. File and line, but no symbols, display the address in its place.
            else if pdb_file_and_line_available {
                // we only want the short file name, not the full path:
                let file_name = CStr::from_ptr(line.FileName.cast()).to_bytes();

                let _ = write!(output_buffer, "{frame_index:<2}: 0x{:x} - ", frame as usize);
                output_buffer.extend_from_slice(short_file_name(file_name));
                let _ = writeln!(output_buffer, ":{}", line.LineNumber as c_int);
            }
            // 3. Symbol but no file and line
            else if pdb_symbol_available {
                let _ = write!(output_buffer, "{frame_index:<2}: ");
                output_buffer.extend_from_slice(symbol_name);
                output_buffer.push(b'\n');
            }
            // 4. No symbols, no file and lines, this is likely a MSYS2 DWARF binary:
            else {
                let dwarf_va = (frame as isize).wrapping_add(DWARF_OFFSET) as usize;
                // display on 1 line to pass to addr2line easily:
                let _ = write!(output_buffer, "0x{dwarf_va:x} ");

                if frame_index == nb_frames - 1 {
                    output_buffer.push(b'\n');
                }
            }
        }

        LeaveCriticalSection(addr_of_mut!(DBGHELP_LOCK).cast());
    }

    mem_strdup(&output_buffer)
}

/// C: `bool Sys_IsInDebugger (void)`
///
/// # Safety
/// Any thread.
pub unsafe fn is_in_debugger() -> bool {
    // skip the pop-up when in a debugger:
    let mut debugger_attached: i32 = 0;
    // SAFETY: plain Win32 query on the current process.
    unsafe {
        CheckRemoteDebuggerPresent(GetCurrentProcess(), &mut debugger_attached);
    }

    debugger_attached != 0
}

/// C: `void Sys_DebugBreak (void)`
///
/// # Safety
/// Any thread.
pub unsafe fn debug_break() {
    // SAFETY: caller contract.
    unsafe {
        if is_in_debugger() {
            DebugBreak();
        }
    }
}
