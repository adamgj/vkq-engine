//! C ABI shims for `Quake/cfgfile.c` (declarations stay in `Quake/cfgfile.h`).
//!
//! The single static config-file handle survives as a Rust static (the C had
//! `static fshandle_t *cfg_file`); these run on the single-threaded early
//! boot path, before video/cvar registration, exactly like the C.

use core::ffi::{c_char, c_int, CStr};
use quake_util::cfgfile::process_line;

const CONFIG_NAME: &CStr = c"vkQuake.cfg";

static mut CFG_FILE: *mut quake_c_sys::fshandle_t = core::ptr::null_mut();

/// ASCII-only case-insensitive equality (C `q_strcasecmp` == 0).
fn ascii_eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Borrows the cvar-name list as byte slices, stopping at the first NULL entry
/// like the C scan loop does.
///
/// This is the only place the `vars` array is dereferenced, so the "are these
/// really `num_vars` readable, NUL-terminated pointers?" argument is made once
/// (ADR-004) rather than at each use.
///
/// # Safety
/// `vars` must point to `num_vars` readable pointers, each NULL or
/// NUL-terminated; the strings must outlive the returned slices.
unsafe fn collect_var_bytes<'a>(vars: *mut *const c_char, num_vars: c_int) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    for i in 0..num_vars.max(0) as usize {
        // SAFETY: caller contract above
        let p = unsafe { *vars.add(i) };
        if p.is_null() {
            break;
        }
        // SAFETY: non-NULL entries are NUL-terminated per the caller contract
        out.push(unsafe { CStr::from_ptr(p) }.to_bytes());
    }
    out
}

/// C: `int CFG_OpenConfig (const char *cfg_name);` — 0 on success, -1 on fail.
///
/// # Safety
/// `cfg_name` must be a NUL-terminated string; single-threaded boot path.
#[no_mangle]
pub unsafe extern "C" fn CFG_OpenConfig(cfg_name: *const c_char) -> c_int {
    // SAFETY: CFG_FILE is only touched on the boot thread
    unsafe { CFG_CloseConfig() };

    // SAFETY: NUL-terminated per the cfgfile.h contract
    let name = unsafe { CStr::from_ptr(cfg_name) };

    let mut f: *mut quake_c_sys::FILE = core::ptr::null_mut();
    let mut pak = false;

    if ascii_eq_ignore_case(name.to_bytes(), CONFIG_NAME.to_bytes()) {
        // SAFETY: engine C API; both arguments are static NUL-terminated strings
        f = unsafe { quake_c_sys::COM_FOpenPrefFile(CONFIG_NAME.as_ptr(), c"rb".as_ptr()) };
        if f.is_null() {
            return -1;
        }
    }

    let length = if !f.is_null() {
        // SAFETY: f is the open handle COM_FOpenPrefFile just returned
        unsafe { quake_c_sys::Sys_filelength(f) }
    } else {
        // SAFETY: engine C API; cfg_name is NUL-terminated and `f` is a
        // writable out-parameter. COM_ThreadFileFromPak reports on the file
        // COM_FOpenFile just opened, so the order matters.
        let (length, from_pak) = unsafe {
            let length = quake_c_sys::COM_FOpenFile(cfg_name, &mut f, core::ptr::null_mut());
            (length, quake_c_sys::COM_ThreadFileFromPak() != 0)
        };
        pak = from_pak;
        if length == -1 {
            return -1;
        }
        length
    };

    // SAFETY: the block is sized for an fshandle_t and outlives this call
    // (ADR-013: it crosses the FFI boundary and is released by Mem_Free in
    // CFG_CloseConfig); f is open, so Sys_ftell accepts it; CFG_FILE is only
    // touched on the boot thread
    unsafe {
        let fh = quake_c_sys::Mem_Alloc(core::mem::size_of::<quake_c_sys::fshandle_t>())
            as *mut quake_c_sys::fshandle_t;
        (*fh).file = f;
        (*fh).start = quake_c_sys::Sys_ftell(f);
        (*fh).pos = 0;
        (*fh).length = length;
        (*fh).pak = pak;
        CFG_FILE = fh;
    }

    0
}

