//! `pr_ext.c` FRIK_FILE + string-buffer group (Phase 7 M9f group C):
//! `PF_fopen`/`PF_fgets`/... and the `strbuflist` builtins
//! (`Quake/pr_ext.c:3130-3773`).
//!
//! Seventeen `builtin_t` slots plus two non-builtin teardowns
//! (`PF_frikfile_shutdown`, `PF_buf_shutdown`), whose only callers are
//! `PR_ShutdownExtensions` (`pr_ext.c:6180-6181`). The flip is Pattern C: the
//! C bodies stay compiled as the oracle, `pr_cmds_glue.c`'s `RUST_PF` wraps
//! the seventeen, and the two teardowns get hand-written frames beside
//! `rust_pr_UnzoneAll`, exactly as M9d did for `PR_UnzoneAll`.
//!
//! # Shared state
//!
//! Two file-scope statics move wholesale into this module, because every
//! function that touches them flips in the same milestone:
//!
//! * `qcfiles` / `qcfiles_max` (`pr_ext.c:3140-3143`) -- the open-file table.
//! * `strbuflist[NUMSTRINGBUFS]` (`pr_ext.c:3389`) -- the string buffers.
//!
//! plus `PF_buf_sort_sortprefixlen` (`pr_ext.c:3499`), which stays a
//! process-global exactly as C has it; see the sort section below for why that
//! is load-bearing rather than incidental.
//!
//! Both tables are keyed by `owningvm`, so the C representation of *ownership*
//! is preserved even though the storage moved: a buffer or file created under
//! one qcvm is still invisible to another and is still torn down by that
//! qcvm's `PR_ShutdownExtensions`.
//!
//! # The sort (ADR-010) -- the decision for this group
//!
//! `PF_buf_sort` and `PF_buf_cvarlist` both `qsort` a `char **` with
//! `PF_buf_sort_ascending` / `PF_buf_sort_descending`, which are `strncmp`
//! against the file-scope `PF_buf_sort_sortprefixlen`. `strncmp` returns 0 for
//! any two strings sharing a `sortprefixlen`-byte prefix, so ties are
//! reachable and `qsort`'s ordering of them is implementation-defined --
//! and `bufstr_get` after a sort makes that ordering observable to QuakeC.
//!
//! COMPAT (ADR-010): the *platform* `qsort` therefore keeps deciding it. No
//! Rust sort is substituted. What moved into Rust is only the comparator, and
//! it is a transcription: `strncmp` on the same two `char *`, returning the
//! same `int`. `qsort`'s output is a function of the initial array and the
//! signs the comparator returns, so the same libc `qsort` over the same array
//! with the same signs produces the same permutation it does today, ties
//! included.
//!
//! This differs from M9d's `ED_freetime_compare_func`, which had to stay in C
//! (`pr_edict_arena_glue.c:81`): *that* comparator calls `EDICT_NUM_NO_CHECK`
//! and lives one careless edit away from a `Host_Error` inside `qsort`, and
//! its C body could not be reached from Rust because it is `static`. Neither
//! applies here. These two comparators are pure leaves -- `strncmp` on two
//! NUL-terminated pointers -- so ADR-009's "no `longjmp` out of a `qsort`
//! comparator" is satisfied structurally, not by convention: there is no seam
//! inside them that can raise. `panic = "abort"` closes the other direction:
//! a hypothetical Rust panic aborts the process rather than unwinding through
//! libc's frames.
//!
//! COMPAT (ADR-010): `PF_buf_cvarlist` sorts with whatever
//! `PF_buf_sort_sortprefixlen` the *last* `PF_buf_sort` left behind -- it never
//! sets it. With the initial 0 the comparator is `strncmp (a, b, 0)`, which is
//! always 0: a completely inconsistent comparator, so the cvar list comes back
//! in whatever order `qsort` happens to leave it. That is why the prefix
//! length is a real process-global here and not a parameter: making it one
//! would silently fix the bug and change the observable cvar ordering.
//!
//! # ADR-009 audit
//!
//! Raising seams, all reported as statuses and re-issued by `PRBI_Raise` from
//! the C frame:
//!
//! * `PR_GetString` (`pr_edict_arena.c:307`), via every `G_STRING` here:
//!   `PF_fopen`'s name, `PF_whichpack`'s name, `PF_buf_create`'s type,
//!   `PF_buf_implode`'s glue, `PF_bufstr_set`'s string, `PF_bufstr_add`'s
//!   string and `PF_buf_cvarlist`'s two patterns. Reported as
//!   `PRBI_ERR_NO_STRING`, exactly as `progs_builtins_zone.rs` does.
//!
//! Non-raising seams, each checked at its definition: `Mem_Alloc` /
//! `Mem_Realloc` / `Mem_Free` only `Sys_Error` (ADR-013); `PR_GetTempString`
//! steps a static index; `PR_SetEngineString`'s one `Host_Error` is inside an
//! `#if 0` (`pr_edict_arena.c:351-353`); `COM_FOpenFile`, `COM_FileExists`,
//! `Sys_fopen`, `Sys_fseek`, `Sys_ftell`, `Cvar_FindVarAfter`,
//! `COM_FileGetExtension`, `va`, `wildcmp`, the `q_str*` helpers and the four
//! libc file calls are leaves; `Con_Printf`/`Con_Warning` are queued on
//! `SvConsole` and flushed by `run_sv` after the Rust frame returns.
//!
//! REPORTED (ADR-009): `PF_fputs` calls `PF_VarString (1)`
//! (`pr_ext.c:3272`), whose `G_STRING` can `Host_Error` on a corrupt handle.
//! It is called unguarded here because that is the standing precedent for this
//! seam -- `progs_builtins.rs:212` already calls it unguarded through
//! `PRBI_Glue_VarString` -- not because it was found to be safe. Guarding it
//! is a pre-existing gap shared with the already-flipped `PF_bprint` family.
//!
//! # ADR-005 audit (float formatter)
//!
//! No `%g`/`%e` is reachable. The only formatted number in the group is
//! `Con_Warning ("PF_fopen: unsupported mode: %i\n", fmode)`, an `int`, which
//! this port renders with Rust's integer formatting (identical output for
//! every `i32`). Everything else concatenates bytes that are already strings.
//!
//! # Bounds / panic audit (`panic = "abort"` in every profile)
//!
//! * Every buffer handle is narrowed to `u32` once, then range-checked
//!   against `NUMSTRINGBUFS` before it indexes `strbuflist`. C spells the same
//!   check ten different ways (`unsigned int`, `int` + `(unsigned int)` cast,
//!   `size_t` + `(unsigned int)` cast); all ten agree with the `u32` form for
//!   every value a 32-bit float can name.
//! * File handles are `usize` and are compared against the table length
//!   before indexing, as C compares against `qcfiles_max`.
//! * `PR_GetTempString`'s buffer is written through a cursor that is clamped
//!   to `STRINGTEMP_LENGTH - 1`, reproducing C's `if (s == end) s--;` rewind.
//! * `VmRaw::new`'s `assert!(!vm.is_null())` is the one abort left; the C
//!   originals dereference the ambient qcvm unconditionally.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void, CStr};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cvar_cmd::q_strdup;
use quake_c_sys::host_cmd::{com_searchpaths, q_snprintf, q_strcasecmp, va};
use quake_c_sys::net_dgrm_orch::Cvar_FindVarAfter;
use quake_c_sys::progs_builtins_sv as g;
use quake_c_sys::stdio::{fclose, fread};
use quake_c_sys::sv_main::q_strncasecmp;
use quake_c_sys::{qfileofs_t, qfilesize_t, FILE, MAX_OSPATH};
use quake_progs::arena::{StringError, VmRaw};
use quake_types::progs::{QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_RETURN};

