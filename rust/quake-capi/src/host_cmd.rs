//! `Quake/host_cmd.c` -- the console command surface, the map/mod/demo/save
//! file lists and the savegame reader/writer (Rust migration Phase 7 M8,
//! T8.3).
//!
//! Compiled instead of `host_cmd.c` under `-Duse_rust_host`, the same
//! Pattern A whole-file swap `host.c` took at T8.2. `Quake/host_cmd_glue.c`
//! owns the C-visible data and the ADR-009 guarded seams; this module owns
//! every behavioural body.
//!
//! ADR-007: no dual-view row opens or closes here. All seven C-visible data
//! objects `host_cmd.c` defined -- `current_skill`, `noclip_anglehack`,
//! `extralevels`, `extralevels_sorted`, `modlist`, `demolist`, `savelist` --
//! keep C storage in the glue, because `pr_edict.c`, `sv_main.c`,
//! `cl_parse.c`, `in_sdl.c`, `view.c`, `console.c` and `menu.c` all resolve
//! them by name, and three of those translation units are already Rust
//! (`sv_main.rs`, `cl_parse.rs`, `view.rs`) with `extern "C"` declarations
//! that would collide with a Rust definition -- the M5 defect MSVC merges
//! silently and Linux hard-errors. Only the three file-`static`s
//! (`maxlevelnamelen`, `extralevels_parsing_thread`,
//! `extralevels_cancel_parsing`) and `RightPad`'s function-scope buffer had
//! internal linkage; of those, the thread handle and the atomic stay C
//! because `atomics.h`'s accessors are `static inline` with
//! compiler-specific barriers that must not be re-derived in Rust.
//!
//! ADR-016: `ExtraMaps_ParseDescriptions` is the body of a `QThread_Create`
//! worker. It stays reachable as a plain `extern "C"` function pointer and
//! touches no Rust thread-local, no ambient qcvm and no `Host_Guard` frame;
//! the thread creation, join and cancel flag remain C in the glue.
//!
//! ADR-005: `Host_Savegame_f` and `Host_Loadgame_f` are byte-diff subjects.
//! Neither body contains a `%g` or `%e` specifier -- the five `%g` a
//! savegame carries are formatted inside `gl_fog.c` and `gl_sky.c`, which
//! are out of Phase 7 scope; this module calls `Fog_GetFogCommand` and
//! `Sky_GetSkyCommand` through capi and emits the returned string opaquely
//! through `%s`, never re-deriving those numbers.

#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::host_cmd as g;
use quake_types::host::{Client, ClientState, ClientStatic, FileListItem, Server, ServerStatic};

use crate::cl_main::{cl, cls};
use crate::sv_main::{sv, svs};

// ---------------------------------------------------------------------------
// ADR-009 plumbing. Identical to `host.rs`: a `Host_Guard` status of 0 =
// returned normally, 1 = `Host_Error` / `Host_EndGame`, 2 = `screen_error`.

/// A `Host_Guard` status.
pub(crate) type Raise = c_int;

macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

// ---------------------------------------------------------------------------
// shared accessors, mirroring `host.rs:178-256`.

#[inline]
fn sv_p() -> *mut Server {
    ptr::addr_of_mut!(sv).cast::<Server>()
}

#[inline]
fn svs_p() -> *mut ServerStatic {
    ptr::addr_of_mut!(svs).cast::<ServerStatic>()
}

#[inline]
fn cl_p() -> *mut ClientState {
    ptr::addr_of_mut!(cl).cast::<ClientState>()
}

#[inline]
fn cls_p() -> *mut ClientStatic {
    ptr::addr_of_mut!(cls).cast::<ClientStatic>()
}

// `host.c`'s `host_client`, which `host_cmd.c` reads and writes. Typed with
// the ADR-011 mirror, so it is declared here rather than in `quake-c-sys`
// (the `sv_main.rs:103` / `sv_send.rs:121` precedent).
extern "C" {
    static mut host_client: *mut Client;
}

#[inline]
unsafe fn host_client_get() -> *mut Client {
    // SAFETY: engine global, single-threaded.
    unsafe { ptr::addr_of!(host_client).read() }
}

#[inline]
unsafe fn host_client_set(v: *mut Client) {
    // SAFETY: engine global, single-threaded.
    unsafe { ptr::addr_of_mut!(host_client).write(v) }
}

/// `cvar_t::value` without taking a reference to the C-owned storage.
#[inline]
unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: caller passes a live engine cvar.
    unsafe { ptr::addr_of!((*var).value).read() }
}

// ===========================================================================
// CHUNK A
// ===========================================================================

// ===========================================================================
// ==== CHUNK A (host_cmd.c:24-898) ====
//
// The include/extern header block, `Host_Quit_f`, the `filelist_item_t`
// primitives, the extramaps subsystem (including the ADR-016 background
// parsing worker), the mod list, the demo and savegame lists, and the three
// listing commands `maps`, `mods` and `mapname`.
//
// ADR-009 audit for this chunk. Guarded, because each can reach
// `Host_Error`/`Host_EndGame`: `M_Menu_Quit_f` (:56, via `Cbuf_AddText`),
// `CL_Disconnect` (:59), `Host_ShutdownServer` (:60 -- a re-raising thunk
// under `-Duse_rust_host`), `Sys_Quit` (:62, which runs `Host_Shutdown`
// before `exit`) and `COM_LoadFile` (:795, the `host_glue.c:683` precedent).
// Everything else is plain: `Con_Printf`/`Con_SafePrintf` (the
// `cl_demo_glue.c:47-51` convention), `Mem_Alloc`/`Mem_Realloc`/`Mem_Free`,
// `Sys_GetPrefPath`, `Sys_FindFirst`/`Sys_FindNext`, `Sys_FileType`,
// `Sys_Quit`'s siblings, `COM_GetGameNames`, `COM_StripExtension`,
// `COM_FileGetExtension`, `COM_TintSubstring`, `COM_ModForbiddenChars`,
// `COM_LoadMallocFile_TextMode_OSPath`, `LOC_GetRawString`, `Cmd_Argc`,
// `Cmd_Argv`, `q_strcasecmp`, `q_strcasestr`, `q_strtrim`, the `JSON_*` four
// (already Rust, `crate::json`) and `Mod_LoadMapDescription` -- the last of
// which runs only on the parsing worker, where ADR-016 forbids a `Host_Guard`
// frame and where the C build has no `setjmp` target either.
//
// ADR-005: the only specifiers this chunk emits are `%s`, `%c` and `%i`
// (`Host_Maps_f`, `Host_Mods_f`, `Host_Mapname_f`), all through C variadics.
// There is no `%g`/`%e`/`%a` and nothing formats through the Rust formatter.
// The six `q_snprintf` sites build paths from `%s` only and are reproduced by
// [`SnBuf`], which matches `vsnprintf` truncation and its return value.

use core::ffi::CStr;

use quake_types::host::{
    MAPTYPE_BMODEL, MAPTYPE_COUNT, MAPTYPE_CUSTOM_ID_DM, MAPTYPE_CUSTOM_ID_END,
    MAPTYPE_CUSTOM_ID_LEVEL, MAPTYPE_CUSTOM_ID_START, MAPTYPE_CUSTOM_MOD_DM,
    MAPTYPE_CUSTOM_MOD_END, MAPTYPE_CUSTOM_MOD_LEVEL, MAPTYPE_CUSTOM_MOD_START, MAPTYPE_ID_DM,
    MAPTYPE_ID_END, MAPTYPE_ID_EP1_LEVEL, MAPTYPE_ID_LEVEL, MAPTYPE_ID_START, MAPTYPE_MOD_START,
};
use quake_types::json::{Json, JsonEntry, JSON_ARRAY};

// ---------------------------------------------------------------------------
// engine constants

/// `host_cmd.c:45` -- `MIN_BSP_MAP_SIZE`.
const MIN_BSP_MAP_SIZE: c_int = 32 * 1024;
/// `quakedef.h:36` -- `GAMENAME`.
const GAMENAME: &CStr = c"id1";
/// `keys.h:137-142` -- `key_console`, the second `keydest_t` enumerator.
const KEY_CONSOLE: c_int = 1;
/// `common.h:446-447` -- `FS_ENT_FILE` / `FS_ENT_DIRECTORY`.
const FS_ENT_FILE: c_int = 1 << 0;
const FS_ENT_DIRECTORY: c_int = 1 << 1;
/// `q_types.h:240` -- `MAX_QPATH`; `sys.h:110-114` -- `MAX_OSPATH`.
const MAX_OSPATH: usize = c::MAX_OSPATH;

// ---------------------------------------------------------------------------
// Rust-owned storage: the file-`static`s of this chunk that had internal
// linkage in C. The three that stay C (`extralevels_parsing_thread`,
// `extralevels_cancel_parsing` and the four C-visible list heads) are reached
// through `quake-c-sys`.

/// `host_cmd.c:195` -- `static size_t maxlevelnamelen;`.
static mut MAXLEVELNAMELEN: usize = 0;

/// `host_cmd.c:178` -- `RightPad`'s function-scope `static char buf[1024]`.
static mut RIGHTPAD_BUF: [c_char; 1024] = [0; 1024];

/// `host_cmd.c:576-586` -- `knownmods`. Held as `&CStr` rather than raw
/// pointers so the table itself is `Sync`; `Modlist_GetFullName` hands the
/// `'static` payload straight back to C, exactly as the C table does.
static KNOWNMODS: [[&CStr; 2]; 9] = [
    [c"id1", c"Quake"],
    [c"hipnotic", c"Scourge of Armagon"],
    [c"rogue", c"Dissolution of Eternity"],
    [c"dopa", c"Dimension of the Past"],
    [c"mg1", c"Dimension of the Machine"],
    [c"q64", c"Quake (Nintendo 64)"],
    [c"ctf", c"Capture The Flag"],
    [c"udob", c"Underdark Overbright"],
    [c"ad", c"Arcane Dimensions"],
];

// ---------------------------------------------------------------------------
// string helpers. These reproduce the libc and `q_ctype.h` primitives this
// chunk uses; `q_ctype.h`'s are `static inline` and have no linkable symbol.

/// `strlen`.
unsafe fn c_strlen(s: *const c_char) -> usize {
    // SAFETY: caller passes a NUL-terminated buffer.
    unsafe {
        let mut n = 0usize;
        while *s.add(n) != 0 {
            n += 1;
        }
        n
    }
}

/// `strcmp (a, b) == 0`.
unsafe fn c_streq(a: *const c_char, b: *const c_char) -> bool {
    // SAFETY: both operands are NUL-terminated.
    unsafe {
        let mut i = 0usize;
        loop {
            let (x, y) = (*a.add(i), *b.add(i));
            if x != y {
                return false;
            }
            if x == 0 {
                return true;
            }
            i += 1;
        }
    }
}

/// `strncmp (a, b, n) == 0`.
unsafe fn c_strneq(a: *const c_char, b: *const c_char, n: usize) -> bool {
    // SAFETY: both operands are readable for `n` bytes or up to a NUL.
    unsafe {
        let mut i = 0usize;
        while i < n {
            let (x, y) = (*a.add(i), *b.add(i));
            if x != y {
                return false;
            }
            if x == 0 {
                return true;
            }
            i += 1;
        }
        true
    }
}

/// `memcmp (p, lit, lit.len ()) == 0`.
unsafe fn c_memeq(p: *const c_char, lit: &[u8]) -> bool {
    // SAFETY: callers check the length before calling, as the C does.
    unsafe {
        for (i, &b) in lit.iter().enumerate() {
            if *p.add(i) != b as c_char {
                return false;
            }
        }
        true
    }
}

/// `strchr (s, ch)`.
unsafe fn c_strchr(s: *const c_char, ch: c_char) -> *mut c_char {
    // SAFETY: `s` is NUL-terminated.
    unsafe {
        let mut i = 0usize;
        loop {
            let x = *s.add(i);
            if x == ch {
                return s.add(i) as *mut c_char;
            }
            if x == 0 {
                return ptr::null_mut();
            }
            i += 1;
        }
    }
}

/// `strstr (haystack, needle) != NULL`.
unsafe fn c_strstr_found(haystack: *const c_char, needle: *const c_char) -> bool {
    // SAFETY: both operands are NUL-terminated.
    unsafe {
        let nl = c_strlen(needle);
        if nl == 0 {
            return true;
        }
        let hl = c_strlen(haystack);
        if hl < nl {
            return false;
        }
        let mut i = 0usize;
        while i + nl <= hl {
            if c_strneq(haystack.add(i), needle, nl) {
                return true;
            }
            i += 1;
        }
        false
    }
}

/// `strcpy (dst, src)` -- unbounded, as `FileList_Init` uses it.
unsafe fn c_strcpy(dst: *mut c_char, src: *const c_char) {
    // SAFETY: the C contract; `dst` is large enough for `src`.
    unsafe {
        let mut i = 0usize;
        loop {
            let ch = *src.add(i);
            *dst.add(i) = ch;
            if ch == 0 {
                return;
            }
            i += 1;
        }
    }
}

/// `common.c` `q_strlcpy` -- truncating bounded copy, always NUL-terminated.
unsafe fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) {
    // SAFETY: callers pass a fixed-size destination array and its length.
    unsafe {
        if size == 0 {
            return;
        }
        let mut i = 0usize;
        while i + 1 < size {
            let ch = *src.add(i);
            if ch == 0 {
                break;
            }
            *dst.add(i) = ch;
            i += 1;
        }
        *dst.add(i) = 0;
    }
}

/// `q_ctype.h:63` -- `q_isspace`.
fn q_isspace(ch: c_int) -> c_int {
    match ch {
        0x20 | 0x09 | 0x0a | 0x0d | 0x0c | 0x0b => 1,
        _ => 0,
    }
}

/// `q_ctype.h:93` -- `q_tolower`, via `q_isupper` (`q_ctype.h:34`).
fn q_tolower(ch: c_int) -> c_int {
    if ch >= b'A' as c_int && ch <= b'Z' as c_int {
        ch | (b'a' as c_int - b'A' as c_int)
    } else {
        ch
    }
}

/// A truncating byte writer over a fixed C buffer, reproducing `q_snprintf`
/// exactly: at most `size - 1` payload bytes, always NUL-terminated when
/// `size != 0`, and a return value of the length the call *would* have
/// produced (`common.c:617-631`). The six `q_snprintf` sites in this chunk
/// interpolate `%s` only, so this replaces the formatter entirely and
/// ADR-005's `%g`/`%e` panic stays unreachable.
struct SnBuf {
    p: *mut c_char,
    size: usize,
    used: usize,
    want: usize,
}

impl SnBuf {
    /// # Safety
    /// `p` must point at `size` writable bytes.
    unsafe fn new(p: *mut c_char, size: usize) -> Self {
        if size != 0 {
            // SAFETY: `size != 0`, so byte 0 is in bounds.
            unsafe { *p = 0 };
        }
        SnBuf {
            p,
            size,
            used: 0,
            want: 0,
        }
    }

    fn push(&mut self, ch: c_char) {
        self.want += 1;
        if self.size == 0 || self.used + 1 >= self.size {
            return;
        }
        // SAFETY: bounded by the check above; the NUL always fits after it.
        unsafe {
            *self.p.add(self.used) = ch;
            self.used += 1;
            *self.p.add(self.used) = 0;
        }
    }

    fn lit(&mut self, s: &[u8]) {
        for &b in s {
            self.push(b as c_char);
        }
    }

    /// `%s`
    ///
    /// # Safety
    /// `s` must be NUL-terminated.
    unsafe fn cstr(&mut self, s: *const c_char) {
        // SAFETY: caller contract.
        unsafe {
            let mut i = 0usize;
            while *s.add(i) != 0 {
                self.push(*s.add(i));
                i += 1;
            }
        }
    }

    /// `q_snprintf`'s return value.
    fn finish(&self) -> c_int {
        self.want as c_int
    }
}

// ---------------------------------------------------------------------------
// C-owned data accessors (ADR-007: all four list heads keep C storage).

#[inline]
fn extralevels_p() -> *mut *mut FileListItem {
    ptr::addr_of_mut!(g::extralevels).cast::<*mut FileListItem>()
}

#[inline]
fn modlist_p() -> *mut *mut FileListItem {
    ptr::addr_of_mut!(g::modlist).cast::<*mut FileListItem>()
}

#[inline]
fn demolist_p() -> *mut *mut FileListItem {
    ptr::addr_of_mut!(g::demolist).cast::<*mut FileListItem>()
}

#[inline]
fn savelist_p() -> *mut *mut FileListItem {
    ptr::addr_of_mut!(g::savelist).cast::<*mut FileListItem>()
}

#[inline]
unsafe fn extralevels_sorted_get() -> *mut *mut FileListItem {
    // SAFETY: engine global, read on the main thread and on the parsing
    // worker after `ExtraMaps_Init` published it (as in C).
    unsafe {
        ptr::addr_of!(g::extralevels_sorted)
            .read()
            .cast::<*mut FileListItem>()
    }
}

#[inline]
unsafe fn extralevels_sorted_set(v: *mut *mut FileListItem) {
    // SAFETY: engine global, written on the main thread only.
    unsafe { ptr::addr_of_mut!(g::extralevels_sorted).write(v.cast::<*mut c_void>()) }
}

// ---------------------------------------------------------------------------
// atomics. `Quake/atomics.h`'s accessors are `static inline` with
// compiler-specific barriers (`_ReadBarrier`/`_WriteBarrier` on MSVC), so
// they are reached through the glue rather than re-derived with Rust
// orderings -- the `levelinfo_t` fields below are written by the parsing
// worker and read by the main thread.

#[inline]
unsafe fn atomic_load_u32(p: *mut c_uint) -> c_uint {
    // SAFETY: `p` addresses a live `atomic_uint32_t`.
    unsafe { g::HostCmd_Glue_AtomicLoadU32(p.cast::<c_void>()) }
}

#[inline]
unsafe fn atomic_store_u32(p: *mut c_uint, desired: c_uint) {
    // SAFETY: `p` addresses a live `atomic_uint32_t`.
    unsafe { g::HostCmd_Glue_AtomicStoreU32(p.cast::<c_void>(), desired) }
}

#[inline]
unsafe fn atomic_load_ptr(p: *mut *mut c_void) -> *mut c_void {
    // SAFETY: `p` addresses a live `atomic_ptr_t`.
    unsafe { g::HostCmd_Glue_AtomicLoadPtr(p.cast::<c_void>()) }
}

#[inline]
unsafe fn atomic_store_ptr(p: *mut *mut c_void, desired: *mut c_void) {
    // SAFETY: `p` addresses a live `atomic_ptr_t`.
    unsafe { g::HostCmd_Glue_AtomicStorePtr(p.cast::<c_void>(), desired) }
}

// ---------------------------------------------------------------------------
// host_cmd.c:52 -- Host_Quit_f

