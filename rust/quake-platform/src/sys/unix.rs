//! `Quake/sys_sdl_unix.c`: the POSIX arm. Linux takes the plain branches;
//! macOS takes the `PLATFORM_OSX` ones (`realpath` + app-bundle stripping
//! for the basedir, `sysctl` for the debugger check, the extra one-line
//! address dump in the stack trace).
//!
//! `Sys_Error` here runs `Host_Shutdown` off the worker threads, so
//! [`error_core`] can return the guard status for the C wrapper to
//! re-raise (ADR-009).

use core::ffi::{c_char, c_int, c_void, CStr};
use core::mem::zeroed;
use core::ptr::{self, addr_of_mut};
use std::ffi::CString;
use std::io::Write as _;

use quake_c_sys::sys as c;
use quake_c_sys::{
    fileattribs_t_FA_DIRECTORY, findfile_t, qfileofs_t, steamgame_t, FILE, MAX_OSPATH,
};

use super::{copy_into, mem_strdup, sys_error, sys_printf, ERRORTXT1, ERRORTXT2, MAX_STACK_FRAMES};

/// `common.h` -- `FS_ENT_NONE` / `FS_ENT_FILE` / `FS_ENT_DIRECTORY`.
const FS_ENT_NONE: c_int = 0;
const FS_ENT_FILE: c_int = 1;
const FS_ENT_DIRECTORY: c_int = 2;

/// `steam.h` -- `QUAKE_STEAM_APPID`.
const QUAKE_STEAM_APPID: c_int = 2310;

/// `static char cwd[MAX_OSPATH];`
static mut CWD: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];
/// `static char userdir[MAX_OSPATH];`
#[cfg(feature = "userdirs")]
static mut USERDIR: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];

/// `#define SYS_USERDIR`
#[cfg(all(feature = "userdirs", target_os = "macos"))]
const SYS_USERDIR: &CStr = c"Library/Application Support/vkQuake";
#[cfg(all(feature = "userdirs", not(target_os = "macos")))]
const SYS_USERDIR: &CStr = c".vkquake";

/// `isDedicated` reads.
unsafe fn is_dedicated() -> bool {
    // SAFETY: caller contract -- main thread or a worker reading a flag
    // `main` set before the threads existed.
    unsafe { quake_c_sys::isDedicated }
}

/// `errno`.
pub(super) fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `S_ISDIR (st.st_mode)`
fn is_dir(st: &libc::stat) -> bool {
    (st.st_mode & libc::S_IFMT) == libc::S_IFDIR
}

/// `%s` of a C string: the bytes, lossily decoded for a Rust `format!`.
///
/// # Safety
/// `s` is a NUL-terminated string.
unsafe fn lossy(s: *const c_char) -> String {
    // SAFETY: caller contract.
    unsafe { CStr::from_ptr(s).to_string_lossy().into_owned() }
}

/// C: `int Sys_fseek (FILE *file, qfileofs_t ofs, int origin)`
///
/// # Safety
/// `file` is an open stream.
pub unsafe fn fseek(file: *mut FILE, ofs: qfileofs_t, origin: c_int) -> c_int {
    // SAFETY: caller contract.
    unsafe { c::fseeko(file, ofs, origin) }
}

/// C: `qfileofs_t Sys_ftell (FILE *file)`
///
/// # Safety
/// `file` is an open stream.
pub unsafe fn ftell(file: *mut FILE) -> qfileofs_t {
    // SAFETY: caller contract.
    unsafe { c::ftello(file) }
}

/// C: `int Sys_FileType (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string.
pub unsafe fn file_type(path: *const c_char) -> c_int {
    // SAFETY: caller contract; zero is a valid `stat`.
    let mut st: libc::stat = unsafe { zeroed() };

    // SAFETY: caller contract.
    if unsafe { libc::stat(path, &mut st) } != 0 {
        return FS_ENT_NONE;
    }
    if is_dir(&st) {
        return FS_ENT_DIRECTORY;
    }
    if (st.st_mode & libc::S_IFMT) == libc::S_IFREG {
        return FS_ENT_FILE;
    }

    FS_ENT_NONE
}