use crate::progs_builtins_sv::{run_sv, SvConsole, SvRaise, SvResult};

/* ---------------------------------------------------------------------------
 * M9f integration note: C symbols `quake-c-sys` does not declare yet. Each is
 * a leaf (see the ADR-009 audit above); none belongs to a mirror type, so
 * declaring them here rather than growing `quake-c-sys` keeps the group's diff
 * inside this module. The main session should fold them into `quake-c-sys`
 * when the five M9f groups land together.
 */
extern "C" {
    /// `pr_cmds.c:133` -- steps the temp-string ring and returns the buffer.
    /// A static index bump; it cannot raise. Used directly (rather than
    /// through `PRBI_Glue_StoreTempString`) because `PF_fgets` steps the ring
    /// on a path where it then returns 0 without interning anything, and the
    /// ring position is observable through later handles.
    fn PR_GetTempString() -> *mut c_char;

    /// `pr_cmds.c:155` -- `PF_fputs`'s argument joiner (`pr_ext.c:3272`).
    /// See the ADR-009 note above: its `G_STRING` can raise.
    fn PF_VarString(first: c_int) -> *mut c_char;

    /// `common.c:723` -- `PF_buf_cvarlist`'s wildcard matcher
    /// (`pr_ext.c:3762`, `:3764`). Pure.
    fn wildcmp(wild: *const c_char, string: *const c_char) -> c_int;

    /* libc. `fread` and `fclose` come from `quake_c_sys::stdio`, which already
     * carries them; `fputs`, `qsort` and `strncmp` are not in it yet. */
    fn qsort(
        base: *mut c_void,
        nmemb: usize,
        size: usize,
        compar: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int,
    );
    fn strncmp(a: *const c_char, b: *const c_char, n: usize) -> c_int;
    fn fputs(s: *const c_char, stream: *mut FILE) -> c_int;
}

/* ---------------------------------------------------------------------------
 * Constants.
 */

/// `pr_cmds_glue.c:38` `PRBI_ERR_NO_STRING`.
const PRBI_ERR_NO_STRING: c_int = 2;

/// `pr_ext.c:3145`
const QC_FILE_BASE: c_float = 1.0;
/// `pr_ext.c:3387-3388`
const BUFSTRBASE: c_float = 1.0;
const NUMSTRINGBUFS: u32 = 64;
/// `progs.h:210`
const STRINGTEMP_LENGTH: usize = 1024;
/// `pr_ext.c:3132` `char cache[4 * 4096]`
const QCFILE_CACHE: usize = 4 * 4096;
/// `<stdio.h>`
const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;

/// `PR_GetString`'s one live failure, as the status `PRBI_Raise` decodes.
fn no_string(e: StringError) -> SvRaise {
    match e {
        StringError::NonExistent(num) => SvRaise {
            status: PRBI_ERR_NO_STRING,
            detail: num,
        },
    }
}

/// `progs.h:174` `G_STRING (o)`.
fn g_string(raw: &VmRaw, ofs: usize) -> Result<*const c_char, SvRaise> {
    raw.get_string(raw.g_i32(ofs)).map_err(no_string)
}

/// The bytes of a NUL-terminated engine string.
///
/// # Safety
/// `s` must be a live NUL-terminated pointer, as every `PR_GetString` result
/// and every engine string used here is.
unsafe fn c_bytes<'a>(s: *const c_char) -> &'a [u8] {
    // SAFETY: caller contract.
    unsafe { CStr::from_ptr(s) }.to_bytes()
}

/* ---------------------------------------------------------------------------
 * Process-global state, the two `pr_ext.c` file statics plus the sort's.
 *
 * ADR-008/ADR-010: QuakeC executes on the main thread only, and every reader
 * below runs inside one builtin call, so the cell is never aliased across a
 * suspension point. The C originals are plain file-scope objects with exactly
 * this contract.
 */

struct Global<T>(core::cell::UnsafeCell<T>);

// SAFETY: see the module note above -- single-threaded QuakeC execution.
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Self(core::cell::UnsafeCell::new(v))
    }

    fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// `pr_ext.c:3131-3141` `struct qcfile_s`, field for field. C keeps the array
/// in one `Mem_Realloc`'d block; a `Vec` has the same growth-only shape and
/// the same "first slot with a null `file`" allocation order.
struct QcFile {
    owningvm: *mut QcVm,
    cache: [u8; QCFILE_CACHE],
    cacheoffset: qfileofs_t,
    cachesize: qfilesize_t,
    file: *mut FILE,
    fileoffset: qfileofs_t,
    filesize: qfilesize_t,
    filebase: qfileofs_t,
    mode: c_int,
}

impl QcFile {
    /// `PF_fopen` grows the table with `Mem_Realloc`, which does *not* zero
    /// (`mem.c:120`), so C used to read `qcfiles[i].file` out of the byte it
    /// had just added -- an uninitialised read that made handle numbering
    /// allocator-dependent and irreproducible here. `pr_ext.c` now `memset`s
    /// the grown slot, which is the realisation the surrounding code was
    /// written for; zeroing here matches it exactly.
    const fn new() -> Self {
        Self {
            owningvm: ptr::null_mut(),
            cache: [0; QCFILE_CACHE],
            cacheoffset: 0,
            cachesize: 0,
            file: ptr::null_mut(),
            fileoffset: 0,
            filesize: 0,
            filebase: 0,
            mode: 0,
        }
    }
}

/// `pr_ext.c:3381-3387` `struct strbuf`. `strings` stays a raw
/// `Mem_Alloc`/`Mem_Realloc` block rather than a `Vec`: `PF_buf_sort` hands it
/// straight to `qsort`, and the growth arithmetic (`index + 256`, only the
/// bytes past the old count zeroed) is observable through `buf_getsize`.
struct StrBuf {
    owningvm: *mut QcVm,
    strings: *mut *mut c_char,
    used: usize,
    allocated: usize,
}

