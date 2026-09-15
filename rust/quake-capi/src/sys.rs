//! `Sys_*` exports over `quake_platform::sys` (Rust migration Phase 9 M5,
//! ADR-017/ADR-011). Built under the `platform` feature with whichever SDL
//! arm Meson selected; `Quake/sys_glue.c` keeps the variadic `Sys_Error` /
//! `Sys_Printf` (formatted in C, task plan I5) and wraps the entry points
//! that can raise through `Host_Shutdown` (`Sys_Error` on unix, `Sys_Quit`)
//! or the input pump (`Sys_SendKeyEvents`) in `Host_Reraise` (ADR-009), so
//! those are exported as status-returning `quake_rs_sys_*` cores. Everything
//! else in `sys.h` / the `steam.h` pair cannot raise and is exported under
//! its C name directly. `isDedicated` (`sys_sdl_win.c:42` /
//! `sys_sdl_unix.c:48`) moves here with the files that defined it.

use core::ffi::{c_char, c_int, c_ulong, c_void};

use quake_c_sys::{
    findfile_t, qboolean, qfileofs_t, qfilesize_t, quakeflavor_t, steamgame_t, FILE,
};
use quake_platform::sys as backend;
use quake_platform::sys::handles;

/// C: `qboolean isDedicated;` -- storage for `quakedef.h:505`'s extern.
#[no_mangle]
pub static mut isDedicated: qboolean = false;

/* ---------------------------------------------------------------------------
 * Status-returning cores (Quake/sys_glue.c re-raises).
 */

/// C: `void Sys_Error (const char *error, ...)` after `sys_glue.c` formatted
/// `text` into a `Mem_Alloc`ed buffer this call owns. Exits the process;
/// returns only with the status of a raise the guarded `Host_Shutdown`
/// caught (unix), which the wrapper re-issues.
///
/// # Safety
/// `text` is a `Mem_Alloc`ed NUL-terminated string. Any thread.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sys_error(text: *mut c_char) -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::error_core(text) }
}

/// C: `void Sys_Printf (const char *fmt, ...)` after formatting.
///
/// # Safety
/// `text` is a NUL-terminated string. Any thread.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sys_printf(text: *const c_char) {
    // SAFETY: forwarded contract
    unsafe { backend::print_text(text) }
}

/// C: `void Sys_Quit (void)` -- the status-returning core.
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sys_quit() -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::quit_core() }
}

/// C: `void Sys_SendKeyEvents (void)` -- the status-returning core.
///
/// # Safety
/// Main thread, after `IN_Init`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sys_send_key_events() -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::send_key_events() }
}

/* ---------------------------------------------------------------------------
 * sys_sdl.c: the handle table and SDL helpers.
 */

/// C: `void Sys_FileInit (void)`
///
/// # Safety
/// Once, before any handle call.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileInit() {
    // SAFETY: forwarded contract
    unsafe { handles::file_init() }
}

/// C: `qfilesize_t Sys_filelength (FILE *f)`
///
/// # Safety
/// `f` is an open stream.
#[no_mangle]
pub unsafe extern "C" fn Sys_filelength(f: *mut FILE) -> qfilesize_t {
    // SAFETY: forwarded contract
    unsafe { handles::filelength(f) }
}

/// C: `qfilesize_t Sys_FileOpenRead (const char *path, int *hndl)`
///
/// # Safety
/// `path` is a NUL-terminated string; `hndl` is writable.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileOpenRead(path: *const c_char, hndl: *mut c_int) -> qfilesize_t {
    // SAFETY: forwarded contract
    unsafe { handles::file_open_read(path, hndl) }
}

/// C: `void Sys_MemFileOpenRead (const byte *memory, qfilesize_t size, int *hndl)`
///
/// # Safety
/// `memory` is readable for `size` bytes for the handle's lifetime; `hndl`
/// is writable.
#[no_mangle]
pub unsafe extern "C" fn Sys_MemFileOpenRead(
    memory: *const u8,
    size: qfilesize_t,
    hndl: *mut c_int,
) {
    // SAFETY: forwarded contract
    unsafe { handles::mem_file_open_read(memory, size, hndl) }
}