/// `mkdir (dir, 0777)` with the `EEXIST`-on-a-directory forgiveness both
/// `Sys_fopen` and `Sys_mkdir` spell.
///
/// # Safety
/// `dir` is a NUL-terminated string.
unsafe fn mkdir_rc(dir: *const c_char) -> c_int {
    // SAFETY: caller contract; zero is a valid `stat`.
    unsafe {
        let mut rc = libc::mkdir(dir, 0o777);
        if rc != 0 && errno() == libc::EEXIST {
            let mut st: libc::stat = zeroed();
            if libc::stat(dir, &mut st) == 0 && is_dir(&st) {
                rc = 0;
            }
        }
        rc
    }
}

/// C: `FILE *Sys_fopen (const char *path, const char *mode)`
///
/// # Safety
/// `path` and `mode` are NUL-terminated strings.
pub unsafe fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    // SAFETY: caller contract; `dir` stays NUL-terminated by `q_strlcpy`.
    unsafe {
        if CStr::from_ptr(mode).to_bytes().contains(&b'w') {
            let mut dir = [0 as c_char; MAX_OSPATH];
            c::q_strlcpy(dir.as_mut_ptr(), path, dir.len());
            let mut i = 1;
            while dir[i] != 0 {
                if dir[i] != b'/' as c_char {
                    i += 1;
                    continue;
                }
                dir[i] = 0;
                let rc = mkdir_rc(dir.as_ptr());
                if rc != 0 {
                    return ptr::null_mut();
                }
                dir[i] = b'/' as c_char;
                i += 1;
            }
        }

        c::fopen(path, mode)
    }
}

/// `static qboolean Sys_Exec (const char *cmd, ...)` -- `fork` + `execvp`
/// with the child's stdout/stderr on `/dev/null` (the C `freopen`s are
/// spelled as the `dup2` they amount to; the libc crate has no `stdout`).
///
/// # Safety
/// `cmd` and `args` are NUL-terminated strings.
unsafe fn exec(cmd: *const c_char, args: &[*const c_char]) -> bool {
    // SAFETY: caller contract; the child only calls async-signal-safe
    // libc entry points before `execvp`.
    unsafe {
        let p = libc::fork();
        if p < 0 {
            // fork failed
            return false;
        }
        if p == 0 {
            // child process
            let mut argv: Vec<*const c_char> = Vec::with_capacity(args.len() + 2);
            argv.push(cmd);
            argv.extend_from_slice(args);
            argv.push(ptr::null());

            // Disable stdout/stderr
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            if null >= 0 {
                libc::dup2(null, 1);
                libc::dup2(null, 2);
            }

            libc::execvp(cmd, argv.as_ptr());
            c::exit(libc::EXIT_FAILURE);
        }
        // original process
        true
    }
}

/// C: `qboolean Sys_Explore (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string. Main thread.
pub unsafe fn explore(path: *const c_char) -> bool {
    #[cfg(feature = "sdl2")]
    use sdl2::sys::SDL_OpenURL;
    #[cfg(feature = "sdl3")]
    use sdl3::sys::misc::SDL_OpenURL;

    // SAFETY: caller contract.
    unsafe {
        if file_type(path) == FS_ENT_NONE {
            return false;
        }

        // Try to identify the current desktop so we can open the parent dir in the file manager *and* select the right file in it
        let s = libc::getenv(c"XDG_CURRENT_DESKTOP".as_ptr());
        if !s.is_null() {
            let mut buf = CStr::from_ptr(s).to_bytes().to_vec();
            buf.truncate(32767);
            for desktop in buf.split(|&b| b == b':').filter(|d| !d.is_empty()) {
                if desktop.eq_ignore_ascii_case(b"gnome") {
                    return exec(c"nautilus".as_ptr(), &[c"--select".as_ptr(), path]);
                }
                if desktop.eq_ignore_ascii_case(b"kde") {
                    return exec(c"dolphin".as_ptr(), &[c"--select".as_ptr(), path]);
                }
            }
        }

        // Fall back to just opening the parent dir without selecting the file
        let mut buf = CStr::from_ptr(path).to_bytes().to_vec();
        buf.truncate(32767);
        let Some(slash) = buf.iter().rposition(|&b| b == b'/') else {
            return false;
        };
        buf.truncate(slash + 1); // terminate after the slash
        let buf = CString::new(buf).unwrap_or_default();
        #[cfg(feature = "sdl3")]
        {
            SDL_OpenURL(buf.as_ptr())
        }
        #[cfg(feature = "sdl2")]
        {
            SDL_OpenURL(buf.as_ptr()) == 0
        }
    }
}

/*
==============================================================================
STORE INSTALL LOCATIONS (from Ironwail)
==============================================================================
*/