impl StrBuf {
    const fn new() -> Self {
        Self {
            owningvm: ptr::null_mut(),
            strings: ptr::null_mut(),
            used: 0,
            allocated: 0,
        }
    }
}

const EMPTY_STRBUF: StrBuf = StrBuf::new();

/// `pr_ext.c:3141` `static struct qcfile_s *qcfiles` + `:3143` `qcfiles_max`.
static QCFILES: Global<Vec<QcFile>> = Global::new(Vec::new());
/// `pr_ext.c:3389` `static struct strbuf strbuflist[NUMSTRINGBUFS]`.
static STRBUFLIST: Global<[StrBuf; NUMSTRINGBUFS as usize]> =
    Global::new([EMPTY_STRBUF; NUMSTRINGBUFS as usize]);
/// `pr_ext.c:3499` `static int PF_buf_sort_sortprefixlen` -- "eww, a global.
/// burn in hell." It is kept a global deliberately; see the module note.
static SORT_PREFIX_LEN: Global<c_int> = Global::new(0);

/* ---------------------------------------------------------------------------
 * FRIK_FILE.
 */

/// `pr_ext.c:3104` `QC_FixFileName`, transcribed. Returns
/// `(result, fallbackread)` on success. `result` points at `va`'s rotating
/// buffer, exactly as C's does, so it must be consumed before the next `va`.
///
/// # Safety
/// `name` must be a live NUL-terminated engine string.
unsafe fn qc_fix_file_name(name: *const c_char) -> Option<(*const c_char, *const c_char)> {
    // SAFETY: caller contract.
    let bytes = unsafe { c_bytes(name) };
    if bytes.is_empty()
        || bytes.contains(&b':')
        || bytes.contains(&b'\\')
        || bytes.first() == Some(&b'/')
        || bytes.windows(2).any(|w| w == b"..")
    {
        return None;
    }

    let mut fallbackread = name;
    // SAFETY: leaves over the same NUL-terminated pointer.
    let is_cfg = unsafe {
        (!bytes.contains(&b'/') || q_strncasecmp(name, c"configs/".as_ptr(), 8) != 0)
            && q_strcasecmp(c::COM_FileGetExtension(name), c"cfg".as_ptr()) == 0
            && q_strncasecmp(name, c"particles/".as_ptr(), 10) != 0
            && q_strncasecmp(name, c"huds/".as_ptr(), 5) != 0
            && q_strncasecmp(name, c"models/".as_ptr(), 7) != 0
    };
    if is_cfg {
        fallbackread = ptr::null();
    }
    // SAFETY: `va` formats into its own rotating static buffer.
    let result = unsafe { va(c"data/%s".as_ptr(), name) };
    Some((result, fallbackread))
}

fn pf_fopen(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: ADR-008 -- a builtin only runs inside PR_ExecuteProgram.
    let mut raw = unsafe { VmRaw::new(vm) };
    let name_in = g_string(&raw, OFS_PARM0)?;
    let fmode = raw.g_f32(OFS_PARM1) as c_int;

    raw.set_g_f32(OFS_RETURN, -1.0); // assume failure

    // SAFETY: `name_in` is a live engine string.
    let Some((fname, fallback)) = (unsafe { qc_fix_file_name(name_in) }) else {
        // COMPAT: C prints the *unfixed* name here -- QC_FixFileName leaves
        // `*result` untouched when it returns false, so `fname` in the caller
        // is still the argument.
        let mut msg = b"PF_fopen: Access denied: ".to_vec();
        // SAFETY: as above.
        msg.extend_from_slice(unsafe { c_bytes(name_in) });
        msg.push(b'\n');
        con.print(&msg);
        return Ok(());
    };

    let mut file: *mut FILE = ptr::null_mut();
    let mut filesize: qfilesize_t = 0;
    let mut name = [0 as c_char; MAX_OSPATH];
    match fmode {
        0 => {
            // SAFETY: leaves; `file` is an out-parameter.
            filesize = unsafe { c::COM_FOpenFile(fname, &mut file, ptr::null_mut()) };
            if file.is_null() && !fallback.is_null() {
                // SAFETY: as above.
                filesize = unsafe { c::COM_FOpenFile(fallback, &mut file, ptr::null_mut()) };
            }
        }
        1 => {
            // SAFETY: `name` is MAX_OSPATH bytes, which is what C passes.
            unsafe {
                q_snprintf(
                    name.as_mut_ptr(),
                    MAX_OSPATH,
                    c"%s/%s".as_ptr(),
                    ptr::addr_of!(c::com_gamedir).cast::<c_char>(),
                    fname,
                );
                // COMPAT: mode 1 is documented as "append" but opens "w+b",
                // which truncates. Preserved verbatim (pr_ext.c:3181).
                file = c::Sys_fopen(name.as_ptr(), c"w+b".as_ptr());
                if !file.is_null() {
                    c::Sys_fseek(file, 0, SEEK_END);
                }
            }
        }
        2 => {
            // SAFETY: as the mode-1 arm.
            unsafe {
                q_snprintf(
                    name.as_mut_ptr(),
                    MAX_OSPATH,
                    c"%s/%s".as_ptr(),
                    ptr::addr_of!(c::com_gamedir).cast::<c_char>(),
                    fname,
                );
                file = c::Sys_fopen(name.as_ptr(), c"wb".as_ptr());
            }
        }
        _ => {
            con.warn(format!("PF_fopen: unsupported mode: {fmode}\n").as_bytes());
            return Ok(());
        }
    }
    if file.is_null() {
        return Ok(());
    }

    // SAFETY: the table is this module's own; no reference to it escapes.
    let files = unsafe { &mut *QCFILES.get() };
    let mut i = 0usize;
    loop {
        if i == files.len() {
            files.push(QcFile::new());
        }
        if files[i].file.is_null() {
            break;
        }
        i += 1;
    }

    let f = &mut files[i];
    // SAFETY: `file` is a live FILE* from the open above.
    f.filebase = unsafe { c::Sys_ftell(file) };
    f.owningvm = vm;
    f.file = file;
    f.mode = fmode;
    f.filesize = filesize;
    f.fileoffset = 0;
    f.cacheoffset = 0;
    f.cachesize = 0;

    raw.set_g_f32(OFS_RETURN, i as c_float + QC_FILE_BASE);
    Ok(())
}

