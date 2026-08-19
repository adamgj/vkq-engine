//! C ABI shims for the pak-aware stdio replacements in `Quake/common_fs.c`
//! (declarations stay in `Quake/common.h`).
//!
//! The `fshandle_t` bookkeeping (start/length/pos) is what lets
//! non-sequential reads work on files reopened inside pak files. Every
//! function replicates its C original's control flow and errno behavior
//! exactly: the NULL-handle paths set `errno = EBADF`, reads and seeks clamp
//! to the pak section (`fh->start .. fh->start + fh->length`), and the
//! underlying stdio calls go through `quake_c_sys::stdio` /
//! `Sys_fseek`/`Sys_ftell` just like the C.

use core::ffi::{c_char, c_int, c_void};
use quake_c_sys::{fshandle_t, qfileofs_t, qfilesize_t, stdio};

/// Thread-local `errno` slot accessors.
///
/// These are C-standard-library (CRT) declarations, not engine symbols, so
/// they are hand-written here rather than bindgen-generated — mirroring
/// `quake_c_sys::libm`'s hand-written CRT-declaration pattern (ADR-011
/// amendment, Phase 1). Candidate for a later move into quake-c-sys next to
/// `libm`.
mod cerrno {
    use core::ffi::c_int;

    extern "C" {
        /// Apple libc: `int *__error (void);`
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        #[link_name = "__error"]
        pub fn errno_location() -> *mut c_int;

        /// MSVC CRT: `int *_errno (void);`
        #[cfg(target_os = "windows")]
        #[link_name = "_errno"]
        pub fn errno_location() -> *mut c_int;

        /// glibc/musl: `int *__errno_location (void);`
        #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
        #[link_name = "__errno_location"]
        pub fn errno_location() -> *mut c_int;
    }
}

/// C: `errno = value;`
fn set_errno(value: c_int) {
    // SAFETY: the CRT accessor returns a valid pointer to the calling
    // thread's errno slot
    unsafe { *cerrno::errno_location() = value };
}

// errno values, identical on the targeted CRTs (Apple libc, glibc/musl,
// MSVC UCRT)
const EBADF: c_int = 9;
const EFAULT: c_int = 14;
const EINVAL: c_int = 22;

// stdio constants, identical on the targeted CRTs
const EOF: c_int = -1;
const SEEK_SET: c_int = 0;
const SEEK_CUR: c_int = 1;
const SEEK_END: c_int = 2;

/// C: `size_t FS_fread (void *ptr, size_t size, size_t nmemb, fshandle_t *fh);`
///
/// Reads at most to the end of the pak section (`fh->length - fh->pos`); a
/// partially read trailing member counts as a whole one in the return value,
/// like the C.
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle; `ptr` must be NULL or
/// writable for `size * nmemb` bytes. Per-handle single-threaded use, like
/// the C.
#[no_mangle]
pub unsafe extern "C" fn FS_fread(
    ptr: *mut c_void,
    size: usize,
    nmemb: usize,
    fh: *mut fshandle_t,
) -> usize {
    if fh.is_null() {
        set_errno(EBADF);
        return 0;
    }
    if ptr.is_null() {
        set_errno(EFAULT);
        return 0;
    }
    if size == 0 || nmemb == 0 {
        /* no error, just zero bytes wanted */
        set_errno(0);
        return 0;
    }

    // C: `byte_size = nmemb * size;` — a size_t multiply (wrapping, defined
    // in C), then converted to qfilesize_t. common.h caps callers at 2**31
    // elements ("Only read 2**31 elements max"), so the product is in range
    // in practice.
    let mut byte_size = nmemb.wrapping_mul(size) as qfilesize_t;
    // SAFETY: fh is non-NULL per the check above and valid per the caller
    // contract
    let remaining = unsafe { (*fh).length - (*fh).pos };
    if byte_size > remaining {
        /* just read to end */
        byte_size = remaining;
    }
    // SAFETY: fh->file is the open stream and ptr is writable for the
    // requested bytes (byte_size <= nmemb * size) per the caller contract
    let bytes_read = unsafe { stdio::fread(ptr, 1, byte_size as usize, (*fh).file) } as qfilesize_t;
    // SAFETY: as above
    unsafe { (*fh).pos += bytes_read };

    /* fread() must return the number of elements read,
     * not the total number of bytes. */
    // C divides qfilesize_t by size_t: the usual arithmetic conversions do
    // the division and remainder in size_t
    let mut nmemb_read = (bytes_read as usize) / size;
    /* even if the last member is only read partially
     * it is counted as a whole in the return value. */
    if !(bytes_read as usize).is_multiple_of(size) {
        nmemb_read += 1;
    }
    nmemb_read
}