/// The `getpwuid (getuid ())->pw_dir` / `$HOME` lookup `Sys_GetSteamDir`
/// and `Sys_GetUserdir` share; null when neither is available.
unsafe fn home_dir() -> *const c_char {
    // SAFETY: `getpwuid`'s record is process-owned static storage.
    unsafe {
        let mut home_dir: *const c_char = ptr::null();
        let pwent = libc::getpwuid(libc::getuid());
        if pwent.is_null() {
            libc::perror(c"getpwuid".as_ptr());
        } else {
            home_dir = (*pwent).pw_dir;
        }
        if home_dir.is_null() {
            home_dir = libc::getenv(c"HOME".as_ptr());
        }
        home_dir
    }
}

/// C: `qboolean Sys_GetSteamDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
pub unsafe fn get_steam_dir(path: *mut c_char, pathsize: usize) -> bool {
    // SAFETY: caller contract; `path` is NUL-terminated by `copy_into`
    // before `Steam_IsValidPath` reads it.
    unsafe {
        let home_dir = home_dir();
        if home_dir.is_null() {
            return false;
        }
        let home = CStr::from_ptr(home_dir).to_bytes();

        for suffix in [
            &b"/.steam/steam"[..],
            b"/.local/share/Steam",
            b"/.var/app/com.valvesoftware.Steam/.steam/steam",
            b"/.var/app/com.valvesoftware.Steam/.local/share/Steam",
        ] {
            let mut s = home.to_vec();
            s.extend_from_slice(suffix);
            if copy_into(path, pathsize, &s) && c::Steam_IsValidPath(path) {
                return true;
            }
        }

        false
    }
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
    let mut config_info_path = [0 as c_char; MAX_OSPATH];
    let mut line: *mut c_char = ptr::null_mut();
    let mut line_size: usize = 0;

    // SAFETY: caller contract; `line` is libc-allocated by `getline` and
    // `free`d on every path.
    unsafe {
        let mut s = CStr::from_ptr((*game).library.as_ptr()).to_bytes().to_vec();
        s.extend_from_slice(
            format!("/steamapps/compatdata/{}/config_info", (*game).appid).as_bytes(),
        );
        // (the C compares the config path's length against the *output*
        // buffer size; kept as is)
        if s.len() >= pathsize {
            return false;
        }
        copy_into(config_info_path.as_mut_ptr(), config_info_path.len(), &s);

        let config_info = c::fopen(config_info_path.as_ptr(), c"r".as_ptr());
        if config_info.is_null() {
            return false;
        }

        // lib dir is on line 3, lib64 on line 4
        let mut read_lines = if core::mem::size_of::<*const c_void>() == 4 {
            3
        } else {
            4
        };
        while read_lines > 0 {
            read_lines -= 1;
            if libc::getline(&mut line, &mut line_size, config_info.cast::<libc::FILE>()) == -1 {
                c::fclose(config_info);
                libc::free(line.cast()); // getline buffer is libc-allocated, NOT Mem_Free
                return false;
            }
        }

        c::fclose(config_info);
        if line.is_null() {
            return false;
        }

        let mut lib = CStr::from_ptr(line).to_bytes().to_vec();
        if lib.last() == Some(&b'\n') {
            lib.pop();
        }
        if lib.last() == Some(&b'/') {
            lib.pop();
        }
        lib.extend_from_slice(b"/libsteam_api.so");

        let result = copy_into(path, pathsize, &lib);

        libc::free(line.cast()); // getline buffer is libc-allocated, NOT Mem_Free

        result
    }
}

/// C: `qboolean Sys_GetNightdiveUserDir (char *path, size_t pathsize, const char *steamlibrary)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes; `steamlibrary` NUL-terminated
/// or null.
pub unsafe fn get_nightdive_user_dir(
    path: *mut c_char,
    pathsize: usize,
    steamlibrary: *const c_char,
) -> bool {
    if steamlibrary.is_null() {
        return false;
    }

    // SAFETY: caller contract.
    unsafe {
        let mut s = CStr::from_ptr(steamlibrary).to_bytes().to_vec();
        s.extend_from_slice(
            format!("/steamapps/compatdata/{QUAKE_STEAM_APPID}/pfx/drive_c/users/steamuser/Saved Games/Nightdive Studios/Quake")
                .as_bytes(),
        );
        copy_into(path, pathsize, &s)
    }
}