/// `host_cmd.c:52` -- `Host_Quit_f`.
unsafe fn Host_Quit_f() -> Raise {
    // SAFETY: console command, main thread.
    unsafe {
        if ptr::addr_of!(c::cl_demo::key_dest).read() != KEY_CONSOLE
            && ptr::addr_of!((*cls_p()).state).read() != CA_DEDICATED
            && !ptr::addr_of!(c::harness_active).read()
        {
            raise!(g::HostCmd_Glue_M_Menu_Quit_f());
            return c::host::HOST_GUARD_OK;
        }
        raise!(g::HostCmd_Glue_CL_Disconnect());
        raise!(g::HostCmd_Glue_Host_ShutdownServer(0));

        raise!(g::HostCmd_Glue_Sys_Quit());
    }
    c::host::HOST_GUARD_OK
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_quit_f() -> Raise {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { Host_Quit_f() }
}

//==============================================================================
// johnfitz -- extramaps management
//==============================================================================

/// `host_cmd.c:77` -- `FileList_AddEx`.
///
/// Like `FileList_Add`, but allocates `extrabytes` of zeroed payload behind
/// the item and returns it (also when the name was already in the list).
unsafe fn FileList_AddEx(
    name: *const c_char,
    extrabytes: usize,
    list: *mut *mut FileListItem,
) -> *mut FileListItem {
    // SAFETY: `name` is NUL-terminated and `list` addresses a live head.
    unsafe {
        // return existing entry for duplicates
        let mut item = *list;
        while !item.is_null() {
            if c_streq(name, ptr::addr_of!((*item).name).cast::<c_char>()) {
                return item;
            }
            item = (*item).next;
        }

        let item =
            c::Mem_Alloc(core::mem::size_of::<FileListItem>() + extrabytes).cast::<FileListItem>();
        q_strlcpy(ptr::addr_of_mut!((*item).name).cast::<c_char>(), name, 32);

        // insert each entry in alphabetical order
        if (*list).is_null()
            || c::cvar_cmd::q_strcasecmp(
                ptr::addr_of!((*item).name).cast::<c_char>(),
                ptr::addr_of!((**list).name).cast::<c_char>(),
            ) < 0
        {
            // insert at front
            (*item).next = *list;
            *list = item;
        } else {
            // insert later
            let mut prev = *list;
            let mut cursor = (**list).next;
            while !cursor.is_null()
                && c::cvar_cmd::q_strcasecmp(
                    ptr::addr_of!((*item).name).cast::<c_char>(),
                    ptr::addr_of!((*cursor).name).cast::<c_char>(),
                ) > 0
            {
                prev = cursor;
                cursor = (*cursor).next;
            }
            (*item).next = (*prev).next;
            (*prev).next = item;
        }

        item
    }
}

/// `host_cmd.c:118` -- `FileList_Add`.
unsafe fn FileList_Add(name: *const c_char, list: *mut *mut FileListItem) {
    // SAFETY: forwarded contract.
    unsafe {
        FileList_AddEx(name, 0, list);
    }
}

/// `host_cmd.c:123` -- `FileList_Clear`.
unsafe fn FileList_Clear(list: *mut *mut FileListItem) {
    // SAFETY: `list` addresses a live head; every node came from `Mem_Alloc`.
    unsafe {
        while !(*list).is_null() {
            let blah = (**list).next;
            c::Mem_Free((*list).cast::<c_void>());
            *list = blah;
        }
    }
}

/// `host_cmd.c:135` -- `FileList_Init`. C spells the two string parameters
/// `char *`; they are only ever read, and the function has internal linkage,
/// so the port takes them as `*const c_char`.
unsafe fn FileList_Init(path: *const c_char, ext: *const c_char, list: *mut *mut FileListItem) {
    // SAFETY: engine bring-up / list rebuild, main thread.
    unsafe {
        let mut filestring = [0 as c_char; MAX_OSPATH];
        let mut filename = [0 as c_char; 32];

        // C leaves `multiuser_saves` uninitialized and writes only the two
        // fields it goes on to read; zeroing it here is observationally
        // identical and avoids reading uninitialized memory.
        let mut multiuser_saves: g::searchpath_t = core::mem::zeroed();

        if ptr::addr_of!(c::multiuser).read() && c_streq(ext, c"sav".as_ptr()) {
            let pref_path =
                c::Sys_GetPrefPath(c"vkQuake".as_ptr(), c::sv_main::COM_GetGameNames(true));
            c_strcpy(
                ptr::addr_of_mut!(multiuser_saves.filename).cast::<c_char>(),
                pref_path,
            );
            c::Mem_Free(pref_path.cast::<c_void>());
            multiuser_saves.next = ptr::addr_of!(g::com_searchpaths).read();
        } else {
            multiuser_saves.next = ptr::null_mut();
        }

        let mut search = if !multiuser_saves.next.is_null() {
            ptr::addr_of_mut!(multiuser_saves)
        } else {
            ptr::addr_of!(g::com_searchpaths).read()
        };
        while !search.is_null() {
            if *ptr::addr_of!((*search).filename).cast::<c_char>() != 0 {
                // directory
                let mut w = SnBuf::new(filestring.as_mut_ptr(), filestring.len());
                w.cstr(ptr::addr_of!((*search).filename).cast::<c_char>());
                w.lit(b"/");
                w.cstr(path);

                let mut find = c::Sys_FindFirst(filestring.as_ptr(), ext);
                while !find.is_null() {
                    if (*find).attribs & c::fileattribs_t_FA_DIRECTORY != 0 {
                        find = c::Sys_FindNext(find);
                        continue;
                    }
                    c::COM_StripExtension(
                        ptr::addr_of!((*find).name).cast::<c_char>(),
                        filename.as_mut_ptr(),
                        filename.len(),
                    );
                    FileList_Add(filename.as_ptr(), list);
                    find = c::Sys_FindNext(find);
                }
                if c_streq(ext, c"sav".as_ptr())
                    && (!ptr::addr_of!(c::multiuser).read()
                        || search != ptr::addr_of_mut!(multiuser_saves))
                {
                    // only game dir for savegames
                    break;
                }
            }
            search = (*search).next;
        }
    }
}

/// `host_cmd.c:176` -- `RightPad`.
unsafe fn RightPad(str_: *const c_char, minlen: usize, ch: c_char) -> *const c_char {
    // SAFETY: `str_` is NUL-terminated; the buffer is the file static.
    unsafe {
        let buf = ptr::addr_of_mut!(RIGHTPAD_BUF).cast::<c_char>();
        let mut len = c_strlen(str_);

        let minlen = core::cmp::min(minlen, 1024 - 1);
        if len >= minlen {
            return str_;
        }

        core::ptr::copy_nonoverlapping(str_, buf, len);
        while len < minlen {
            *buf.add(len) = ch;
            len += 1;
        }
        *buf.add(len) = 0;

        buf
    }
}

/// `host_cmd.c:205` -- `ExtraMaps_Categorize`.
unsafe fn ExtraMaps_Categorize(name: *const c_char, source: *const g::searchpath_t) -> c_uint {
    // SAFETY: `name` is NUL-terminated; `source` is NULL or a live searchpath.
    unsafe {
        let mut len = c_strlen(name);

        if source.is_null() {
            match *name as u8 {
                b'd' => {
                    if *name.add(1) == b'm' as c_char {
                        return MAPTYPE_ID_DM;
                    }
                }
                b's' => {
                    if c_streq(name.add(1), c"tart".as_ptr()) {
                        return MAPTYPE_ID_START;
                    }
                }
                b'e' => {
                    if *name.add(1) >= b'1' as c_char && *name.add(1) <= b'4' as c_char {
                        return MAPTYPE_ID_EP1_LEVEL
                            .wrapping_add((*name.add(1) as c_int - b'1' as c_int) as c_uint);
                    }
                    if c_streq(name.add(1), c"nd".as_ptr()) {
                        return MAPTYPE_ID_END;
                    }
                }
                _ => {}
            }
            return MAPTYPE_ID_LEVEL;
        }

        let is_start = len >= 5
            && (c_memeq(name.add(len - 5), b"start")
                || c_memeq(name, b"start")
                || c_memeq(name.add(len - 5), b"intro"));
        let is_end = len >= 3 && c_memeq(name.add(len - 3), b"end");
        while len > 0 && ((*name.add(len - 1) as c_int - b'0' as c_int) as c_uint) <= 9 {
            len -= 1;
        }
        let is_dm = len >= 2 && c_memeq(name.add(len - 2), b"dm");

        if (*source).path_id != (*ptr::addr_of!(g::com_searchpaths).read()).path_id {
            if is_start {
                return MAPTYPE_CUSTOM_ID_START;
            }
            if is_end {
                return MAPTYPE_CUSTOM_ID_END;
            }
            if is_dm {
                return MAPTYPE_CUSTOM_ID_DM;
            }
            return MAPTYPE_CUSTOM_ID_LEVEL;
        }

        let base = if *ptr::addr_of!((*source).filename).cast::<c_char>() != 0 {
            MAPTYPE_CUSTOM_MOD_START
        } else {
            MAPTYPE_MOD_START
        };
        if is_start {
            return base + MAPTYPE_CUSTOM_MOD_START;
        }
        if is_end {
            return base + MAPTYPE_CUSTOM_MOD_END;
        }
        if is_dm {
            return base + MAPTYPE_CUSTOM_MOD_DM;
        }
        base + MAPTYPE_CUSTOM_MOD_LEVEL
    }
}

/// `host_cmd.c:262-266` -- `levelinfo_t`, the per-`extralevels` payload
/// `FileList_AddEx` over-allocates behind the node. `atomic_uint32_t` is
/// `struct { volatile uint32_t value; }` under MSVC and `_Atomic uint32_t`
/// under C11 -- 4 bytes either way; `atomic_ptr_t` is pointer-sized either
/// way. Both fields are only ever touched through the `HostCmd_Glue_Atomic*`
/// seams, never read or written directly.
#[repr(C)]
struct LevelInfo {
    type_: c_uint,
    message: *mut c_void,
}

const _: () = {
    assert!(core::mem::size_of::<LevelInfo>() == 2 * core::mem::size_of::<*mut u8>());
    assert!(core::mem::offset_of!(LevelInfo, message) == core::mem::size_of::<*mut u8>());
};

/// `host_cmd.c:273` -- `ExtraMaps_GetInfo`.
unsafe fn ExtraMaps_GetInfo(item: *const FileListItem) -> *const LevelInfo {
    // SAFETY: every `extralevels` node was over-allocated with a `LevelInfo`.
    unsafe { item.add(1).cast::<LevelInfo>() }
}

/// `host_cmd.c:283` -- `ExtraMaps_GetType`.
unsafe fn ExtraMaps_GetType(item: *const FileListItem) -> c_uint {
    // SAFETY: forwarded contract.
    unsafe {
        let info = ExtraMaps_GetInfo(item);
        atomic_load_u32(ptr::addr_of!((*info).type_) as *mut c_uint)
    }
}

/// # Safety
/// FFI entry point. `item` must be a live `extralevels` node.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_extra_maps_get_type(item: *const FileListItem) -> c_uint {
    // SAFETY: caller contract.
    unsafe { ExtraMaps_GetType(item) }
}

/// `host_cmd.c:294` -- `ExtraMaps_GetMessage`.
unsafe fn ExtraMaps_GetMessage(item: *const FileListItem) -> *const c_char {
    // SAFETY: forwarded contract.
    unsafe {
        let info = ExtraMaps_GetInfo(item);
        atomic_load_ptr(ptr::addr_of!((*info).message) as *mut *mut c_void).cast::<c_char>()
    }
}

/// # Safety
/// FFI entry point. `item` must be a live `extralevels` node.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_extra_maps_get_message(
    item: *const FileListItem,
) -> *const c_char {
    // SAFETY: caller contract.
    unsafe { ExtraMaps_GetMessage(item) }
}

/// `host_cmd.c:305` -- `ExtraMaps_IsStart`.
fn ExtraMaps_IsStart(type_: c_uint) -> bool {
    type_ == MAPTYPE_CUSTOM_MOD_START
        || type_ == MAPTYPE_MOD_START
        || type_ == MAPTYPE_CUSTOM_ID_START
        || type_ == MAPTYPE_ID_START
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_extra_maps_is_start(type_: c_uint) -> c::qboolean {
    ExtraMaps_IsStart(type_)
}

/// `host_cmd.c:315` -- `ExtraMaps_Sort`.
unsafe fn ExtraMaps_Sort() {
    // SAFETY: main thread, with no parsing worker running.
    unsafe {
        let mut counts = [0 as c_int; MAPTYPE_COUNT as usize];

        let mut item = *extralevels_p();
        while !item.is_null() {
            counts[ExtraMaps_GetType(item) as usize] += 1;
            item = (*item).next;
        }

        let mut sum: c_int = 0;
        for slot in counts.iter_mut() {
            let tmp = *slot;
            *slot = sum;
            sum += tmp;
        }
        sum += 1; // NULL terminator

        extralevels_sorted_set(
            c::Mem_Realloc(
                extralevels_sorted_get().cast::<c_void>(),
                core::mem::size_of::<*mut FileListItem>() * sum as usize,
            )
            .cast::<*mut FileListItem>(),
        );

        let sorted = extralevels_sorted_get();
        let mut item = *extralevels_p();
        while !item.is_null() {
            let slot = &mut counts[ExtraMaps_GetType(item) as usize];
            *sorted.add(*slot as usize) = item;
            *slot += 1;
            item = (*item).next;
        }
        *sorted.add(sum as usize - 1) = ptr::null_mut();
    }
}

/// `host_cmd.c:345` -- `ExtraMaps_Add`.
unsafe fn ExtraMaps_Add(name: *const c_char, source: *const g::searchpath_t) {
    // SAFETY: main thread; `name` is NUL-terminated.
    unsafe {
        // ignore duplicates so the first (highest priority) searchpath
        // determines the type
        let mut item = *extralevels_p();
        while !item.is_null() {
            if c_streq(name, ptr::addr_of!((*item).name).cast::<c_char>()) {
                return;
            }
            item = (*item).next;
        }

        let item = FileList_AddEx(name, core::mem::size_of::<LevelInfo>(), extralevels_p());
        let info = item.add(1).cast::<LevelInfo>();
        atomic_store_u32(
            ptr::addr_of_mut!((*info).type_),
            ExtraMaps_Categorize(name, source),
        );
        let len = c_strlen(name);
        let cur = ptr::addr_of!(MAXLEVELNAMELEN).read();
        ptr::addr_of_mut!(MAXLEVELNAMELEN).write(core::cmp::max(cur, len));
    }
}

/// `host_cmd.c:366` -- `ExtraMaps_ParseDescriptions`, the `QThread_Create`
/// worker (ADR-016). It runs off the main thread, so it holds no Rust
/// thread-local, assumes no ambient qcvm and opens no `Host_Guard` frame;
/// `Mod_LoadMapDescription` is therefore a plain call, exactly as in C.
///
/// # Safety
/// FFI entry point, invoked by `QThread_Create` with the `unused` argument
/// `HostCmd_Glue_StartParsingThread` passed it.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_extra_maps_parse_descriptions(
    _unused: *mut c_void,
) -> c_int {
    // SAFETY: the worker only touches the published `extralevels_sorted`
    // array and the atomics behind the glue seams.
    unsafe {
        let mut buf = [0 as c_char; 1024];

        let mut i = 0isize;
        while !(*extralevels_sorted_get().offset(i)).is_null() {
            let item = *extralevels_sorted_get().offset(i);
            let info = item.add(1).cast::<LevelInfo>();
            let mut message: *mut c_char = ptr::null_mut();

            if g::HostCmd_Glue_GetCancelParsing() != 0 {
                return 1;
            }

            if !g::Mod_LoadMapDescription(
                buf.as_mut_ptr(),
                buf.len(),
                ptr::addr_of!((*item).name).cast::<c_char>(),
            ) {
                atomic_store_u32(ptr::addr_of_mut!((*info).type_), MAPTYPE_BMODEL);
            }
            if buf[0] != 0 {
                message = c::Mem_Alloc(c_strlen(buf.as_ptr()) + 1).cast::<c_char>();
                c_strcpy(message, buf.as_ptr());
            }
            atomic_store_ptr(
                ptr::addr_of_mut!((*info).message),
                if !message.is_null() {
                    message.cast::<c_void>()
                } else {
                    c"".as_ptr() as *mut c_void
                },
            );

            i += 1;
        }

        0
    }
}

/// `host_cmd.c:398` -- `ExtraMaps_WaitForParsingThread`. The handle test, the
/// `QThread_Wait` and the cancel-flag reset all live in the glue: the thread
/// handle stays C (ADR-016).
unsafe fn ExtraMaps_WaitForParsingThread() {
    // SAFETY: main thread.
    unsafe { g::HostCmd_Glue_WaitForParsingThread() }
}