/// `float` -> unsigned integer with the *host target's* conversion semantics.
///
/// COMPAT (ADR-010): C leaves this conversion undefined for negative values,
/// and the targets the engine builds for genuinely disagree. On x86 and
/// x86-64 the compiler emits a `cvttss2si`-style truncation, so a -1 failure
/// handle arrives as `SIZE_MAX`/`0xffffffff` and is rejected by the
/// `>= qcfiles_max` / `>= NUMSTRINGBUFS` bounds test. On aarch64 `fcvtzu`
/// *saturates* to 0, so C accepts the handle and aliases slot 0. Rust's `as`
/// saturates like aarch64. ADR-010 is a per-platform determinism policy, so
/// the port mirrors whichever host it is built for rather than hardcoding
/// one: the `i64` hop reproduces the truncation, the bare cast the
/// saturation.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn c_f32_to_unsigned(f: c_float) -> u64 {
    f as i64 as u64
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn c_f32_to_unsigned(f: c_float) -> u64 {
    f as u64
}

/// The `size_t fileid = G_FLOAT (OFS_PARM0) - QC_FILE_BASE` narrowing every
/// file builtin opens with; see [`c_f32_to_unsigned`].
fn file_id(raw: &VmRaw) -> usize {
    c_f32_to_unsigned(raw.g_f32(OFS_PARM0) - QC_FILE_BASE) as usize
}

fn pf_fgets(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    let fileid = file_id(&raw);
    raw.set_g_i32(OFS_RETURN, 0);

    // SAFETY: this module's own table.
    let files = unsafe { &mut *QCFILES.get() };
    if fileid >= files.len() {
        con.warn(b"PF_fgets: invalid file handle\n");
        return Ok(());
    }
    if files[fileid].file.is_null() {
        con.warn(b"PF_fgets: file not open\n");
        return Ok(());
    }
    if files[fileid].mode != 0 {
        con.warn(b"PF_fgets: file not open for reading\n");
        return Ok(());
    }

    let f = &mut files[fileid];
    // SAFETY: a leaf that steps the temp-string ring; the buffer it returns is
    // STRINGTEMP_LENGTH bytes and stays valid until the ring wraps round.
    let ret = unsafe { PR_GetTempString() };
    // C walks `char *s` from `ret` to `end = ret + STRINGTEMP_LENGTH`; the
    // cursor here is the same walk as an index, so the `s == end` rewind is a
    // plain `len - 1` clamp.
    let mut n = 0usize;
    loop {
        if f.cacheoffset == f.cachesize {
            let mut sz = f.filesize - f.fileoffset;
            if sz < 0 || f.fileoffset < 0 {
                sz = 0;
            } else if sz as usize > QCFILE_CACHE {
                sz = QCFILE_CACHE as qfilesize_t;
            }
            f.cacheoffset = 0;
            // SAFETY: `sz` is clamped to the cache's length and `f.file` is a
            // live FILE* opened for reading.
            f.cachesize = unsafe { fread(f.cache.as_mut_ptr().cast(), 1, sz as usize, f.file) }
                as qfilesize_t;
            f.fileoffset += f.cachesize;
            if f.cachesize == 0 {
                if n == 0 {
                    // absolutely nothing to spew
                    raw.set_g_i32(OFS_RETURN, 0);
                    return Ok(());
                }
                break; // classic eof...
            }
        }
        let b = f.cache[f.cacheoffset as usize];
        f.cacheoffset += 1;
        // SAFETY: `n <= STRINGTEMP_LENGTH - 1` by the clamp below.
        unsafe { ret.add(n).write(b as c_char) };
        if b == b'\n' {
            break;
        }
        n += 1;
        if n == STRINGTEMP_LENGTH {
            n -= 1; // rewind if we're overflowing, such that we truncate
        }
    }
    // SAFETY: `n <= STRINGTEMP_LENGTH - 1`, and `n > 0` guards the read back.
    unsafe {
        if n > 0 && ret.add(n - 1).read() == b'\r' as c_char {
            n -= 1; // terminate it on the \r of a \r\n pair
        }
        ret.add(n).write(0);
        raw.set_g_i32(OFS_RETURN, g::PR_SetEngineString(ret));
    }
    Ok(())
}

fn pf_fputs(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let fileid = file_id(&raw);
    // COMPAT: C evaluates PF_VarString(1) in the declaration's initialiser,
    // i.e. before any of the handle tests below. See the module's ADR-009
    // note: this seam is unguarded, matching progs_builtins.rs:212.
    // SAFETY: returns PF_VarString's `static char out[1024]`, live until the
    // next call.
    let str_ = unsafe { PF_VarString(1) };

    // SAFETY: this module's own table.
    let files = unsafe { &mut *QCFILES.get() };
    if fileid >= files.len() {
        con.warn(b"PF_fputs: invalid file handle\n");
    } else if files[fileid].file.is_null() {
        con.warn(b"PF_fputs: file not open\n");
    } else if files[fileid].mode == 0 {
        // COMPAT: the message really does say "PF_fgets" (pr_ext.c:3280).
        con.warn(b"PF_fgets: file not open for writing\n");
    } else {
        // SAFETY: both pointers are live; fputs is a leaf.
        unsafe { fputs(str_, files[fileid].file) };
    }
    Ok(())
}

fn pf_fclose(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let fileid = file_id(&raw);

    // SAFETY: this module's own table.
    let files = unsafe { &mut *QCFILES.get() };
    if fileid >= files.len() {
        con.warn(b"PF_fclose: invalid file handle\n");
    } else if files[fileid].file.is_null() {
        con.warn(b"PF_fclose: file not open\n");
    } else {
        // SAFETY: a live FILE* this module opened.
        unsafe { fclose(files[fileid].file) };
        files[fileid].file = ptr::null_mut();
        files[fileid].owningvm = ptr::null_mut();
    }
    Ok(())
}

fn pr_frikfile_shutdown(vm: *mut QcVm) -> SvResult {
    // SAFETY: this module's own table.
    let files = unsafe { &mut *QCFILES.get() };
    for f in files.iter_mut() {
        if f.owningvm == vm {
            // SAFETY: `owningvm` is only ever non-null alongside a live
            // `file` (PF_fopen sets both, PF_fclose clears both), so C's
            // unchecked fclose here is never handed a null.
            unsafe { fclose(f.file) };
            f.file = ptr::null_mut();
            f.owningvm = ptr::null_mut();
        }
    }
    Ok(())
}