/// C: `qboolean Sys_GetGOGQuakeDir (char *path, size_t pathsize)`
///
/// # Safety
/// Always safe; nothing is written.
pub unsafe fn get_gog_quake_dir(_path: *mut c_char, _pathsize: usize) -> bool {
    false
}

/// C: `qboolean Sys_GetGOGQuakeEnhancedDir (char *path, size_t pathsize)`
///
/// # Safety
/// Always safe; nothing is written.
pub unsafe fn get_gog_quake_enhanced_dir(_path: *mut c_char, _pathsize: usize) -> bool {
    false
}

/// C: `qboolean Sys_GetEGSManifestDir (char *path, size_t pathsize)`
///
/// # Safety
/// Always safe; nothing is written.
pub unsafe fn get_egs_manifest_dir(_path: *mut c_char, _pathsize: usize) -> bool {
    false
}

/// C: `const char *Sys_GetEGSLauncherData (void)`
///
/// # Safety
/// Always safe.
pub unsafe fn get_egs_launcher_data() -> *const c_char {
    ptr::null()
}

/*
==============================================================================
DIRECTORY ENUMERATION (from Ironwail)
==============================================================================
*/

/// `unixfindfile_t`.
#[repr(C)]
struct UnixFindFile {
    base: findfile_t,
    handle: *mut libc::DIR,
    data: *mut libc::dirent,
    filter: [c_char; 8],
}

/// `static void Sys_FillFindData (unixfindfile_t *find)`
///
/// # Safety
/// `find` is a live entry whose `data` is the current `readdir` record.
unsafe fn fill_find_data(find: *mut UnixFindFile) {
    // SAFETY: caller contract.
    unsafe {
        c::q_strlcpy(
            (*find).base.name.as_mut_ptr(),
            (*(*find).data).d_name.as_ptr(),
            (*find).base.name.len(),
        );
        (*find).base.attribs = 0;
        if (*(*find).data).d_type & libc::DT_DIR != 0 {
            (*find).base.attribs |= fileattribs_t_FA_DIRECTORY;
        }
    }
}

/// `static struct dirent *readdir_filtered (DIR *handle, const char *ext)`
///
/// # Safety
/// `handle` is an open directory stream; `ext` a NUL-terminated string.
unsafe fn readdir_filtered(handle: *mut libc::DIR, ext: *const c_char) -> *mut libc::dirent {
    // SAFETY: caller contract.
    unsafe {
        loop {
            let data = libc::readdir(handle);
            if data.is_null()
                || *ext == b'*' as c_char
                || c::q_strcasecmp(ext, c::COM_FileGetExtension((*data).d_name.as_ptr())) == 0
            {
                return data;
            }
        }
    }
}

/// C: `findfile_t *Sys_FindFirst (const char *dir, const char *ext)`
///
/// # Safety
/// `dir` is a NUL-terminated string; `ext` NUL-terminated or null.
pub unsafe fn find_first(dir: *const c_char, ext: *const c_char) -> *mut findfile_t {
    // SAFETY: caller contract; the entry is `Mem_Alloc`ed (zeroed) before
    // its fields are set.
    unsafe {
        let ext = if ext.is_null() {
            c"*".as_ptr()
        } else if *ext == b'.' as c_char {
            ext.add(1)
        } else {
            ext
        };

        let ext_bytes = CStr::from_ptr(ext).to_bytes();
        if ext_bytes.len() >= 8 {
            sys_error(&format!(
                "Sys_FindFirst: extension too long '{}'",
                String::from_utf8_lossy(ext_bytes)
            ));
        }

        let handle = libc::opendir(dir);
        if handle.is_null() {
            return ptr::null_mut();
        }

        let data = readdir_filtered(handle, ext);
        if data.is_null() {
            libc::closedir(handle);
            return ptr::null_mut();
        }

        let ret = c::Mem_Alloc(core::mem::size_of::<UnixFindFile>()).cast::<UnixFindFile>();
        if ret.is_null() {
            sys_error("Sys_FindFirst: out of memory");
        }
        (*ret).handle = handle;
        (*ret).data = data;
        c::q_strlcpy((*ret).filter.as_mut_ptr(), ext, (*ret).filter.len());
        fill_find_data(ret);

        ret.cast()
    }
}