/// C: `void CFG_CloseConfig (void);`
///
/// # Safety
/// Single-threaded boot path.
#[no_mangle]
pub unsafe extern "C" fn CFG_CloseConfig() {
    // SAFETY: CFG_FILE is only touched on the boot thread
    let fh = unsafe { CFG_FILE };
    if fh.is_null() {
        return;
    }
    // SAFETY: the handle is open and came from CFG_OpenConfig's Mem_Alloc
    unsafe {
        quake_c_sys::FS_fclose(fh);
        quake_c_sys::Mem_Free(fh as *const core::ffi::c_void);
        CFG_FILE = core::ptr::null_mut();
    }
}

/// C: `void CFG_ReadCvars (const char **vars, int num_vars);`
///
/// # Safety
/// `vars` must point to `num_vars` NUL-terminated strings (NULL entries stop
/// the scan, like the C); single-threaded boot path.
#[no_mangle]
pub unsafe extern "C" fn CFG_ReadCvars(vars: *mut *const c_char, num_vars: c_int) {
    // SAFETY: CFG_FILE is only touched on the boot thread
    let cfg_file = unsafe { CFG_FILE };
    if cfg_file.is_null() || num_vars < 1 {
        return;
    }

    // C: `for (i = 0; i < num_vars && vars[i]; i++)` — the scan list ends at
    // the first NULL entry
    // SAFETY: caller contract above; the strings outlive this call
    let var_bytes = unsafe { collect_var_bytes(vars, num_vars) };

    let mut j: c_int = 0;
    loop {
        let mut buff = [0u8; 1024];
        // SAFETY: buff is 1024 writable bytes; cfg_file is the open handle
        let read_line = unsafe {
            !quake_c_sys::FS_fgets(buff.as_mut_ptr() as *mut c_char, 1024, cfg_file).is_null()
        };
        if read_line {
            if let Some((vi, value)) = process_line(&buff, &var_bytes) {
                let mut value_z = value;
                value_z.push(0);
                // SAFETY: vi indexes a non-NULL entry (collect_var_bytes stops
                // at the first NULL); both strings are NUL-terminated
                unsafe { quake_c_sys::Cvar_Set(*vars.add(vi), value_z.as_ptr() as *const c_char) };
                j += 1;
            }
        }

        if j == num_vars {
            break;
        }
        // SAFETY: cfg_file is the open handle
        let exhausted =
            unsafe { quake_c_sys::FS_feof(cfg_file) != 0 || quake_c_sys::FS_ferror(cfg_file) != 0 };
        if exhausted {
            break;
        }
    }

    // SAFETY: cfg_file is the open handle
    unsafe { quake_c_sys::FS_rewind(cfg_file) };
}

/// C: `void CFG_ReadCvarOverrides (const char **vars, int num_vars);` —
/// reads `+cvar value` command-line overrides; touches no file.
///
/// # Safety
/// As CFG_ReadCvars (but every entry up to `num_vars` is dereferenced).
#[no_mangle]
pub unsafe extern "C" fn CFG_ReadCvarOverrides(vars: *mut *const c_char, num_vars: c_int) {
    if num_vars < 1 {
        return;
    }
    for i in 0..num_vars as usize {
        // SAFETY: caller contract above — every entry up to num_vars is a
        // readable, NUL-terminated pointer here
        let (var, var_bytes) = unsafe {
            let var = *vars.add(i);
            (var, CStr::from_ptr(var).to_bytes())
        };

        // C: char buff[64]; buff[0] = '+'; q_strlcpy(&buff[1], var, 63)
        let mut buff = [0u8; 64];
        buff[0] = b'+';
        quake_util::strl::strlcpy(&mut buff[1..], var_bytes);

        // SAFETY: strlcpy always terminates within the 63-byte tail, so buff
        // is a NUL-terminated string; com_argc is an engine global
        let (j, argc) = unsafe {
            (
                quake_c_sys::COM_CheckParm(buff.as_ptr() as *const c_char),
                core::ptr::addr_of!(quake_c_sys::com_argc).read(),
            )
        };
        if j != 0 && j < argc - 1 {
            // SAFETY: j + 1 < argc, so com_argv[j + 1] is one of the engine's
            // own NUL-terminated argument strings (never empty, so the first
            // byte is readable)
            let next = unsafe {
                let argv = core::ptr::addr_of!(quake_c_sys::com_argv).read();
                *argv.add(j as usize + 1)
            };
            // SAFETY: as above
            let first = unsafe { *next } as u8;
            if first != b'-' && first != b'+' {
                // SAFETY: both strings are NUL-terminated
                unsafe { quake_c_sys::Cvar_Set(var, next) };
            }
        }
    }
}