/// `host_cmd.c:413` -- `ExtraMaps_Init`.
unsafe fn ExtraMaps_Init() {
    // SAFETY: engine bring-up / gamedir switch, main thread.
    unsafe {
        let mut mapname = [0 as c_char; 32];
        let mut ignorepakdir = [0 as c_char; 32];

        // we don't want the maps in the id1 pakfiles to be
        // categorized as custom levels
        {
            let mut w = SnBuf::new(ignorepakdir.as_mut_ptr(), ignorepakdir.len());
            w.lit(b"/");
            w.cstr(GAMENAME.as_ptr());
            w.lit(b"/");
        }

        let mut search = ptr::addr_of!(g::com_searchpaths).read();
        while !search.is_null() {
            if *ptr::addr_of!((*search).filename).cast::<c_char>() != 0 {
                // directory
                let mut dir = [0 as c_char; MAX_OSPATH];

                let mut w = SnBuf::new(dir.as_mut_ptr(), dir.len());
                w.cstr(ptr::addr_of!((*search).filename).cast::<c_char>());
                w.lit(b"/maps");

                let mut find = c::Sys_FindFirst(dir.as_ptr(), c"bsp".as_ptr());
                while !find.is_null() {
                    if (*find).attribs & c::fileattribs_t_FA_DIRECTORY != 0 {
                        find = c::Sys_FindNext(find);
                        continue;
                    }
                    c::COM_StripExtension(
                        ptr::addr_of!((*find).name).cast::<c_char>(),
                        mapname.as_mut_ptr(),
                        mapname.len(),
                    );
                    ExtraMaps_Add(mapname.as_ptr(), search);
                    find = c::Sys_FindNext(find);
                }
            } else {
                // pakfile
                let pak = (*search).pack;
                let isbase = c_strstr_found(
                    ptr::addr_of!((*pak).filename).cast::<c_char>(),
                    ignorepakdir.as_ptr(),
                );
                let mut i: c_int = 0;
                while i < (*pak).numfiles {
                    let file = (*pak).files.offset(i as isize);
                    let fname = ptr::addr_of!((*file).name).cast::<c_char>();
                    if (*file).filelen > MIN_BSP_MAP_SIZE // don't list files under 32k (ammo boxes etc)
                        && c_strneq(fname, c"maps/".as_ptr(), 5) // don't list files outside of maps/
                        && c_strchr(fname.add(5), b'/' as c_char).is_null() // don't list files in subdirectories
                        && c_streq(c::COM_FileGetExtension(fname), c"bsp".as_ptr())
                    {
                        c::COM_StripExtension(fname.add(5), mapname.as_mut_ptr(), mapname.len());
                        ExtraMaps_Add(mapname.as_ptr(), if isbase { ptr::null() } else { search });
                    }
                    i += 1;
                }
            }
            search = (*search).next;
        }

        ExtraMaps_Sort();

        g::HostCmd_Glue_SetCancelParsing(0);
        g::HostCmd_Glue_StartParsingThread(quake_rs_hostcmd_extra_maps_parse_descriptions);
    }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_extra_maps_init() {
    // SAFETY: FFI entry point; no pointer arguments. Nothing here can raise.
    unsafe { ExtraMaps_Init() }
}

/// `host_cmd.c:469` -- `ExtraMaps_Clear`.
unsafe fn ExtraMaps_Clear() {
    // SAFETY: main thread.
    unsafe {
        g::HostCmd_Glue_SetCancelParsing(1);
        ExtraMaps_WaitForParsingThread();

        ptr::addr_of_mut!(MAXLEVELNAMELEN).write(0);
        let mut item = *extralevels_p();
        while !item.is_null() {
            let info = item.add(1).cast::<LevelInfo>();
            let message = atomic_load_ptr(ptr::addr_of_mut!((*info).message)).cast::<c_char>();
            if !message.is_null() && *message != 0 {
                c::Mem_Free(message.cast::<c_void>());
            }
            item = (*item).next;
        }

        FileList_Clear(extralevels_p());
    }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_extra_maps_clear() {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { ExtraMaps_Clear() }
}

/// `host_cmd.c:493` -- `ExtraMaps_ShutDown`.
unsafe fn ExtraMaps_ShutDown() {
    // SAFETY: shutdown path, main thread.
    unsafe { ExtraMaps_Clear() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_extra_maps_shut_down() {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { ExtraMaps_ShutDown() }
}

/// `host_cmd.c:498` -- `ExtraMaps_NewGame`.
unsafe fn ExtraMaps_NewGame() {
    // SAFETY: gamedir switch, main thread.
    unsafe {
        ExtraMaps_Clear();
        ExtraMaps_Init();
    }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_extra_maps_new_game() {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { ExtraMaps_NewGame() }
}

/// `host_cmd.c:509` -- `Host_Maps_f`. Reaches only `Con_SafePrintf` and the
/// plain `COM_*`/`Cmd_*` helpers, so it crosses the FFI as a plain
/// `extern "C"` command callback rather than a glue trampoline (the
/// `snd_dma.rs:258-265` precedent).
#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_maps_f() {
    // SAFETY: console command, main thread.
    unsafe {
        let substr = if c::Cmd_Argc() >= 2 {
            c::Cmd_Argv(1)
        } else {
            ptr::null()
        };
        let mut buf = [0 as c_char; 256];
        let mut buf2 = [0 as c_char; 256];
        // same bits as ('.' | 0x80) without truncating a constant
        let padchar: c_char = (b'.' as c_int - 0x80) as c_char;
        let ofsdesc = ptr::addr_of!(MAXLEVELNAMELEN).read() + 2;

        let mut item = *extralevels_p();
        let mut i: c_int = 0;
        while !item.is_null() {
            if ExtraMaps_GetType(item) >= MAPTYPE_ID_START {
                item = (*item).next;
                continue;
            }
            let mut desc = ExtraMaps_GetMessage(item);
            if desc.is_null() {
                desc = c"".as_ptr();
            }
            if !substr.is_null() && *substr != 0 {
                if c::cvar_cmd::q_strcasestr(ptr::addr_of!((*item).name).cast::<c_char>(), substr)
                    .is_null()
                    && c::cvar_cmd::q_strcasestr(desc, substr).is_null()
                {
                    item = (*item).next;
                    continue;
                }
                let tinted_name = g::COM_TintSubstring(
                    ptr::addr_of!((*item).name).cast::<c_char>(),
                    substr,
                    buf.as_mut_ptr(),
                    buf.len(),
                );
                let tinted_desc = g::COM_TintSubstring(desc, substr, buf2.as_mut_ptr(), buf2.len());
                if *desc != 0 {
                    c::Con_SafePrintf(
                        c"   %s%c%s\n".as_ptr(),
                        RightPad(tinted_name, ofsdesc, padchar),
                        padchar as c_int,
                        tinted_desc,
                    );
                } else {
                    c::Con_SafePrintf(c"   %s\n".as_ptr(), tinted_name);
                }
            } else if *desc != 0 {
                c::Con_SafePrintf(
                    c"   %s%c%s\n".as_ptr(),
                    RightPad(
                        ptr::addr_of!((*item).name).cast::<c_char>(),
                        ofsdesc,
                        padchar,
                    ),
                    padchar as c_int,
                    desc,
                );
            } else {
                c::Con_SafePrintf(
                    c"   %s\n".as_ptr(),
                    ptr::addr_of!((*item).name).cast::<c_char>(),
                );
            }
            i += 1;
            item = (*item).next;
        }

        if !substr.is_null() && *substr != 0 {
            if i != 0 {
                c::Con_SafePrintf(
                    c"%i map%s containing \"%s\"\n".as_ptr(),
                    i,
                    if i == 1 { c"".as_ptr() } else { c"s".as_ptr() },
                    substr,
                );
            } else {
                c::Con_SafePrintf(c"no maps found containing \"%s\"\n".as_ptr(), substr);
            }
        } else if i != 0 {
            c::Con_SafePrintf(
                c"%i map%s\n".as_ptr(),
                i,
                if i == 1 { c"".as_ptr() } else { c"s".as_ptr() },
            );
        } else {
            c::Con_SafePrintf(c"no maps found\n".as_ptr());
        }
    }
}

//==============================================================================
// johnfitz -- modlist management
//==============================================================================

/// `host_cmd.c:569-572` -- `modinfo_t`, the per-`modlist` payload.
#[repr(C)]
struct ModInfo {
    full_name: [c_char; 64],
}

/// `host_cmd.c:596` -- `Modlist_GetFullName`.
///
/// Returns the friendly display name for a mod list entry, or NULL.
unsafe fn Modlist_GetFullName(item: *const FileListItem) -> *const c_char {
    // SAFETY: every `modlist` node was over-allocated with a `ModInfo`.
    unsafe {
        let info = item.add(1).cast::<ModInfo>();
        let full_name = ptr::addr_of!((*info).full_name).cast::<c_char>();

        if *full_name != 0 {
            // the rerelease mapdb.json uses localization keys like $m_quake;
            // resolve them here since the loc files aren't loaded yet when
            // the list is built
            if *full_name == b'$' as c_char {
                let value = g::LOC_GetRawString(full_name);
                if !value.is_null() {
                    return value;
                }
            } else {
                return full_name;
            }
        }
        for entry in KNOWNMODS.iter() {
            if c::cvar_cmd::q_strcasecmp(
                ptr::addr_of!((*item).name).cast::<c_char>(),
                entry[0].as_ptr(),
            ) == 0
            {
                return entry[1].as_ptr();
            }
        }
        ptr::null()
    }
}

/// # Safety
/// FFI entry point. `item` must be a live `modlist` node.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_modlist_get_full_name(
    item: *const FileListItem,
) -> *const c_char {
    // SAFETY: caller contract.
    unsafe { Modlist_GetFullName(item) }
}

/// `host_cmd.c:630` -- `Modlist_SetNameFromMapDB`.
///
/// Takes the display name from a mapdb.json episode list (from Ironwail).
/// Base files describe several episodes, so the dir has to match; the same
/// goes for Copper to avoid naming every Copper-based mod "Underdark
/// Overbright" when it ships Copper's mapdb.json unmodified.
///
/// `Quake/json.c` is already Rust (Phase 1), so the four `JSON_*` entry
/// points are called in-crate rather than through a C extern.
unsafe fn Modlist_SetNameFromMapDB(
    info: *mut ModInfo,
    mapdb: *const c_char,
    name: *const c_char,
    is_base: bool,
) {
    // SAFETY: `mapdb` is a NUL-terminated text buffer; `info` a live payload.
    unsafe {
        let json: *mut Json = crate::json::JSON_Parse(mapdb);
        if json.is_null() {
            return;
        }

        let episodes = crate::json::JSON_Find(
            (*json).root as *const JsonEntry,
            c"episodes".as_ptr(),
            JSON_ARRAY,
        );
        if !episodes.is_null() {
            let mut entry = (*episodes).firstchild as *const JsonEntry;
            while !entry.is_null() {
                let mod_name = crate::json::JSON_FindString(entry, c"name".as_ptr());
                let mod_dir = crate::json::JSON_FindString(entry, c"dir".as_ptr());
                if mod_name.is_null() || mod_dir.is_null() {
                    entry = (*entry).next as *const JsonEntry;
                    continue;
                }
                if (is_base || c::cvar_cmd::q_strcasecmp(mod_dir, c"copper".as_ptr()) == 0)
                    && c::cvar_cmd::q_strcasecmp(mod_dir, name) != 0
                {
                    entry = (*entry).next as *const JsonEntry;
                    continue;
                }
                q_strlcpy(
                    ptr::addr_of_mut!((*info).full_name).cast::<c_char>(),
                    mod_name,
                    64,
                );
                break;
            }
        }
        crate::json::JSON_Free(json);
    }
}

/// `host_cmd.c:655` -- `Modlist_Add`.
unsafe fn Modlist_Add(base: *const c_char, name: *const c_char) {
    // SAFETY: engine bring-up, main thread.
    unsafe {
        if c_strlen(name) == 3
            && q_tolower(*name as c_int) == b'i' as c_int
            && q_tolower(*name.add(1) as c_int) == b'd' as c_int
            && *name.add(2) == b'1' as c_char
        {
            return;
        }
        if g::COM_ModForbiddenChars(name) {
            return;
        }
        let mut path = [0 as c_char; MAX_OSPATH];
        mod_path(&mut path, base, name, b"/pak0.pak");
        if c::Sys_FileType(path.as_ptr()) != FS_ENT_FILE {
            mod_path(&mut path, base, name, b"/progs.dat");
            if c::Sys_FileType(path.as_ptr()) != FS_ENT_FILE {
                mod_path(&mut path, base, name, b"/csprogs.dat");
                if c::Sys_FileType(path.as_ptr()) != FS_ENT_FILE {
                    mod_path(&mut path, base, name, b"/maps");
                    if c::Sys_FileType(path.as_ptr()) != FS_ENT_DIRECTORY {
                        return;
                    }
                }
            }
        }

        let item = FileList_AddEx(name, core::mem::size_of::<ModInfo>(), modlist_p());
        let info = item.add(1).cast::<ModInfo>();
        let full_name = ptr::addr_of_mut!((*info).full_name).cast::<c_char>();

        // use the first non-empty line of a descript.ion file in the mod dir
        // as display name
        if *full_name == 0 {
            mod_path(&mut path, base, name, b"/descript.ion");
            let buf = c::COM_LoadMallocFile_TextMode_OSPath(path.as_ptr(), ptr::null_mut())
                .cast::<c_char>();
            if !buf.is_null() {
                let mut description = buf;
                while q_isspace(*description as c_int) != 0 {
                    description = description.add(1);
                }
                let end = c_strchr(description, b'\n' as c_char);
                if !end.is_null() {
                    *end = 0;
                }
                q_strlcpy(full_name, c::q_strtrim(description), 64);
                c::Mem_Free(buf.cast::<c_void>());
            }
        }

        // otherwise try a loose mapdb.json in the mod dir (Kex add-ons ship one)
        if *full_name == 0 {
            mod_path(&mut path, base, name, b"/mapdb.json");
            let mapdb = c::COM_LoadMallocFile_TextMode_OSPath(path.as_ptr(), ptr::null_mut())
                .cast::<c_char>();
            if !mapdb.is_null() {
                Modlist_SetNameFromMapDB(info, mapdb, name, false);
                c::Mem_Free(mapdb.cast::<c_void>());
            }
        }
    }
}

/// The five `q_snprintf (path, sizeof (path), "%s/%s<tail>", base, name)`
/// calls `Modlist_Add` makes, factored to one writer. The truncation is
/// `q_snprintf`'s and the byte sequence is identical.
unsafe fn mod_path(
    path: &mut [c_char; MAX_OSPATH],
    base: *const c_char,
    name: *const c_char,
    tail: &[u8],
) {
    // SAFETY: `base` and `name` are NUL-terminated; `path` is the caller's.
    unsafe {
        let mut w = SnBuf::new(path.as_mut_ptr(), MAX_OSPATH);
        w.cstr(base);
        w.lit(b"/");
        w.cstr(name);
        w.lit(tail);
    }
}

/// `host_cmd.c:712` -- `Modlist_AddRoot`.
unsafe fn Modlist_AddRoot(base: *const c_char) {
    // SAFETY: engine bring-up, main thread.
    unsafe {
        let mut find = c::Sys_FindFirst(base, ptr::null());
        while !find.is_null() {
            let fname = ptr::addr_of!((*find).name).cast::<c_char>();
            if (*find).attribs & c::fileattribs_t_FA_DIRECTORY == 0 {
                find = c::Sys_FindNext(find);
                continue;
            }
            if c_streq(fname, c".".as_ptr()) || c_streq(fname, c"..".as_ptr()) {
                find = c::Sys_FindNext(find);
                continue;
            }
            if c::cvar_cmd::q_strcasecmp(c::COM_FileGetExtension(fname), c"app".as_ptr()) == 0 {
                // skip .app bundles on macOS
                find = c::Sys_FindNext(find);
                continue;
            }
            Modlist_Add(base, fname);
            find = c::Sys_FindNext(find);
        }
    }
}

/// `host_cmd.c:737` -- `Modlist_LoadAddonsJSON`.
///
/// The official rerelease client keeps a catalog of the add-ons it has
/// downloaded in an addons.json next to them; use it for display names
/// (their mapdb.json sits inside the pak where the loose-file scan can't see it).
unsafe fn Modlist_LoadAddonsJSON(base: *const c_char) {
    // SAFETY: engine bring-up, main thread.
    unsafe {
        let mut path = [0 as c_char; MAX_OSPATH];

        let written = {
            let mut w = SnBuf::new(path.as_mut_ptr(), MAX_OSPATH);
            w.cstr(base);
            w.lit(b"/addons.json");
            w.finish()
        };
        if written as usize >= MAX_OSPATH {
            return;
        }
        let text =
            c::COM_LoadMallocFile_TextMode_OSPath(path.as_ptr(), ptr::null_mut()).cast::<c_char>();
        if text.is_null() {
            return;
        }

        let json: *mut Json = crate::json::JSON_Parse(text);
        c::Mem_Free(text.cast::<c_void>());
        if json.is_null() {
            return;
        }

        let addons = crate::json::JSON_Find(
            (*json).root as *const JsonEntry,
            c"addons".as_ptr(),
            JSON_ARRAY,
        );
        if !addons.is_null() {
            let mut entry = (*addons).firstchild as *const JsonEntry;
            while !entry.is_null() {
                let gamedir = crate::json::JSON_FindString(entry, c"gamedir".as_ptr());
                let name = crate::json::JSON_FindString(entry, c"name".as_ptr());
                if gamedir.is_null() || name.is_null() {
                    entry = (*entry).next as *const JsonEntry;
                    continue;
                }
                let mut item = *modlist_p();
                while !item.is_null() {
                    if c::cvar_cmd::q_strcasecmp(
                        ptr::addr_of!((*item).name).cast::<c_char>(),
                        gamedir,
                    ) == 0
                    {
                        let info = item.add(1).cast::<ModInfo>();
                        let full_name = ptr::addr_of_mut!((*info).full_name).cast::<c_char>();
                        if *full_name == 0 {
                            q_strlcpy(full_name, name, 64);
                        }
                        break;
                    }
                    item = (*item).next;
                }
                entry = (*entry).next as *const JsonEntry;
            }
        }
        crate::json::JSON_Free(json);
    }
}

/// `host_cmd.c:779` -- `Modlist_Init`.
unsafe fn Modlist_Init() -> Raise {
    // SAFETY: engine bring-up, main thread.
    unsafe {
        let numbasedirs = ptr::addr_of!(g::com_numbasedirs).read();
        let basedirs = ptr::addr_of!(g::com_basedirs).cast::<[c_char; MAX_OSPATH]>();

        let mut i: c_int = 0;
        while i < numbasedirs {
            Modlist_AddRoot(basedirs.offset(i as isize).cast::<c_char>());
            i += 1;
        }

        // names from the official client's download catalog
        i = 0;
        while i < numbasedirs {
            Modlist_LoadAddonsJSON(basedirs.offset(i as isize).cast::<c_char>());
            i += 1;
        }

        // the rerelease describes its bundled episodes (hipnotic, rogue, dopa, mg1)
        // in a single mapdb.json inside id1, so look that one up through the searchpath
        let mut path_id: c_uint = 0;
        let mut mapdb_out: *mut c_void = ptr::null_mut();
        raise!(g::HostCmd_Glue_ComLoadFile(
            c"mapdb.json".as_ptr(),
            ptr::addr_of_mut!(path_id),
            ptr::addr_of_mut!(mapdb_out)
        ));
        let mapdb = mapdb_out.cast::<c_char>();
        if !mapdb.is_null() {
            // base = the id1 group; only a mapdb.json from an active mod on top of it
            // may name entries without a dir match (e.g. a renamed mod dir)
            let base_searchpaths = ptr::addr_of!(g::com_base_searchpaths).read();
            let is_base = base_searchpaths.is_null() || path_id <= (*base_searchpaths).path_id;
            let mut item = *modlist_p();
            while !item.is_null() {
                let info = item.add(1).cast::<ModInfo>();
                if *ptr::addr_of!((*info).full_name).cast::<c_char>() == 0 {
                    Modlist_SetNameFromMapDB(
                        info,
                        mapdb,
                        ptr::addr_of!((*item).name).cast::<c_char>(),
                        is_base,
                    );
                }
                item = (*item).next;
            }
            c::Mem_Free(mapdb.cast::<c_void>());
        }
    }
    c::host::HOST_GUARD_OK
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_modlist_init() -> Raise {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { Modlist_Init() }
}

//==============================================================================
// ericw -- demo list management
//==============================================================================

/// `host_cmd.c:817` -- `DemoList_Clear`.
unsafe fn DemoList_Clear() {
    // SAFETY: main thread.
    unsafe { FileList_Clear(demolist_p()) }
}

/// `host_cmd.c:822` -- `DemoList_Rebuild`.
unsafe fn DemoList_Rebuild() {
    // SAFETY: main thread.
    unsafe {
        DemoList_Clear();
        DemoList_Init();
    }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_demo_list_rebuild() {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { DemoList_Rebuild() }
}

/// `host_cmd.c:828` -- `DemoList_Init`.
unsafe fn DemoList_Init() {
    // SAFETY: main thread.
    unsafe { FileList_Init(c"".as_ptr(), c"dem".as_ptr(), demolist_p()) }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_demo_list_init() {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { DemoList_Init() }
}

//==============================================================================
// savegame list management
//==============================================================================

/// `host_cmd.c:839` -- `SaveList_Clear`.
unsafe fn SaveList_Clear() {
    // SAFETY: main thread.
    unsafe { FileList_Clear(savelist_p()) }
}

/// `host_cmd.c:844` -- `SaveList_Rebuild`.
unsafe fn SaveList_Rebuild() {
    // SAFETY: main thread.
    unsafe {
        SaveList_Clear();
        SaveList_Init();
    }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_save_list_rebuild() {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { SaveList_Rebuild() }
}

/// `host_cmd.c:850` -- `SaveList_Init`.
unsafe fn SaveList_Init() {
    // SAFETY: main thread.
    unsafe { FileList_Init(c"".as_ptr(), c"sav".as_ptr(), savelist_p()) }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_save_list_init() {
    // SAFETY: FFI entry point; no pointer arguments.
    unsafe { SaveList_Init() }
}

/// `host_cmd.c:862` -- `Host_Mods_f` -- johnfitz.
///
/// List all potential mod directories (contain either a pak file or a progs.dat).
#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_mods_f() {
    // SAFETY: console command, main thread.
    unsafe {
        let mut modp = *modlist_p();
        let mut i: c_int = 0;
        while !modp.is_null() {
            c::Con_SafePrintf(
                c"   %s\n".as_ptr(),
                ptr::addr_of!((*modp).name).cast::<c_char>(),
            );
            modp = (*modp).next;
            i += 1;
        }

        if i != 0 {
            c::Con_SafePrintf(c"%i mod(s)\n".as_ptr(), i);
        } else {
            c::Con_SafePrintf(c"no mods found\n".as_ptr());
        }
    }
}

//==============================================================================

/// `host_cmd.c:883` -- `Host_Mapname_f` -- johnfitz.
#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_mapname_f() {
    // SAFETY: console command, main thread.
    unsafe {
        if ptr::addr_of!((*sv_p()).active).read() {
            c::Con_Printf(
                c"\"mapname\" is \"%s\"\n".as_ptr(),
                ptr::addr_of!((*sv_p()).name).cast::<c_char>(),
            );
            return;
        }

        if ptr::addr_of!((*cls_p()).state).read() == CA_CONNECTED {
            c::Con_Printf(
                c"\"mapname\" is \"%s\"\n".as_ptr(),
                ptr::addr_of!((*cl_p()).mapname).cast::<c_char>(),
            );
            return;
        }

        c::Con_Printf(c"no map loaded\n".as_ptr());
    }
}

// ==== end CHUNK A ====

// ===========================================================================
// CHUNK B
// ===========================================================================

// ===========================================================================
// CHUNK B (host_cmd.c:899-1509) -- Host_Status_f, Host_God_f, Host_Notarget_f,
// Host_Noclip_f, Host_SetPos_f, Host_Fly_f, Host_Ping_f, Host_Map_f,
// Host_Randmap_f, Host_Changelevel_f, Host_Restart_f, Host_Reconnect_f,
// Host_Connect_f.
//
// `noclip_anglehack` (host_cmd.c:1065) is NOT redeclared here -- it is
// [`quake_c_sys::view::noclip_anglehack`], reused per the task's ADR-007
// finding. `current_skill` is reused from [`quake_c_sys::sv_main`] the same
// way.
//
// ADR-009 guarded/plain audit and the ADR-005 specifier inventory are in
// t83_B_notes.md; every seam this chunk declares or reuses is named at each
// call site below.
// ===========================================================================

use quake_c_sys::host as chost;
use quake_c_sys::sv_main as csv;
use quake_c_sys::sv_user as csu;
use quake_c_sys::view as cview;

/// `Quake/net.h:34` -- `#define NET_NAMELEN 64`. Mirrors
/// `quake_types::net::NET_NAMELEN`, which this module does not otherwise
/// depend on `quake_types::net` for.
const NET_NAMELEN: usize = 64;

const SRC_CLIENT: c::cmd_source_t = c::cmd_source_t_src_client;
const SRC_COMMAND: c::cmd_source_t = c::cmd_source_t_src_command;

/// `Quake/server.h:278`.
const FL_GODMODE: c_int = 64;
/// `Quake/pr_edict.c` `BIT_CASE (FL_NOTARGET)` confirms `server.h`'s value
/// (already visible to this crate as `progs_builtins_sv.rs`'s local
/// `FL_NOTARGET` const).
const FL_NOTARGET: c_int = 128;

/// `Quake/sv_phys.rs` locals confirm these three `MOVETYPE_*` values.
const MOVETYPE_WALK: c_float = 3.0;
const MOVETYPE_FLY: c_float = 5.0;
const MOVETYPE_NOCLIP: c_float = 8.0;

/// `Quake/keys.h` `keydest_t::key_game` (the enum's first, so `0`). Typed
/// `c_int` to match `key_dest`'s declaration (`quake-c-sys` mirrors the
/// existing `cl_demo.rs:123` / `sv_user.rs:94` copies).
const KEY_GAME: c_int = 0;

#[inline]
unsafe fn cmd_src() -> c::cmd_source_t {
    // SAFETY: engine global, single-threaded host frame.
    unsafe { ptr::addr_of!(g::cmd_source).read() }
}

/// `SV_ClientPrintf` (already-formatted string; matches the `SV_ClientPrintf`
/// C wrapper's own internal `q_vsnprintf` + forward shape). Callers build the
/// formatted bytes with `va`/`q_snprintf` first -- never with the Rust
/// formatter (ADR-005).
#[inline]
unsafe fn client_print(msg: &[u8]) -> Raise {
    // SAFETY: `msg` is a caller-supplied NUL-terminated byte string literal.
    unsafe { crate::host::quake_rs_host_sv_client_printf(msg.as_ptr().cast()) }
}

/// `Host_Status_f` (`host_cmd.c:905-971`).
///
/// # Safety
/// FFI entry point; touches the live `sv`/`svs` engine globals.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_status_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe { host_status_f() }
}

unsafe fn host_status_f() -> Raise {
    // SAFETY: engine globals (`sv`/`svs`/net status) accessed on the
    // single-threaded host frame; see the `# Safety` doc on the caller.
    unsafe {
        // `host_cmd.c:918-928` -- `print_fn` selection.
        let use_client: bool = if cmd_src() != SRC_CLIENT {
            if !(*sv_p()).active {
                raise!(g::HostCmd_Glue_CmdForwardToServer());
                return 0;
            }
            false
        } else {
            true
        };

        // Replicates C's dynamic `print_fn (fmt, ...)` function pointer: a
        // real function pointer to a C variadic cannot be formed generically
        // in Rust, so each call site branches on `use_client` instead. The
        // client branch pre-formats through `q_snprintf` (real C formatting,
        // ADR-005) and hands the finished bytes to the generic-convention
        // `quake_rs_host_sv_client_printf` core; the console branch calls
        // `Con_Printf` directly, exactly as `host_cmd.c`'s `Con_Printf` arm
        // did.
        //
        // COMPAT: the 1024-byte buffer is `SV_ClientPrintf`'s own (`host.c:522`
        // / `host_glue.c:832` -- `char string[1024]; q_vsnprintf (string,
        // sizeof (string), ...)`), not `va`'s. `va` writes `VA_BUFFERLEN`
        // bytes (`common.c:1602-1606` -- `MAX_OSPATH` where that is >= 1024,
        // i.e. `PATH_MAX`, 4096 on Linux), so routing through it would widen
        // the truncation point for an over-long `status` line and change the
        // bytes that reach the client.
        macro_rules! status_print {
            ($fmt:expr $(, $arg:expr)* $(,)?) => {{
                if use_client {
                    let mut sbuf: [c_char; 1024] = [0; 1024];
                    g::q_snprintf(sbuf.as_mut_ptr(), sbuf.len(), ($fmt).as_ptr().cast() $(, $arg)*);
                    raise!(crate::host::quake_rs_host_sv_client_printf(sbuf.as_ptr()));
                } else {
                    c::Con_Printf(($fmt).as_ptr().cast() $(, $arg)*);
                }
            }};
        }

        // `host_cmd.c:930` -- `print_fn ("host:    %s\n", Cvar_VariableString ("hostname"));`
        status_print!(
            b"host:    %s\n\0",
            g::Cvar_VariableString(c"hostname".as_ptr())
        );

        // `host_cmd.c:931` -- `print_fn ("version: " ENGINE_NAME_AND_VER "\n");`.
        // `ENGINE_NAME_AND_VER` bakes in a build-date/platform macro that
        // cannot be reproduced faithfully as a Rust literal, so the glue
        // exports the already-expanded C string (t83_B_glue.c). The call
        // itself has no format specifiers, matching the C call exactly.
        if use_client {
            raise!(crate::host::quake_rs_host_sv_client_printf(
                g::HostCmd_EngineVersionLine
            ));
        } else {
            c::Con_Printf(g::HostCmd_EngineVersionLine);
        }

        // `host_cmd.c:933-940`.
        let mut addresses: [[c_char; NET_NAMELEN]; 32] = [[0; NET_NAMELEN]; 32];
        let numaddresses = g::NET_ListAddresses(addresses.as_mut_ptr().cast(), 32);
        for addr in addresses.iter().take(numaddresses as usize) {
            if addr[0] == b'[' as c_char {
                status_print!(b"ipv6:    %s\n\0", addr.as_ptr());
            } else {
                status_print!(b"tcp/ip:  %s\n\0", addr.as_ptr());
            }
        }
        // `host_cmd.c:941-944`.
        if c::ipv4Available {
            status_print!(
                b"tcp/ip:  %s\n\0",
                ptr::addr_of!(c::my_ipv4_address).cast::<c_char>()
            );
        }
        if c::ipv6Available {
            status_print!(
                b"ipv6:    %s\n\0",
                ptr::addr_of!(c::my_ipv6_address).cast::<c_char>()
            );
        }
        // `host_cmd.c:945-946`.
        status_print!(
            b"map:     %s\n\0",
            ptr::addr_of!((*sv_p()).name).cast::<c_char>()
        );
        status_print!(
            b"players: %i active (%i max)\n\n\0",
            c::net_activeconnections,
            (*svs_p()).maxclients,
        );

        // `host_cmd.c:947-970`.
        for j in 0..(*svs_p()).maxclients {
            let client = (*svs_p()).clients.add(j as usize);
            if !(*client).active {
                continue;
            }
            let mut seconds: c_int = if !(*client).netconnection.is_null() {
                (c::net_time - g::NET_QSocketGetTime((*client).netconnection.cast())) as c_int
            } else {
                0
            };
            let mut minutes = seconds / 60;
            let hours: c_int;
            if minutes != 0 {
                seconds -= minutes * 60;
                hours = minutes / 60;
                if hours != 0 {
                    minutes -= hours * 60;
                }
            } else {
                hours = 0;
            }
            status_print!(
                b"#%-2u %-16.16s  %3i  %2i:%02i:%02i\n\0",
                (j + 1) as c_uint,
                (*client).name.as_ptr(),
                (*(*client).edict).v.frags as c_int,
                hours,
                minutes,
                seconds,
            );
            if cmd_src() != SRC_CLIENT {
                if !(*client).netconnection.is_null() {
                    status_print!(
                        b"   %s\n\0",
                        g::NET_QSocketGetTrueAddressString((*client).netconnection.cast())
                    );
                } else {
                    status_print!(b"   %s\n\0", c"botclient".as_ptr());
                }
            } else if !(*client).netconnection.is_null() {
                status_print!(
                    b"   %s\n\0",
                    g::NET_QSocketGetMaskedAddressString((*client).netconnection.cast())
                );
            } else {
                status_print!(b"   %s\n\0", c"botclient".as_ptr());
            }
        }
        0
    }
}

/// Shared body of `Host_God_f` (`host_cmd.c:980-1018`) and `Host_Notarget_f`
/// (`host_cmd.c:1025-1063`) -- textually identical in C except the flag bit
/// and the three message strings. Branch order and arithmetic are preserved
/// exactly; only the flag bit and messages are parameterised.
unsafe fn host_toggle_flag_f(flag: c_int, msg_on: &[u8], msg_off: &[u8], usage: &[u8]) -> Raise {
    // SAFETY: engine globals (`pr_global_struct`/`sv_player`) accessed on the
    // single-threaded host frame; see the `# Safety` doc on the callers.
    unsafe {
        if cmd_src() != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return 0;
        }
        let pgs = ptr::addr_of!(csv::pr_global_struct)
            .read()
            .cast::<GlobalVars>();
        if (*pgs).deathmatch != 0.0 {
            return 0;
        }
        let player = ptr::addr_of!(csu::sv_player).read().cast::<Edict>();
        match c::Cmd_Argc() {
            1 => {
                (*player).v.flags = ((*player).v.flags as c_int ^ flag) as c_float;
                if (*player).v.flags as c_int & flag == 0 {
                    raise!(client_print(msg_off));
                } else {
                    raise!(client_print(msg_on));
                }
            }
            2 => {
                if g::atof(c::Cmd_Argv(1)) != 0.0 {
                    (*player).v.flags = ((*player).v.flags as c_int | flag) as c_float;
                    raise!(client_print(msg_on));
                } else {
                    (*player).v.flags = ((*player).v.flags as c_int & !flag) as c_float;
                    raise!(client_print(msg_off));
                }
            }
            _ => {
                c::Con_Printf(usage.as_ptr().cast());
            }
        }
        0
    }
}

/// # Safety
/// FFI entry point; touches the live `sv_player` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_god_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        host_toggle_flag_f(
            FL_GODMODE,
            b"godmode ON\n\0",
            b"godmode OFF\n\0",
            b"god [value] : toggle god mode. values: 0 = off, 1 = on\n\0",
        )
    }
}

/// # Safety
/// FFI entry point; touches the live `sv_player` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_notarget_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        host_toggle_flag_f(
            FL_NOTARGET,
            b"notarget ON\n\0",
            b"notarget OFF\n\0",
            b"notarget [value] : toggle notarget mode. values: 0 = off, 1 = on\n\0",
        )
    }
}

/// `Host_Noclip_f` (`host_cmd.c:1072-1119`).
///
/// # Safety
/// FFI entry point; touches the live `sv_player` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_noclip_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe { host_noclip_f() }
}