/// C: `findfile_t *Sys_FindNext (findfile_t *find)`
///
/// # Safety
/// `find` came from [`find_first`] and has not been closed.
pub unsafe fn find_next(find: *mut findfile_t) -> *mut findfile_t {
    let ufind = find.cast::<UnixFindFile>();
    // SAFETY: caller contract -- `findfile_t` is the first member.
    unsafe {
        (*ufind).data = readdir_filtered((*ufind).handle, (*ufind).filter.as_ptr());
        if (*ufind).data.is_null() {
            find_close(find);
            return ptr::null_mut();
        }
        fill_find_data(ufind);
    }
    find
}

/// C: `void Sys_FindClose (findfile_t *find)`
///
/// # Safety
/// `find` came from [`find_first`] and has not been closed, or is null.
pub unsafe fn find_close(find: *mut findfile_t) {
    if !find.is_null() {
        let ufind = find.cast::<UnixFindFile>();
        // SAFETY: caller contract.
        unsafe {
            libc::closedir((*ufind).handle);
            c::Mem_Free(ufind.cast());
        }
    }
}

/// `static qboolean Sys_GetUserdirArgs (int argc, char **argv, char *dst, size_t dstsize)`
///
/// # Safety
/// `argv` holds `argc` NUL-terminated strings; `dst` is writable for
/// `dstsize` bytes.
#[cfg(feature = "userdirs")]
unsafe fn get_userdir_args(
    argc: c_int,
    argv: *mut *mut c_char,
    dst: *mut c_char,
    dstsize: usize,
) -> bool {
    // SAFETY: caller contract; `dst` is NUL-terminated by `q_strlcpy`
    // before the slash walk.
    unsafe {
        let mut i = 1;
        while i < argc - 1 {
            if libc::strcmp(*argv.add(i as usize), c"-userdir".as_ptr()) == 0 {
                let mut p = dst;
                let arg = *argv.add(i as usize + 1);
                let n = libc::strlen(arg);
                if n < 1 {
                    sys_error("Bad argument to -userdir");
                }
                if c::q_strlcpy(dst, arg, dstsize) >= dstsize {
                    sys_error("Insufficient array size for userspace directory");
                }
                if *dst.add(n - 1) == b'/' as c_char {
                    *dst.add(n - 1) = 0;
                }
                if *p == b'/' as c_char {
                    p = p.add(1);
                }
                while *p != 0 {
                    let ch = *p;
                    if ch == b'/' as c_char {
                        *p = 0;
                        mkdir(dst);
                        *p = ch;
                    }
                    p = p.add(1);
                }
                return true;
            }
            i += 1;
        }
        false
    }
}

/// `static void Sys_GetUserdir (int argc, char **argv, char *dst, size_t dstsize)`
///
/// # Safety
/// As [`get_userdir_args`].
#[cfg(feature = "userdirs")]
unsafe fn get_userdir(argc: c_int, argv: *mut *mut c_char, dst: *mut c_char, dstsize: usize) {
    // SAFETY: caller contract.
    unsafe {
        if get_userdir_args(argc, argv, dst, dstsize) {
            return;
        }

        let home_dir = home_dir();
        if home_dir.is_null() {
            sys_error("Couldn't determine userspace directory");
        }
        let home = CStr::from_ptr(home_dir).to_bytes();

        /* what would be a maximum path for a file in the user's directory...
         * $HOME/SYS_USERDIR/game_dir/dirname1/dirname2/dirname3/filename.ext
         * still fits in the MAX_OSPATH == 256 definition, but just in case :
         */
        let n = home.len() + SYS_USERDIR.to_bytes().len() + 50;
        if n >= dstsize {
            sys_error("Insufficient array size for userspace directory");
        }

        let mut s = home.to_vec();
        s.push(b'/');
        s.extend_from_slice(SYS_USERDIR.to_bytes());
        copy_into(dst, dstsize, &s);
    }
}

