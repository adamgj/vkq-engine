//! C ABI shims for the discovery half of `Quake/steam.c` (declarations stay
//! in `Quake/steam.h`): Steam_IsValidPath, Steam_FindGame, Steam_ResolvePath
//! and EGS_FindGame. The Steam API runtime (Steam_Init/Steam_Shutdown/
//! achievements) lives in steam_api.c, ChooseQuakeFlavor in sys_sdl.c, and
//! Sys_GetSteamAPILibraryPath in sys_sdl_*.c — all still C.
//!
//! The VDF/ACF and EGS-manifest decision logic is pure (`quake_fs::vdf`,
//! `quake_fs::egs`); this shim owns the engine IO (Sys_GetSteamDir,
//! COM_LoadMallocFile_TextMode_OSPath/Mem_Free, Sys_Find*) and the
//! `steamgame_t` packing quirk, where `subdir` points *into* the `library`
//! buffer ("<library>\0<subdir>\0" in the one array, exactly like the C —
//! steam_api.c and Sys_GetSteamAPILibraryPath read it back that way).

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use quake_c_sys::{qboolean, steamgame_t, MAX_OSPATH};

/// FS_ENT_FILE (common.h); Steam_IsValidPath wants exactly a file
const FS_ENT_FILE: c_int = 1 << 0;

/// Joins `parts` into one path with q_snprintf's truncation check against a
/// `char[MAX_OSPATH]` buffer: None when the formatted length would not fit
/// (C: `ret >= sizeof (buf)`), otherwise the NUL-terminated bytes. The same
/// check covers Steam_ReadLibFolders' q_strlcat, whose failure condition
/// (`dirlen + srclen >= sizeof (buf)`) is identical.
fn build_path(parts: &[&[u8]]) -> Option<Vec<u8>> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    if len >= MAX_OSPATH {
        return None;
    }
    let mut out = Vec::with_capacity(len + 1);
    for p in parts {
        out.extend_from_slice(p);
    }
    out.push(0);
    Some(out)
}

/// The write half of q_snprintf/q_strlcpy: at most `dstsize - 1` bytes of
/// `src` plus a NUL terminator; nothing at all when `dstsize == 0`. The C
/// writes the truncated string even when the caller then reports failure
/// (Steam_ResolvePath), so this must run unconditionally too.
///
/// # Safety
/// `dst` must be writable for `dstsize` bytes.
unsafe fn write_truncated(dst: *mut c_char, dstsize: usize, src: &[u8]) {
    if dstsize == 0 {
        return;
    }
    let n = src.len().min(dstsize - 1);
    // SAFETY: n < dstsize and dst is writable for dstsize bytes per the
    // caller contract above
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr() as *const c_char, dst, n);
        *dst.add(n) = 0;
    }
}

/// A Mem-owned, NUL-terminated text buffer (COM_LoadMallocFile_TextMode_OSPath
/// or Sys_GetEGSLauncherData both allocate `len + 1` and terminate);
/// Mem_Free'd on drop, standing in for the C's explicit Mem_Free calls.
struct MemText(ptr::NonNull<u8>);

impl MemText {
    /// The buffer as bytes up to the first NUL — the C parses it as a string.
    fn bytes(&self) -> &[u8] {
        // SAFETY: the loaders NUL-terminate the buffer (see the type doc)
        unsafe { CStr::from_ptr(self.0.as_ptr() as *const c_char) }.to_bytes()
    }
}

impl Drop for MemText {
    fn drop(&mut self) {
        // SAFETY: the pointer came from the engine's Mem_Alloc-backed loaders
        // and is released exactly once, here
        unsafe { quake_c_sys::Mem_Free(self.0.as_ptr() as *const c_void) };
    }
}

/// `COM_LoadMallocFile_TextMode_OSPath (path, NULL)` with NULL mapped to None.
fn load_text_file(path_z: &[u8]) -> Option<MemText> {
    debug_assert_eq!(path_z.last(), Some(&0));
    // SAFETY: engine C API; path_z is NUL-terminated and a NULL len_out is
    // allowed (the C passes NULL here too)
    let data = unsafe {
        quake_c_sys::COM_LoadMallocFile_TextMode_OSPath(
            path_z.as_ptr() as *const c_char,
            ptr::null_mut(),
        )
    };
    ptr::NonNull::new(data).map(MemText)
}