unsafe fn host_noclip_f() -> Raise {
    // SAFETY: engine globals (`pr_global_struct`/`sv_player`/`noclip_anglehack`)
    // accessed on the single-threaded host frame; see the `# Safety` doc on
    // the caller.
    unsafe {
        if cmd_src() != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return 0;
        }
        let pgs = ptr::addr_of!(csv::pr_global_struct)
            .read()
            .cast::<GlobalVars>();
        if (*pgs).deathmatch != 0.0 {
            return 0;
        }
        let player = ptr::addr_of!(csu::sv_player).read().cast::<Edict>();
        match c::Cmd_Argc() {
            1 => {
                if (*player).v.movetype != MOVETYPE_NOCLIP {
                    ptr::addr_of_mut!(cview::noclip_anglehack).write(true);
                    (*player).v.movetype = MOVETYPE_NOCLIP;
                    raise!(client_print(b"noclip ON\n\0"));
                } else {
                    ptr::addr_of_mut!(cview::noclip_anglehack).write(false);
                    (*player).v.movetype = MOVETYPE_WALK;
                    raise!(client_print(b"noclip OFF\n\0"));
                }
            }
            2 => {
                if g::atof(c::Cmd_Argv(1)) != 0.0 {
                    ptr::addr_of_mut!(cview::noclip_anglehack).write(true);
                    (*player).v.movetype = MOVETYPE_NOCLIP;
                    raise!(client_print(b"noclip ON\n\0"));
                } else {
                    ptr::addr_of_mut!(cview::noclip_anglehack).write(false);
                    (*player).v.movetype = MOVETYPE_WALK;
                    raise!(client_print(b"noclip OFF\n\0"));
                }
            }
            _ => {
                c::Con_Printf(
                    c"noclip [value] : toggle noclip mode. values: 0 = off, 1 = on\n".as_ptr(),
                );
            }
        }
        0
    }
}

/// `Host_SetPos_f` (`host_cmd.c:1128-1176`).
///
/// # Safety
/// FFI entry point; touches the live `sv_player` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_setpos_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if cmd_src() != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return 0;
        }
        let pgs = ptr::addr_of!(csv::pr_global_struct)
            .read()
            .cast::<GlobalVars>();
        if (*pgs).deathmatch != 0.0 {
            return 0;
        }
        let player = ptr::addr_of!(csu::sv_player).read().cast::<Edict>();

        let argc = c::Cmd_Argc();
        if argc != 7 && argc != 4 {
            raise!(client_print(b"usage:\n\0"));
            raise!(client_print(b"   setpos <x> <y> <z>\n\0"));
            raise!(client_print(
                b"   setpos <x> <y> <z> <pitch> <yaw> <roll>\n\0"
            ));
            raise!(client_print(b"current values:\n\0"));
            raise!(crate::host::quake_rs_host_sv_client_printf(g::va(
                c"   %i %i %i %i %i %i\n".as_ptr(),
                (*player).v.origin[0] as c_int,
                (*player).v.origin[1] as c_int,
                (*player).v.origin[2] as c_int,
                (*player).v.v_angle[0] as c_int,
                (*player).v.v_angle[1] as c_int,
                (*player).v.v_angle[2] as c_int,
            )));
            return 0;
        }

        if (*player).v.movetype != MOVETYPE_NOCLIP {
            ptr::addr_of_mut!(cview::noclip_anglehack).write(true);
            (*player).v.movetype = MOVETYPE_NOCLIP;
            raise!(client_print(b"noclip ON\n\0"));
        }

        (*player).v.velocity = [0.0, 0.0, 0.0];

        (*player).v.origin[0] = g::atof(c::Cmd_Argv(1)) as c_float;
        (*player).v.origin[1] = g::atof(c::Cmd_Argv(2)) as c_float;
        (*player).v.origin[2] = g::atof(c::Cmd_Argv(3)) as c_float;

        if argc == 7 {
            (*player).v.angles[0] = g::atof(c::Cmd_Argv(4)) as c_float;
            (*player).v.angles[1] = g::atof(c::Cmd_Argv(5)) as c_float;
            (*player).v.angles[2] = g::atof(c::Cmd_Argv(6)) as c_float;
            (*player).v.fixangle = 1.0;
        }

        g::SV_LinkEdict(player.cast(), false);
        0
    }
}

/// `Host_Fly_f` (`host_cmd.c:1185-1228`).
///
/// # Safety
/// FFI entry point; touches the live `sv_player` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_fly_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if cmd_src() != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return 0;
        }
        let pgs = ptr::addr_of!(csv::pr_global_struct)
            .read()
            .cast::<GlobalVars>();
        if (*pgs).deathmatch != 0.0 {
            return 0;
        }
        let player = ptr::addr_of!(csu::sv_player).read().cast::<Edict>();
        match c::Cmd_Argc() {
            1 => {
                if (*player).v.movetype != MOVETYPE_FLY {
                    (*player).v.movetype = MOVETYPE_FLY;
                    raise!(client_print(b"flymode ON\n\0"));
                } else {
                    (*player).v.movetype = MOVETYPE_WALK;
                    raise!(client_print(b"flymode OFF\n\0"));
                }
            }
            2 => {
                if g::atof(c::Cmd_Argv(1)) != 0.0 {
                    (*player).v.movetype = MOVETYPE_FLY;
                    raise!(client_print(b"flymode ON\n\0"));
                } else {
                    (*player).v.movetype = MOVETYPE_WALK;
                    raise!(client_print(b"flymode OFF\n\0"));
                }
            }
            _ => {
                c::Con_Printf(c"fly [value] : toggle fly mode. values: 0 = off, 1 = on\n".as_ptr());
            }
        }
        0
    }
}

/// `Host_Ping_f` (`host_cmd.c:1236-1259`).
///
/// # Safety
/// FFI entry point; touches the live `svs` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_ping_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if cmd_src() != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return 0;
        }
        raise!(client_print(b"Client ping times:\n\0"));
        for i in 0..(*svs_p()).maxclients {
            let client = (*svs_p()).clients.add(i as usize);
            if !(*client).spawned || (*client).netconnection.is_null() {
                continue;
            }
            let mut total: c_float = 0.0;
            for j in 0..quake_types::host::NUM_PING_TIMES {
                total += (*client).ping_times[j];
            }
            total /= quake_types::host::NUM_PING_TIMES as c_float;
            raise!(crate::host::quake_rs_host_sv_client_printf(g::va(
                c"%4i %s\n".as_ptr(),
                (total * 1000.0) as c_int,
                (*client).name.as_ptr(),
            )));
        }
        0
    }
}

/// `Host_Map_f` (`host_cmd.c:1278-1342`).
///
/// # Safety
/// FFI entry point; touches the live `sv`/`svs`/`cl`/`cls` engine globals.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_map_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut name: [c_char; quake_types::fs::MAX_QPATH] = [0; quake_types::fs::MAX_QPATH];

        if c::Cmd_Argc() < 2 {
            if (*cls_p()).state == CA_DEDICATED {
                if (*sv_p()).active {
                    c::Con_Printf(
                        c"Current map: %s\n".as_ptr(),
                        ptr::addr_of!((*sv_p()).name).cast::<c_char>(),
                    );
                } else {
                    c::Con_Printf(c"Server not active\n".as_ptr());
                }
            } else if (*cls_p()).state == CA_CONNECTED {
                c::Con_Printf(
                    c"Current map: %s ( %s )\n".as_ptr(),
                    ptr::addr_of!((*cl_p()).levelname).cast::<c_char>(),
                    ptr::addr_of!((*cl_p()).mapname).cast::<c_char>(),
                );
            } else {
                c::Con_Printf(c"map <levelname>: start a new server\n".as_ptr());
            }
            return 0;
        }

        if cmd_src() != SRC_COMMAND {
            return 0;
        }

        (*cls_p()).demonum = -1;

        raise!(chost::Host_Glue_CL_Disconnect());
        raise!(crate::host::quake_rs_host_shutdown_server(false));

        if (*cls_p()).state != CA_DEDICATED {
            g::IN_Activate();
        }
        ptr::addr_of_mut!(g::key_dest).write(KEY_GAME);
        raise!(c::cl_main::ClMain_Glue_BeginLoadingPlaque());

        (*svs_p()).serverflags = 0;
        crate::strl::q_strlcpy(
            name.as_mut_ptr(),
            c::Cmd_Argv(1),
            quake_types::fs::MAX_QPATH,
        );
        let p = g::strrchr(name.as_ptr(), b'.' as c_int);
        if !p.is_null() && g::strcmp(p, c".bsp".as_ptr()) == 0 {
            *p = 0;
        }
        csv::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast());
        raise!(crate::sv_main::quake_rs_sv_spawn_server(name.as_ptr()));
        csv::PR_SwitchQCVM(ptr::null_mut());
        if !(*sv_p()).active {
            return 0;
        }

        if (*cls_p()).state != CA_DEDICATED {
            (*cls_p()).spawnparms = [0; quake_types::host::MAX_MAPSTRING];
            let argc = c::Cmd_Argc();
            let mut i = 2;
            while i < argc {
                crate::strl::q_strlcat(
                    ptr::addr_of_mut!((*cls_p()).spawnparms).cast(),
                    c::Cmd_Argv(i),
                    quake_types::host::MAX_MAPSTRING,
                );
                crate::strl::q_strlcat(
                    ptr::addr_of_mut!((*cls_p()).spawnparms).cast(),
                    c" ".as_ptr(),
                    quake_types::host::MAX_MAPSTRING,
                );
                i += 1;
            }
            raise!(g::HostCmd_Glue_CmdExecuteString(
                c"connect local".as_ptr(),
                SRC_COMMAND
            ));
        }
        0
    }
}

/// `Host_Randmap_f` (`host_cmd.c:1351-1379`).
///
/// # Safety
/// FFI entry point; walks the live `extralevels` list.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_randmap_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if cmd_src() != SRC_COMMAND {
            return 0;
        }

        let mut numlevels: c_int = 0;
        let mut level = ptr::addr_of!(g::extralevels).read().cast::<FileListItem>();
        while !level.is_null() {
            numlevels += 1;
            level = (*level).next;
        }

        if numlevels == 0 {
            c::Con_Printf(c"no maps\n".as_ptr());
            return 0;
        }

        let randlevel = c::COM_Rand() % numlevels;

        let mut level = ptr::addr_of!(g::extralevels).read().cast::<FileListItem>();
        let mut i = 0;
        while !level.is_null() {
            if i == randlevel {
                c::Con_Printf(c"Starting map %s...\n".as_ptr(), (*level).name.as_ptr());
                raise!(chost::Host_Glue_CbufAddText(g::va(
                    c"map %s\n".as_ptr(),
                    (*level).name.as_ptr()
                )));
                return 0;
            }
            level = (*level).next;
            i += 1;
        }
        0
    }
}

/// `Host_Changelevel_f` (`host_cmd.c:1388-1427`).
///
/// # Safety
/// FFI entry point; touches the live `sv`/`svs`/`cl`/`cls` engine globals.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_changelevel_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut level: [c_char; quake_types::fs::MAX_QPATH] = [0; quake_types::fs::MAX_QPATH];

        if c::Cmd_Argc() != 2 {
            c::Con_Printf(c"changelevel <levelname> : continue game on a new level\n".as_ptr());
            return 0;
        }
        if !(*sv_p()).active || (*cls_p()).demoplayback {
            c::Con_Printf(c"Only the server may changelevel\n".as_ptr());
            return 0;
        }

        if cvar_value(ptr::addr_of!(chost::autoload)) != 0.0
            && (*sv_p()).lastsave[0] != 0
            && g::q_strcasecmp(ptr::addr_of!((*sv_p()).name).cast(), c::Cmd_Argv(1)) == 0
            && ptr::addr_of!(csv::current_skill).read()
                == (cvar_value(ptr::addr_of!(csv::skill)) + 0.5) as c_int
            && (*svs_p()).maxclients == 1
            && (*cl_p()).intermission == 0
            && (*(*svs_p()).clients.add(0)).active
            && (*(*(*svs_p()).clients.add(0)).edict).v.health <= 0.0
        {
            raise!(chost::Host_Glue_CbufAddText(c"\nrestart\n".as_ptr()));
            return 0;
        }

        // johnfitz -- check for client having map before anything else
        g::q_snprintf(
            level.as_mut_ptr(),
            quake_types::fs::MAX_QPATH,
            c"maps/%s.bsp".as_ptr(),
            c::Cmd_Argv(1),
        );
        if !c::COM_FileExists(level.as_ptr(), ptr::null_mut()) {
            raise!(g::HostCmd_Glue_ErrorCannotFindMap(level.as_ptr()));
            return 0;
        }

        if (*cls_p()).state != CA_DEDICATED {
            g::IN_Activate();
        }
        ptr::addr_of_mut!(g::key_dest).write(KEY_GAME);
        csv::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast());
        raise!(crate::sv_main::quake_rs_sv_save_spawnparms());
        crate::strl::q_strlcpy(
            level.as_mut_ptr(),
            c::Cmd_Argv(1),
            quake_types::fs::MAX_QPATH,
        );
        raise!(crate::sv_main::quake_rs_sv_spawn_server(level.as_ptr()));
        csv::PR_SwitchQCVM(ptr::null_mut());
        // also issue an error if spawn failed -- O.S.
        if !(*sv_p()).active {
            raise!(g::HostCmd_Glue_ErrorCannotRunMap(level.as_ptr()));
        }
        0
    }
}

/// `Host_Restart_f` (`host_cmd.c:1436-1459`).
///
/// # Safety
/// FFI entry point; touches the live `sv`/`svs`/`cls` engine globals.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_restart_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut mapname: [c_char; quake_types::fs::MAX_QPATH] = [0; quake_types::fs::MAX_QPATH];

        if (*cls_p()).demoplayback || !(*sv_p()).active {
            return 0;
        }
        if cmd_src() != SRC_COMMAND {
            return 0;
        }

        if cvar_value(ptr::addr_of!(chost::autoload)) != 0.0
            && (*sv_p()).lastsave[0] != 0
            && g::q_strcasecmp(c::Cmd_Argv(1), c"noload".as_ptr()) != 0
            && g::q_strcasecmp(c::Cmd_Argv(1), c"force".as_ptr()) != 0
        {
            let fast: *const c_char = if cvar_value(ptr::addr_of!(chost::autofastload)) > 0.0 {
                c"fast".as_ptr()
            } else {
                c"".as_ptr()
            };
            raise!(chost::Host_Glue_CbufAddText(g::va(
                c"-use;-jump;-attack;%sload \"%s\"\n".as_ptr(),
                fast,
                ptr::addr_of!((*sv_p()).lastsave).cast::<c_char>(),
            )));
            (*svs_p()).changelevel_issued = false;
            return 0;
        }

        // mapname gets cleared in spawnserver
        crate::strl::q_strlcpy(
            mapname.as_mut_ptr(),
            ptr::addr_of!((*sv_p()).name).cast(),
            quake_types::fs::MAX_QPATH,
        );
        csv::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast());
        raise!(crate::sv_main::quake_rs_sv_spawn_server(mapname.as_ptr()));
        csv::PR_SwitchQCVM(ptr::null_mut());
        if !(*sv_p()).active {
            raise!(g::HostCmd_Glue_ErrorCannotRestartMap(mapname.as_ptr()));
        }
        0
    }
}

/// `Host_Reconnect_f` (`host_cmd.c:1469-1478`).
///
/// # Safety
/// FFI entry point; touches the live `cls` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_reconnect_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe { host_reconnect_f() }
}

unsafe fn host_reconnect_f() -> Raise {
    // SAFETY: engine global (`cls`) accessed on the single-threaded host
    // frame; see the `# Safety` doc on the callers.
    unsafe {
        // cross-map demo playback fix from Baker
        if (*cls_p()).demoplayback {
            return 0;
        }
        if ptr::addr_of!(g::key_dest).read() == KEY_GAME {
            g::IN_Activate();
        }
        raise!(c::cl_main::ClMain_Glue_BeginLoadingPlaque());
        // need new connection messages
        (*cls_p()).signon = 0;
        0
    }
}

/// `Host_Connect_f` (`host_cmd.c:1487-1500`).
///
/// # Safety
/// FFI entry point; touches the live `cls` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_connect_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut name: [c_char; quake_types::fs::MAX_QPATH] = [0; quake_types::fs::MAX_QPATH];

        // stop demo loop in case this fails
        (*cls_p()).demonum = -1;
        if (*cls_p()).demoplayback {
            crate::cl_demo::quake_rs_cl_stop_playback();
            raise!(chost::Host_Glue_CL_Disconnect());
        }
        crate::strl::q_strlcpy(
            name.as_mut_ptr(),
            c::Cmd_Argv(1),
            quake_types::fs::MAX_QPATH,
        );
        raise!(g::HostCmd_Glue_CLEstablishConnection(name.as_ptr()));
        raise!(host_reconnect_f());
        0
    }
}

// ===========================================================================
// CHUNK C
// ===========================================================================

// ===========================================================================
// ==== CHUNK C -- host_cmd.c:1510-2156 ====
//
// `SAVEGAME_VERSION`, `Host_SavegameComment`, `Host_Savegame_f`,
// `Send_Spawn_Info`, `Host_Loadgame_f`.
//
// These bodies are the savegame byte-diff subjects
// (`scripts/harness/save_diff.py`). Every `fprintf` below is issued as a C
// variadic with the same format string and the same argument order as the C,
// and the reader consumes the file through the same `COM_Parse*Newline`
// helpers, so the emitted bytes and the consumed bytes are C's by
// construction rather than by re-derivation (ADR-005).
// ===========================================================================

use quake_types::fs::MAX_QPATH;
use quake_types::host::{
    EntityOpaque, CA_CONNECTED, CA_DEDICATED, MAX_DEMONAME, MAX_DEMOS, MAX_LIGHTSTYLES, MAX_MODELS,
    MAX_PARTICLETYPES, NUM_TOTAL_SPAWN_PARMS,
};
use quake_types::progs::{Edict, EntityState, GlobalVars, QcVm};
use quake_types::sound::MAX_SOUNDS;

// ---------------------------------------------------------------------------
// constants

/// `host_cmd.c:1510`.
const SAVEGAME_VERSION: c_int = 5;
/// `quakedef.h:97`.
const SAVEGAME_COMMENT_LENGTH: usize = 39;
/// `server.h:116`.
const NUM_BASIC_SPAWN_PARMS: usize = 16;
/// `client.h:68`.
const SIGNONS: c_int = 4;
/// `client.h:70`, `:85`, `:330`.
const MAX_DLIGHTS: usize = 64;
const MAX_BEAMS: usize = 32;
const MAX_TEMP_ENTITIES: usize = 256;
/// `quakedef.h:124-127`.
const STAT_TOTALSECRETS: c_int = 11;
const STAT_TOTALMONSTERS: c_int = 12;
const STAT_SECRETS: c_int = 13;
const STAT_MONSTERS: c_int = 14;
/// `protocol.h:248`, `:252`, `:257`, `:263`, `:264`, `:265`, `:267`.
const SVC_UPDATESTAT: c_int = 3;
const SVC_TIME: c_int = 7;
const SVC_SETANGLE: c_int = 10;
const SVC_LIGHTSTYLE: c_int = 12;
const SVC_UPDATENAME: c_int = 13;
const SVC_UPDATEFRAGS: c_int = 14;
const SVC_UPDATECOLORS: c_int = 17;
/// `protocol.h` -- the two `PEXT2_*` bits `Send_Spawn_Info` tests.
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;
const PEXT2_PREDINFO: c_uint = 0x0000_0020;

/// `hostcmd_write_t.kind` values -- must match `Quake/host_cmd_glue.c`.
const W_BYTE: c_int = 0;
const W_SHORT: c_int = 1;
const W_LONG: c_int = 2;
const W_FLOAT: c_int = 3;
const W_STRING: c_int = 4;
const W_ANGLE: c_int = 5;

/// How many `MSG_Write*` ops are buffered before a flush. `Send_Spawn_Info`
/// never reads `cursize`, so this is a headroom figure only -- the emitted
/// bytes do not depend on it.
const WRITE_BATCH: usize = 16;

// ---------------------------------------------------------------------------
// ADR-011 exception: symbols typed with a `quake-types` mirror. `quake-c-sys`
// has no `[dependencies]`, so these are declared here, exactly as
// `cl_main.rs:166-173` does.

extern "C" {
    /// `Quake/protocol.c` -- the all-zero baseline `entity_state_t`.
    static nullentitystate: EntityState;
    /// `Quake/cl_tent_glue.c` (`cl_tent.c:27`).
    static mut cl_temp_entities: [EntityOpaque; MAX_TEMP_ENTITIES];
}

// ---------------------------------------------------------------------------
// chunk-C helpers

/// The ambient qcvm (ADR-008).
#[inline]
unsafe fn qcvm_p() -> *mut QcVm {
    // SAFETY: engine global, single-threaded.
    unsafe { ptr::addr_of_mut!(c::qcvm).read().cast::<QcVm>() }
}

/// `pr_global_struct`, the ambient qcvm's `globals` under another name.
#[inline]
unsafe fn globals_p() -> *mut GlobalVars {
    // SAFETY: engine global, single-threaded.
    unsafe {
        ptr::addr_of_mut!(c::sv_main::pr_global_struct)
            .read()
            .cast::<GlobalVars>()
    }
}

/// `com_token`, which is `THREAD_LOCAL` -- it must be reached through the
/// accessor, never through a plain `extern` (`common.h`).
#[inline]
unsafe fn com_token() -> *const c_char {
    // SAFETY: returns this thread's buffer, valid until the next COM_Parse.
    unsafe { c::COM_ThreadToken() }
}

/// Accumulates `MSG_Write*` calls against one `sizebuf_t` and hands them to
/// `HostCmd_Glue_WriteBatch`, which replays the run inside a single
/// `Host_Guard` frame (ADR-009 rule 3) -- every writer reaches `SZ_GetSpace`,
/// which `Host_Error`s on overflow (`net_msg.c:488`).
struct Writer {
    sb: *mut c_void,
    ops: [g::HostCmdWriteOp; WRITE_BATCH],
    n: usize,
}

impl Writer {
    fn new(sb: *mut c_void) -> Self {
        Writer {
            sb,
            ops: [g::HostCmdWriteOp {
                kind: 0,
                i: 0,
                f: 0.0,
                u: 0,
                p: ptr::null(),
            }; WRITE_BATCH],
            n: 0,
        }
    }

    unsafe fn flush(&mut self) -> Raise {
        if self.n == 0 {
            return 0;
        }
        let count = self.n;
        self.n = 0;
        // SAFETY: `sb` points at a live `sizebuf_t`; `ops[..count]` is
        // initialised and every `p` pointer is still live at this point.
        unsafe { g::HostCmd_Glue_WriteBatch(self.sb, self.ops.as_ptr(), count as c_int) }
    }

    unsafe fn push(
        &mut self,
        kind: c_int,
        i: c_int,
        f: c_float,
        u: c_uint,
        p: *const c_void,
    ) -> Raise {
        if self.n == WRITE_BATCH {
            // SAFETY: see `flush`.
            let r = unsafe { self.flush() };
            if r != 0 {
                return r;
            }
        }
        self.ops[self.n] = g::HostCmdWriteOp { kind, i, f, u, p };
        self.n += 1;
        0
    }

    unsafe fn byte(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_BYTE, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn short(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_SHORT, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn long(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_LONG, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn float(&mut self, v: c_float) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_FLOAT, 0, v, 0, ptr::null()) }
    }

    /// `s` must stay live until the next flush.
    unsafe fn string(&mut self, s: *const c_char) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_STRING, 0, 0.0, 0, s.cast::<c_void>()) }
    }

    unsafe fn angle(&mut self, f: c_float, flags: c_uint) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_ANGLE, 0, f, flags, ptr::null()) }
    }
}

// ---------------------------------------------------------------------------
// ADR-009 raise codes issued by chunk C itself.
//
// `host_cmd.c:1879` and `host_cmd.c:2066` call `Host_Error` from inside the
// ported bodies. A Rust frame must not longjmp, so each site returns a
// file-local code that `HostCmd_Raise` in `Quake/host_cmd_glue.c` turns back
// into the identical `Host_Error` call from a pure C frame.
//
//   code   site               C call the glue re-issues
//   -101   host_cmd.c:1879    Host_Error ("Savegame is version %i, not %i",
//                               <raise detail>, SAVEGAME_VERSION)
//   -102   host_cmd.c:2066    Host_Error ("First token isn't a brace")
//
// Codes stay negative so they can never collide with a `Host_Guard` status
// (0/1/2), matching `Quake/cl_main_glue.c`.