/// C: `int Sys_DuplicateHandle (int handle)`
///
/// # Safety
/// `handle` is an open read handle.
#[no_mangle]
pub unsafe extern "C" fn Sys_DuplicateHandle(handle: c_int) -> c_int {
    // SAFETY: forwarded contract
    unsafe { handles::duplicate_handle(handle) }
}

/// C: `int Sys_FileOpenWrite (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileOpenWrite(path: *const c_char) -> c_int {
    // SAFETY: forwarded contract
    unsafe { handles::file_open_write(path) }
}

/// C: `qfileofs_t Sys_FilePos (int handle)`
///
/// # Safety
/// `handle` is an open handle.
#[no_mangle]
pub unsafe extern "C" fn Sys_FilePos(handle: c_int) -> qfileofs_t {
    // SAFETY: forwarded contract
    unsafe { handles::file_pos(handle) }
}

/// C: `void Sys_FileClose (int handle)`
///
/// # Safety
/// `handle` is an open handle.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileClose(handle: c_int) {
    // SAFETY: forwarded contract
    unsafe { handles::file_close(handle) }
}

/// C: `int Sys_FileSeek (int handle, qfileofs_t position)`
///
/// # Safety
/// `handle` is an open handle.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileSeek(handle: c_int, position: qfileofs_t) -> c_int {
    // SAFETY: forwarded contract
    unsafe { handles::file_seek(handle, position) }
}

/// C: `bool Sys_feof (int handle)`
///
/// # Safety
/// `handle` is an open read handle.
#[no_mangle]
pub unsafe extern "C" fn Sys_feof(handle: c_int) -> bool {
    // SAFETY: forwarded contract
    unsafe { handles::feof(handle) }
}

/// C: `int Sys_FileRead (int handle, void *dest, int count)`
///
/// # Safety
/// `handle` is an open read handle; `dest` is writable for `count` bytes.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileRead(handle: c_int, dest: *mut c_void, count: c_int) -> c_int {
    // SAFETY: forwarded contract
    unsafe { handles::file_read(handle, dest, count) }
}

/// C: `int Sys_fgetc (int handle)`
///
/// # Safety
/// `handle` is an open read handle.
#[no_mangle]
pub unsafe extern "C" fn Sys_fgetc(handle: c_int) -> c_int {
    // SAFETY: forwarded contract
    unsafe { handles::fgetc(handle) }
}

/// C: `int Sys_FileWrite (int handle, const void *data, int count)`
///
/// # Safety
/// `handle` is an open write handle; `data` is readable for `count` bytes.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileWrite(handle: c_int, data: *const c_void, count: c_int) -> c_int {
    // SAFETY: forwarded contract
    unsafe { handles::file_write(handle, data, count) }
}

/// C: `int Sys_SelectFolder (const char *title, const char *default_location, char *dst, size_t dstsize)`
///
/// # Safety
/// `title` is NUL-terminated, `default_location` NUL-terminated or null,
/// `dst` writable for `dstsize` bytes. Main thread.
#[cfg(feature = "sdl3")]
#[no_mangle]
pub unsafe extern "C" fn Sys_SelectFolder(
    title: *const c_char,
    default_location: *const c_char,
    dst: *mut c_char,
    dstsize: usize,
) -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::select_folder(title, default_location, dst, dstsize) }
}

/// C: `char *Sys_GetPrefPath (const char *org, const char *app)`
///
/// # Safety
/// `org`/`app` are NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetPrefPath(org: *const c_char, app: *const c_char) -> *mut c_char {
    // SAFETY: forwarded contract
    unsafe { backend::get_pref_path(org, app) }
}