/// `static char *OSX_StripAppBundle (char *dir)`
///
/// # Safety
/// `dir` is a NUL-terminated string. Main thread (static result buffer).
#[cfg(target_os = "macos")]
unsafe fn osx_strip_app_bundle(dir: *mut c_char) -> *mut c_char {
    /* based on the ioquake3 project at icculus.org. */
    static mut OSX_PATH: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];

    // SAFETY: caller contract; `basename`/`dirname` may modify their
    // argument, which is the static copy here.
    unsafe {
        let osx_path = addr_of_mut!(OSX_PATH).cast::<c_char>();

        c::q_strlcpy(osx_path, dir, MAX_OSPATH);
        if libc::strcmp(libc::basename(osx_path), c"MacOS".as_ptr()) != 0 {
            return dir;
        }
        c::q_strlcpy(osx_path, libc::dirname(osx_path), MAX_OSPATH);
        if libc::strcmp(libc::basename(osx_path), c"Contents".as_ptr()) != 0 {
            return dir;
        }
        c::q_strlcpy(osx_path, libc::dirname(osx_path), MAX_OSPATH);
        if libc::strstr(libc::basename(osx_path), c".app".as_ptr()).is_null() {
            return dir;
        }
        c::q_strlcpy(osx_path, libc::dirname(osx_path), MAX_OSPATH);
        osx_path
    }
}

/// `static void Sys_GetBasedir (char *argv0, char *dst, size_t dstsize)` --
/// the `PLATFORM_OSX` arm.
///
/// # Safety
/// `argv0` is a NUL-terminated string; `dst` writable for `dstsize`
/// (`>= PATH_MAX`) bytes.
#[cfg(target_os = "macos")]
unsafe fn get_basedir(argv0: *const c_char, dst: *mut c_char, dstsize: usize) {
    // SAFETY: caller contract.
    unsafe {
        if libc::realpath(argv0, dst).is_null() {
            libc::perror(c"realpath".as_ptr());
            if libc::getcwd(dst, dstsize - 1).is_null() {
                sys_error("Couldn't determine current directory");
            }
        } else {
            /* strip off the binary name */
            let tmp = libc::strdup(dst);
            if tmp.is_null() {
                sys_error("Couldn't determine current directory");
            }
            c::q_strlcpy(dst, libc::dirname(tmp), dstsize);
            libc::free(tmp.cast());
        }

        let tmp = osx_strip_app_bundle(dst);
        if tmp != dst {
            c::q_strlcpy(dst, tmp, dstsize);
        }
    }
}

/// `static void Sys_GetBasedir (char *argv0, char *dst, size_t dstsize)`
///
/// # Safety
/// `dst` is writable for `dstsize` bytes.
#[cfg(not(target_os = "macos"))]
unsafe fn get_basedir(_argv0: *const c_char, dst: *mut c_char, dstsize: usize) {
    // SAFETY: caller contract.
    unsafe {
        if libc::getcwd(dst, dstsize - 1).is_null() {
            sys_error("Couldn't determine current directory");
        }

        let mut tmp = dst;
        while *tmp != 0 {
            tmp = tmp.add(1);
        }
        while *tmp == 0 && tmp != dst {
            tmp = tmp.sub(1);
            if tmp != dst && *tmp == b'/' as c_char {
                *tmp = 0;
            }
        }
    }
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

        let parms = quake_c_sys::host::host_parms;
        let cwd = addr_of_mut!(CWD).cast::<c_char>();
        cwd.write_bytes(0, MAX_OSPATH);
        get_basedir(*(*parms).argv, cwd, MAX_OSPATH);
        (*parms).basedir = cwd;
        #[cfg(not(feature = "userdirs"))]
        {
            (*parms).userdir = (*parms).basedir; /* code elsewhere relies on this ! */
        }
        #[cfg(feature = "userdirs")]
        {
            let userdir = addr_of_mut!(USERDIR).cast::<c_char>();
            userdir.write_bytes(0, MAX_OSPATH);
            get_userdir((*parms).argc, (*parms).argv, userdir, MAX_OSPATH);
            mkdir(userdir);
            (*parms).userdir = userdir;
        }

        super::common::init_counter_freq();
    }
}

/// C: `void Sys_mkdir (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string.
pub unsafe fn mkdir(path: *const c_char) {
    // SAFETY: caller contract.
    unsafe {
        if mkdir_rc(path) != 0 {
            let rc = errno();
            sys_error(&format!(
                "Unable to create directory {}: {}",
                lossy(path),
                lossy(c::strerror(rc))
            ));
        }
    }
}