/// C: `int FS_fseek (fshandle_t *fh, qfileofs_t offset, int whence);`
///
/// The relative position is clamped to `0 ..= fh->length`; the real seek is
/// always `Sys_fseek (fh->file, fh->start + offset, SEEK_SET)`, like the C.
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle. Per-handle
/// single-threaded use, like the C.
#[no_mangle]
pub unsafe extern "C" fn FS_fseek(
    fh: *mut fshandle_t,
    mut offset: qfileofs_t,
    whence: c_int,
) -> c_int {
    if fh.is_null() {
        set_errno(EBADF);
        return -1;
    }

    // SAFETY: fh is non-NULL per the check above and valid per the caller
    // contract
    let (pos, length) = unsafe { ((*fh).pos, (*fh).length) };

    /* the relative file position shouldn't be smaller
     * than zero or bigger than the filesize. */
    match whence {
        SEEK_SET => {}
        SEEK_CUR => offset += pos,
        SEEK_END => offset += length,
        _ => {
            set_errno(EINVAL);
            return -1;
        }
    }

    if offset < 0 {
        set_errno(EINVAL);
        return -1;
    }

    if offset > length {
        /* just seek to end */
        offset = length;
    }

    // SAFETY: engine C API; fh->file is the open stream
    let ret = unsafe { quake_c_sys::Sys_fseek((*fh).file, (*fh).start + offset, SEEK_SET) };
    if ret < 0 {
        return ret;
    }

    // SAFETY: as above
    unsafe { (*fh).pos = offset };
    0
}

/// C: `int FS_fclose (fshandle_t *fh);`
///
/// Closes the underlying stream only; freeing the fshandle_t stays the
/// caller's job, like the C.
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle; the stream must not be
/// used again after this call.
#[no_mangle]
pub unsafe extern "C" fn FS_fclose(fh: *mut fshandle_t) -> c_int {
    if fh.is_null() {
        set_errno(EBADF);
        return -1;
    }
    // SAFETY: fh->file is the open stream, closed exactly once per the caller
    // contract
    unsafe { stdio::fclose((*fh).file) }
}

/// C: `qfileofs_t FS_ftell (fshandle_t *fh);`
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle.
#[no_mangle]
pub unsafe extern "C" fn FS_ftell(fh: *mut fshandle_t) -> qfileofs_t {
    if fh.is_null() {
        set_errno(EBADF);
        return -1;
    }
    // SAFETY: fh is non-NULL per the check above and valid per the caller
    // contract
    unsafe { (*fh).pos }
}

/// C: `void FS_rewind (fshandle_t *fh);`
///
/// NULL is a silent no-op (no errno), and the `Sys_fseek` result is ignored,
/// like the C.
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle. Per-handle
/// single-threaded use, like the C.
#[no_mangle]
pub unsafe extern "C" fn FS_rewind(fh: *mut fshandle_t) {
    if fh.is_null() {
        return;
    }
    // SAFETY: fh->file is the open stream; the seek result is deliberately
    // ignored, like the C
    unsafe {
        stdio::clearerr((*fh).file);
        quake_c_sys::Sys_fseek((*fh).file, (*fh).start, SEEK_SET);
        (*fh).pos = 0;
    }
}

