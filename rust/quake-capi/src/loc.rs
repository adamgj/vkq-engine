//! C ABI shims for the LOCALIZATION block of `Quake/common_fs.c`
//! (declarations stay in `Quake/common.h`).
//!
//! Parsing/lookup/format logic is pure (`quake_fs::loc`); this shim owns the
//! C's `static localization_t localization` global and performs the file
//! acquisition chain (searchpaths via COM_LoadFile, then direct basedir IO,
//! then extraction from QuakeEX.kpf via `quake_fs::zipdir`). The C's flat
//! text buffer survives as the Vec inside [`LocData`]: entry offsets index
//! into it, and LOC_GetRawString hands out `const char *` pointers into that
//! heap allocation, which stay valid until LOC_Shutdown or a reload drops it
//! — exactly the lifetime the C's `Mem_Free (localization.text)` gave.
//!
//! LOC_LoadFile is non-static in the C but declared in no header and called
//! only by LOC_Init; it stays a private fn here (deliberately not exported).

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::ptr::{addr_of, addr_of_mut};
use quake_c_sys::qboolean;
use quake_fs::loc::{self, FormatWarning, LocData, LocError, LocWarning};
use quake_fs::zipdir::ZipArchive;

/// The C's `static localization_t localization`. Main-thread only, like every
/// access to the unlocked C global (init/shutdown and the console/menu string
/// lookups).
static mut LOCALIZATION: Option<LocData> = None;

/// Shared read access to [`LOCALIZATION`].
///
/// # Safety
/// Main thread only; the reference must not be held across
/// [`loc_load_file`]/[`LOC_Shutdown`], which drop the pointee.
unsafe fn loc_data() -> Option<&'static LocData> {
    // SAFETY: main-thread-only access per this function's contract; going
    // through addr_of! avoids a reference to the static mut itself
    unsafe { (*addr_of!(LOCALIZATION)).as_ref() }
}

/// C: `unsigned COM_HashString (const char *str);` — FNV-1a over the
/// string's chars (sign-extended through `char`; see
/// `quake_fs::loc::hash_string`).
///
/// # Safety
/// `str_` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn COM_HashString(str_: *const c_char) -> c_uint {
    // SAFETY: caller contract above
    loc::hash_string(unsafe { CStr::from_ptr(str_) }.to_bytes())
}

/// C: `unsigned COM_HashBlock (const void *data, size_t size);` — FNV-1a
/// over unsigned bytes.
///
/// # Safety
/// `data` must be readable for `size` bytes (anything, including NULL, is
/// fine when `size` is 0 — the C never dereferences then either).
#[no_mangle]
pub unsafe extern "C" fn COM_HashBlock(data: *const c_void, size: usize) -> c_uint {
    if size == 0 {
        return loc::hash_block(&[]);
    }
    // SAFETY: caller contract above; size > 0, so data is a real block
    loc::hash_block(unsafe { core::slice::from_raw_parts(data.cast::<u8>(), size) })
}

/// Maps a parse warning to the exact console line the C emits from inside
/// LOC_LoadFile's parse loop (Con_DPrintf for the comment, plain Con_Printf
/// for the escape — the asymmetry is the C's).
fn parse_warn(w: LocWarning) {
    match w {
        LocWarning::MalformedComment { line } => {
            // SAFETY: engine C API; static NUL-terminated format, int arg
            unsafe {
                quake_c_sys::Con_DPrintf(
                    c"LOC_LoadFile: malformed comment on line %d\n".as_ptr(),
                    line,
                );
            }
        }
        LocWarning::UnrecognizedEscape { c, line } => {
            // the C passes a plain `char`, promoted through the platform's
            // char signedness; %c prints the same byte either way
            // SAFETY: engine C API; static NUL-terminated format, int args
            unsafe {
                quake_c_sys::Con_Printf(
                    c"LOC_LoadFile: unrecognized escape sequence \\%c on line %d\n".as_ptr(),
                    c as c_char as c_int,
                    line,
                );
            }
        }
    }
}