/// C: `static char *Steam_ReadLibFolders (void)` — the Steam base dir's
/// config/libraryfolders.vdf, or None if Steam or the file is absent.
fn read_lib_folders() -> Option<MemText> {
    let mut path = [0 as c_char; MAX_OSPATH];
    // SAFETY: engine C API; `path` is writable for the MAX_OSPATH bytes passed
    let ok = unsafe { quake_c_sys::Sys_GetSteamDir(path.as_mut_ptr(), MAX_OSPATH) };
    if !ok {
        return None;
    }
    // SAFETY: Sys_GetSteamDir filled `path` with a NUL-terminated string
    let dir = unsafe { CStr::from_ptr(path.as_ptr()) }.to_bytes();
    let full = build_path(&[dir, b"/config/libraryfolders.vdf"])?;
    load_text_file(&full)
}

/// C: `qboolean Steam_IsValidPath (const char *path);` — true if the path
/// contains a valid Steam install (a config/libraryfolders.vdf file).
///
/// # Safety
/// `path` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn Steam_IsValidPath(path: *const c_char) -> qboolean {
    // SAFETY: NUL-terminated per the steam.h contract
    let path = unsafe { CStr::from_ptr(path) }.to_bytes();
    // C: a q_snprintf result that would overflow char[MAX_OSPATH] fails
    let Some(libpath) = build_path(&[path, b"/config/libraryfolders.vdf"]) else {
        return false;
    };
    // SAFETY: engine C API; libpath is NUL-terminated
    unsafe { quake_c_sys::Sys_FileType(libpath.as_ptr() as *const c_char) == FS_ENT_FILE }
}

/// C: `qboolean Steam_FindGame (steamgame_t *game, int appid);` — finds the
/// Steam library and subdirectory for the given appid.
///
/// On success the C packs both strings into the one `library` array —
/// `"<library>\0<installdir>\0"` — and points `subdir` at the second string
/// (`game->library + liblen + 1`); this does the same, byte for byte. On any
/// failure `appid` is still set, `subdir` stays NULL and `library[0]` stays 0.
///
/// # Safety
/// `game` must point to a writable `steamgame_t`.
#[no_mangle]
pub unsafe extern "C" fn Steam_FindGame(game: *mut steamgame_t, appid: c_int) -> qboolean {
    // C initializes the out-struct before any early return
    // SAFETY: `game` is writable per the steam.h contract
    unsafe {
        (*game).appid = appid;
        (*game).subdir = ptr::null_mut();
        (*game).library[0] = 0;
    }

    let Some(steamcfg) = read_lib_folders() else {
        // SAFETY: engine C API; static NUL-terminated format
        unsafe { quake_c_sys::Sys_Printf(c"Steam library not found.\n".as_ptr()) };
        return false;
    };

    // C: q_snprintf (appidstr, sizeof (appidstr), "%d", appid) — used both in
    // paths and as the %s argument of the error messages below
    let mut appid_z = appid.to_string().into_bytes();
    appid_z.push(0);
    let appid_str = &appid_z[..appid_z.len() - 1];

    // VDB_Parse + VDB_OnLibFolderProperty; None covers both a parse abort and
    // a missing match, like the C's `!VDB_Parse (...) || !libparser.result`
    let Some(library) = quake_fs::vdf::find_library_for_app(steamcfg.bytes(), appid) else {
        // SAFETY: engine C API; static NUL-terminated format
        unsafe { quake_c_sys::Sys_Printf(c"ERROR: Couldn't parse Steam library.\n".as_ptr()) };
        return false;
    };

    let Some(manifest_path) =
        build_path(&[&library, b"/steamapps/appmanifest_", appid_str, b".acf"])
    else {
        // SAFETY: engine C API; NUL-terminated format and argument
        unsafe {
            quake_c_sys::Sys_Printf(
                c"ERROR: Couldn't read Steam manifest for app %s (path too long).\n".as_ptr(),
                appid_z.as_ptr() as *const c_char,
            );
        }
        return false;
    };

    let Some(manifest) = load_text_file(&manifest_path) else {
        // SAFETY: engine C API; NUL-terminated format and argument
        unsafe {
            quake_c_sys::Sys_Printf(
                c"ERROR: Couldn't read Steam manifest for app %s.\n".as_ptr(),
                appid_z.as_ptr() as *const c_char,
            );
        }
        return false;
    };

    // VDB_Parse + ACF_OnManifestProperty, same abort-or-missing collapse
    let Some(subdir) = quake_fs::vdf::acf_installdir(manifest.bytes()) else {
        // SAFETY: engine C API; NUL-terminated format and argument
        unsafe {
            quake_c_sys::Sys_Printf(
                c"ERROR: Couldn't parse Steam manifest for app %s.\n".as_ptr(),
                appid_z.as_ptr() as *const c_char,
            );
        }
        return false;
    };

    let liblen = library.len();
    let sublen = subdir.len();
    if liblen + 1 + sublen + 1 > MAX_OSPATH {
        // SAFETY: engine C API; NUL-terminated format and argument
        unsafe {
            quake_c_sys::Sys_Printf(
                c"ERROR: Path for Steam app %s is too long.\n".as_ptr(),
                appid_z.as_ptr() as *const c_char,
            );
        }
        return false;
    }

    // The packing quirk: both strings in the one buffer, subdir pointing at
    // the second (C: memcpy library; subdir = library + liblen + 1; memcpy)
    // SAFETY: `game` is writable per the contract above and the bounds check
    // just proved liblen + 1 + sublen + 1 fits the MAX_OSPATH array, so every
    // write lands inside `library`; the values contain no interior NULs (the
    // VDF parser stops at NUL and no escape produces one), so C strlen/strcmp
    // over the packed buffer see exactly these strings
    unsafe {
        let base = ptr::addr_of_mut!((*game).library) as *mut c_char;
        ptr::copy_nonoverlapping(library.as_ptr() as *const c_char, base, liblen);
        *base.add(liblen) = 0;
        let sub = base.add(liblen + 1);
        ptr::copy_nonoverlapping(subdir.as_ptr() as *const c_char, sub, sublen);
        *sub.add(sublen) = 0;
        (*game).subdir = sub;
    }
    true
}