/// C: `void Sys_Error (const char *error, ...)` after the C wrapper
/// formatted `text` (a `Mem_Alloc`ed buffer this function frees). Returns
/// only when the guarded `Host_Shutdown` raised, with its status.
///
/// # Safety
/// `text` is a `Mem_Alloc`ed NUL-terminated string.
pub unsafe fn error_core(text: *mut c_char) -> c_int {
    // SAFETY: caller contract.
    unsafe {
        let full = super::error_prologue(text);
        let full_c = CString::new(full).unwrap_or_default();
        let worker = c::Tasks_IsWorker();

        c::fputs(ERRORTXT1.as_ptr(), c::stdout_stream());

        if !worker {
            let r = c::SysGlue_HostShutdown();
            if r != 0 {
                return r;
            }
        }

        c::fputs(ERRORTXT2.as_ptr(), c::stdout_stream());

        let mut with_newlines = full_c.as_bytes().to_vec();
        with_newlines.extend_from_slice(b"\n\n");
        print_text(CString::new(with_newlines).unwrap_or_default().as_ptr());

        if !worker && !is_dedicated() && !is_in_debugger() {
            crate::pl::error_dialog(full_c.as_ptr());
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
        c::fputs(text, c::stdout_stream());
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

        c::exit(0)
    }
}

/// C: `const char *Sys_ConsoleInput (void)`
///
/// # Safety
/// Main thread of a dedicated server.
pub unsafe fn console_input() -> *const c_char {
    static mut CON_TEXT: [c_char; 256] = [0; 256];
    static mut TEXTLEN: usize = 0;

    // SAFETY: caller contract -- main thread only, so the statics are not
    // shared; `set`/`timeout` are plain descriptor-set values.
    unsafe {
        let con_text = addr_of_mut!(CON_TEXT).cast::<c_char>();
        let mut ch: c_char = 0;
        let mut set: libc::fd_set = zeroed();
        let mut timeout = libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        };

        libc::FD_ZERO(&mut set);
        libc::FD_SET(0, &mut set); // stdin

        while libc::select(1, &mut set, ptr::null_mut(), ptr::null_mut(), &mut timeout) != 0 {
            if libc::read(0, addr_of_mut!(ch).cast(), 1) != 1 {
                return ptr::null();
            }
            if ch == b'\n' as c_char || ch == b'\r' as c_char {
                *con_text.add(TEXTLEN) = 0;
                TEXTLEN = 0;
                return con_text;
            } else if ch == 8 {
                if TEXTLEN != 0 {
                    TEXTLEN -= 1;
                    *con_text.add(TEXTLEN) = 0;
                }
                continue;
            }
            *con_text.add(TEXTLEN) = ch;
            TEXTLEN += 1;
            if TEXTLEN < 256 {
                *con_text.add(TEXTLEN) = 0;
            } else {
                // buffer is full
                TEXTLEN = 0;
                *con_text = 0;
                sys_printf("\nConsole input too long!\n");
                break;
            }
        }
    }

    ptr::null()
}

/// C: `bool Sys_PinCurrentThread (int core_index)` -- the
/// `pthread_setaffinity_np` arm (`PLATFORM_UNIX` less `PLATFORM_OSX` /
/// `PLATFORM_BSD`; the Meson `TASK_AFFINITY_NOT_AVAILABLE` fallback only
/// fires when `sched.h` lacks `CPU_ZERO`, which glibc/musl never do).
///
/// # Safety
/// Any thread.
#[cfg(target_os = "linux")]
pub unsafe fn pin_current_thread(core_index: c_int) -> bool {
    // valid for *Nix with GNU pthread extension pthread_setaffinity_np()
    //  which apparently is not available on OSX so skip it in that case.
    // (an index past the 1024-bit `cpu_set_t` is undefined for the C
    // `CPU_SET`; the libc crate's would panic, so it is refused instead)
    if core_index < 0 || core_index as usize >= 8 * core::mem::size_of::<libc::cpu_set_t>() {
        return false;
    }

    // SAFETY: plain pthread calls on the calling thread's own handle.
    unsafe {
        let mut cpuset: libc::cpu_set_t = zeroed();
        libc::CPU_ZERO(&mut cpuset);
        libc::CPU_SET(core_index as usize, &mut cpuset);

        let current_thread = libc::pthread_self();
        if libc::pthread_setaffinity_np(
            current_thread,
            core::mem::size_of::<libc::cpu_set_t>(),
            &cpuset,
        ) != 0
        {
            return false;
        }
    }

    true
}

/// C: `bool Sys_PinCurrentThread (int core_index)` -- the arm without
/// `pthread_setaffinity_np`.
///
/// # Safety
/// Any thread.
#[cfg(not(target_os = "linux"))]
pub unsafe fn pin_current_thread(_core_index: c_int) -> bool {
    false
}