/// The console line of the C's `fail:` label.
///
/// # Safety
/// `file` must be a NUL-terminated string; com_basedir must be initialized
/// (COM_InitFilesystem has run).
unsafe fn print_load_failure(file: *const c_char) {
    // SAFETY: caller contract above; the format string is static and
    // NUL-terminated, and com_basedir is the engine's NUL-terminated
    // char[MAX_OSPATH] global
    unsafe {
        quake_c_sys::Con_Printf(
            c"Couldn't load '%s'\nfrom '%s'\n".as_ptr(),
            file,
            addr_of!(quake_c_sys::manual::com_basedir) as *const c_char,
        );
    }
}

/// The C's `q_snprintf (path, sizeof (path), "%s/%s", a, b)` over
/// `char path[1024]`: the joined path truncated to 1023 bytes, plus the NUL.
fn join_path(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(a.len() + b.len() + 2);
    v.extend_from_slice(a);
    v.push(b'/');
    v.extend_from_slice(b);
    v.truncate(1023);
    v.push(0);
    v
}

/// Reads `size` bytes from an open Sys file handle (chunked to the `int`
/// counts Sys_FileRead takes). A short read leaves the zeroed tail, which
/// then fails the zip open just as miniz's short reads fail it in the C.
///
/// # Safety
/// `handle` must be an open Sys file handle.
unsafe fn read_all(handle: c_int, size: usize) -> Option<Vec<u8>> {
    // The C streams the archive through miniz's read callback in 64 KB
    // chunks and so never sizes an allocation off the file length; this
    // buffers the whole image instead (see the module note), which makes a
    // bogus or hostile length an allocation request. Fail it gracefully into
    // the caller's existing "Couldn't load" path rather than aborting.
    let mut buf = Vec::new();
    buf.try_reserve_exact(size).ok()?;
    buf.resize(size, 0u8);
    let mut off = 0usize;
    while off < buf.len() {
        let count = (buf.len() - off).min(c_int::MAX as usize) as c_int;
        // SAFETY: `handle` is open per the contract above, and `buf[off..]`
        // is writable for `count` bytes
        let n = unsafe {
            quake_c_sys::Sys_FileRead(handle, buf[off..].as_mut_ptr() as *mut c_void, count)
        };
        if n <= 0 {
            break;
        }
        off += n as usize;
    }
    Some(buf)
}