/// C: `void Sys_MessageBoxWarning (const char *title, const char *message)`
///
/// # Safety
/// `title`/`message` are NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn Sys_MessageBoxWarning(title: *const c_char, message: *const c_char) {
    // SAFETY: forwarded contract
    unsafe { backend::message_box_warning(title, message) }
}

/// C: `FUNC_NORETURN void Sys_QuitNoShutdown (void)`
#[no_mangle]
pub extern "C" fn Sys_QuitNoShutdown() -> ! {
    backend::quit_no_shutdown()
}

/// C: `quakeflavor_t ChooseQuakeFlavor (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn ChooseQuakeFlavor() -> quakeflavor_t {
    // SAFETY: forwarded contract
    unsafe { backend::choose_quake_flavor() }
}

/// C: `double Sys_DoubleTime (void)`
///
/// # Safety
/// After `Sys_Init`.
#[no_mangle]
pub unsafe extern "C" fn Sys_DoubleTime() -> f64 {
    // SAFETY: forwarded contract
    unsafe { backend::double_time() }
}

/// C: `void Sys_Sleep (unsigned long msecs)`
#[no_mangle]
pub extern "C" fn Sys_Sleep(msecs: c_ulong) {
    backend::sleep(msecs)
}

/* ---------------------------------------------------------------------------
 * sys_sdl_win.c / sys_sdl_unix.c.
 */

/// C: `int Sys_fseek (FILE *file, qfileofs_t ofs, int origin)`
///
/// # Safety
/// `file` is an open stream.
#[no_mangle]
pub unsafe extern "C" fn Sys_fseek(file: *mut FILE, ofs: qfileofs_t, origin: c_int) -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::fseek(file, ofs, origin) }
}

/// C: `qfileofs_t Sys_ftell (FILE *file)`
///
/// # Safety
/// `file` is an open stream.
#[no_mangle]
pub unsafe extern "C" fn Sys_ftell(file: *mut FILE) -> qfileofs_t {
    // SAFETY: forwarded contract
    unsafe { backend::ftell(file) }
}

/// C: `int Sys_FileType (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn Sys_FileType(path: *const c_char) -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::file_type(path) }
}

/// C: `FILE *Sys_fopen (const char *path, const char *mode)`
///
/// # Safety
/// `path` and `mode` are NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn Sys_fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    // SAFETY: forwarded contract
    unsafe { backend::fopen(path, mode) }
}

/// C: `void Sys_mkdir (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn Sys_mkdir(path: *const c_char) {
    // SAFETY: forwarded contract
    unsafe { backend::mkdir(path) }
}

/// C: `qboolean Sys_Explore (const char *path)`
///
/// # Safety
/// `path` is a NUL-terminated string. Main thread.
#[no_mangle]
pub unsafe extern "C" fn Sys_Explore(path: *const c_char) -> qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::explore(path) }
}

/// C: `qboolean Sys_GetSteamDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetSteamDir(path: *mut c_char, pathsize: usize) -> qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::get_steam_dir(path, pathsize) }
}

/// C: `qboolean Sys_GetSteamAPILibraryPath (char *path, size_t pathsize, const steamgame_t *game)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes; `game` is a valid entry.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetSteamAPILibraryPath(
    path: *mut c_char,
    pathsize: usize,
    game: *const steamgame_t,
) -> qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::get_steam_api_library_path(path, pathsize, game) }
}

/// C: `qboolean Sys_GetGOGQuakeDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetGOGQuakeDir(path: *mut c_char, pathsize: usize) -> qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::get_gog_quake_dir(path, pathsize) }
}

/// C: `qboolean Sys_GetGOGQuakeEnhancedDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetGOGQuakeEnhancedDir(
    path: *mut c_char,
    pathsize: usize,
) -> qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::get_gog_quake_enhanced_dir(path, pathsize) }
}