/// C: `int FS_feof (fshandle_t *fh);`
///
/// Returns -1 at end of the section (not 1), 0 otherwise; end-of-data is
/// judged from the bookkeeping (`pos >= length`), never from `feof()`.
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle.
#[no_mangle]
pub unsafe extern "C" fn FS_feof(fh: *mut fshandle_t) -> c_int {
    if fh.is_null() {
        set_errno(EBADF);
        return -1;
    }
    // SAFETY: fh is non-NULL per the check above and valid per the caller
    // contract
    if unsafe { (*fh).pos >= (*fh).length } {
        return -1;
    }
    0
}

/// C: `int FS_ferror (fshandle_t *fh);`
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle.
#[no_mangle]
pub unsafe extern "C" fn FS_ferror(fh: *mut fshandle_t) -> c_int {
    if fh.is_null() {
        set_errno(EBADF);
        return -1;
    }
    // SAFETY: fh->file is the open stream
    unsafe { stdio::ferror((*fh).file) }
}

/// C: `int FS_fgetc (fshandle_t *fh);`
///
/// EOF at the section end comes from the bookkeeping; `pos` is advanced
/// before the `fgetc` call and regardless of its result, like the C.
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle. Per-handle
/// single-threaded use, like the C.
#[no_mangle]
pub unsafe extern "C" fn FS_fgetc(fh: *mut fshandle_t) -> c_int {
    if fh.is_null() {
        set_errno(EBADF);
        return EOF;
    }
    // SAFETY: fh is non-NULL per the check above and valid per the caller
    // contract
    if unsafe { (*fh).pos >= (*fh).length } {
        return EOF;
    }
    // SAFETY: as above; fh->file is the open stream
    unsafe {
        (*fh).pos += 1;
        stdio::fgetc((*fh).file)
    }
}

/// C: `char *FS_fgets (char *s, int size, fshandle_t *fh);`
///
/// `size` is clamped to the remaining section bytes plus the terminator, and
/// `pos` is resynced from `Sys_ftell` even when `fgets` returns NULL, like
/// the C. A NULL `fh` returns NULL with errno EBADF (via `FS_feof`).
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle; `s` must be writable
/// for `size` bytes. Per-handle single-threaded use, like the C.
#[no_mangle]
pub unsafe extern "C" fn FS_fgets(
    s: *mut c_char,
    mut size: c_int,
    fh: *mut fshandle_t,
) -> *mut c_char {
    // SAFETY: FS_feof accepts NULL (setting EBADF) and valid handles alike
    if unsafe { FS_feof(fh) } != 0 {
        return core::ptr::null_mut();
    }

    // SAFETY: FS_feof returned 0, so fh is non-NULL and valid
    let remaining = unsafe { (*fh).length - (*fh).pos };
    // C: `if (size > (fh->length - fh->pos) + 1)` — the int is promoted to
    // long long for the comparison; the assignment back is in int range
    // because it only happens when remaining + 1 < size
    if qfileofs_t::from(size) > remaining + 1 {
        size = (remaining + 1) as c_int;
    }

    // SAFETY: s is writable for `size` bytes per the caller contract and
    // fh->file is the open stream
    let ret = unsafe { stdio::fgets(s, size, (*fh).file) };
    // SAFETY: as above; engine C API. pos is resynced from the real file
    // position even when fgets returned NULL, like the C.
    unsafe { (*fh).pos = quake_c_sys::Sys_ftell((*fh).file) - (*fh).start };

    ret
}

/// C: `qfilesize_t FS_filelength (fshandle_t *fh);`
///
/// # Safety
/// `fh` must be NULL or point to a valid open handle.
#[no_mangle]
pub unsafe extern "C" fn FS_filelength(fh: *mut fshandle_t) -> qfilesize_t {
    if fh.is_null() {
        set_errno(EBADF);
        return -1;
    }
    // SAFETY: fh is non-NULL per the check above and valid per the caller
    // contract
    unsafe { (*fh).length }
}