/// C: `qboolean Steam_ResolvePath (char *path, size_t pathsize, const steamgame_t *game);`
/// — fills in the OS path where the game is installed
/// ("<library>/steamapps/common/<subdir>").
///
/// Like the C's q_snprintf, the (possibly truncated) string is written into
/// `path` even when the length check then fails; true only when `subdir` is
/// set and the full string fit.
///
/// # Safety
/// `path` must be writable for `pathsize` bytes; `game` must point to a
/// readable `steamgame_t` from Steam_FindGame (its `subdir`, when set, is the
/// second NUL-terminated string packed inside its `library` buffer).
#[no_mangle]
pub unsafe extern "C" fn Steam_ResolvePath(
    path: *mut c_char,
    pathsize: usize,
    game: *const steamgame_t,
) -> qboolean {
    // SAFETY: `game` is readable per the contract above
    let subdir = unsafe { (*game).subdir };
    if subdir.is_null() {
        return false;
    }
    // SAFETY: both are NUL-terminated strings inside game->library, packed by
    // Steam_FindGame per the contract above
    let (library, subdir) = unsafe {
        (
            CStr::from_ptr(ptr::addr_of!((*game).library) as *const c_char).to_bytes(),
            CStr::from_ptr(subdir).to_bytes(),
        )
    };

    const MID: &[u8] = b"/steamapps/common/";
    let mut formatted = Vec::with_capacity(library.len() + MID.len() + subdir.len());
    formatted.extend_from_slice(library);
    formatted.extend_from_slice(MID);
    formatted.extend_from_slice(subdir);

    // SAFETY: `path` is writable for `pathsize` bytes per the contract above
    unsafe { write_truncated(path, pathsize, &formatted) };
    // C: `(size_t)q_snprintf (...) < pathsize`
    formatted.len() < pathsize
}