fn pf_fseek(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    let fileid = file_id(&raw);
    raw.set_g_i32(OFS_RETURN, 0);

    // SAFETY: this module's own table.
    let files = unsafe { &mut *QCFILES.get() };
    // COMPAT: both warnings say "PF_fread", not "PF_fseek" (pr_ext.c:3320,
    // :3322).
    if fileid >= files.len() {
        con.warn(b"PF_fread: invalid file handle\n");
        return Ok(());
    }
    if files[fileid].file.is_null() {
        con.warn(b"PF_fread: file not open\n");
        return Ok(());
    }

    let argc = raw.argc();
    let f = &mut files[fileid];
    if f.mode == 0 {
        // when we're reading, use the cached read offset
        raw.set_g_i32(OFS_RETURN, f.fileoffset as c_int);
    } else {
        // COMPAT: the cast binds to Sys_ftell alone, so the subtraction is
        // `int - qfileofs_t` widened back to long long and then truncated by
        // the G_INT store (pr_ext.c:3329).
        // SAFETY: a live FILE*.
        let tell = unsafe { c::Sys_ftell(f.file) } as c_int;
        raw.set_g_i32(OFS_RETURN, (tell as i64 - f.filebase) as c_int);
    }
    if argc > 1 {
        f.fileoffset = raw.g_i32(OFS_PARM1) as qfileofs_t;
        // SAFETY: a live FILE*; Sys_fseek is a leaf.
        unsafe { c::Sys_fseek(f.file, f.filebase + f.fileoffset, SEEK_SET) };
        f.cachesize = 0;
        f.cacheoffset = 0;
    }
    Ok(())
}

fn pf_whichpack(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    // uses native paths, as this isn't actually reading anything
    let fname = g_string(&raw, OFS_PARM0)?;

    let mut path_id: c_uint = 0;
    // SAFETY: a leaf over a live name; `path_id` is an out-parameter.
    if !unsafe { c::COM_FileExists(fname, &mut path_id) } {
        raw.set_g_i32(OFS_RETURN, 0);
        return Ok(());
    }

    // FIXME (C's own): quakespasm reports which gamedir the file is in, but
    // paks are hidden.
    // SAFETY: `com_searchpaths` is the engine's live list; every `next` walk
    // ends at null.
    let handle = unsafe {
        let mut path = com_searchpaths;
        while !path.is_null() {
            if (*path).pack.is_null() && (*path).path_id == path_id {
                break; // okay, this one looks like one we can report
            }
            path = (*path).next;
        }

        // COMPAT (REPORTED): C dereferences `path` unconditionally here
        // (pr_ext.c:3358). COM_FileExists can report a path_id that only a
        // pack carries, in which case the loop ends at NULL and C crashes.
        // Reproduced rather than papered over: both sides fault on the same
        // input, and inserting a null check would invent a return value the
        // engine has never had.
        let mut name: *const c_char = (*path).filename.as_ptr();

        // sandbox it by stripping the basedir
        let base = c_bytes(ptr::addr_of!(c::manual::com_basedir).cast::<c_char>());
        if c_bytes(name).starts_with(base) {
            name = name.add(base.len());
        } else {
            name = c"?".as_ptr(); // something is screwing with us
        }
        // small cleanup, just in case
        while name.read() == b'/' as c_char || name.read() == b'\\' as c_char {
            name = name.add(1);
        }
        g::PR_SetEngineString(name)
    };
    raw.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/* ---------------------------------------------------------------------------
 * String buffers.
 */

/// The `bufno` narrowing shared by all twelve buffer builtins. C spells it as
/// `unsigned int`, as `int` plus an `(unsigned int)` cast, and (three times) as
/// `size_t` plus an `(unsigned int)` cast; all three forms agree with this one
/// for every value a `float` can name inside `i32`'s range, and every value
/// outside it fails the `>= NUMSTRINGBUFS` test on both sides.
///
/// COMPAT (ADR-010): as in [`file_id`], the conversion follows the host
/// target -- see [`c_f32_to_unsigned`] for why `buf_create`'s -1 failure
/// handle is rejected on x86 and aliases buffer 0 on aarch64.
fn buf_no(raw: &VmRaw, ofs: usize) -> u32 {
    c_f32_to_unsigned(raw.g_f32(ofs) - BUFSTRBASE) as u32
}

/// The `unsigned int index = G_FLOAT (OFS_PARM1)` narrowing (`bufstr_get`,
/// `bufstr_set`); see [`c_f32_to_unsigned`] for the conversion.
fn arg_index_u32(raw: &VmRaw, ofs: usize) -> usize {
    c_f32_to_unsigned(raw.g_f32(ofs)) as u32 as usize
}

/// `strbuflist[bufno]`, or `None` when C's `bufno >= NUMSTRINGBUFS` or
/// `!owningvm` test would have returned.
fn live_buf(bufno: u32) -> Option<&'static mut StrBuf> {
    if bufno >= NUMSTRINGBUFS {
        return None;
    }
    // SAFETY: this module's own table, indexed inside its bounds.
    let b = unsafe { &mut (*STRBUFLIST.get())[bufno as usize] };
    if b.owningvm.is_null() {
        return None;
    }
    Some(b)
}

fn pr_buf_shutdown(vm: *mut QcVm) -> SvResult {
    // SAFETY: this module's own table.
    let bufs = unsafe { &mut *STRBUFLIST.get() };
    for b in bufs.iter_mut() {
        if b.owningvm != vm {
            continue;
        }
        for i in 0..b.used {
            // SAFETY: `i < used <= allocated`, and every live slot came from
            // Mem_Alloc. C does not null-check here either; Mem_Free tolerates
            // null, like SAFE_FREE.
            unsafe { c::Mem_Free(b.strings.add(i).read().cast()) };
        }
        // SAFETY: null or a Mem_Alloc block.
        unsafe { c::Mem_Free(b.strings.cast()) };

        b.owningvm = ptr::null_mut();
        b.strings = ptr::null_mut();
        b.used = 0;
        b.allocated = 0;
    }
    Ok(())
}

// #440 float() buf_create (DP_QC_STRINGBUFFERS)
fn pf_buf_create(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    let type_ = if raw.argc() > 0 {
        g_string(&raw, OFS_PARM0)?
    } else {
        c"string".as_ptr()
    };

    // SAFETY: a leaf over two NUL-terminated strings.
    if unsafe { q_strcasecmp(type_, c"string".as_ptr()) } != 0 {
        raw.set_g_f32(OFS_RETURN, -1.0);
        return Ok(());
    }

    // flags&1 == saved. apparently.
    // SAFETY: this module's own table.
    let bufs = unsafe { &mut *STRBUFLIST.get() };
    for (i, b) in bufs.iter_mut().enumerate() {
        if b.owningvm.is_null() {
            b.owningvm = vm;
            b.used = 0;
            b.allocated = 0;
            b.strings = ptr::null_mut();
            raw.set_g_f32(OFS_RETURN, i as c_float + BUFSTRBASE);
            return Ok(());
        }
    }
    raw.set_g_f32(OFS_RETURN, -1.0);
    Ok(())
}

