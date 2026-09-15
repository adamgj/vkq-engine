//! `Quake/sys_sdl.c:37-331`: the engine file-handle table. Same index and
//! free semantics as the C (`0` is never handed out, `allocHandle` scans
//! from `1` under the mutex), same `FILE *` I/O through the C runtime so a
//! `Sys_DuplicateHandle` reopens the recorded path and shares the stdio
//! layer with the `FS_f*` shims.

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;

use quake_c_sys::sys as c;
use quake_c_sys::{qfileofs_t, qfilesize_t, qmutex_t, FILE};

use super::{errno, fopen, fseek, ftell, sys_error};

/// `#define MAX_HANDLES 32 /* johnfitz -- was 10 */`
const MAX_HANDLES: usize = 32;
/// `tasks.h` -- `#define TASKS_MAX_WORKERS 32`.
const TASKS_MAX_WORKERS: usize = 32;
const NUM_HANDLES: usize = MAX_HANDLES * (TASKS_MAX_WORKERS + 1);

/// `stdio.h` -- `EOF`.
const EOF: c_int = -1;
pub(super) const SEEK_SET: c_int = 0;
pub(super) const SEEK_END: c_int = 2;

/// `file_handle_t`.
#[derive(Clone, Copy)]
struct FileHandle {
    free: bool,
    file_path: *mut c_char,
    file: *mut FILE,
    memory: *const u8,
    pos: qfileofs_t,
    size: qfilesize_t,
    eof_condition: bool,
}

const EMPTY: FileHandle = FileHandle {
    free: false,
    file_path: ptr::null_mut(),
    file: ptr::null_mut(),
    memory: ptr::null(),
    pos: 0,
    size: 0,
    eof_condition: false,
};

/// `static qmutex_t *sys_handles_mutex;` -- protects the table when
/// requesting a new handle / freeing a handle.
static mut SYS_HANDLES_MUTEX: *mut qmutex_t = ptr::null_mut();
/// `static file_handle_t sys_handles[MAX_HANDLES * (TASKS_MAX_WORKERS + 1)];`
static mut SYS_HANDLES: [FileHandle; NUM_HANDLES] = [EMPTY; NUM_HANDLES];

/// `sys_handles[handle]`.
///
/// # Safety
/// `handle` is a live index the caller owns; worker threads only ever touch
/// their own handles, as in C.
#[inline]
unsafe fn entry(handle: c_int) -> *mut FileHandle {
    // SAFETY: caller contract; the index is bounds-checked against the
    // array where C's `sys_handles[handle]` is not.
    unsafe { ptr::addr_of_mut!(SYS_HANDLES[handle as usize]) }
}

/// C: `void Sys_FileInit (void)`
///
/// # Safety
/// Once, from `Sys_Init`, before any handle is used.
pub unsafe fn file_init() {
    // SAFETY: caller contract -- single-threaded init.
    unsafe {
        SYS_HANDLES_MUTEX = c::QMutex_Create();
        let table = ptr::addr_of_mut!(SYS_HANDLES);
        (*table).fill(EMPTY);
        for h in (*table).iter_mut().skip(1) {
            h.free = true;
        }
    }
}

/// `static int allocHandle (void)`
unsafe fn alloc_handle() -> c_int {
    // SAFETY: the mutex serialises the scan; the returned slot is owned by
    // the caller from then on.
    unsafe {
        c::QMutex_Lock(SYS_HANDLES_MUTEX);
        // TBC : why skipping index 0 ? is it to make
        // 0 as an invalid handle value by design ?
        for i in 1..NUM_HANDLES {
            let h = ptr::addr_of_mut!(SYS_HANDLES).cast::<FileHandle>().add(i);
            if (*h).free {
                // reset all fields
                *h = EMPTY;
                c::QMutex_Unlock(SYS_HANDLES_MUTEX);
                return i as c_int;
            }
        }
        c::QMutex_Unlock(SYS_HANDLES_MUTEX);
        sys_error("out of handles");
    }
}

/// `static void freeHandle (int handle)`
unsafe fn free_handle(handle: c_int) {
    // SAFETY: the mutex serialises the write.
    unsafe {
        c::QMutex_Lock(SYS_HANDLES_MUTEX);
        (*entry(handle)).free = true;
        c::QMutex_Unlock(SYS_HANDLES_MUTEX);
    }
}