/// C: `void LOC_LoadFile (const char *file);` — non-static in the C but
/// declared in no header and called only by LOC_Init, so it is deliberately
/// not exported.
///
/// # Safety
/// `file` must be NULL or a NUL-terminated string; engine single-threaded
/// init path after COM_InitFilesystem (it rebuilds the data that
/// LOC_GetRawString pointers borrow).
unsafe fn loc_load_file(file: *const c_char) {
    // clear existing data — drops the previous text buffer exactly where the
    // C Mem_Free's localization.text and zeroes the entry/index counts
    // SAFETY: main-thread-only static access per the contract above; any
    // pointers LOC_GetRawString handed out die here, as in the C
    unsafe { *addr_of_mut!(LOCALIZATION) = None };

    // SAFETY: `file` is NULL or NUL-terminated per the contract above
    if file.is_null() || unsafe { *file } == 0 {
        return;
    }

    // SAFETY: engine C API; static NUL-terminated string
    unsafe { quake_c_sys::Con_Printf(c"\nLanguage initialization\n".as_ptr()) };

    // SAFETY: `file` is non-NULL (checked above), so NUL-terminated
    let file_bytes = unsafe { CStr::from_ptr(file) }.to_bytes();

    // File acquisition chain: searchpaths, then "<com_basedir>/<file>", then
    // "<com_basedir>/QuakeEX.kpf". The C's DO_USERDIRS variants (userdir
    // fallbacks after each basedir attempt) are compiled out in this build:
    // meson_options.txt defaults `do_userdirs` to disabled and no Rust
    // feature is plumbed for it, so this matches the C built without
    // DO_USERDIRS.
    let bytes: Vec<u8>;

    // SAFETY: engine C API (link-resolves to the Rust COM_LoadFile export in
    // crate::fs, which keeps the C contract); `file` is NUL-terminated and
    // the path_id out-parameter is nullable
    let com_buf = unsafe { quake_c_sys::COM_LoadFile(file, core::ptr::null_mut()) };
    if !com_buf.is_null() {
        // COM_LoadFile "allways appends a 0 byte", so the buffer is a
        // NUL-terminated image; the parse stops at the first NUL exactly
        // like the C's `while (*cursor)`, so its NUL-free prefix is all the
        // parse can ever see
        // SAFETY: com_buf is that NUL-terminated buffer
        bytes = unsafe { CStr::from_ptr(com_buf as *const c_char) }
            .to_bytes()
            .to_vec();
        // SAFETY: the loaded buffer is owned here (as `localization.text`
        // was in the C) and its bytes were copied out above
        unsafe { quake_c_sys::Mem_Free(com_buf as *const c_void) };
    } else {
        // SAFETY: com_basedir is set by COM_InitFilesystem before LOC_Init
        // runs (Host_Init order) and is NUL-terminated
        let basedir =
            unsafe { CStr::from_ptr(addr_of!(quake_c_sys::manual::com_basedir) as *const c_char) }
                .to_bytes();

        let mut handle: c_int = -1;
        let path = join_path(basedir, file_bytes);
        // SAFETY: `path` is NUL-terminated and `handle` a writable out-param
        let sz =
            unsafe { quake_c_sys::Sys_FileOpenRead(path.as_ptr() as *const c_char, &mut handle) };

        if handle < 0 {
            let path = join_path(basedir, b"QuakeEX.kpf");
            // SAFETY: as above
            let sz = unsafe {
                quake_c_sys::Sys_FileOpenRead(path.as_ptr() as *const c_char, &mut handle)
            };
            if handle < 0 {
                // C fail path with no open handle: just the console line
                // SAFETY: `file` is NUL-terminated
                unsafe { print_load_failure(file) };
                return;
            }
            if sz <= 0 {
                // SAFETY: `handle` is the open handle; `file` NUL-terminated
                unsafe {
                    quake_c_sys::Sys_FileClose(handle);
                    print_load_failure(file);
                }
                return;
            }
            // the C streams the zip through mz's read callback; zipdir works
            // over the whole image, so read it all up front
            // SAFETY: `handle` is open and `sz` is its (positive) size
            let kpf = unsafe { read_all(handle, sz as usize) };
            // SAFETY: `handle` is the open handle (closed after extraction
            // in the C; the whole image is already read out here)
            unsafe { quake_c_sys::Sys_FileClose(handle) };

            // buffering the image failed (absurd length): same user-visible
            // outcome as a reader-init failure
            let Some(kpf) = kpf else {
                // SAFETY: `file` is NUL-terminated
                unsafe { print_load_failure(file) };
                return;
            };

            match ZipArchive::open(&kpf).and_then(|zip| zip.extract(file_bytes)) {
                Ok(extracted) => bytes = extracted,
                Err(_) => {
                    // the C's fail path for reader-init/extract failure
                    // prints the same line (its Sys_FileClose already
                    // happened above)
                    // SAFETY: `file` is NUL-terminated
                    unsafe { print_load_failure(file) };
                    return;
                }
            }
        } else {
            if sz <= 0 {
                // SAFETY: `handle` is the open handle; `file` NUL-terminated
                unsafe {
                    quake_c_sys::Sys_FileClose(handle);
                    print_load_failure(file);
                }
                return;
            }
            // C: `Mem_Alloc (sz + 1)` (zeroed) then one Sys_FileRead of
            // `(int)sz` — same single read with the same int truncation; a
            // short read leaves the zeroed tail, which just ends the parse
            let mut buf = vec![0u8; sz as usize];
            // SAFETY: `handle` is open and `buf` is writable for `sz` bytes;
            // a negative (truncated) count is Sys_FileRead's to reject, as
            // it is in the C
            unsafe {
                quake_c_sys::Sys_FileRead(handle, buf.as_mut_ptr() as *mut c_void, sz as c_int);
                quake_c_sys::Sys_FileClose(handle);
            }
            bytes = buf;
        }
    }

    let data = match loc::parse(&bytes, &mut parse_warn) {
        Ok(data) => data,
        Err(LocError::IndexFull) => {
            // SAFETY: engine C API; static NUL-terminated string (no
            // conversion specifiers)
            unsafe { quake_c_sys::Sys_Error(c"LOC_LoadFile failed".as_ptr()) };
        }
    };

    let numentries = data.entries().len();
    // the text buffer lives in the static from here on; entry offsets keep
    // indexing its heap allocation, which is never grown again — so the
    // pointers LOC_GetRawString derives from it stay put
    // SAFETY: main-thread-only static access per the contract above
    unsafe { *addr_of_mut!(LOCALIZATION) = Some(data) };

    if numentries == 0 {
        // C: numindices stays 0 but the text remains owned (as here) and
        // every lookup misses
        // SAFETY: engine C API; `file` is NUL-terminated
        unsafe {
            quake_c_sys::Con_Printf(c"No localized strings in file '%s'\n".as_ptr(), file);
        }
        return;
    }

    // SAFETY: engine C API; `file` is NUL-terminated
    unsafe {
        quake_c_sys::Con_Printf(
            c"Loaded %d strings from '%s'\n".as_ptr(),
            numentries as c_int,
            file,
        );
    }
}