// #441 void(float bufhandle) buf_del (DP_QC_STRINGBUFFERS)
fn pf_buf_del(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let Some(b) = live_buf(buf_no(&raw, OFS_PARM0)) else {
        return Ok(());
    };

    for i in 0..b.used {
        // SAFETY: `i < used <= allocated`.
        unsafe {
            let s = b.strings.add(i).read();
            if !s.is_null() {
                c::Mem_Free(s.cast());
            }
        }
    }
    // SAFETY: null or a Mem_Alloc block.
    unsafe { c::Mem_Free(b.strings.cast()) };

    b.strings = ptr::null_mut();
    b.used = 0;
    b.allocated = 0;
    b.owningvm = ptr::null_mut();
    Ok(())
}

// #442 float(float bufhandle) buf_getsize (DP_QC_STRINGBUFFERS)
fn pf_buf_getsize(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    // COMPAT: on a bad handle C returns without touching OFS_RETURN, so the
    // caller reads whatever the previous builtin left there.
    let Some(b) = live_buf(buf_no(&raw, OFS_PARM0)) else {
        return Ok(());
    };
    let used = b.used;
    raw.set_g_f32(OFS_RETURN, used as c_float);
    Ok(())
}

// #443 void(float bufhandle_from, float bufhandle_to) buf_copy
fn pf_buf_copy(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let buffrom = buf_no(&raw, OFS_PARM0);
    let bufto = buf_no(&raw, OFS_PARM1);

    // COMPAT: the self-copy test runs *before* either range check, so a pair
    // of equal out-of-range handles returns here (pr_ext.c:3475).
    if bufto == buffrom {
        return Ok(());
    }
    if live_buf(buffrom).is_none() || live_buf(bufto).is_none() {
        return Ok(());
    }

    let bufs = STRBUFLIST.get();
    // SAFETY: both indices were just range-checked; the two borrows are of
    // distinct elements (buffrom != bufto), so they are taken through the raw
    // table pointer rather than by splitting a slice borrow.
    unsafe {
        let from = &raw const (*bufs)[buffrom as usize];
        let to = &raw mut (*bufs)[bufto as usize];

        // obliterate any and all existing data.
        for i in 0..(*to).used {
            let s = (*to).strings.add(i).read();
            if !s.is_null() {
                c::Mem_Free(s.cast());
            }
        }
        c::Mem_Free((*to).strings.cast());

        // copy new data over.
        let n = (*from).used;
        (*to).used = n;
        (*to).allocated = n;
        (*to).strings = c::Mem_Alloc(n * size_of::<*mut c_char>()).cast();
        for i in 0..n {
            let s = (*from).strings.add(i).read();
            let dup = if s.is_null() {
                ptr::null_mut()
            } else {
                q_strdup(s)
            };
            (*to).strings.add(i).write(dup);
        }
    }
    Ok(())
}

/// `pr_ext.c:3500` `PF_buf_sort_ascending`, transcribed. Pure; see the module
/// note on why the platform `qsort` keeps deciding tie order (ADR-010).
///
/// # Safety
/// `qsort` contract: both arguments point at `char *` elements of the array.
unsafe extern "C" fn buf_sort_ascending(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: caller contract; the strings are NUL-terminated Mem_Alloc
    // blocks, and PF_buf_sort compacted the nulls out first.
    unsafe {
        strncmp(
            a.cast::<*const c_char>().read(),
            b.cast::<*const c_char>().read(),
            (*SORT_PREFIX_LEN.get()) as usize,
        )
    }
}

/// `pr_ext.c:3504` `PF_buf_sort_descending`, transcribed -- including its
/// parameter swap, which is how C spells the reversal.
///
/// # Safety
/// As [`buf_sort_ascending`].
unsafe extern "C" fn buf_sort_descending(b: *const c_void, a: *const c_void) -> c_int {
    // SAFETY: caller contract.
    unsafe {
        strncmp(
            a.cast::<*const c_char>().read(),
            b.cast::<*const c_char>().read(),
            (*SORT_PREFIX_LEN.get()) as usize,
        )
    }
}

// #444 void(float bufhandle, float sortprefixlen, float backward) buf_sort
fn pf_buf_sort(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let mut sortprefixlen = raw.g_f32(OFS_PARM1) as c_int;
    let backwards = raw.g_f32(OFS_PARM2) as c_int;
    let Some(b) = live_buf(buf_no(&raw, OFS_PARM0)) else {
        return Ok(());
    };

    if sortprefixlen <= 0 {
        sortprefixlen = 0x7fff_ffff;
    }

    // take out the nulls first, to avoid weird/crashy sorting
    let strings = b.strings;
    let mut d = 0usize;
    for s in 0..b.used {
        // SAFETY: `s < used <= allocated`.
        let p = unsafe { strings.add(s).read() };
        if p.is_null() {
            continue;
        }
        // SAFETY: `d <= s`, so the write stays inside the same block.
        unsafe { strings.add(d).write(p) };
        d += 1;
    }
    b.used = d;

    // no nulls now, sort it.
    // COMPAT (ADR-010): the prefix length is a process-global, exactly as
    // `PF_buf_sort_sortprefixlen` is -- PF_buf_cvarlist reads whatever this
    // call leaves behind. See the module note.
    // SAFETY: single-threaded QuakeC execution.
    unsafe { *SORT_PREFIX_LEN.get() = sortprefixlen };
    // SAFETY: `strings` holds `b.used` live `char *` and the comparator is a
    // pure leaf, so nothing can longjmp out of qsort (ADR-009).
    unsafe {
        if backwards != 0 {
            // z first
            qsort(
                strings.cast(),
                b.used,
                size_of::<*mut c_char>(),
                buf_sort_descending,
            );
        } else {
            // a first
            qsort(
                strings.cast(),
                b.used,
                size_of::<*mut c_char>(),
                buf_sort_ascending,
            );
        }
    }
    Ok(())
}