/// `Mem_Alloc (strlen (path) + 1)` + `q_strlcpy`.
unsafe fn dup_path(path: *const c_char) -> *mut c_char {
    // SAFETY: `path` is NUL-terminated (caller contract).
    unsafe {
        let len = CStr::from_ptr(path).to_bytes().len() + 1;
        let copy = c::Mem_Alloc(len).cast::<c_char>();
        c::q_strlcpy(copy, path, len);
        copy
    }
}

/// C: `qfilesize_t Sys_filelength (FILE *f)`
///
/// # Safety
/// `f` is an open stream.
pub unsafe fn filelength(f: *mut FILE) -> qfilesize_t {
    // SAFETY: caller contract.
    unsafe {
        let pos = ftell(f);
        fseek(f, 0, SEEK_END);
        let end = ftell(f);
        fseek(f, pos, SEEK_SET);
        end
    }
}

/// C: `qfilesize_t Sys_FileOpenRead (const char *path, int *hndl)`
///
/// # Safety
/// `path` is NUL-terminated; `hndl` is writable.
pub unsafe fn file_open_read(path: *const c_char, hndl: *mut c_int) -> qfilesize_t {
    // SAFETY: caller contract; the slot comes from alloc_handle.
    unsafe {
        let i = alloc_handle();
        let f = fopen(path, c"rb".as_ptr());
        if f.is_null() {
            free_handle(i);
            *hndl = -1;
            return -1;
        }
        let h = entry(i);
        (*h).memory = ptr::null();
        (*h).file_path = dup_path(path);
        (*h).file = f;
        (*h).pos = 0;
        (*h).eof_condition = false;
        *hndl = i;
        let retval = filelength(f);
        (*h).size = retval;
        retval
    }
}

/// C: `void Sys_MemFileOpenRead (const byte *memory, qfilesize_t size, int *hndl)`
///
/// # Safety
/// `memory` stays readable for `size` bytes while the handle is open;
/// `hndl` is writable.
pub unsafe fn mem_file_open_read(memory: *const u8, size: qfilesize_t, hndl: *mut c_int) {
    // SAFETY: caller contract; the slot comes from alloc_handle.
    unsafe {
        let i = alloc_handle();
        let h = entry(i);
        (*h).file = ptr::null_mut();
        (*h).file_path = ptr::null_mut();
        (*h).memory = memory;
        (*h).size = size;
        // position to start reading from :
        (*h).pos = 0;
        (*h).eof_condition = false;
        *hndl = i;
    }
}

/// C: `int Sys_DuplicateHandle (int handle)`
///
/// # Safety
/// `handle` is open.
pub unsafe fn duplicate_handle(handle: c_int) -> c_int {
    // SAFETY: caller contract.
    unsafe {
        let src = entry(handle);
        let mut new_file: *mut FILE = ptr::null_mut();
        if !(*src).file.is_null() {
            new_file = fopen((*src).file_path, c"rb".as_ptr());
            if new_file.is_null() {
                return -1;
            }
        }

        let new_handle = alloc_handle();
        let dst = entry(new_handle);

        // duplicate all data
        *dst = *src;

        // replace with the new file
        if !(*src).file.is_null() {
            (*dst).file = new_file;
            // we want our own copy, will be freed in Sys_FileClose()
            (*dst).file_path = dup_path((*src).file_path);
        }

        // Re-Seek:
        file_seek(new_handle, (*src).pos);

        // Technically, the EOF condition of the original must be preserved,
        // even if duplicating a handle of an out-of bound file/memory pointer (sys_handles[handle].pos)
        // would be ludicrous in practice.

        new_handle
    }
}

/// C: `int Sys_FileOpenWrite (const char *path)`
///
/// # Safety
/// `path` is NUL-terminated.
pub unsafe fn file_open_write(path: *const c_char) -> c_int {
    // SAFETY: caller contract; the slot comes from alloc_handle.
    unsafe {
        let i = alloc_handle();
        let f = fopen(path, c"wb".as_ptr());
        if f.is_null() {
            let err = CStr::from_ptr(c::strerror(errno()))
                .to_string_lossy()
                .into_owned();
            sys_error(&format!(
                "Error opening {}: {}",
                CStr::from_ptr(path).to_string_lossy(),
                err
            ));
        }
        let h = entry(i);
        (*h).file = f;
        (*h).file_path = dup_path(path);
        (*h).size = filelength(f);
        // position to start writing from :
        (*h).pos = (*h).size;
        (*h).eof_condition = false;
        (*h).memory = ptr::null();
        i
    }
}