/// C: `void LOC_Init (void);`
///
/// # Safety
/// Engine single-threaded init path (after COM_InitFilesystem has set
/// com_basedir), like the C original.
#[no_mangle]
pub unsafe extern "C" fn LOC_Init() {
    // SAFETY: init-path contract forwarded; the literal is NUL-terminated
    unsafe { loc_load_file(c"localization/loc_english.txt".as_ptr()) };
}

/// C: `void LOC_Shutdown (void);` — drops the loaded data; every pointer
/// LOC_GetRawString/LOC_GetString handed out of the text buffer dies here,
/// exactly like the C's `Mem_Free (localization.text)`.
///
/// # Safety
/// Engine single-threaded shutdown path; no LOC_GetRawString result may be
/// dereferenced afterwards.
#[no_mangle]
pub unsafe extern "C" fn LOC_Shutdown() {
    // SAFETY: main-thread-only static access per the contract above
    unsafe { *addr_of_mut!(LOCALIZATION) = None };
}

/// C: `const char *LOC_GetRawString (const char *key);` — the localized
/// string for a `$`-prefixed key, or NULL when nothing is loaded, the key is
/// NULL/empty/unprefixed, or no entry matches. The returned pointer aims
/// into the owned text buffer and stays valid until LOC_Shutdown or a reload
/// drops it — the same lifetime the C pointer had until
/// `Mem_Free (localization.text)`.
///
/// # Safety
/// `key` must be NULL or a NUL-terminated string; main thread (the C global
/// had no locking either).
#[no_mangle]
pub unsafe extern "C" fn LOC_GetRawString(key: *const c_char) -> *const c_char {
    // SAFETY: main thread per the contract above; the borrow ends before any
    // reload can drop the pointee
    let Some(data) = (unsafe { loc_data() }) else {
        return core::ptr::null();
    };
    // C: `if (!localization.numindices || !key || ...)` — no entries means an
    // empty index, and both checks come before any read through `key`
    if data.entries().is_empty() || key.is_null() {
        return core::ptr::null();
    }
    // SAFETY: `key` is non-NULL, so NUL-terminated per the contract above
    let key = unsafe { CStr::from_ptr(key) }.to_bytes();
    match data.get_raw(key) {
        Some(offset) => data.text()[offset..].as_ptr() as *const c_char,
        None => core::ptr::null(),
    }
}