/// C: `const char *Sys_StackTrace (void)` -- a `Mem_Alloc`ed buffer the
/// caller `Mem_Free`s (empty rather than null when no frame was captured).
///
/// # Safety
/// Any thread.
pub unsafe fn stack_trace() -> *const c_char {
    let mut output_buffer: Vec<u8> = Vec::new();

    let mut buffer = [ptr::null_mut::<c_void>(); MAX_STACK_FRAMES];

    // SAFETY: `buffer` holds `MAX_STACK_FRAMES` slots; `backtrace_symbols`
    // returns one libc-allocated array (or null) freed below.
    unsafe {
        let nb_frames =
            libc::backtrace(buffer.as_mut_ptr(), MAX_STACK_FRAMES as c_int).max(0) as usize;

        #[cfg(target_os = "macos")]
        {
            // display on 1 line to pass to atos easily on MacOS:
            for (frame_index, &frame) in buffer.iter().enumerate().take(nb_frames) {
                let _ = write!(output_buffer, "0x{:x} ", frame as usize);

                if frame_index == nb_frames - 1 {
                    output_buffer.push(b'\n');
                }
            }
        }
        // Then print 1 frame per line, together with its symbol using backtrace_symbols()
        let symbols = libc::backtrace_symbols(buffer.as_ptr(), nb_frames as c_int);

        for frame_index in 0..nb_frames {
            let symbol = if symbols.is_null() {
                ptr::null_mut()
            } else {
                *symbols.add(frame_index)
            };
            let _ = write!(output_buffer, "{frame_index:<2}: ");
            if symbol.is_null() {
                output_buffer.extend_from_slice(b"[no symbols]");
            } else {
                output_buffer.extend_from_slice(CStr::from_ptr(symbol).to_bytes());
            }
            output_buffer.push(b'\n');
        }

        libc::free(symbols.cast());
    }

    mem_strdup(&output_buffer)
}

/// C: `bool Sys_IsInDebugger (void)` -- the `/proc/self/status`
/// `TracerPid:` read.
///
/// # Safety
/// Any thread.
#[cfg(not(target_os = "macos"))]
pub unsafe fn is_in_debugger() -> bool {
    use std::io::BufRead as _;

    let Ok(f) = std::fs::File::open("/proc/self/status") else {
        return false;
    };

    for line in std::io::BufReader::new(f).split(b'\n') {
        let Ok(line) = line else {
            break;
        };
        if let Some(rest) = line.strip_prefix(b"TracerPid:") {
            // atoi (line + 10)
            let rest = String::from_utf8_lossy(rest);
            let digits: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let pid: i64 = digits.parse().unwrap_or(0);
            return pid != 0;
        }
    }

    false
}

/// C: `bool Sys_IsInDebugger (void)` -- the `PLATFORM_OSX` `sysctl` arm.
///
/// # Safety
/// Any thread.
#[cfg(target_os = "macos")]
pub unsafe fn is_in_debugger() -> bool {
    // `struct kinfo_proc` (sys/sysctl.h) is not in the libc crate: it is
    // 648 bytes with `kp_proc.p_flag` at byte 32 (after the 16-byte `p_un`
    // union and the `p_vmspace` / `p_sigacts` pointers), so the query gets
    // an oversized aligned buffer and the flag is read at that offset.
    const P_TRACED: c_int = 0x0000_0800;
    const P_FLAG_OFFSET: usize = 32;

    let mut info = [0u64; 128];
    let mut size = core::mem::size_of::<[u64; 128]>();

    // SAFETY: `mib` has four entries and `info` is `size` bytes.
    unsafe {
        let mut mib = [
            libc::CTL_KERN,
            libc::KERN_PROC,
            libc::KERN_PROC_PID,
            libc::getpid(),
        ];

        if libc::sysctl(
            mib.as_mut_ptr(),
            4,
            info.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        ) == -1
        {
            return false;
        }

        let p_flag = ptr::read_unaligned(
            info.as_ptr()
                .cast::<u8>()
                .add(P_FLAG_OFFSET)
                .cast::<c_int>(),
        );
        (p_flag & P_TRACED) != 0
    }
}

/// C: `void Sys_DebugBreak (void)`
///
/// # Safety
/// Any thread.
pub unsafe fn debug_break() {
    // SAFETY: caller contract.
    unsafe {
        if is_in_debugger() {
            libc::raise(libc::SIGTRAP);
        }
    }
}