/// C: `qfileofs_t Sys_FilePos (int handle)`
///
/// # Safety
/// `handle` is open.
pub unsafe fn file_pos(handle: c_int) -> qfileofs_t {
    // SAFETY: caller contract.
    unsafe { (*entry(handle)).pos }
}

/// C: `void Sys_FileClose (int handle)`
///
/// # Safety
/// `handle` is open.
pub unsafe fn file_close(handle: c_int) {
    // SAFETY: caller contract.
    unsafe {
        let h = entry(handle);
        if !(*h).file.is_null() {
            c::fclose((*h).file);
            c::Mem_Free((*h).file_path.cast());
        }
        free_handle(handle);
    }
}

/// C: `int Sys_FileSeek (int handle, qfileofs_t position)`
///
/// # Safety
/// `handle` is open.
pub unsafe fn file_seek(handle: c_int, position: qfileofs_t) -> c_int {
    // like fseek(), going beyond the actual file
    // without error is expected. Attempting to read afterwards however will trigger
    // an EOF condition.
    // SAFETY: caller contract.
    unsafe {
        let h = entry(handle);
        if position >= 0 {
            if !(*h).file.is_null() {
                fseek((*h).file, position, SEEK_SET);
            }
            (*h).pos = position;
            return 0;
        }
    }
    1
}

/// C: `bool Sys_feof (int handle)`
///
/// # Safety
/// `handle` is open.
pub unsafe fn feof(handle: c_int) -> bool {
    // SAFETY: caller contract.
    unsafe { (*entry(handle)).eof_condition }
}

/// C: `int Sys_FileRead (int handle, void *dest, int count)`
///
/// # Safety
/// `handle` is open; `dest` is writable for `count` bytes.
pub unsafe fn file_read(handle: c_int, dest: *mut c_void, count: c_int) -> c_int {
    if count <= 0 {
        return 0;
    }
    // SAFETY: caller contract.
    unsafe {
        let h = entry(handle);

        // test EOF condition
        (*h).eof_condition = (*h).eof_condition || ((*h).size - (*h).pos) <= 0;

        if (*h).eof_condition {
            return 0;
        }

        let mut computed_read_count = qfilesize_t::from(count).min((*h).size - (*h).pos);
        computed_read_count = computed_read_count.max(0);

        if !(*h).file.is_null() {
            // file-based:
            let fread_count = c::fread(dest, 1, count as usize, (*h).file) as qfilesize_t;

            (*h).pos += fread_count;
            // if partial read, triggers EOF
            (*h).eof_condition = c::feof((*h).file) != 0;

            fread_count as c_int
        } else {
            // memory-based:
            ptr::copy_nonoverlapping(
                (*h).memory.offset((*h).pos as isize),
                dest.cast::<u8>(),
                computed_read_count as usize,
            );

            (*h).pos += computed_read_count;

            // if partial read, triggers EOF
            (*h).eof_condition = computed_read_count < qfilesize_t::from(count);

            computed_read_count as c_int
        }
    }
}

/// C: `int Sys_fgetc (int handle)`
///
/// # Safety
/// `handle` is open.
pub unsafe fn fgetc(handle: c_int) -> c_int {
    // SAFETY: caller contract.
    unsafe {
        if (*entry(handle)).eof_condition {
            return EOF;
        }

        // C reads the byte into the low end of a zeroed `int`.
        let mut next_byte_read: u8 = 0;
        if file_read(handle, ptr::addr_of_mut!(next_byte_read).cast(), 1) != 1 {
            return EOF;
        }
        c_int::from(next_byte_read)
    }
}

/// C: `int Sys_FileWrite (int handle, const void *data, int count)`
///
/// # Safety
/// `handle` is an open write handle; `data` is readable for `count` bytes.
pub unsafe fn file_write(handle: c_int, data: *const c_void, count: c_int) -> c_int {
    // SAFETY: caller contract.
    unsafe {
        let h = entry(handle);

        let effective_nb_write = c::fwrite(data, 1, count as usize, (*h).file) as c_int;

        (*h).pos += qfileofs_t::from(effective_nb_write);
        (*h).size += qfilesize_t::from(effective_nb_write);

        effective_nb_write
    }
}