const HOSTCMD_RAISE_SAVEGAME_VERSION: Raise = -101;
const HOSTCMD_RAISE_FIRST_TOKEN_BRACE: Raise = -102;

/// The one `int` argument `HOSTCMD_RAISE_SAVEGAME_VERSION` carries. Written
/// immediately before the code is returned and read by `HostCmd_Raise`
/// through `quake_rs_hostcmd_raise_detail` before the `Host_Error` fires;
/// nothing else runs in between.
static mut RAISE_DETAIL: c_int = 0;

/// The `int` argument of the pending chunk-C `Host_Error`.
///
/// FFI entry point.
#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_raise_detail() -> c_int {
    // SAFETY: engine-thread-only scalar.
    unsafe { ptr::addr_of!(RAISE_DETAIL).read() }
}

// ---------------------------------------------------------------------------
// host_cmd.c:1519 -- Host_SavegameComment
// ---------------------------------------------------------------------------

/// Writes a `SAVEGAME_COMMENT_LENGTH` character comment describing the
/// current level. No callee here can raise, so this is a plain `void`.
unsafe fn Host_SavegameComment(text: *mut c_char) {
    // SAFETY: `text` is the caller's `char[SAVEGAME_COMMENT_LENGTH + 1]`.
    unsafe {
        let mut i: c_int;

        for k in 0..SAVEGAME_COMMENT_LENGTH {
            text.add(k).write(b' ' as c_char);
        }
        text.add(SAVEGAME_COMMENT_LENGTH).write(0);

        // Remove CR/LFs from level name to avoid broken saves, e.g. with
        // autumn_sp map: sanitize Level name:
        let mut cleanname: [c_char; 128] = [0; 128];
        g::COM_SanitizeDescriptionString(
            cleanname.as_mut_ptr(),
            core::mem::size_of::<[c_char; 128]>(),
            ptr::addr_of!((*cl_p()).levelname).cast::<c_char>(),
            true,
        );

        i = c::sv_send::strlen(cleanname.as_ptr()) as c_int;
        if i > 22 {
            i = 22;
        }
        ptr::copy_nonoverlapping(cleanname.as_ptr(), text, i as usize);

        let mut kills: [c_char; 20] = [0; 20];
        g::sprintf(
            kills.as_mut_ptr(),
            c"kills:%3i/%3i".as_ptr(),
            (*cl_p()).stats[STAT_MONSTERS as usize],
            (*cl_p()).stats[STAT_TOTALMONSTERS as usize],
        );
        ptr::copy_nonoverlapping(
            kills.as_ptr(),
            text.add(22),
            c::sv_send::strlen(kills.as_ptr()),
        );

        // convert space to _ to make stdio happy
        for k in 0..SAVEGAME_COMMENT_LENGTH {
            if text.add(k).read() == b' ' as c_char {
                text.add(k).write(b'_' as c_char);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:1554 -- Host_Savegame_f
// ---------------------------------------------------------------------------

unsafe fn Host_Savegame_f() -> Raise {
    // SAFETY: engine state, single-threaded; `f` is a libc FILE this body
    // opens, appends to and closes.
    unsafe {
        let mut name: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];
        let mut i: c_int;
        let mut comment: [c_char; SAVEGAME_COMMENT_LENGTH + 1] = [0; SAVEGAME_COMMENT_LENGTH + 1];

        if ptr::addr_of!(c::cvar_cmd::cmd_source).read() != c::cmd_source_t_src_command {
            return 0;
        }

        if !(*sv_p()).active {
            c::Con_Printf(c"Not playing a local game.\n".as_ptr());
            return 0;
        }

        if (*sv_p()).nomonsters {
            c::Con_Printf(c"Can't save when using \"nomonsters\".\n".as_ptr());
            return 0;
        }

        if (*cl_p()).intermission != 0 {
            c::Con_Printf(c"Can't save in intermission.\n".as_ptr());
            return 0;
        }

        if (*svs_p()).maxclients != 1 {
            c::Con_Printf(c"Can't save multiplayer games.\n".as_ptr());
            return 0;
        }

        if c::Cmd_Argc() != 2 {
            c::Con_Printf(c"save <savename> : save a game\n".as_ptr());
            return 0;
        }

        if !c::sv_send::strstr(c::Cmd_Argv(1), c"..".as_ptr()).is_null() {
            c::Con_Printf(c"Relative pathnames are not allowed.\n".as_ptr());
            return 0;
        }

        i = 0;
        while i < (*svs_p()).maxclients {
            let cl_i = (*svs_p()).clients.add(i as usize);
            if (*cl_i).active && (*(*cl_i).edict).v.health <= 0.0 {
                c::Con_Printf(c"Can't savegame with a dead player\n".as_ptr());
                return 0;
            }
            i += 1;
        }

        if ptr::addr_of!(c::multiuser).read() {
            let save_path =
                c::Sys_GetPrefPath(c"vkQuake".as_ptr(), c::sv_main::COM_GetGameNames(true));
            c::cl_main::q_snprintf(
                name.as_mut_ptr(),
                MAX_OSPATH,
                c"%s%s".as_ptr(),
                save_path,
                c::Cmd_Argv(1),
            );
            c::Mem_Free(save_path.cast::<c_void>());
        } else {
            c::cl_main::q_snprintf(
                name.as_mut_ptr(),
                MAX_OSPATH,
                c"%s/%s".as_ptr(),
                ptr::addr_of!(c::com_gamedir).cast::<c_char>(),
                c::Cmd_Argv(1),
            );
        }
        c::COM_AddExtension(name.as_mut_ptr(), c".sav".as_ptr(), MAX_OSPATH);

        c::Con_SafePrintf(c"Saving game to ".as_ptr());
        c::cl_demo::Con_LinkPrintf(name.as_ptr(), c"%s".as_ptr(), name.as_ptr());
        c::Con_SafePrintf(c"...\n".as_ptr());
        let f: *mut c::FILE = c::Sys_fopen(name.as_ptr(), c"w".as_ptr());
        if f.is_null() {
            c::Con_Printf(c"ERROR: couldn't open.\n".as_ptr());
            return 0;
        }

        // COMPAT: ADR-008 -- host_cmd.c:1629 hardcodes the server qcvm and
        // switches back out at host_cmd.c:1707. A raise between the two
        // leaves the server qcvm current, exactly as C's longjmp does.
        c::sv_main::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast::<c_void>());

        c::cvar_cmd::fprintf(f, c"%i\n".as_ptr(), SAVEGAME_VERSION);
        Host_SavegameComment(comment.as_mut_ptr());
        c::cvar_cmd::fprintf(f, c"%s\n".as_ptr(), comment.as_ptr());
        i = 0;
        while (i as usize) < NUM_BASIC_SPAWN_PARMS {
            // COMPAT: ADR-005 -- `%f` takes the default argument promotion, so
            // the `float` is widened to `double` before the call, exactly as C
            // does. The digits come from the platform libc, not from Rust's
            // formatter.
            c::cvar_cmd::fprintf(
                f,
                c"%f\n".as_ptr(),
                (*(*svs_p()).clients).spawn_parms[i as usize] as f64,
            );
            i += 1;
        }
        c::cvar_cmd::fprintf(
            f,
            c"%d\n".as_ptr(),
            ptr::addr_of!(c::sv_main::current_skill).read(),
        );
        c::cvar_cmd::fprintf(
            f,
            c"%s\n".as_ptr(),
            ptr::addr_of!((*sv_p()).name).cast::<c_char>(),
        );
        c::cvar_cmd::fprintf(f, c"%f\n".as_ptr(), (*qcvm_p()).time);

        // write the light styles
        i = 0;
        while (i as usize) < MAX_LIGHTSTYLES {
            if !(*sv_p()).lightstyles[i as usize].is_null() {
                c::cvar_cmd::fprintf(f, c"%s\n".as_ptr(), (*sv_p()).lightstyles[i as usize]);
            } else {
                c::cvar_cmd::fprintf(f, c"m\n".as_ptr());
            }
            i += 1;
        }

        raise!(g::HostCmd_Glue_EDWriteGlobals(f.cast::<c_void>()));
        i = 0;
        while i < (*qcvm_p()).num_edicts {
            let mut ed: *mut c_void = ptr::null_mut();
            raise!(g::HostCmd_Glue_EdictNum(i, ptr::addr_of_mut!(ed)));
            raise!(g::HostCmd_Glue_EDWrite(f.cast::<c_void>(), ed));
            i += 1;
        }

        // add extra info (lightstyles, precaches, etc) in a way that's
        // supposed to be compatible with DP.
        c::cvar_cmd::fprintf(f, c"/*\n".as_ptr());
        c::cvar_cmd::fprintf(f, c"// QuakeSpasm extended savegame\n".as_ptr());
        // COMPAT: host_cmd.c:1661 starts this loop at MAX_LIGHTSTYLES, so it
        // never executes. Transliterated as written -- see t83_C_notes.md.
        i = MAX_LIGHTSTYLES as c_int;
        while (i as usize) < MAX_LIGHTSTYLES {
            if !(*sv_p()).lightstyles[i as usize].is_null() {
                c::cvar_cmd::fprintf(
                    f,
                    c"sv.lightstyles %i \"%s\"\n".as_ptr(),
                    i,
                    (*sv_p()).lightstyles[i as usize],
                );
            }
            i += 1;
        }
        i = 1;
        while (i as usize) < MAX_MODELS {
            if !(*sv_p()).model_precache[i as usize].is_null() {
                c::cvar_cmd::fprintf(
                    f,
                    c"sv.model_precache %i \"%s\"\n".as_ptr(),
                    i,
                    (*sv_p()).model_precache[i as usize],
                );
            }
            i += 1;
        }
        i = 1;
        while (i as usize) < MAX_SOUNDS {
            if !(*sv_p()).sound_precache[i as usize].is_null() {
                c::cvar_cmd::fprintf(
                    f,
                    c"sv.sound_precache %i \"%s\"\n".as_ptr(),
                    i,
                    (*sv_p()).sound_precache[i as usize],
                );
            }
            i += 1;
        }
        i = 1;
        while (i as usize) < MAX_PARTICLETYPES {
            if !(*sv_p()).particle_precache[i as usize].is_null() {
                c::cvar_cmd::fprintf(
                    f,
                    c"sv.particle_precache %i \"%s\"\n".as_ptr(),
                    i,
                    (*sv_p()).particle_precache[i as usize],
                );
            }
            i += 1;
        }

        c::cvar_cmd::fprintf(f, c"sv.serverflags %i\n".as_ptr(), (*svs_p()).serverflags);
        i = NUM_BASIC_SPAWN_PARMS as c_int;
        while (i as usize) < NUM_TOTAL_SPAWN_PARMS {
            if (*(*svs_p()).clients).spawn_parms[i as usize] != 0.0 {
                c::cvar_cmd::fprintf(
                    f,
                    c"spawnparm %i \"%f\"\n".as_ptr(),
                    i + 1,
                    (*(*svs_p()).clients).spawn_parms[i as usize] as f64,
                );
            }
            i += 1;
        }

        // COMPAT: ADR-005 -- the five `%g` numbers a savegame carries are
        // formatted inside `gl_fog.c:281` and `gl_sky.c:541`. Both strings are
        // emitted opaquely through `%s`; nothing here re-derives them.
        let fog_cmd: *const c_char = g::Fog_GetFogCommand(true);
        if !fog_cmd.is_null() {
            c::cvar_cmd::fprintf(f, c"%s".as_ptr(), fog_cmd.add(1));
        }

        let sky_cmd: *const c_char = g::Sky_GetSkyCommand(true);
        if !sky_cmd.is_null() {
            c::cvar_cmd::fprintf(f, c"%s".as_ptr(), sky_cmd.add(1));
        }

        c::cvar_cmd::fprintf(f, c"*/\n".as_ptr());

        c::stdio::fclose(f);

        // Take the occasion to check the free-list
        // this is a long operation anyway.
        raise!(g::HostCmd_Glue_EDCheckFreeList());

        c::Con_Printf(c"done.\n".as_ptr());

        c::sv_main::PR_SwitchQCVM(ptr::null_mut());
        raise!(g::HostCmd_Glue_SaveListRebuild());

        if c::sv_send::strlen(c::Cmd_Argv(1)) < core::mem::size_of::<[c_char; 128]>() - 1 {
            c::cl_main::strcpy(
                ptr::addr_of_mut!((*sv_p()).lastsave).cast::<c_char>(),
                c::Cmd_Argv(1),
            );
        }
        0
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:1714 -- Send_Spawn_Info
// ---------------------------------------------------------------------------

unsafe fn Send_Spawn_Info(cl_c: *mut Client, loadgame: bool) -> Raise {
    // SAFETY: `cl_c` points into `svs.clients`; `sv`/`svs` and the ambient
    // qcvm are engine state.
    unsafe {
        let mut i: c_int;
        let mut client: *mut Client;

        // send all current names, colors, and frag counts
        c::cvar_cmd::SZ_Clear(ptr::addr_of_mut!((*cl_c).message).cast());

        let mut wr = Writer::new(ptr::addr_of_mut!((*cl_c).message).cast::<c_void>());

        // send time of update
        raise!(wr.byte(SVC_TIME));
        // COMPAT: ADR-010 -- `qcvm->time` is a `double`; `MSG_WriteFloat` takes
        // a `float`, so the narrowing happens at the call exactly as C does it.
        raise!(wr.float((*qcvm_p()).time as c_float));
        if (*cl_c).protocol_pext2 & PEXT2_PREDINFO != 0 {
            raise!(wr.short((*cl_c).lastmovemessage & 0xffff));
        }

        i = 0;
        client = (*svs_p()).clients;
        while i < (*svs_p()).maxclients {
            if !(*client).knowntoqc {
                i += 1;
                client = client.add(1);
                continue;
            }

            raise!(wr.byte(SVC_UPDATENAME));
            raise!(wr.byte(i));
            raise!(wr.string(ptr::addr_of!((*client).name).cast::<c_char>()));
            raise!(wr.byte(SVC_UPDATECOLORS));
            raise!(wr.byte(i));
            raise!(wr.byte((*client).colors));

            raise!(wr.byte(SVC_UPDATEFRAGS));
            raise!(wr.byte(i));
            raise!(wr.short((*client).old_frags));

            i += 1;
            client = client.add(1);
        }

        // send all current light styles
        i = 0;
        while (i as usize) < MAX_LIGHTSTYLES {
            raise!(wr.byte(SVC_LIGHTSTYLE));
            raise!(wr.byte((i as c_char) as c_int));
            raise!(wr.string((*sv_p()).lightstyles[i as usize]));
            i += 1;
        }

        //
        // send some stats
        //
        // COMPAT: ADR-010 -- `total_secrets` and friends are `float` in
        // `pr_global_struct`; `MSG_WriteLong` takes an `int`, so C converts by
        // truncation toward zero at the call.
        raise!(wr.byte(SVC_UPDATESTAT));
        raise!(wr.byte(STAT_TOTALSECRETS));
        raise!(wr.long((*globals_p()).total_secrets as c_int));

        raise!(wr.byte(SVC_UPDATESTAT));
        raise!(wr.byte(STAT_TOTALMONSTERS));
        raise!(wr.long((*globals_p()).total_monsters as c_int));

        raise!(wr.byte(SVC_UPDATESTAT));
        raise!(wr.byte(STAT_SECRETS));
        raise!(wr.long((*globals_p()).found_secrets as c_int));

        raise!(wr.byte(SVC_UPDATESTAT));
        raise!(wr.byte(STAT_MONSTERS));
        raise!(wr.long((*globals_p()).killed_monsters as c_int));

        //
        // send a fixangle
        // Never send a roll angle, because savegames can catch the server
        // in a state where it is expecting the client to correct the angle
        // and it won't happen if the game was just loaded, so you wind up
        // with a permanent head tilt
        let mut ent_p: *mut c_void = ptr::null_mut();
        raise!(wr.flush());
        raise!(g::HostCmd_Glue_EdictNum(
            1 + cl_c.offset_from((*svs_p()).clients) as c_int,
            ptr::addr_of_mut!(ent_p)
        ));
        let ent: *mut Edict = ent_p.cast::<Edict>();
        raise!(wr.byte(SVC_SETANGLE));
        i = 0;
        while i < 2 {
            if loadgame {
                raise!(wr.angle((*ent).v.v_angle[i as usize], (*sv_p()).protocolflags));
            } else {
                raise!(wr.angle((*ent).v.angles[i as usize], (*sv_p()).protocolflags));
            }
            i += 1;
        }
        raise!(wr.angle(0.0, (*sv_p()).protocolflags));
        raise!(wr.flush());

        if (*cl_c).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
            raise!(g::HostCmd_Glue_SVWriteClientdataToMessage(
                cl_c.cast::<c_void>(),
                ptr::addr_of_mut!((*cl_c).message).cast::<c_void>()
            ));
        }
        0
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:1797 -- Host_Loadgame_f
// ---------------------------------------------------------------------------

/// `host_cmd.c:1799` `static char *start;` -- a function-`static` with
/// internal linkage, so no ADR-007 row opens: nothing outside `host_cmd.c`
/// could ever name it. It survives across calls so a `Host_Error` in the
/// middle of a load can be cleaned up by the next one.
static mut LOADGAME_START: *mut c_char = ptr::null_mut();

// COMPAT: host_cmd.c's extended-block parser assigns `ext = COM_Parse (ext)`
// for the last field of several branches and then unconditionally overwrites
// it with `ext = end + 1` at host_cmd.c:2057. Those dead stores are the C's,
// and the parse call still has to happen for `com_token` to be advanced.
#[allow(unused_assignments)]
unsafe fn Host_Loadgame_f() -> Raise {
    // SAFETY: engine state, single-threaded.
    unsafe {
        let mut name: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];
        let mut mapname: [c_char; MAX_QPATH] = [0; MAX_QPATH];
        let mut time: c_float = 0.0;
        let mut tfloat: c_float = 0.0;
        let mut data: *const c_char;
        let mut i: c_int;
        let mut entnum: c_int;
        let mut version: c_int = 0;
        let mut spawn_parms: [c_float; NUM_TOTAL_SPAWN_PARMS] = [0.0; NUM_TOTAL_SPAWN_PARMS];
        let was_recording: bool = (*cls_p()).demorecording;
        let old_skill: c_int = ptr::addr_of!(c::sv_main::current_skill).read();
        let mut fastload: bool = !c::sv_send::strstr(c::Cmd_Argv(0), c"fast".as_ptr()).is_null()
            || cvar_value(ptr::addr_of!(c::host::autofastload)) != 0.0;

        if ptr::addr_of!(c::cvar_cmd::cmd_source).read() != c::cmd_source_t_src_command {
            return 0;
        }

        if c::Cmd_Argc() != 2 {
            c::Con_Printf(c"%s <savename> : load a game\n".as_ptr(), c::Cmd_Argv(0));
            return 0;
        }

        if !c::sv_send::strstr(c::Cmd_Argv(1), c"..".as_ptr()).is_null() {
            c::Con_Printf(c"Relative pathnames are not allowed.\n".as_ptr());
            return 0;
        }

        let nomonsters_p = ptr::addr_of_mut!(c::sv_main::nomonsters);
        if cvar_value(nomonsters_p) != 0.0 {
            c::Con_Warning(
                c"\"%s\" disabled automatically.\n".as_ptr(),
                ptr::addr_of!((*nomonsters_p).name).read(),
            );
            raise!(g::HostCmd_Glue_CvarSetValueQuick(nomonsters_p, 0.0));
        }

        (*cls_p()).demonum = -1; // stop demo loop in case this fails

        let save_path: *mut c_char = if ptr::addr_of!(c::multiuser).read() {
            c::Sys_GetPrefPath(c"vkQuake".as_ptr(), c::sv_main::COM_GetGameNames(true))
        } else {
            ptr::null_mut()
        };
        let mut loadable: bool = false;
        let mut j: c_int = if ptr::addr_of!(c::multiuser).read() {
            0
        } else {
            1
        };
        while j < 2 {
            if j == 0 {
                c::cl_main::q_snprintf(
                    name.as_mut_ptr(),
                    MAX_OSPATH,
                    c"%s%s".as_ptr(),
                    save_path,
                    c::Cmd_Argv(1),
                );
            } else {
                c::cl_main::q_snprintf(
                    name.as_mut_ptr(),
                    MAX_OSPATH,
                    c"%s/%s".as_ptr(),
                    ptr::addr_of!(c::com_gamedir).cast::<c_char>(),
                    c::Cmd_Argv(1),
                );
            }
            c::COM_AddExtension(name.as_mut_ptr(), c".sav".as_ptr(), MAX_OSPATH);

            // avoid leaking if the previous Host_Loadgame_f failed with a
            // Host_Error
            if !ptr::addr_of!(LOADGAME_START).read().is_null() {
                c::Mem_Free(ptr::addr_of!(LOADGAME_START).read().cast::<c_void>());
            }

            ptr::addr_of_mut!(LOADGAME_START).write(
                c::COM_LoadMallocFile_TextMode_OSPath(name.as_ptr(), ptr::null_mut())
                    .cast::<c_char>(),
            );
            if !ptr::addr_of!(LOADGAME_START).read().is_null() {
                loadable = true;
                break;
            }
            j += 1;
        }
        c::Mem_Free(save_path.cast::<c_void>());

        if !loadable {
            g::SCR_EndLoadingPlaque();
            c::Con_Printf(c"ERROR: couldn't open.\n".as_ptr());
            return 0;
        }

        // we can't call SCR_BeginLoadingPlaque, because too much stack space
        // has been used.  The menu calls it before stuffing loadgame command

        c::Con_Printf(c"Loading game from %s...\n".as_ptr(), name.as_ptr());

        data = ptr::addr_of!(LOADGAME_START).read();
        data = g::COM_ParseIntNewline(data, ptr::addr_of_mut!(version));
        if version != SAVEGAME_VERSION {
            c::Mem_Free(ptr::addr_of!(LOADGAME_START).read().cast::<c_void>());
            ptr::addr_of_mut!(LOADGAME_START).write(ptr::null_mut());
            ptr::addr_of_mut!(RAISE_DETAIL).write(version);
            return HOSTCMD_RAISE_SAVEGAME_VERSION;
        }
        data = g::COM_ParseStringNewline(data);
        i = 0;
        while (i as usize) < NUM_BASIC_SPAWN_PARMS {
            data = g::COM_ParseFloatNewline(data, ptr::addr_of_mut!(spawn_parms[i as usize]));
            i += 1;
        }
        while (i as usize) < NUM_TOTAL_SPAWN_PARMS {
            spawn_parms[i as usize] = 0.0;
            i += 1;
        }
        // this silliness is so we can load 1.06 save files, which have float
        // skill values
        data = g::COM_ParseFloatNewline(data, ptr::addr_of_mut!(tfloat));
        // COMPAT: ADR-010 -- `(int)(tfloat + 0.1)` promotes to `double` before
        // the truncation, so the addition happens in `f64`.
        ptr::addr_of_mut!(c::sv_main::current_skill).write((tfloat as f64 + 0.1) as c_int);
        raise!(g::HostCmd_Glue_CvarSetValue(
            c"skill".as_ptr(),
            ptr::addr_of!(c::sv_main::current_skill).read() as c_float
        ));

        data = g::COM_ParseStringNewline(data);
        c::cl_main::q_strlcpy(mapname.as_mut_ptr(), com_token(), MAX_QPATH);
        data = g::COM_ParseFloatNewline(data, ptr::addr_of_mut!(time));

        if fastload
            && (!(*sv_p()).active || (*cls_p()).signon != SIGNONS || (*svs_p()).maxclients != 1)
        {
            c::Con_Printf(
                c"Can't fastload (first load, or is a client multiplayer game)\n".as_ptr(),
            );
            fastload = false;
        }
        if fastload
            && (c::sv_main::strcmp(
                mapname.as_ptr(),
                ptr::addr_of!((*sv_p()).name).cast::<c_char>(),
            ) != 0
                || ptr::addr_of!(c::sv_main::current_skill).read() != old_skill)
        {
            c::Con_Printf(
                c"Can't fastload (%s skill %d vs %s skill %d)\n".as_ptr(),
                mapname.as_ptr(),
                ptr::addr_of!(c::sv_main::current_skill).read(),
                ptr::addr_of!((*sv_p()).name).cast::<c_char>(),
                old_skill,
            );
            fastload = false;
        }
        if fastload && (*cl_p()).intermission != 0 {
            // we could if we reset cl.intermission and the music, but some mods
            // still struggle
            c::Con_Printf(c"Can't fastload during an intermission\n".as_ptr());
            fastload = false;
        }

        if !fastload {
            raise!(g::HostCmd_Glue_CLDisconnect_f());
        } else if (*cls_p()).demorecording {
            // demo playback can't deal with backward timestamps, so record a
            // map change
            raise!(g::HostCmd_Glue_CLStop_f());
        }

        // COMPAT: ADR-008 -- host_cmd.c:1918 switches to the server qcvm and
        // stays there until host_cmd.c:2144.
        c::sv_main::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast::<c_void>());

        if !fastload {
            raise!(g::HostCmd_Glue_SVSpawnServer(mapname.as_ptr()));
        }

        if !(*sv_p()).active {
            c::sv_main::PR_SwitchQCVM(ptr::null_mut());
            c::Mem_Free(ptr::addr_of!(LOADGAME_START).read().cast::<c_void>());
            ptr::addr_of_mut!(LOADGAME_START).write(ptr::null_mut());
            g::SCR_EndLoadingPlaque();
            c::Con_Printf(c"Couldn't load map\n".as_ptr());
            return 0;
        }
        if !fastload {
            (*sv_p()).paused = true; // pause until all clients connect
            (*sv_p()).loadgame = true;
        } else {
            // do this before parsing the edicts, since that may take a while
            g::S_StopAllSounds(true, true);
        }

        if was_recording {
            raise!(g::HostCmd_Glue_CLResumeRecord(fastload));
        }

        // load the light styles
        i = 0;
        while (i as usize) < MAX_LIGHTSTYLES {
            data = g::COM_ParseStringNewline(data);
            (*sv_p()).lightstyles[i as usize] = c::cvar_cmd::q_strdup(com_token());
            i += 1;
        }

        if fastload {
            // can be done for normal loads too, but keep the previous behavior
            g::PR_ClearEdictStrings();
        }

        // load the edicts out of the savegame file
        entnum = -1; // -1 is the globals
        while data.read() != 0 {
            while data.read() == b' ' as c_char
                || data.read() == b'\r' as c_char
                || data.read() == b'\n' as c_char
            {
                data = data.add(1);
            }
            if data.read() == b'/' as c_char
                && data.add(1).read() == b'*' as c_char
                && (data.add(2).read() == b'\r' as c_char || data.add(2).read() == b'\n' as c_char)
            {
                // looks like an extended saved game
                let mut ext: *const c_char = data.add(2);
                loop {
                    let end: *mut c_char = g::strchr(ext, b'\n' as c_int);
                    if end.is_null() {
                        break;
                    }
                    end.write(0);
                    ext = c::COM_Parse(ext);
                    if c::sv_main::strcmp(com_token(), c"sv.lightstyles".as_ptr()) == 0 {
                        ext = c::COM_Parse(ext);
                        let idx: c_int = c::sv_main::atoi(com_token());
                        ext = c::COM_Parse(ext);
                        if idx >= 0 && (idx as usize) < MAX_LIGHTSTYLES {
                            if com_token().read() != 0 {
                                (*sv_p()).lightstyles[idx as usize] =
                                    c::cvar_cmd::q_strdup(com_token());
                            } else {
                                (*sv_p()).lightstyles[idx as usize] = ptr::null();
                            }
                        }
                    } else if c::sv_main::strcmp(com_token(), c"sv.model_precache".as_ptr()) == 0 {
                        ext = c::COM_Parse(ext);
                        let idx: c_int = c::sv_main::atoi(com_token());
                        ext = c::COM_Parse(ext);
                        if idx >= 1 && (idx as usize) < MAX_MODELS {
                            (*sv_p()).model_precache[idx as usize] =
                                c::cvar_cmd::q_strdup(com_token());
                            let mut mod_p: *mut c_void = ptr::null_mut();
                            raise!(g::HostCmd_Glue_ModForName(
                                (*sv_p()).model_precache[idx as usize],
                                idx == 1,
                                ptr::addr_of_mut!(mod_p)
                            ));
                            (*sv_p()).models[idx as usize] = mod_p.cast();
                        }
                    } else if c::sv_main::strcmp(com_token(), c"sv.sound_precache".as_ptr()) == 0 {
                        ext = c::COM_Parse(ext);
                        let idx: c_int = c::sv_main::atoi(com_token());
                        ext = c::COM_Parse(ext);
                        // COMPAT: host_cmd.c:2002 bounds-checks the sound index
                        // against MAX_MODELS, not MAX_SOUNDS. Kept -- see
                        // t83_C_notes.md.
                        if idx >= 1 && (idx as usize) < MAX_MODELS {
                            (*sv_p()).sound_precache[idx as usize] =
                                c::cvar_cmd::q_strdup(com_token());
                        }
                    } else if c::sv_main::strcmp(com_token(), c"sv.particle_precache".as_ptr()) == 0
                    {
                        ext = c::COM_Parse(ext);
                        let idx: c_int = c::sv_main::atoi(com_token());
                        ext = c::COM_Parse(ext);
                        if idx >= 1 && (idx as usize) < MAX_PARTICLETYPES {
                            c::Mem_Free((*sv_p()).particle_precache[idx as usize].cast::<c_void>());
                            (*sv_p()).particle_precache[idx as usize] =
                                c::cvar_cmd::q_strdup(com_token());
                        }
                    } else if c::sv_main::strcmp(com_token(), c"sv.serverflags".as_ptr()) == 0
                        || c::sv_main::strcmp(com_token(), c"svs.serverflags".as_ptr()) == 0
                    {
                        ext = c::COM_Parse(ext);
                        let fl: c_int = c::sv_main::atoi(com_token());
                        (*svs_p()).serverflags = fl;
                    } else if c::sv_main::strcmp(com_token(), c"spawnparm".as_ptr()) == 0 {
                        ext = c::COM_Parse(ext);
                        let idx: c_int = c::sv_main::atoi(com_token());
                        ext = c::COM_Parse(ext);
                        if idx >= 1 && (idx as usize) <= NUM_TOTAL_SPAWN_PARMS {
                            spawn_parms[(idx - 1) as usize] =
                                c::cvar_cmd::atof(com_token()) as c_float;
                        }
                    } else if c::sv_main::strcmp(com_token(), c"fog".as_ptr()) == 0 && fastload {
                        ext = c::COM_Parse(ext);
                        let d: c_float = c::cvar_cmd::atof(com_token()) as c_float;
                        ext = c::COM_Parse(ext);
                        let r: c_float = c::cvar_cmd::atof(com_token()) as c_float;
                        ext = c::COM_Parse(ext);
                        let gr: c_float = c::cvar_cmd::atof(com_token()) as c_float;
                        ext = c::COM_Parse(ext);
                        let b: c_float = c::cvar_cmd::atof(com_token()) as c_float;
                        g::Fog_Update(d, r, gr, b, 0.0);
                    } else if c::sv_main::strcmp(com_token(), c"sky".as_ptr()) == 0 && fastload {
                        ext = c::COM_Parse(ext);
                        raise!(g::HostCmd_Glue_SkyLoadSkyBox(com_token()));
                    } else if c::sv_main::strcmp(com_token(), c"skyfog".as_ptr()) == 0 && fastload {
                        ext = c::COM_Parse(ext);
                        g::Sky_SetSkyfog(c::cvar_cmd::atof(com_token()) as c_float);
                    }
                    end.write(b'\n' as c_char);
                    ext = end.add(1);
                }
            }

            data = c::COM_Parse(data);
            if com_token().read() == 0 {
                break; // end of file
            }
            if c::sv_main::strcmp(com_token(), c"{".as_ptr()) != 0 {
                return HOSTCMD_RAISE_FIRST_TOKEN_BRACE;
            }

            if entnum == -1 {
                // parse the global vars
                let mut out: *const c_char = ptr::null();
                raise!(g::HostCmd_Glue_EDParseGlobals(data, ptr::addr_of_mut!(out)));
                data = out;
            } else {
                // parse an edict
                let mut ent_v: *mut c_void = ptr::null_mut();
                raise!(g::HostCmd_Glue_EdictNum(entnum, ptr::addr_of_mut!(ent_v)));
                let ent: *mut Edict = ent_v.cast::<Edict>();
                if entnum < (*qcvm_p()).num_edicts {
                    (*ent).free = false;
                    ptr::write_bytes(
                        ptr::addr_of_mut!((*ent).v).cast::<u8>(),
                        0,
                        ((*(*qcvm_p()).progs).entityfields * 4) as usize,
                    );
                } else {
                    // adjust qcvm->num_edicts for consistency:
                    let n = (*qcvm_p()).num_edicts;
                    (*qcvm_p()).num_edicts = if n > entnum + 1 { n } else { entnum + 1 };
                    ptr::write_bytes(ent.cast::<u8>(), 0, (*qcvm_p()).edict_size as usize);

                    // host_cmd.c:2087 `assert (!ent->free)` -- omitted, see
                    // t83_C_notes.md: the memset above just cleared the flag,
                    // so it can never fire, and a Rust panic must not cross the
                    // FFI boundary.

                    (*ent).baseline = ptr::addr_of!(nullentitystate).read();
                    // fill debug fields, they were overwriten above:
                    #[cfg(feature = "engine-debug")]
                    {
                        (*ent).qcvm_owner = qcvm_p();
                        (*ent).edict_ptr = ent;
                        (*ent).edict_num = entnum as u64;
                    }
                }
                let mut out: *const c_char = ptr::null();
                raise!(g::HostCmd_Glue_EDParseEdict(
                    data,
                    ent.cast::<c_void>(),
                    ptr::addr_of_mut!(out)
                ));
                data = out;

                // link it into the bsp tree
                if !(*ent).free {
                    raise!(g::HostCmd_Glue_SVLinkEdict(ent.cast::<c_void>(), false));
                }
            }

            entnum += 1;
        }

        (*qcvm_p()).time = time as f64;

        // we finished the edicts loading, free the excess > entnum
        i = entnum;
        while i < (*qcvm_p()).num_edicts {
            let mut ed: *mut c_void = ptr::null_mut();
            raise!(g::HostCmd_Glue_EdictNum(i, ptr::addr_of_mut!(ed)));
            raise!(g::HostCmd_Glue_EDFree(ed));
            i += 1;
        }
        // adjust to the effective nb of edicts
        (*qcvm_p()).num_edicts = entnum;

        // The loading process purposefully bypassed the free-list
        // usage, so rebuild it now
        raise!(g::HostCmd_Glue_EDRebuildFreeList(true));

        if fastload {
            (*sv_p()).lastchecktime = 0.0;
            ptr::write_bytes(
                ptr::addr_of_mut!(c::cl_main::cl_dlights).cast::<u8>(),
                0,
                core::mem::size_of::<[c::cl_tent::dlight_t; MAX_DLIGHTS]>(),
            );
            ptr::write_bytes(
                ptr::addr_of_mut!(cl_temp_entities).cast::<u8>(),
                0,
                core::mem::size_of::<[EntityOpaque; MAX_TEMP_ENTITIES]>(),
            );
            ptr::write_bytes(
                ptr::addr_of_mut!(c::cl_tent::cl_beams).cast::<u8>(),
                0,
                core::mem::size_of::<[c::cl_tent::beam_t; MAX_BEAMS]>(),
            );
            g::V_ResetBlend();
            g::Fog_ResetFade();
            g::R_ClearParticles();
            // `quakedef.h:38` defines PSET_SCRIPT unconditionally, so
            // host_cmd.c:2131's `#ifdef` is always taken.
            g::PScript_ClearParticles(false);
            c::sv_main::SCR_CenterPrintClear();

            raise!(Send_Spawn_Info((*svs_p()).clients, true));
        }

        c::Mem_Free(ptr::addr_of!(LOADGAME_START).read().cast::<c_void>());
        ptr::addr_of_mut!(LOADGAME_START).write(ptr::null_mut());

        i = 0;
        while (i as usize) < NUM_TOTAL_SPAWN_PARMS {
            (*(*svs_p()).clients).spawn_parms[i as usize] = spawn_parms[i as usize];
            i += 1;
        }

        c::sv_main::PR_SwitchQCVM(ptr::null_mut());

        if (*cls_p()).state != CA_DEDICATED && !fastload {
            raise!(g::HostCmd_Glue_CLEstablishConnection(c"local".as_ptr()));
            raise!(g::HostCmd_Glue_HostReconnect_f());
        } else {
            g::SCR_EndLoadingPlaque();
        }

        if c::sv_send::strlen(c::Cmd_Argv(1)) < core::mem::size_of::<[c_char; 128]>() - 1 {
            c::cl_main::strcpy(
                ptr::addr_of_mut!((*sv_p()).lastsave).cast::<c_char>(),
                c::Cmd_Argv(1),
            );
        }
        0
    }
}

// ---------------------------------------------------------------------------
// chunk-C FFI entry points
// ---------------------------------------------------------------------------

/// `host_cmd.c:1519` `Host_SavegameComment`.
///
/// # Safety
/// `text` must be writable for `SAVEGAME_COMMENT_LENGTH + 1` bytes.
///
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_savegame_comment(text: *mut c_char) {
    // SAFETY: caller guarantees the buffer.
    unsafe { Host_SavegameComment(text) }
}

/// `host_cmd.c:1554` `Host_Savegame_f`.
///
/// FFI entry point.
#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_savegame_f() -> Raise {
    // SAFETY: engine state; called only from `Quake/host_cmd_glue.c`.
    unsafe { Host_Savegame_f() }
}

/// `host_cmd.c:1714` `Send_Spawn_Info`. Chunk D's `Host_Spawn_f` calls the
/// private `Send_Spawn_Info` core directly; this export exists so the C side
/// (and the ctest oracle) can reach it too.
///
/// # Safety
/// `client` must point at a live `client_t` inside `svs.clients`.
///
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_send_spawn_info(
    client: *mut c_void,
    loadgame: bool,
) -> Raise {
    // SAFETY: caller guarantees `client`.
    unsafe { Send_Spawn_Info(client.cast::<Client>(), loadgame) }
}

/// `host_cmd.c:1797` `Host_Loadgame_f`.
///
/// FFI entry point.
#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_loadgame_f() -> Raise {
    // SAFETY: engine state; called only from `Quake/host_cmd_glue.c`.
    unsafe { Host_Loadgame_f() }
}

// ==== end CHUNK C ====

// ===========================================================================
// CHUNK D
// ===========================================================================

// ============================================================================
// Chunk D (T8.3): Host_Name_f, Host_Say[_f/_Team_f], Host_Tell_f, Host_Color_f,
// Host_Kill_f, Host_Pause_f, Host_PreSpawn_f, Host_Spawn_f, Host_Begin_f,
// Host_Kick_f -- `host_cmd.c:2158-2649`.
//
// ADR-009: every callee that can reach `Host_Error`/`Host_EndGame` goes
// through a `HostCmd_Glue_*` seam (this chunk's own, declared in
// `t83_D_csys.rs`, or a generic one reused cross-module:
// `Host_Glue_EdictToProg`/`Host_Glue_PR_ExecuteProgram`/`Host_Glue_WriteBatch`
// from `quake_c_sys::host`, `World_Glue_NumForEdict` from `quake_c_sys::world`).
// `PR_SetEngineString`, `ED_FindGlobal` (through `HostCmd_Glue_SetSpawnParmGlobal`),
// `COM_Parse`, `Cmd_Argc`/`Cmd_Argv`/`Cmd_Args`, `NET_QSocketGetTime`,
// `strcmp`/`q_strcasecmp`, `atof`, `q_strlcpy`/`q_snprintf` have no error path
// (confirmed by reading their C bodies, or by the existing plain-call
// precedent for the same symbol elsewhere in this crate) and are called as
// plain externs. No `Host_Error` call exists directly in this line range.
//
// ADR-005: none of this chunk's format strings carry `%g`/`%e`/`%a`; every
// interpolation is `%s`/`%i`. Message text (`Host_Say`, `Host_Tell_f`,
// `Host_Pause_f`'s broadcast, `Host_Kick_f`'s kick message) is formatted with
// the real C `q_snprintf`, called as a variadic extern, so truncation and
// the printed bytes match the C original exactly rather than being
// re-derived by a Rust formatter.
// Note: `g` (`quake_c_sys::host_cmd`) and `Client` are already imported by
// the scaffold this chunk is appended to (`rust/quake-capi/src/host_cmd.rs`
// top-of-file `use` block) -- not re-imported here to avoid an E0252
// duplicate-import error.
use quake_c_sys::host as gh;
use quake_c_sys::progs_builtins_sv as gpb;
use quake_c_sys::sv_main as gsv;
use quake_c_sys::sv_user as gsu;
use quake_c_sys::world as gw;

/// `Host_Guard` returned normally. Defined locally (not imported from
/// `quake_c_sys::host::HOST_GUARD_OK`) because it is a trivial literal and
/// this module should not take a cross-module dependency just for a
/// constant.
const HOST_GUARD_OK: Raise = 0;

/// `Quake/keys.h:134` -- `#define MAXCMDLINE 256`. `Host_Say`/`Host_Tell_f`'s
/// own `text[MAXCMDLINE]` stack buffer.
const MAXCMDLINE: usize = 256;

const SVC_SETPAUSE: c_int = 24;
const SVC_SIGNONNUM: c_int = 25;
const WRITE_BYTE: c_int = 0;

/// `cvar_t::string`, mirroring this module's own `cvar_value` (`.value`).
#[inline]
unsafe fn cvar_string(var: *const c::cvar_t) -> *const c_char {
    // SAFETY: caller passes a live engine cvar.
    unsafe { ptr::addr_of!((*var).string).read() }
}

/// The post-`q_snprintf` truncate/trim step `Host_Say` (`host_cmd.c:2229-2245`)
/// and `Host_Tell_f` (`host_cmd.c:2313-2329`) both run verbatim: if the
/// formatted text filled the buffer, force a trailing `"\n"`; otherwise strip
/// trailing `\r`/`\n`/a single matched closing `"` and append `"\n"`.
unsafe fn trim_chat_text(text: &mut [c_char], quoted: &mut bool) {
    // SAFETY: `text` is NUL-terminated by the preceding `q_snprintf`.
    unsafe {
        let cap = text.len();
        let j = c_strlen(text.as_ptr());
        if j >= cap - 1 {
            text[cap - 2] = b'\n' as c_char;
            text[cap - 1] = 0;
        } else {
            let mut p2 = j;
            while p2 > 0 {
                let ch = text[p2 - 1];
                if ch == b'\r' as c_char
                    || ch == b'\n' as c_char
                    || (ch == b'"' as c_char && *quoted)
                {
                    if ch == b'"' as c_char && *quoted {
                        *quoted = false;
                    }
                    text[p2 - 1] = 0;
                    p2 -= 1;
                } else {
                    break;
                }
            }
            text[p2] = b'\n' as c_char;
            text[p2 + 1] = 0;
        }
    }
}

/// One guarded `MSG_Write*` batch, duplicated from `host.rs:406-410` (the
/// helper there is private to that module).
#[inline]
unsafe fn write_batch(sb: *mut c_void, ops: &[gh::HostWriteOp]) -> Raise {
    // SAFETY: `sb` is a live engine `sizebuf_t`; every `p` outlives the call.
    unsafe { gh::Host_Glue_WriteBatch(sb, ops.as_ptr(), ops.len() as c_int) }
}

#[inline]
fn op_byte(v: c_int) -> gh::HostWriteOp {
    gh::HostWriteOp {
        kind: WRITE_BYTE,
        i: v,
        p: ptr::null(),
    }
}

/// `pr_global_struct`, cast to the ADR-011 mirror. Duplicated from
/// `host.rs:206-213` (private there).
#[inline]
unsafe fn pgs() -> *mut GlobalVars {
    // SAFETY: `pr_global_struct` is a live engine global once `PR_LoadProgs`
    // has run, which every caller here requires transitively (all reachable
    // only once a game is active).
    unsafe {
        ptr::addr_of_mut!(gsv::pr_global_struct)
            .read()
            .cast::<GlobalVars>()
    }
}

/// `qcvm`, cast to the ADR-011 mirror. Duplicated from `host.rs:198-202`
/// (private there).
#[inline]
unsafe fn vm() -> *mut QcVm {
    // SAFETY: same as `pgs()`.
    unsafe { ptr::addr_of_mut!(c::qcvm).read().cast::<QcVm>() }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2165-2193

unsafe fn Host_Name_f() -> Raise {
    // SAFETY: engine globals (`cl_name`, `host_client`, `svs`) are live and
    // single-threaded for the duration of console-command dispatch.
    unsafe {
        let mut new_name = [0 as c_char; 32];

        if c::Cmd_Argc() == 1 {
            c::Con_Printf(
                c"\"name\" is \"%s\"\n".as_ptr(),
                cvar_string(ptr::addr_of!(g::cl_name)),
            );
            return HOST_GUARD_OK;
        }
        if c::Cmd_Argc() == 2 {
            g::q_strlcpy(new_name.as_mut_ptr(), c::Cmd_Argv(1), new_name.len());
        } else {
            g::q_strlcpy(new_name.as_mut_ptr(), g::Cmd_Args(), new_name.len());
        }
        new_name[15] = 0; // client_t structure actually says name[32].

        if g::cmd_source != SRC_CLIENT {
            if g::strcmp(cvar_string(ptr::addr_of!(g::cl_name)), new_name.as_ptr()) == 0 {
                return HOST_GUARD_OK;
            }
            raise!(g::HostCmd_Glue_CvarSet(
                c"_cl_name".as_ptr(),
                new_name.as_ptr()
            ));
            if (*cls_p()).state == CA_CONNECTED {
                raise!(g::HostCmd_Glue_CmdForwardToServer());
            }
            return HOST_GUARD_OK;
        }

        // set the name into the userinfo, otherwise SV_DecodeUserInfo resets
        // it to "unnamed" as soon as another key (e.g. topcolor) is updated
        let hc = host_client_get();
        let base = (*svs_p()).clients;
        let idx = (hc.offset_from(base) as c_int) + 1;
        raise!(crate::cl_main::quake_rs_sv_update_info(
            idx,
            c"name".as_ptr(),
            new_name.as_ptr()
        ));
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2195-2279

unsafe fn Host_Say(mut teamonly: bool) -> Raise {
    // SAFETY: engine globals (`host_client`, `svs`, its `clients` array) are
    // live and single-threaded for the duration of console-command dispatch.
    unsafe {
        let mut from_server = false;

        if g::cmd_source == SRC_COMMAND {
            if (*cls_p()).state != CA_DEDICATED {
                raise!(g::HostCmd_Glue_CmdForwardToServer());
                return HOST_GUARD_OK;
            }
            from_server = true;
            teamonly = false;
        }

        if c::Cmd_Argc() < 2 {
            return HOST_GUARD_OK;
        }

        let save = host_client_get();

        let mut p = g::Cmd_Args();
        let mut quoted = false;
        if *p == b'"' as c_char {
            p = p.add(1);
            quoted = true;
        }

        let mut text = [0 as c_char; MAXCMDLINE];
        if !from_server {
            g::q_snprintf(
                text.as_mut_ptr(),
                MAXCMDLINE,
                c"\x01%s: %s".as_ptr(),
                ptr::addr_of!((*save).name).cast::<c_char>(),
                p,
            );
        } else {
            g::q_snprintf(
                text.as_mut_ptr(),
                MAXCMDLINE,
                c"\x01<%s> %s".as_ptr(),
                cvar_string(ptr::addr_of!(gsv::hostname)),
                p,
            );
        }
        trim_chat_text(&mut text, &mut quoted);

        let svsp = svs_p();
        let maxc = (*svsp).maxclients;
        let clients_base = (*svsp).clients;
        let mut jc: c_int = 0;
        while jc < maxc {
            let client = clients_base.add(jc as usize);
            jc += 1;
            if client.is_null() || !(*client).active || !(*client).spawned {
                continue;
            }
            if cvar_value(ptr::addr_of!(gpb::teamplay)) != 0.0
                && teamonly
                && (*(*client).edict).v.team != (*(*save).edict).v.team
            {
                continue;
            }
            host_client_set(client);
            raise!(crate::host::quake_rs_host_sv_client_printf(text.as_ptr()));
        }
        host_client_set(save);

        if (*cls_p()).state == CA_DEDICATED {
            c::Sys_Printf(c"%s".as_ptr(), text.as_ptr().add(1));
        }
        HOST_GUARD_OK
    }
}

unsafe fn Host_Say_f() -> Raise {
    // SAFETY: see `Host_Say`.
    unsafe { Host_Say(false) }
}

unsafe fn Host_Say_Team_f() -> Raise {
    // SAFETY: see `Host_Say`.
    unsafe { Host_Say(true) }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2281-2342

unsafe fn Host_Tell_f() -> Raise {
    // SAFETY: engine globals (`host_client`, `svs`, its `clients` array) are
    // live and single-threaded for the duration of console-command dispatch.
    unsafe {
        if g::cmd_source != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return HOST_GUARD_OK;
        }

        if c::Cmd_Argc() < 3 {
            return HOST_GUARD_OK;
        }

        let mut p = g::Cmd_Args();
        let mut quoted = false;
        if *p == b'"' as c_char {
            p = p.add(1);
            quoted = true;
        }

        let hc_name = ptr::addr_of!((*host_client_get()).name).cast::<c_char>();
        let mut text = [0 as c_char; MAXCMDLINE];
        g::q_snprintf(
            text.as_mut_ptr(),
            MAXCMDLINE,
            c"%s: %s".as_ptr(),
            hc_name,
            p,
        );
        trim_chat_text(&mut text, &mut quoted);

        let save = host_client_get();
        let svsp = svs_p();
        let maxc = (*svsp).maxclients;
        let clients_base = (*svsp).clients;
        let argv1 = c::Cmd_Argv(1);
        let mut jc: c_int = 0;
        while jc < maxc {
            let client = clients_base.add(jc as usize);
            jc += 1;
            if !(*client).active || !(*client).spawned {
                continue;
            }
            if c::cvar_cmd::q_strcasecmp(ptr::addr_of!((*client).name).cast::<c_char>(), argv1) != 0
            {
                continue;
            }
            host_client_set(client);
            raise!(crate::host::quake_rs_host_sv_client_printf(text.as_ptr()));
            break;
        }
        host_client_set(save);
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2349-2381

unsafe fn Host_Color_f() -> Raise {
    // SAFETY: engine globals (`cl_topcolor`, `cl_bottomcolor`, `host_client`,
    // `svs`) are live and single-threaded for the duration of console-command
    // dispatch.
    unsafe {
        if c::Cmd_Argc() == 1 {
            c::Con_Printf(
                c"\"color\" is \"%i %i\"\n".as_ptr(),
                cvar_value(ptr::addr_of!(g::cl_topcolor)) as c_int,
                cvar_value(ptr::addr_of!(g::cl_bottomcolor)) as c_int,
            );
            c::Con_Printf(c"color <0-13> [0-13]\n".as_ptr());
            return HOST_GUARD_OK;
        }

        let top: *const c_char;
        let bottom: *const c_char;
        if c::Cmd_Argc() == 2 {
            top = c::Cmd_Argv(1);
            bottom = c::Cmd_Argv(1);
        } else {
            top = c::Cmd_Argv(1);
            bottom = c::Cmd_Argv(2);
        }

        if g::cmd_source != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CvarSet(c"topcolor".as_ptr(), top));
            raise!(g::HostCmd_Glue_CvarSet(c"bottomcolor".as_ptr(), bottom));
            if (*cls_p()).state == CA_CONNECTED {
                raise!(g::HostCmd_Glue_CmdForwardToServer());
            }
            return HOST_GUARD_OK;
        }

        let hc = host_client_get();
        let base = (*svs_p()).clients;
        let idx = (hc.offset_from(base) as c_int) + 1;
        raise!(crate::cl_main::quake_rs_sv_update_info(
            idx,
            c"topcolor".as_ptr(),
            top
        ));
        raise!(crate::cl_main::quake_rs_sv_update_info(
            idx,
            c"bottomcolor".as_ptr(),
            bottom
        ));
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2388-2405

unsafe fn Host_Kill_f() -> Raise {
    // SAFETY: `sv_player` and `qcvm`/`pr_global_struct` are live engine
    // globals once a game is active, which every reachable caller requires.
    unsafe {
        if g::cmd_source != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return HOST_GUARD_OK;
        }

        let sv_player_p = gsu::sv_player.cast::<Edict>();
        if (*sv_player_p).v.health <= 0.0 {
            raise!(crate::host::quake_rs_host_sv_client_printf(
                c"Can't suicide -- already dead!\n".as_ptr()
            ));
            return HOST_GUARD_OK;
        }

        let gv = pgs();
        let qtime = (*vm()).time;
        (*gv).time = qtime as f32;

        let mut prog: c_int = 0;
        raise!(gh::Host_Glue_EdictToProg(sv_player_p.cast(), &mut prog));
        (*gv).self_ = prog;
        let func = (*gv).ClientKill;
        raise!(gh::Host_Glue_PR_ExecuteProgram(func as c_int));
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2412-2448

unsafe fn Host_Pause_f() -> Raise {
    // SAFETY: `cls`/`cl`/`sv`/`sv_player` are live engine globals for the
    // duration of console-command dispatch and, on the guarded branch, once
    // a game is active.
    unsafe {
        // ericw -- demo pause support (inspired by MarkV)
        if (*cls_p()).demoplayback {
            let np = !(*cls_p()).demopaused;
            (*cls_p()).demopaused = np;
            (*cl_p()).paused = np;
            if (*cls_p()).demospeed == 0.0 && !np {
                (*cls_p()).demospeed = 1.0;
            }
            return HOST_GUARD_OK;
        }

        if g::cmd_source != SRC_CLIENT {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return HOST_GUARD_OK;
        }

        if cvar_value(ptr::addr_of!(gh::pausable)) == 0.0 {
            raise!(crate::host::quake_rs_host_sv_client_printf(
                c"Pause not allowed.\n".as_ptr()
            ));
        } else {
            // COMPAT: C toggles via `sv.paused ^= 1`; `paused` is a
            // canonical 0/1 `QBoolean` (bool) so negation is bit-for-bit
            // equivalent and is used here instead.
            let newp = !(*sv_p()).paused;
            (*sv_p()).paused = newp;

            let sv_player_p = gsu::sv_player.cast::<Edict>();
            let netname = (*sv_player_p).v.netname;
            let mut name_ptr: *const c_char = ptr::null();
            raise!(g::HostCmd_Glue_PRGetString(netname, &mut name_ptr));

            let mut msgbuf = [0 as c_char; 1024];
            if newp {
                g::q_snprintf(
                    msgbuf.as_mut_ptr(),
                    1024,
                    c"%s paused the game\n".as_ptr(),
                    name_ptr,
                );
            } else {
                g::q_snprintf(
                    msgbuf.as_mut_ptr(),
                    1024,
                    c"%s unpaused the game\n".as_ptr(),
                    name_ptr,
                );
            }
            raise!(crate::host::quake_rs_host_sv_broadcast_printf(
                msgbuf.as_ptr()
            ));

            // send notification to all clients
            raise!(write_batch(
                ptr::addr_of_mut!((*sv_p()).reliable_datagram).cast(),
                &[op_byte(SVC_SETPAUSE), op_byte(newp as c_int)],
            ));
        }
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2457-2474

unsafe fn Host_PreSpawn_f() -> Raise {
    // SAFETY: `host_client` is a live engine global for the duration of
    // console-command dispatch.
    unsafe {
        if g::cmd_source != SRC_CLIENT {
            c::Con_Printf(c"prespawn is not valid from the console\n".as_ptr());
            return HOST_GUARD_OK;
        }

        let hc = host_client_get();
        if (*hc).spawned {
            c::Con_Printf(c"prespawn not valid -- already spawned\n".as_ptr());
            return HOST_GUARD_OK;
        }

        // will start splurging out prespawn data
        (*hc).sendsignon = 2;
        (*hc).signonidx = 0;
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2481-2544
//
// TEMP-STUB (chunk C owns `Send_Spawn_Info`, `host_cmd.c:1714`). `host_cmd.c`
// declares it `static`, so once every chunk lands in the same
// `rust/quake-capi/src/host_cmd.rs` file, chunk C's real port -- a private
// `fn Send_Spawn_Info(client: *mut Client, loadgame: bool) -> Raise` in that
// same module -- becomes callable here directly (same-module privacy, no
// seam needed). This stub exists only so chunk D's own isolated
// `--features host` build links; delete it the moment chunk C's definition
// merges into this file, since two `fn Send_Spawn_Info` in one module is a
// duplicate-definition error.
unsafe fn Host_Spawn_f() -> Raise {
    // SAFETY: `host_client`, `sv`, `qcvm`/`pr_global_struct` and `sv_player`
    // are live engine globals for the duration of console-command dispatch,
    // once a game is active.
    unsafe {
        if g::cmd_source != SRC_CLIENT {
            c::Con_Printf(c"spawn is not valid from the console\n".as_ptr());
            return HOST_GUARD_OK;
        }

        let hc = host_client_get();
        if (*hc).spawned {
            c::Con_Printf(c"Spawn not valid -- already spawned\n".as_ptr());
            return HOST_GUARD_OK;
        }

        (*hc).knowntoqc = true;
        let qtime = (*vm()).time;
        (*hc).lastmovetime = qtime;

        // run the entrance script
        let loadgame = (*sv_p()).loadgame;
        if loadgame {
            // loaded games are fully inited already
            // if this is the last client to be connected, unpause
            (*sv_p()).paused = false;
        } else {
            // set up the edict
            let ent = (*hc).edict;

            let entityfields = (*(*vm()).progs).entityfields;
            ptr::write_bytes(
                ptr::addr_of_mut!((*ent).v).cast::<u8>(),
                0,
                (entityfields as usize) * 4,
            );
            let mut num: c_int = 0;
            raise!(gw::World_Glue_NumForEdict(ent.cast(), &mut num));
            (*ent).v.colormap = num as f32;
            (*ent).v.team = (((*hc).colors & 15) + 1) as f32;
            (*ent).v.netname = gsv::PR_SetEngineString(ptr::addr_of!((*hc).name).cast::<c_char>());

            // copy spawn parms out of the client_t
            let spawn_parms = (*hc).spawn_parms;
            let parm1_ptr = ptr::addr_of_mut!((*pgs()).parm1);
            for (idx, parm) in spawn_parms.iter().enumerate().take(NUM_BASIC_SPAWN_PARMS) {
                *parm1_ptr.add(idx) = *parm;
            }
            if cvar_value(ptr::addr_of!(gw::pr_checkextension)) != 0.0 {
                // extended spawn parms
                let mut idx = NUM_BASIC_SPAWN_PARMS;
                while idx < NUM_TOTAL_SPAWN_PARMS {
                    g::HostCmd_Glue_SetSpawnParmGlobal((idx + 1) as c_int, spawn_parms[idx]);
                    idx += 1;
                }
            }

            // call the spawn function
            let gv = pgs();
            (*gv).time = qtime as f32;
            let mut prog: c_int = 0;
            raise!(gh::Host_Glue_EdictToProg(gsu::sv_player, &mut prog));
            (*gv).self_ = prog;
            let connect_func = (*gv).ClientConnect;
            raise!(gh::Host_Glue_PR_ExecuteProgram(connect_func as c_int));

            let netconn = (*hc).netconnection;
            if (c::Sys_DoubleTime() - g::NET_QSocketGetTime(netconn.cast())) <= qtime {
                c::Sys_Printf(
                    c"%s entered the game\n".as_ptr(),
                    ptr::addr_of!((*hc).name).cast::<c_char>(),
                );
            }

            let putinserver_func = (*gv).PutClientInServer;
            raise!(gh::Host_Glue_PR_ExecuteProgram(putinserver_func as c_int));
        }

        raise!(Send_Spawn_Info(hc, loadgame));

        raise!(write_batch(
            ptr::addr_of_mut!((*hc).message).cast(),
            &[op_byte(SVC_SIGNONNUM), op_byte(3)],
        ));
        (*hc).sendsignon = 1;
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2551-2560

unsafe fn Host_Begin_f() -> Raise {
    // SAFETY: `host_client` is a live engine global for the duration of
    // console-command dispatch.
    unsafe {
        if g::cmd_source != SRC_CLIENT {
            c::Con_Printf(c"begin is not valid from the console\n".as_ptr());
            return HOST_GUARD_OK;
        }

        (*host_client_get()).spawned = true;
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// host_cmd.c:2571-2648

unsafe fn Host_Kick_f() -> Raise {
    // SAFETY: `sv`, `host_client`, `svs`/`clients` and `pr_global_struct` are
    // live engine globals for the duration of console-command dispatch.
    unsafe {
        if g::cmd_source != SRC_CLIENT {
            if !(*sv_p()).active {
                raise!(g::HostCmd_Glue_CmdForwardToServer());
                return HOST_GUARD_OK;
            }
        } else if (*pgs()).deathmatch != 0.0 {
            return HOST_GUARD_OK;
        }

        let save = host_client_get();
        let svsp = svs_p();
        let maxc = (*svsp).maxclients;
        let clients_base = (*svsp).clients;

        let mut i: c_int;
        let mut by_number = false;
        let argv1 = c::Cmd_Argv(1);

        if c::Cmd_Argc() > 2 && g::strcmp(argv1, c"#".as_ptr()) == 0 {
            let argv2 = c::Cmd_Argv(2);
            i = (c::cvar_cmd::atof(argv2) - 1.0) as c_int;
            if i < 0 || i >= maxc {
                return HOST_GUARD_OK;
            }
            if !(*clients_base.add(i as usize)).active {
                return HOST_GUARD_OK;
            }
            host_client_set(clients_base.add(i as usize));
            by_number = true;
        } else {
            i = 0;
            host_client_set(clients_base);
            loop {
                if i >= maxc {
                    break;
                }
                let hc = host_client_get();
                if (*hc).active
                    && c::cvar_cmd::q_strcasecmp(ptr::addr_of!((*hc).name).cast::<c_char>(), argv1)
                        == 0
                {
                    break;
                }
                i += 1;
                host_client_set(hc.add(1));
            }
        }

        if i < maxc {
            let who: *const c_char = if g::cmd_source != SRC_CLIENT {
                if (*cls_p()).state == CA_DEDICATED {
                    c"Console".as_ptr()
                } else {
                    cvar_string(ptr::addr_of!(g::cl_name))
                }
            } else {
                ptr::addr_of!((*save).name).cast::<c_char>()
            };

            // can't kick yourself!
            if host_client_get() == save {
                return HOST_GUARD_OK;
            }

            let mut message: *const c_char = ptr::null();
            if c::Cmd_Argc() > 2 {
                let args = g::Cmd_Args();
                message = c::COM_Parse(args);
                if by_number {
                    message = message.add(1); // skip the #
                    while *message == b' ' as c_char {
                        // skip white space
                        message = message.add(1);
                    }
                    let argv2 = c::Cmd_Argv(2);
                    message = message.add(c_strlen(argv2)); // skip the number
                }
                while *message != 0 && *message == b' ' as c_char {
                    message = message.add(1);
                }
            }

            let mut kickbuf = [0 as c_char; 1024];
            if !message.is_null() {
                g::q_snprintf(
                    kickbuf.as_mut_ptr(),
                    1024,
                    c"Kicked by %s: %s\n".as_ptr(),
                    who,
                    message,
                );
            } else {
                g::q_snprintf(kickbuf.as_mut_ptr(), 1024, c"Kicked by %s\n".as_ptr(), who);
            }
            raise!(crate::host::quake_rs_host_sv_client_printf(
                kickbuf.as_ptr()
            ));
            raise!(crate::host::quake_rs_host_sv_drop_client(false));
        }

        host_client_set(save);
        HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// Console-command exports (`Host_Reraise`d by the C thunks the main session
// writes; registration also stays with the main session).

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_name_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Name_f`.
    unsafe { Host_Name_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_say_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Say_f`.
    unsafe { Host_Say_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_say_team_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Say_Team_f`.
    unsafe { Host_Say_Team_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_tell_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Tell_f`.
    unsafe { Host_Tell_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_color_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Color_f`.
    unsafe { Host_Color_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_kill_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Kill_f`.
    unsafe { Host_Kill_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_pause_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Pause_f`.
    unsafe { Host_Pause_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_prespawn_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_PreSpawn_f`.
    unsafe { Host_PreSpawn_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_spawn_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Spawn_f`.
    unsafe { Host_Spawn_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_begin_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Begin_f`.
    unsafe { Host_Begin_f() }
}

#[no_mangle]
pub extern "C" fn quake_rs_hostcmd_host_kick_f() -> Raise {
    // SAFETY: no raw-pointer parameters cross this FFI boundary; see `Host_Kick_f`.
    unsafe { Host_Kick_f() }
}

// ===========================================================================
// CHUNK E
// ===========================================================================

// ---------------------------------------------------------------------------
// CHUNK E: Host_Give_f .. Host_User_f (host_cmd.c:2650-3298).
//
// Appends to rust/quake-capi/src/host_cmd.rs below the existing scaffold
// (module doc, Raise/raise!, sv_p/svs_p/cl_p/cls_p, host_client, cvar_value).
// The extra `use` lines below extend that scaffold's own `use` block and
// must be merged there, not duplicated.

use quake_c_sys::cl_main as gcl;
use quake_c_sys::cvar_cmd as gcv;
use quake_c_sys::sv_phys as gph;
use quake_c_sys::sv_user as cu;
use quake_c_sys::world as w;
use quake_types::model_mem::{AliasHdr, MAliasFrameDesc, QModel, MOD_ALIAS};

/// `t[i]` for a `Cmd_Argv`-returned `const char *`, as a byte rather than a
/// possibly-signed `c_char`, to keep the case-label comparisons below in
/// `u8`.
#[inline]
unsafe fn cbyte(p: *const c_char, i: isize) -> u8 {
    // SAFETY: caller passes a NUL-terminated C string and an in-bounds index.
    unsafe { *p.offset(i) as u8 }
}

// ---------------------------------------------------------------------------
// item / weapon bitflags (`quakedef.h:141-206`; `items_t`/`rogueitems_t`/
// `hipnoticitems_t`). Only the members `Host_Give_f` actually reaches.

const IT_SHOTGUN: c_int = 1;
const IT_SUPER_SHOTGUN: c_int = 2;
const IT_NAILGUN: c_int = 4;
const IT_SUPER_NAILGUN: c_int = 8;
const IT_GRENADE_LAUNCHER: c_int = 16;
const IT_ROCKET_LAUNCHER: c_int = 32;
const IT_LIGHTNING: c_int = 64;
const IT_ARMOR1: c_int = 8192;
const IT_ARMOR2: c_int = 16384;
const IT_ARMOR3: c_int = 32768;
const IT_KEY1: c_int = 131072;
const IT_KEY2: c_int = 262144;
const RIT_LAVA_NAILGUN: c_int = 4096;
const RIT_LAVA_SUPER_NAILGUN: c_int = 8192;
const RIT_MULTI_GRENADE: c_int = 16384;
const RIT_MULTI_ROCKET: c_int = 32768;
const RIT_PLASMA_GUN: c_int = 65536; // same bit pattern as HIT_PROXIMITY_GUN
const HIT_PROXIMITY_GUN: c_int = 1 << 16;
const HIT_MJOLNIR: c_int = 1 << 7;
const HIT_LASER_CANNON: c_int = 1 << 23;

// ---------------------------------------------------------------------------
// Host_Give_f -- host_cmd.c:2663-2905.

/// `Host_Give_f` (`host_cmd.c:2663-2905`).
///
/// # Safety
/// FFI entry point; touches `sv_player` and the live `sv`/`svs` engine globals.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_give_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if gcv::cmd_source != c::cmd_source_t_src_client {
            raise!(g::HostCmd_Glue_CmdForwardToServer());
            return 0;
        }
        if (*pgs()).deathmatch != 0.0 {
            return 0;
        }

        let t = c::Cmd_Argv(1);
        let v = gsv::atoi(c::Cmd_Argv(2));
        let ent = cu::sv_player.cast::<Edict>();

        if gsv::strcmp(t, c"all".as_ptr()) == 0 {
            // KNOWN BUG (host_cmd.c:2681-2696): the inner `for (i = 0; i <=
            // 9; ++i)` reuses the outer loop's `i`, so the outer `for`'s
            // `++i` leaves i == 11 once the inner loop finishes and the
            // outer condition (i < 9) fails immediately -- this body runs
            // exactly once, not nine times. Transliterated as-is per
            // ADR-010; see t83_E_notes.md (e).
            let mut i: c_int = 0;
            while i < 9 {
                if g::hipnotic {
                    (*ent).v.items = ((*ent).v.items as c_int
                        | HIT_PROXIMITY_GUN
                        | HIT_LASER_CANNON
                        | HIT_MJOLNIR) as c_float;
                }
                i = 0;
                while i <= 9 {
                    (*ent).v.items = ((*ent).v.items as c_int | (IT_SHOTGUN << i)) as c_float;
                    i += 1;
                }
                (*ent).v.items = (*ent).v.items
                    - (((*ent).v.items as c_int) & (IT_ARMOR1 | IT_ARMOR2 | IT_ARMOR3)) as c_float
                    + IT_ARMOR3 as c_float;
                (*ent).v.items = ((*ent).v.items as c_int | (IT_KEY1 | IT_KEY2)) as c_float;
                (*ent).v.ammo_shells = 999.0;
                (*ent).v.ammo_nails = 999.0;
                (*ent).v.ammo_rockets = 999.0;
                (*ent).v.ammo_cells = 999.0;
                (*ent).v.armortype = 0.8;
                (*ent).v.armorvalue = 200.0;
                i += 1;
            }
        } else {
            match cbyte(t, 0) {
                b'0'..=b'9' => {
                    // MED 01/04/97 added hipnotic give stuff
                    if g::hipnotic {
                        if cbyte(t, 0) == b'6' {
                            if cbyte(t, 1) == b'a' {
                                (*ent).v.items =
                                    ((*ent).v.items as c_int | HIT_PROXIMITY_GUN) as c_float;
                            } else {
                                (*ent).v.items =
                                    ((*ent).v.items as c_int | IT_GRENADE_LAUNCHER) as c_float;
                            }
                        } else if cbyte(t, 0) == b'9' {
                            (*ent).v.items =
                                ((*ent).v.items as c_int | HIT_LASER_CANNON) as c_float;
                        } else if cbyte(t, 0) == b'0' {
                            (*ent).v.items = ((*ent).v.items as c_int | HIT_MJOLNIR) as c_float;
                        } else if cbyte(t, 0) >= b'2' {
                            (*ent).v.items = ((*ent).v.items as c_int
                                | (IT_SHOTGUN << (cbyte(t, 0) - b'2')))
                                as c_float;
                        }
                    } else if cbyte(t, 0) >= b'2' {
                        (*ent).v.items = ((*ent).v.items as c_int
                            | (IT_SHOTGUN << (cbyte(t, 0) - b'2')))
                            as c_float;
                    }
                }
                b's' => {
                    if g::rogue {
                        let val = gph::GetEdictFieldValue(
                            ent.cast::<c_void>(),
                            gph::ED_FindFieldOffset(c"ammo_shells1".as_ptr()),
                        );
                        if !val.is_null() {
                            *val = v as c_float;
                        }
                    }
                    (*ent).v.ammo_shells = v as c_float;
                }
                b'n' => {
                    if g::rogue {
                        let val = gph::GetEdictFieldValue(
                            ent.cast::<c_void>(),
                            gph::ED_FindFieldOffset(c"ammo_nails1".as_ptr()),
                        );
                        if !val.is_null() {
                            *val = v as c_float;
                            if (*ent).v.weapon <= IT_LIGHTNING as c_float {
                                (*ent).v.ammo_nails = v as c_float;
                            }
                        }
                    } else {
                        (*ent).v.ammo_nails = v as c_float;
                    }
                }
                b'l' => {
                    if g::rogue {
                        let val = gph::GetEdictFieldValue(
                            ent.cast::<c_void>(),
                            gph::ED_FindFieldOffset(c"ammo_lava_nails".as_ptr()),
                        );
                        if !val.is_null() {
                            *val = v as c_float;
                            if (*ent).v.weapon > IT_LIGHTNING as c_float {
                                (*ent).v.ammo_nails = v as c_float;
                            }
                        }
                    }
                }
                b'r' => {
                    if g::rogue {
                        let val = gph::GetEdictFieldValue(
                            ent.cast::<c_void>(),
                            gph::ED_FindFieldOffset(c"ammo_rockets1".as_ptr()),
                        );
                        if !val.is_null() {
                            *val = v as c_float;
                            if (*ent).v.weapon <= IT_LIGHTNING as c_float {
                                (*ent).v.ammo_rockets = v as c_float;
                            }
                        }
                    } else {
                        (*ent).v.ammo_rockets = v as c_float;
                    }
                }
                b'm' => {
                    if g::rogue {
                        let val = gph::GetEdictFieldValue(
                            ent.cast::<c_void>(),
                            gph::ED_FindFieldOffset(c"ammo_multi_rockets".as_ptr()),
                        );
                        if !val.is_null() {
                            *val = v as c_float;
                            if (*ent).v.weapon > IT_LIGHTNING as c_float {
                                (*ent).v.ammo_rockets = v as c_float;
                            }
                        }
                    }
                }
                b'h' => {
                    (*ent).v.health = v as c_float;
                }
                b'c' => {
                    if g::rogue {
                        let val = gph::GetEdictFieldValue(
                            ent.cast::<c_void>(),
                            gph::ED_FindFieldOffset(c"ammo_cells1".as_ptr()),
                        );
                        if !val.is_null() {
                            *val = v as c_float;
                            if (*ent).v.weapon <= IT_LIGHTNING as c_float {
                                (*ent).v.ammo_cells = v as c_float;
                            }
                        }
                    } else {
                        (*ent).v.ammo_cells = v as c_float;
                    }
                }
                b'p' => {
                    if g::rogue {
                        let val = gph::GetEdictFieldValue(
                            ent.cast::<c_void>(),
                            gph::ED_FindFieldOffset(c"ammo_plasma".as_ptr()),
                        );
                        if !val.is_null() {
                            *val = v as c_float;
                            if (*ent).v.weapon > IT_LIGHTNING as c_float {
                                (*ent).v.ammo_cells = v as c_float;
                            }
                        }
                    }
                }
                // johnfitz -- give armour
                b'a' => {
                    if v > 150 {
                        (*ent).v.armortype = 0.8;
                        (*ent).v.armorvalue = v as c_float;
                        (*ent).v.items = (*ent).v.items
                            - (((*ent).v.items as c_int) & (IT_ARMOR1 | IT_ARMOR2 | IT_ARMOR3))
                                as c_float
                            + IT_ARMOR3 as c_float;
                    } else if v > 100 {
                        (*ent).v.armortype = 0.6;
                        (*ent).v.armorvalue = v as c_float;
                        (*ent).v.items = (*ent).v.items
                            - (((*ent).v.items as c_int) & (IT_ARMOR1 | IT_ARMOR2 | IT_ARMOR3))
                                as c_float
                            + IT_ARMOR2 as c_float;
                    } else if v >= 0 {
                        (*ent).v.armortype = 0.3;
                        (*ent).v.armorvalue = v as c_float;
                        (*ent).v.items = (*ent).v.items
                            - (((*ent).v.items as c_int) & (IT_ARMOR1 | IT_ARMOR2 | IT_ARMOR3))
                                as c_float
                            + IT_ARMOR1 as c_float;
                    }
                }
                // johnfitz
                b'k' => {
                    (*ent).v.items = ((*ent).v.items as c_int | (IT_KEY1 | IT_KEY2)) as c_float;
                }
                _ => {}
            }
        }

        // johnfitz -- update currentammo to match new ammo (so statusbar updates correctly)
        match (*ent).v.weapon as c_int {
            IT_SHOTGUN | IT_SUPER_SHOTGUN => {
                (*ent).v.currentammo = (*ent).v.ammo_shells;
            }
            IT_NAILGUN | IT_SUPER_NAILGUN | RIT_LAVA_SUPER_NAILGUN => {
                (*ent).v.currentammo = (*ent).v.ammo_nails;
            }
            IT_GRENADE_LAUNCHER | IT_ROCKET_LAUNCHER | RIT_MULTI_GRENADE | RIT_MULTI_ROCKET => {
                (*ent).v.currentammo = (*ent).v.ammo_rockets;
            }
            IT_LIGHTNING | HIT_LASER_CANNON | HIT_MJOLNIR => {
                (*ent).v.currentammo = (*ent).v.ammo_cells;
            }
            RIT_LAVA_NAILGUN => {
                // same as IT_AXE
                if g::rogue {
                    (*ent).v.currentammo = (*ent).v.ammo_nails;
                }
            }
            RIT_PLASMA_GUN => {
                // same as HIT_PROXIMITY_GUN
                if g::rogue {
                    (*ent).v.currentammo = (*ent).v.ammo_cells;
                }
                if g::hipnotic {
                    (*ent).v.currentammo = (*ent).v.ammo_rockets;
                }
            }
            _ => {}
        }
        // johnfitz
        0
    }
}

// ---------------------------------------------------------------------------
// FindViewthing -- host_cmd.c:2907-2943. Static helper, not console-facing.

unsafe fn FindViewthing(out: *mut *mut Edict) -> Raise {
    // SAFETY: `out` is a caller-owned stack slot; `sv`/the ambient QCVM are
    // the live engine globals this runs on the host thread with.
    unsafe {
        *out = ptr::null_mut();
        gsv::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast::<c_void>());

        let num = (*vm()).num_edicts;
        let mut i: c_int = 0;
        let mut e: *mut c_void = ptr::null_mut();

        // First `if (i == qcvm->num_edicts)` is trivially true right after
        // `i = qcvm->num_edicts;`, so this loop always runs.
        while i < num {
            raise!(w::World_Glue_EdictNum(i, &mut e));
            let mut name: *const c_char = ptr::null();
            raise!(gsv::SvMain_Glue_GetString(
                (*e.cast::<Edict>()).v.classname,
                &mut name
            ));
            if gsv::strcmp(name, c"viewthing".as_ptr()) == 0 {
                break;
            }
            i += 1;
        }

        if i == num {
            i = 0;
            e = ptr::null_mut();
            while i < num {
                raise!(w::World_Glue_EdictNum(i, &mut e));
                let mut name: *const c_char = ptr::null();
                raise!(gsv::SvMain_Glue_GetString(
                    (*e.cast::<Edict>()).v.classname,
                    &mut name
                ));
                if gsv::strcmp(name, c"info_player_start".as_ptr()) == 0 {
                    break;
                }
                i += 1;
            }
        }

        let result = if i == num {
            c::Con_Printf(c"No viewthing on map\n".as_ptr());
            ptr::null_mut()
        } else {
            e.cast::<Edict>()
        };

        gsv::PR_SwitchQCVM(ptr::null_mut());
        *out = result;
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Viewmodel_f -- host_cmd.c:2950-2976.

/// `Host_Viewmodel_f` (`host_cmd.c:2950-2976`).
///
/// # Safety
/// FFI entry point; touches the live `sv` engine global and switches the ambient QCVM.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_viewmodel_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut e: *mut Edict = ptr::null_mut();
        raise!(FindViewthing(&mut e));
        if e.is_null() {
            return 0;
        }

        let arg1 = c::Cmd_Argv(1);
        let mut m: *mut c_void = ptr::null_mut();
        if cbyte(arg1, 0) != 0 {
            raise!(g::HostCmd_Glue_ModForName(arg1, false, &mut m));
            if m.is_null() {
                c::Con_Printf(c"Can't load %s\n".as_ptr(), arg1);
                return 0;
            }
        }

        gsv::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast::<c_void>());
        if !m.is_null() {
            let mut idx: c_int = 0;
            raise!(g::HostCmd_Glue_PrecacheModel(
                ptr::addr_of!((*m.cast::<QModel>()).name).cast::<c_char>(),
                &mut idx
            ));
            (*e).v.modelindex = idx as c_float;
        } else {
            (*e).v.modelindex = 0.0;
        }
        let name_ptr = (*sv_p()).model_precache[(*e).v.modelindex as c_int as usize];
        (*e).v.model = gsv::PR_SetEngineString(name_ptr);
        (*e).v.frame = 0.0;
        gsv::PR_SwitchQCVM(ptr::null_mut());
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Viewframe_f -- host_cmd.c:2983-3001.

/// `Host_Viewframe_f` (`host_cmd.c:2983-3001`).
///
/// # Safety
/// FFI entry point; touches the live `sv` engine global and switches the ambient QCVM.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_viewframe_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut e: *mut Edict = ptr::null_mut();
        raise!(FindViewthing(&mut e));
        if e.is_null() {
            return 0;
        }
        let m = (*cl_p()).model_precache[(*e).v.modelindex as c_int as usize];
        if !m.is_null() {
            let mut f = gsv::atoi(c::Cmd_Argv(1));
            if f >= (*m).numframes {
                f = (*m).numframes - 1;
            }
            (*e).v.frame = f as c_float;
        }
        0
    }
}

// ---------------------------------------------------------------------------
// PrintFrameName -- host_cmd.c:3003-3014. Static helper, not console-facing.

unsafe fn PrintFrameName(m: *mut QModel, frame: c_int) -> Raise {
    // SAFETY: `m` is the model `FindViewthing` just resolved through the live
    // `sv.models` table.
    unsafe {
        let mut hdr: *mut c_void = ptr::null_mut();
        raise!(g::HostCmd_Glue_ModExtradata(m.cast::<c_void>(), &mut hdr));
        if hdr.is_null() || (*m).type_ != MOD_ALIAS {
            return 0;
        }
        let pframedesc = ptr::addr_of_mut!((*hdr.cast::<AliasHdr>()).frames)
            .cast::<MAliasFrameDesc>()
            .offset(frame as isize);
        c::Con_Printf(
            c"frame %i: %s\n".as_ptr(),
            frame,
            ptr::addr_of!((*pframedesc).name).cast::<c_char>(),
        );
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Viewnext_f -- host_cmd.c:3021-3038.

/// `Host_Viewnext_f` (`host_cmd.c:3021-3038`).
///
/// # Safety
/// FFI entry point; touches the live `sv` engine global and switches the ambient QCVM.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_viewnext_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut e: *mut Edict = ptr::null_mut();
        raise!(FindViewthing(&mut e));
        if e.is_null() {
            return 0;
        }
        let m = (*cl_p()).model_precache[(*e).v.modelindex as c_int as usize];
        if !m.is_null() {
            (*e).v.frame += 1.0;
            if (*e).v.frame >= (*m).numframes as c_float {
                (*e).v.frame = ((*m).numframes - 1) as c_float;
            }
            raise!(PrintFrameName(m, (*e).v.frame as c_int));
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Viewprev_f -- host_cmd.c:3045-3063.

/// `Host_Viewprev_f` (`host_cmd.c:3045-3063`).
///
/// # Safety
/// FFI entry point; touches the live `sv` engine global and switches the ambient QCVM.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_viewprev_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let mut e: *mut Edict = ptr::null_mut();
        raise!(FindViewthing(&mut e));
        if e.is_null() {
            return 0;
        }
        let m = (*cl_p()).model_precache[(*e).v.modelindex as c_int as usize];
        if !m.is_null() {
            (*e).v.frame -= 1.0;
            if (*e).v.frame < 0.0 {
                (*e).v.frame = 0.0;
            }
            raise!(PrintFrameName(m, (*e).v.frame as c_int));
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Startdemos_f -- host_cmd.c:3078-3112.

/// `Host_Startdemos_f` (`host_cmd.c:3078-3112`).
///
/// # Safety
/// FFI entry point; touches the live `cls` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_startdemos_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if ptr::addr_of!((*cls_p()).state).read() == CA_DEDICATED {
            return 0;
        }

        let mut cnt = c::Cmd_Argc() - 1;
        if cnt > MAX_DEMOS as c_int {
            c::Con_Printf(c"Max %i demos in demoloop\n".as_ptr(), MAX_DEMOS as c_int);
            cnt = MAX_DEMOS as c_int;
        }
        c::Con_Printf(c"%i demo(s) in loop\n".as_ptr(), cnt);

        let mut i: c_int = 1;
        while i < cnt + 1 {
            g::q_strlcpy(
                ptr::addr_of_mut!((*cls_p()).demos[(i - 1) as usize]).cast::<c_char>(),
                c::Cmd_Argv(i),
                core::mem::size_of::<[c_char; MAX_DEMONAME]>(),
            );
            i += 1;
        }

        if !ptr::addr_of!((*sv_p()).active).read()
            && (*cls_p()).demonum != -1
            && !ptr::addr_of!((*cls_p()).demoplayback).read()
        {
            (*cls_p()).demonum = 0;
            if cvar_value(ptr::addr_of!(gcl::cl_startdemos)) == 0.0 {
                // QuakeSpasm customization: go straight to menu, no CL_NextDemo
                (*cls_p()).demonum = -1;
                raise!(gcl::ClMain_Glue_CbufInsertText(c"menu_main\n".as_ptr()));
                return 0;
            }
            raise!(g::HostCmd_Glue_CLNextDemo());
        } else {
            (*cls_p()).demonum = -1;
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Demos_f -- host_cmd.c:3121-3129. Return to looping demos.

/// `Host_Demos_f` (`host_cmd.c:3121-3129`).
///
/// # Safety
/// FFI entry point; touches the live `cls` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_demos_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if ptr::addr_of!((*cls_p()).state).read() == CA_DEDICATED {
            return 0;
        }
        if (*cls_p()).demonum == -1 {
            (*cls_p()).demonum = 1;
        }
        raise!(g::HostCmd_Glue_CLDisconnect_f());
        raise!(g::HostCmd_Glue_CLNextDemo());
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Stopdemo_f -- host_cmd.c:3138-3146. Return to looping demos.

/// `Host_Stopdemo_f` (`host_cmd.c:3138-3146`).
///
/// # Safety
/// FFI entry point; touches the live `cls` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_stopdemo_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if ptr::addr_of!((*cls_p()).state).read() == CA_DEDICATED {
            return 0;
        }
        if !ptr::addr_of!((*cls_p()).demoplayback).read() {
            return 0;
        }
        g::CL_StopPlayback();
        raise!(gh::Host_Glue_CL_Disconnect());
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Resetdemos -- host_cmd.c:3155-3159. External linkage in C
// (common.c:1763 calls it directly), so this exports both the literal C
// name and the `quake_rs_hostcmd_*` convention.

unsafe fn host_resetdemos_core() {
    // SAFETY: `cls` is the live engine global, written on the host thread.
    unsafe {
        ptr::write_bytes(
            ptr::addr_of_mut!((*cls_p()).demos).cast::<u8>(),
            0,
            core::mem::size_of::<[[c_char; MAX_DEMONAME]; MAX_DEMOS]>(),
        );
        (*cls_p()).demonum = 0;
    }
}

/// # Safety
/// Must run on the host's own thread with `cls` valid.
#[no_mangle]
pub unsafe extern "C" fn Host_Resetdemos() {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        host_resetdemos_core();
    }
}

/// `Host_Resetdemos` under the `quake_rs_hostcmd_*` convention, so the ctest
/// oracle can drive it the same way it drives every other entry point.
///
/// # Safety
/// Must run on the host's own thread with `cls` valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_resetdemos() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        host_resetdemos_core();
    }
    0
}

// ---------------------------------------------------------------------------
// Info_ClientPrint_Callback -- host_cmd.c:3161-3164.
//
// Passed to `Info_Enumerate` as a plain `void (*) (void *, const char *,
// const char *)` function pointer (matching the C signature exactly, per
// rule 8), so it cannot itself return a `Raise`. `SV_ClientPrintf` can
// Host_Error; that status is written through `Info_Enumerate`'s own `cbctx`
// pointer -- a `Raise` the caller owns on its own stack frame -- and drained
// by the two callers (`Host_Serverinfo_f`, `Host_Setinfo_f`) immediately
// after their `Info_Enumerate` call returns. `Info_Enumerate`'s loop body
// has no side effects besides this callback (verified against `common.c`),
// so finishing the enumeration before re-raising is observably equivalent to
// the original's longjmp-on-first-error. Only the first non-zero status is
// kept -- the one the original's longjmp would have carried.

/// # Safety
/// `ctx` must be a live, exclusively-borrowed `*mut Raise`; `key` and `val`
/// are `Info_Enumerate`'s NUL-terminated key/value slices.
unsafe extern "C" fn Info_ClientPrint_Callback(
    ctx: *mut c_void,
    key: *const c_char,
    val: *const c_char,
) {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let pending = ctx.cast::<Raise>();
        let r = g::HostCmd_Glue_SVClientPrintfKV(key, val);
        if r != 0 && *pending == 0 {
            *pending = r;
        }
    }
}

// ---------------------------------------------------------------------------
// Host_Serverinfo_f -- host_cmd.c:3166-3195.

/// `Host_Serverinfo_f` (`host_cmd.c:3166-3195`).
///
/// # Safety
/// FFI entry point; touches the live `svs` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_serverinfo_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        // serverinfo command
        if gcv::cmd_source == c::cmd_source_t_src_client {
            let mut pending: Raise = 0;
            g::Info_Enumerate(
                ptr::addr_of!((*svs_p()).serverinfo).cast::<c_char>(),
                Info_ClientPrint_Callback,
                ptr::addr_of_mut!(pending).cast::<c_void>(),
            );
            raise!(pending);
            return 0;
        }
        if c::Cmd_Argc() != 3 {
            c::Con_Printf(c"Serverinfo:\n".as_ptr());
            if ptr::addr_of!((*cls_p()).state).read() >= CA_CONNECTED
                && gcv::cmd_source != c::cmd_source_t_src_client
            {
                g::Info_Print(ptr::addr_of!((*cl_p()).serverinfo).cast::<c_char>());
            } else {
                g::Info_Print(ptr::addr_of!((*svs_p()).serverinfo).cast::<c_char>());
            }
        } else if gcv::cmd_source == c::cmd_source_t_src_command {
            let key = c::Cmd_Argv(1);
            let val = c::Cmd_Argv(2);
            if cbyte(key, 0) == b'*' {
                c::Con_Printf(c"Refusing to set key \"%s\"\n".as_ptr(), key);
                return 0;
            }
            raise!(g::HostCmd_Glue_SVUpdateInfo(0, key, val));
        } else {
            c::Con_Printf(c"Serverinfo may not be changed here\n".as_ptr());
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Host_Setinfo_f -- host_cmd.c:3197-3236.

/// `Host_Setinfo_f` (`host_cmd.c:3197-3236`).
///
/// # Safety
/// FFI entry point; touches the live `cls`/`svs` engine globals.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_setinfo_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        let key = c::Cmd_Argv(1);
        let val = c::Cmd_Argv(2);

        if gcv::cmd_source == c::cmd_source_t_src_client {
            // clc_stringcmd version
            if c::Cmd_Argc() != 3 {
                raise!(g::HostCmd_Glue_SVClientPrintfUserInfoHeader());
                let mut pending: Raise = 0;
                g::Info_Enumerate(
                    ptr::addr_of!((*host_client_get()).userinfo).cast::<c_char>(),
                    Info_ClientPrint_Callback,
                    ptr::addr_of_mut!(pending).cast::<c_void>(),
                );
                raise!(pending);
            } else {
                if cbyte(key, 0) == b'*' {
                    // users may not change * keys (beyond initial connection anyway).
                    return 0;
                }
                let idx = host_client_get().offset_from(ptr::addr_of!((*svs_p()).clients).read())
                    as c_int
                    + 1;
                raise!(g::HostCmd_Glue_SVUpdateInfo(idx, key, val));
            }
        } else {
            // console version
            if c::Cmd_Argc() != 3 {
                c::Con_Printf(c"User Info:\n".as_ptr());
                g::Info_Print(ptr::addr_of!((*cls_p()).userinfo).cast::<c_char>());
            } else {
                let var = g::Cvar_FindVar(key);
                if !var.is_null()
                    && (ptr::addr_of!((*var).flags).read() & c::cvarflags_t_CVAR_USERINFO) != 0
                {
                    raise!(gcl::ClMain_Glue_CvarSet(key, val));
                } else {
                    gcv::Info_SetKey(
                        ptr::addr_of_mut!((*cls_p()).userinfo).cast::<c_char>(),
                        core::mem::size_of_val(&(*cls_p()).userinfo),
                        key,
                        val,
                    );
                    if ptr::addr_of!((*cls_p()).state).read() == CA_CONNECTED {
                        raise!(g::HostCmd_Glue_CmdForwardToServer());
                    }
                }
            }
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Host_User_f -- host_cmd.c:3237-3290.
//
// The `if (sv.active) { ... svs.clients[i] ... }` block (host_cmd.c:3239-
// 3263) is entirely inside a `/* */` C comment in the original -- dead code,
// not ported. Only the `cls.state == ca_connected` branch is live (see
// t83_E_notes.md (d)/(e)).

/// `Host_User_f` (`host_cmd.c:3237-3290`).
///
/// # Safety
/// FFI entry point; touches the live `cls` engine global.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_hostcmd_user_f() -> Raise {
    // SAFETY: caller-verified per the `# Safety` doc above.
    unsafe {
        if ptr::addr_of!((*cls_p()).state).read() == CA_CONNECTED {
            if c::Cmd_Argc() == 2 {
                let i = gsv::atoi(c::Cmd_Argv(1));
                if i >= (*cl_p()).maxclients {
                    return 0; // not a valid slot.
                }
                let sb = (*cl_p()).scores.offset(i as isize);
                c::Con_Printf(
                    c"User %i (%s):\n".as_ptr(),
                    i,
                    ptr::addr_of!((*sb).name).cast::<c_char>(),
                );
                g::Info_Print(ptr::addr_of!((*sb).userinfo).cast::<c_char>());
            } else {
                let mut i: c_int = 0;
                while i < (*cl_p()).maxclients {
                    let sb = (*cl_p()).scores.offset(i as isize);
                    if cbyte(ptr::addr_of!((*sb).name).cast::<c_char>(), 0) != 0 {
                        c::Con_Printf(
                            c"User %i (%s):\n".as_ptr(),
                            i,
                            ptr::addr_of!((*sb).name).cast::<c_char>(),
                        );
                        g::Info_Print(ptr::addr_of!((*sb).userinfo).cast::<c_char>());
                    }
                    i += 1;
                }
            }
        }
        0
    }
}