// #445 string(float bufhandle, string glue) buf_implode
fn pf_buf_implode(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    let bufno = buf_no(&raw, OFS_PARM0);
    let glue = g_string(&raw, OFS_PARM1)?;
    // SAFETY: a live engine string.
    let glue = unsafe { c_bytes(glue) };
    let Some(b) = live_buf(bufno) else {
        return Ok(());
    };

    // generate the output
    // SAFETY: a leaf that steps the temp-string ring; C steps it here too,
    // before the loop that may bail out early.
    let ret = unsafe { PR_GetTempString() };
    let mut out: Vec<u8> = Vec::new();
    for i in 0..b.used {
        // SAFETY: `i < used <= allocated`.
        let s = unsafe { b.strings.add(i).read() };
        if s.is_null() {
            continue;
        }
        if !out.is_empty() {
            if out.len() + glue.len() + 1 > STRINGTEMP_LENGTH {
                con.print(b"PF_buf_implode: tempstring overflow\n");
                break;
            }
            out.extend_from_slice(glue);
        }
        // SAFETY: a live NUL-terminated Mem_Alloc block.
        let bytes = unsafe { c_bytes(s) };
        if out.len() + bytes.len() + 1 > STRINGTEMP_LENGTH {
            con.print(b"PF_buf_implode: tempstring overflow\n");
            break;
        }
        out.extend_from_slice(bytes);
    }

    // add the null and return
    // SAFETY: `out.len() < STRINGTEMP_LENGTH` by the two checks above, so the
    // copy plus terminator fits the temp buffer.
    unsafe {
        ptr::copy_nonoverlapping(out.as_ptr().cast::<c_char>(), ret, out.len());
        ret.add(out.len()).write(0);
        raw.set_g_i32(OFS_RETURN, g::PR_SetEngineString(ret));
    }
    Ok(())
}

// #446 string(float bufhandle, float string_index) bufstr_get
fn pf_bufstr_get(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    let index = arg_index_u32(&raw, OFS_PARM1);
    let Some(b) = live_buf(buf_no(&raw, OFS_PARM0)) else {
        raw.set_g_i32(OFS_RETURN, 0);
        return Ok(());
    };
    if index >= b.used {
        raw.set_g_i32(OFS_RETURN, 0);
        return Ok(());
    }

    // SAFETY: `index < used <= allocated`.
    let s = unsafe { b.strings.add(index).read() };
    if s.is_null() {
        raw.set_g_i32(OFS_RETURN, 0);
        return Ok(());
    }

    // C: q_strlcpy (ret, s, STRINGTEMP_LENGTH), i.e. truncate at 1023 bytes.
    // SAFETY: a live NUL-terminated block; the temp buffer is
    // STRINGTEMP_LENGTH bytes and the copy is clamped to one less.
    unsafe {
        let bytes = c_bytes(s);
        let n = bytes.len().min(STRINGTEMP_LENGTH - 1);
        let ret = PR_GetTempString();
        ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), ret, n);
        ret.add(n).write(0);
        raw.set_g_i32(OFS_RETURN, g::PR_SetEngineString(ret));
    }
    Ok(())
}

/// The `index >= allocated` growth shared by `PF_bufstr_set` and
/// `PF_bufstr_add_internal` (`pr_ext.c:3646-3652`, `:3679-3687`).
///
/// COMPAT (deviation -- REPORTED): C computes `allocated = (index + 256)` in
/// `unsigned int`. An `index` above `UINT_MAX - 256` wraps that to a tiny
/// count, and the store below then writes far outside the block. This port
/// keeps the count in `usize`, so the same `index` allocates (and
/// `Sys_Error`s on) 32GB instead of corrupting the heap. Identical for every
/// `index` that does not wrap, which is every `index` a sane mod produces.
///
/// # Safety
/// `b` must be a live buffer; `index` is the slot about to be written.
unsafe fn strbuf_reserve(b: &mut StrBuf, index: usize) {
    if index < b.allocated {
        return;
    }
    let oldcount = b.allocated;
    b.allocated = index + 256;
    // SAFETY: `strings` is null or a Mem_Alloc block; Mem_Realloc keeps the
    // old bytes and returns memory or Sys_Errors (ADR-013).
    unsafe {
        b.strings = c::Mem_Realloc(b.strings.cast(), b.allocated * size_of::<*mut c_char>()).cast();
        ptr::write_bytes(b.strings.add(oldcount), 0, b.allocated - oldcount);
    }
}

/// `Mem_Alloc (strlen (string) + 1)` + `strcpy`, the store both setters share.
///
/// # Safety
/// `b` must be live and `index` inside `b.allocated`.
unsafe fn strbuf_store(b: &mut StrBuf, index: usize, string: *const c_char) {
    // SAFETY: caller contract; `string` is a live engine string.
    unsafe {
        let old = b.strings.add(index).read();
        if !old.is_null() {
            c::Mem_Free(old.cast());
        }
        let bytes = c_bytes(string);
        let dst = c::Mem_Alloc(bytes.len() + 1).cast::<c_char>();
        ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), dst, bytes.len());
        dst.add(bytes.len()).write(0);
        b.strings.add(index).write(dst);
    }
}

// #447 void(float bufhandle, float string_index, string str) bufstr_set
fn pf_bufstr_set(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let bufno = buf_no(&raw, OFS_PARM0);
    let index = arg_index_u32(&raw, OFS_PARM1);
    let string = g_string(&raw, OFS_PARM2)?;
    let Some(b) = live_buf(bufno) else {
        return Ok(());
    };

    // SAFETY: the reserve grows past `index`, so the store is in bounds.
    unsafe {
        strbuf_reserve(b, index);
        strbuf_store(b, index, string);
    }
    if index >= b.used {
        b.used = index + 1;
    }
    Ok(())
}

/// `pr_ext.c:3665` `PF_bufstr_add_internal`.
///
/// # Safety
/// `b` must be a live buffer and `string` a live engine string.
unsafe fn bufstr_add_internal(b: &mut StrBuf, string: *const c_char, appendonend: bool) -> usize {
    let index = if appendonend {
        // add on end
        b.used
    } else {
        // find a hole
        let mut i = 0usize;
        while i < b.used {
            // SAFETY: `i < used <= allocated`.
            if unsafe { b.strings.add(i).read() }.is_null() {
                break;
            }
            i += 1;
        }
        i
    };

    // expand it if needed, then add in the new string.
    // SAFETY: caller contract; the reserve grows past `index`.
    unsafe {
        strbuf_reserve(b, index);
        strbuf_store(b, index, string);
    }

    if index >= b.used {
        b.used = index + 1;
    }
    index
}

// #448 float(float bufhandle, string str, float order) bufstr_add
fn pf_bufstr_add(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let mut raw = unsafe { VmRaw::new(vm) };
    let bufno = buf_no(&raw, OFS_PARM0);
    let string = g_string(&raw, OFS_PARM1)?;
    // `qboolean` is `bool` (q_types.h:122), so any non-zero float is ordered.
    let ordered = raw.g_f32(OFS_PARM2) != 0.0;
    let Some(b) = live_buf(bufno) else {
        return Ok(());
    };

    // SAFETY: `b` is live and `string` is a live engine string.
    let index = unsafe { bufstr_add_internal(b, string, ordered) };
    raw.set_g_f32(OFS_RETURN, index as c_int as c_float);
    Ok(())
}