/// C: `qboolean Sys_GetEGSManifestDir (char *path, size_t pathsize)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetEGSManifestDir(path: *mut c_char, pathsize: usize) -> qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::get_egs_manifest_dir(path, pathsize) }
}

/// C: `const char *Sys_GetEGSLauncherData (void)` -- `Mem_Alloc`ed, caller
/// frees.
///
/// # Safety
/// Any thread.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetEGSLauncherData() -> *const c_char {
    // SAFETY: forwarded contract
    unsafe { backend::get_egs_launcher_data() }
}

/// C: `qboolean Sys_GetNightdiveUserDir (char *path, size_t pathsize, const char *steamlibrary)`
///
/// # Safety
/// `path` is writable for `pathsize` bytes; `steamlibrary` NUL-terminated
/// or null.
#[no_mangle]
pub unsafe extern "C" fn Sys_GetNightdiveUserDir(
    path: *mut c_char,
    pathsize: usize,
    steamlibrary: *const c_char,
) -> qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::get_nightdive_user_dir(path, pathsize, steamlibrary) }
}

/// C: `findfile_t *Sys_FindFirst (const char *dir, const char *ext)`
///
/// # Safety
/// `dir` is a NUL-terminated string; `ext` NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn Sys_FindFirst(dir: *const c_char, ext: *const c_char) -> *mut findfile_t {
    // SAFETY: forwarded contract
    unsafe { backend::find_first(dir, ext) }
}

/// C: `findfile_t *Sys_FindNext (findfile_t *find)`
///
/// # Safety
/// `find` came from `Sys_FindFirst` and has not been closed.
#[no_mangle]
pub unsafe extern "C" fn Sys_FindNext(find: *mut findfile_t) -> *mut findfile_t {
    // SAFETY: forwarded contract
    unsafe { backend::find_next(find) }
}

/// C: `void Sys_FindClose (findfile_t *find)`
///
/// # Safety
/// `find` came from `Sys_FindFirst` and has not been closed, or is null.
#[no_mangle]
pub unsafe extern "C" fn Sys_FindClose(find: *mut findfile_t) {
    // SAFETY: forwarded contract
    unsafe { backend::find_close(find) }
}

/// C: `void Sys_Init (void)`
///
/// # Safety
/// Once, from `main`, after `host_parms` is set.
#[no_mangle]
pub unsafe extern "C" fn Sys_Init() {
    // SAFETY: forwarded contract
    unsafe { backend::init() }
}

/// C: `const char *Sys_ConsoleInput (void)`
///
/// # Safety
/// Main thread of a dedicated server.
#[no_mangle]
pub unsafe extern "C" fn Sys_ConsoleInput() -> *const c_char {
    // SAFETY: forwarded contract
    unsafe { backend::console_input() }
}

/// C: `bool Sys_PinCurrentThread (int core_index)`
///
/// # Safety
/// Any thread.
#[no_mangle]
pub unsafe extern "C" fn Sys_PinCurrentThread(core_index: c_int) -> bool {
    // SAFETY: forwarded contract
    unsafe { backend::pin_current_thread(core_index) }
}

/// C: `const char *Sys_StackTrace (void)` -- `Mem_Alloc`ed, caller frees.
///
/// # Safety
/// Any thread.
#[no_mangle]
pub unsafe extern "C" fn Sys_StackTrace() -> *const c_char {
    // SAFETY: forwarded contract
    unsafe { backend::stack_trace() }
}

/// C: `bool Sys_IsInDebugger (void)`
///
/// # Safety
/// Any thread.
#[no_mangle]
pub unsafe extern "C" fn Sys_IsInDebugger() -> bool {
    // SAFETY: forwarded contract
    unsafe { backend::is_in_debugger() }
}

/// C: `void Sys_DebugBreak (void)`
///
/// # Safety
/// Any thread.
#[no_mangle]
pub unsafe extern "C" fn Sys_DebugBreak() {
    // SAFETY: forwarded contract
    unsafe { backend::debug_break() }
}