/// C: `qboolean EGS_FindGame (char *path, size_t pathsize, const char *nspace,
/// const char *itemid, const char *appname);` — checks the Epic launcher's
/// LauncherInstalled.dat first, then scans the manifest dir's *.item files.
///
/// # Safety
/// `path` must be writable for `pathsize` bytes; `nspace`, `itemid` and
/// `appname` must be NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn EGS_FindGame(
    path: *mut c_char,
    pathsize: usize,
    nspace: *const c_char,
    itemid: *const c_char,
    appname: *const c_char,
) -> qboolean {
    // SAFETY: NUL-terminated per the steam.h contract
    let (nspace, itemid, appname) = unsafe {
        (
            CStr::from_ptr(nspace).to_bytes(),
            CStr::from_ptr(itemid).to_bytes(),
            CStr::from_ptr(appname).to_bytes(),
        )
    };

    // Phase 1: the launcher data (C: JSON_Parse, then Mem_Free the buffer;
    // here the guard frees it once the match result is owned)
    // SAFETY: engine C API, no arguments; a non-NULL result is a Mem-owned
    // NUL-terminated buffer this call now owns
    let launcherdata = unsafe { quake_c_sys::Sys_GetEGSLauncherData() };
    if let Some(data) = ptr::NonNull::new(launcherdata.cast_mut()) {
        let data = MemText(data.cast::<u8>());
        if let Some(location) =
            quake_fs::egs::find_in_launcher_data(data.bytes(), nspace, itemid, appname)
        {
            // C: q_strlcpy (path, location, pathsize) — true even if truncated
            // SAFETY: `path` is writable for `pathsize` bytes per the contract
            unsafe { write_truncated(path, pathsize, &location) };
            return true;
        }
        // parse failure and no-match both fall through to the manifest scan
    }

    // Phase 2: the *.item manifests
    let mut manifestdir = [0 as c_char; MAX_OSPATH];
    // SAFETY: engine C API; `manifestdir` is writable for the MAX_OSPATH
    // bytes passed
    let ok = unsafe { quake_c_sys::Sys_GetEGSManifestDir(manifestdir.as_mut_ptr(), MAX_OSPATH) };
    if !ok {
        return false;
    }
    // SAFETY: Sys_GetEGSManifestDir filled a NUL-terminated string on success
    let dir = unsafe { CStr::from_ptr(manifestdir.as_ptr()) }.to_bytes();

    // C: `for (find = Sys_FindFirst (manifestdir, "item"); find;
    //      find = Sys_FindNext (find))` — Sys_FindNext releases the handle at
    // the end of iteration, so only the early returns call Sys_FindClose
    // SAFETY: engine C API; both arguments are NUL-terminated strings
    let mut find = unsafe { quake_c_sys::Sys_FindFirst(manifestdir.as_ptr(), c"item".as_ptr()) };
    while !find.is_null() {
        // SAFETY: `find` is the live handle until Sys_FindNext/Sys_FindClose
        let attribs = unsafe { (*find).attribs };
        let matched = if attribs & quake_c_sys::fileattribs_t_FA_DIRECTORY != 0 {
            None // C: continue on directories
        } else {
            // SAFETY: as above; `name` is a NUL-terminated string in the
            // handle, copied into the path before the handle advances
            let name =
                unsafe { CStr::from_ptr(ptr::addr_of!((*find).name) as *const c_char) }.to_bytes();
            // C: a q_snprintf "%s/%s" overflow is a continue, not a failure
            build_path(&[dir, b"/", name])
                .and_then(|filepath| load_text_file(&filepath))
                .and_then(|manifest| {
                    quake_fs::egs::manifest_matches(manifest.bytes(), nspace, itemid, appname)
                })
        };

        if let Some(location) = matched {
            // SAFETY: `path` is writable for `pathsize` bytes per the contract
            unsafe { write_truncated(path, pathsize, &location) };
            // SAFETY: `find` is still the live handle on this early return
            unsafe { quake_c_sys::Sys_FindClose(find) };
            return true;
        }

        // SAFETY: `find` is the live handle; Sys_FindNext returns the next
        // one or NULL after releasing it
        find = unsafe { quake_c_sys::Sys_FindNext(find) };
    }

    false
}