// #449 void(float bufhandle, float string_index) bufstr_free
fn pf_bufstr_free(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let index = raw.g_f32(OFS_PARM1) as i64 as usize;
    let Some(b) = live_buf(buf_no(&raw, OFS_PARM0)) else {
        return Ok(());
    };
    if index >= b.used {
        return Ok(()); // not valid anyway.
    }

    // SAFETY: `index < used <= allocated`.
    unsafe {
        let s = b.strings.add(index).read();
        if !s.is_null() {
            c::Mem_Free(s.cast());
        }
        b.strings.add(index).write(ptr::null_mut());
    }
    Ok(())
}

fn pf_buf_cvarlist(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008.
    let raw = unsafe { VmRaw::new(vm) };
    let bufno = buf_no(&raw, OFS_PARM0);
    let pattern = g_string(&raw, OFS_PARM1)?;
    let antipattern = g_string(&raw, OFS_PARM2)?;
    // SAFETY: both are live engine strings.
    let (pbytes, abytes) = unsafe { (c_bytes(pattern), c_bytes(antipattern)) };
    let plen = pbytes.len();
    let alen = abytes.len();
    let pwc = pbytes.contains(&b'*') || pbytes.contains(&b'?');
    let awc = abytes.contains(&b'*') || abytes.contains(&b'?');

    let Some(b) = live_buf(bufno) else {
        return Ok(());
    };

    // obliterate any and all existing data.
    for i in 0..b.used {
        // SAFETY: `i < used <= allocated`.
        unsafe {
            let s = b.strings.add(i).read();
            if !s.is_null() {
                c::Mem_Free(s.cast());
            }
        }
    }
    if !b.strings.is_null() {
        // SAFETY: a Mem_Alloc block.
        unsafe { c::Mem_Free(b.strings.cast()) };
    }
    b.used = 0;
    b.allocated = 0;
    // COMPAT (deviation -- REPORTED): C leaves `strings` dangling here
    // (pr_ext.c:3757-3759 free it but never null it), so the very next
    // PF_bufstr_add_internal hands the freed pointer back to Mem_Realloc --
    // a use-after-free. Nulling it is the only realisation Rust can offer,
    // and it is what the surrounding code assumes.
    b.strings = ptr::null_mut();

    // SAFETY: Cvar_FindVarAfter walks the engine's registered-cvar list; every
    // `name` on it is a live NUL-terminated string and the walk ends at null.
    unsafe {
        let mut var = Cvar_FindVarAfter(c"".as_ptr(), c::cvarflags_t_CVAR_NONE);
        while !var.is_null() {
            let name = (*var).name;
            let skip_p = plen != 0
                && if pwc {
                    wildcmp(pattern, name) == 0
                } else {
                    strncmp(name, pattern, plen) != 0
                };
            let skip_a = !skip_p
                && alen != 0
                && if awc {
                    wildcmp(antipattern, name) != 0
                } else {
                    strncmp(name, antipattern, alen) == 0
                };
            if !skip_p && !skip_a {
                bufstr_add_internal(b, name, true);
            }
            var = (*var).next;
        }
    }

    // COMPAT (ADR-010): no sortprefixlen is set -- this sort uses whatever the
    // last PF_buf_sort left in the global (0 on a fresh process, which makes
    // the comparator always-equal). See the module note.
    // SAFETY: `strings` holds `b.used` live `char *`; the comparator is pure.
    unsafe {
        qsort(
            b.strings.cast(),
            b.used,
            size_of::<*mut c_char>(),
            buf_sort_ascending,
        )
    };
    Ok(())
}

/* ---------------------------------------------------------------------------
 * The `RUST_PF` / hand-frame entry points.
 */

macro_rules! filebuf_entry {
    ($rs:ident, $core:ident) => {
        /// # Safety
        /// `detail` is `pr_cmds_glue.c`'s `RUST_PF` `&detail`.
        #[no_mangle]
        pub unsafe extern "C" fn $rs(detail: *mut c_int) -> c_int {
            // SAFETY: caller contract; ADR-008 ambient qcvm.
            unsafe { run_sv(detail, |vm, _con| $core(vm)) }
        }
    };
    ($rs:ident, $core:ident, con) => {
        /// # Safety
        /// `detail` is `pr_cmds_glue.c`'s `RUST_PF` `&detail`.
        #[no_mangle]
        pub unsafe extern "C" fn $rs(detail: *mut c_int) -> c_int {
            // SAFETY: caller contract; ADR-008 ambient qcvm.
            unsafe { run_sv(detail, $core) }
        }
    };
}

filebuf_entry!(quake_rs_pf_fopen, pf_fopen, con);
filebuf_entry!(quake_rs_pf_fgets, pf_fgets, con);
filebuf_entry!(quake_rs_pf_fputs, pf_fputs, con);
filebuf_entry!(quake_rs_pf_fclose, pf_fclose, con);
filebuf_entry!(quake_rs_pf_fseek, pf_fseek, con);
filebuf_entry!(quake_rs_pf_whichpack, pf_whichpack);
filebuf_entry!(quake_rs_pf_buf_create, pf_buf_create);
filebuf_entry!(quake_rs_pf_buf_del, pf_buf_del);
filebuf_entry!(quake_rs_pf_buf_getsize, pf_buf_getsize);
filebuf_entry!(quake_rs_pf_buf_copy, pf_buf_copy);
filebuf_entry!(quake_rs_pf_buf_sort, pf_buf_sort);
filebuf_entry!(quake_rs_pf_buf_implode, pf_buf_implode, con);
filebuf_entry!(quake_rs_pf_bufstr_get, pf_bufstr_get);
filebuf_entry!(quake_rs_pf_bufstr_set, pf_bufstr_set);
filebuf_entry!(quake_rs_pf_bufstr_add, pf_bufstr_add);
filebuf_entry!(quake_rs_pf_bufstr_free, pf_bufstr_free);
filebuf_entry!(quake_rs_pf_buf_cvarlist, pf_buf_cvarlist);

/// `pr_ext.c:3291` `PF_frikfile_shutdown` -- not a builtin; called by
/// `PR_ShutdownExtensions` (`pr_ext.c:6180`).
///
/// # Safety
/// `detail` must point at a writable `int`, as `pr_cmds_glue.c`'s
/// `rust_pr_frikfile_shutdown` frame passes.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_frikfile_shutdown(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; the ambient qcvm is the one being shut down.
    unsafe { run_sv(detail, |vm, _con| pr_frikfile_shutdown(vm)) }
}

/// `pr_ext.c:3391` `PF_buf_shutdown` -- not a builtin; called by
/// `PR_ShutdownExtensions` (`pr_ext.c:6181`).
///
/// # Safety
/// As [`quake_rs_pr_frikfile_shutdown`].
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_buf_shutdown(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; the ambient qcvm is the one being shut down.
    unsafe { run_sv(detail, |vm, _con| pr_buf_shutdown(vm)) }
}