/// C: `const char *LOC_GetString (const char *key);` — the localized string,
/// or the input `key` pointer itself (identity preserved) when not found.
///
/// # Safety
/// As [`LOC_GetRawString`].
#[no_mangle]
pub unsafe extern "C" fn LOC_GetString(key: *const c_char) -> *const c_char {
    // SAFETY: caller contract forwarded
    let value = unsafe { LOC_GetRawString(key) };
    if value.is_null() {
        key
    } else {
        value
    }
}

/// C: `qboolean LOC_HasPlaceholders (const char *str);`
///
/// # Safety
/// `str_` must be a NUL-terminated string (the C reads it unconditionally
/// once anything is loaded); main thread.
#[no_mangle]
pub unsafe extern "C" fn LOC_HasPlaceholders(str_: *const c_char) -> qboolean {
    // SAFETY: main thread per the contract above; the borrow ends inside
    // this call
    let Some(data) = (unsafe { loc_data() }) else {
        return false;
    };
    // C: `if (!localization.numindices) return false` — before reading `str`
    if data.entries().is_empty() {
        return false;
    }
    // SAFETY: `str_` is NUL-terminated per the contract above
    let s = unsafe { CStr::from_ptr(str_) }.to_bytes();
    loc::has_placeholders(s)
}

/// C: `size_t LOC_Format (const char *format, const char *(*getarg_fn) (int idx, void *userdata), void *userdata, char *out, size_t len);`
/// — replaces `{}`/`{N}` placeholders with the callback's arguments; returns
/// the written count excluding the NUL; when `len > 0` the output is always
/// NUL-terminated.
///
/// # Safety
/// `format` must be a NUL-terminated string; `out` must be writable for
/// `len` bytes and not overlap anything `getarg_fn` returns (the C memcpy
/// has the same no-overlap requirement); when `format` contains
/// placeholders, `getarg_fn` must be non-NULL and return a NUL-terminated
/// string for every index the format names (the C strlen's the result
/// unconditionally).
#[no_mangle]
pub unsafe extern "C" fn LOC_Format(
    format: *const c_char,
    getarg_fn: Option<unsafe extern "C" fn(idx: c_int, userdata: *mut c_void) -> *const c_char>,
    userdata: *mut c_void,
    out: *mut c_char,
    len: usize,
) -> usize {
    // the pure format() has this arm too, but reaching it there would mean
    // materializing the output slice first; handle it before touching `out`
    if len == 0 {
        // SAFETY: engine C API; static NUL-terminated string
        unsafe { quake_c_sys::Con_DPrintf(c"LOC_Format: no output space\n".as_ptr()) };
        return 0;
    }

    // SAFETY: `format` is NUL-terminated per the contract above
    let fmt = unsafe { CStr::from_ptr(format) }.to_bytes();
    // SAFETY: `out` is writable for `len > 0` bytes per the contract above
    let out = unsafe { core::slice::from_raw_parts_mut(out as *mut u8, len) };

    let mut getarg = |idx: c_int| -> &'static [u8] {
        // a NULL getarg_fn with placeholders present crashes the C in
        // strlen; aborting with a message is this port's equivalent
        let f = getarg_fn.expect("LOC_Format: NULL getarg_fn");
        // SAFETY: caller contract above — getarg_fn returns a NUL-terminated
        // string for this index
        unsafe { CStr::from_ptr(f(idx, userdata)).to_bytes() }
    };

    let mut warn = |w: FormatWarning| {
        // SAFETY: engine C API; static NUL-terminated formats, int arg
        unsafe {
            match w {
                FormatWarning::NoOutputSpace => {
                    quake_c_sys::Con_DPrintf(c"LOC_Format: no output space\n".as_ptr());
                }
                FormatWarning::OverflowAtArgument { numargs } => {
                    quake_c_sys::Con_DPrintf(
                        c"LOC_Format: overflow at argument #%d\n".as_ptr(),
                        numargs,
                    );
                }
                FormatWarning::Overflow => {
                    quake_c_sys::Con_DPrintf(c"LOC_Format: overflow\n".as_ptr());
                }
            }
        }
    };

    loc::format(fmt, &mut getarg, out, &mut warn)
}
