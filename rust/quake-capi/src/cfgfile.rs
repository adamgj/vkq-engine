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

/// C: `int CFG_OpenConfig (const char *cfg_name);` — 0 on success, -1 on fail.
///
/// # Safety
/// `cfg_name` must be a NUL-terminated string; single-threaded boot path.
#[no_mangle]
pub unsafe extern "C" fn CFG_OpenConfig(cfg_name: *const c_char) -> c_int {
    // SAFETY: engine C API calls; CFG_FILE is only touched on the boot thread
    unsafe {
        CFG_CloseConfig();

        let name = CStr::from_ptr(cfg_name);
        let mut f: *mut quake_c_sys::FILE = core::ptr::null_mut();
        let mut pak = false;

        if ascii_eq_ignore_case(name.to_bytes(), CONFIG_NAME.to_bytes()) {
            f = quake_c_sys::COM_FOpenPrefFile(CONFIG_NAME.as_ptr(), c"rb".as_ptr());
            if f.is_null() {
                return -1;
            }
        }

        let length = if !f.is_null() {
            quake_c_sys::Sys_filelength(f)
        } else {
            let length = quake_c_sys::COM_FOpenFile(cfg_name, &mut f, core::ptr::null_mut());
            pak = quake_c_sys::COM_ThreadFileFromPak() != 0;
            if length == -1 {
                return -1;
            }
            length
        };

        let fh = quake_c_sys::Mem_Alloc(core::mem::size_of::<quake_c_sys::fshandle_t>())
            as *mut quake_c_sys::fshandle_t;
        (*fh).file = f;
        (*fh).start = quake_c_sys::Sys_ftell(f);
        (*fh).pos = 0;
        (*fh).length = length;
        (*fh).pak = pak;
        CFG_FILE = fh;

        0
    }
}

/// C: `void CFG_CloseConfig (void);`
///
/// # Safety
/// Single-threaded boot path.
#[no_mangle]
pub unsafe extern "C" fn CFG_CloseConfig() {
    // SAFETY: CFG_FILE is only touched on the boot thread; handle came from
    // CFG_OpenConfig's Mem_Alloc
    unsafe {
        let fh = CFG_FILE;
        if !fh.is_null() {
            quake_c_sys::FS_fclose(fh);
            quake_c_sys::Mem_Free(fh as *const core::ffi::c_void);
            CFG_FILE = core::ptr::null_mut();
        }
    }
}

/// C: `void CFG_ReadCvars (const char **vars, int num_vars);`
///
/// # Safety
/// `vars` must point to `num_vars` NUL-terminated strings (NULL entries stop
/// the scan, like the C); single-threaded boot path.
#[no_mangle]
pub unsafe extern "C" fn CFG_ReadCvars(vars: *mut *const c_char, num_vars: c_int) {
    // SAFETY: caller contract above; the C API calls mirror cfgfile.c exactly
    unsafe {
        let cfg_file = CFG_FILE;
        if cfg_file.is_null() || num_vars < 1 {
            return;
        }

        // C: `for (i = 0; i < num_vars && vars[i]; i++)` — the scan list
        // ends at the first NULL entry
        let mut var_bytes: Vec<&[u8]> = Vec::new();
        for i in 0..num_vars as usize {
            let p = *vars.add(i);
            if p.is_null() {
                break;
            }
            var_bytes.push(CStr::from_ptr(p).to_bytes());
        }

        let mut j: c_int = 0;
        loop {
            let mut buff = [0u8; 1024];
            if !quake_c_sys::FS_fgets(buff.as_mut_ptr() as *mut c_char, 1024, cfg_file).is_null() {
                if let Some((vi, value)) = process_line(&buff, &var_bytes) {
                    let mut value_z = value;
                    value_z.push(0);
                    quake_c_sys::Cvar_Set(*vars.add(vi), value_z.as_ptr() as *const c_char);
                    j += 1;
                }
            }

            if j == num_vars {
                break;
            }
            if quake_c_sys::FS_feof(cfg_file) != 0 || quake_c_sys::FS_ferror(cfg_file) != 0 {
                break;
            }
        }

        quake_c_sys::FS_rewind(cfg_file);
    }
}

/// C: `void CFG_ReadCvarOverrides (const char **vars, int num_vars);` —
/// reads `+cvar value` command-line overrides; touches no file.
///
/// # Safety
/// As CFG_ReadCvars (but every entry up to `num_vars` is dereferenced).
#[no_mangle]
pub unsafe extern "C" fn CFG_ReadCvarOverrides(vars: *mut *const c_char, num_vars: c_int) {
    // SAFETY: caller contract above
    unsafe {
        if num_vars < 1 {
            return;
        }
        for i in 0..num_vars as usize {
            let var = *vars.add(i);
            // C: char buff[64]; buff[0] = '+'; q_strlcpy(&buff[1], var, 63)
            let mut buff = [0u8; 64];
            buff[0] = b'+';
            quake_util::strl::strlcpy(&mut buff[1..], CStr::from_ptr(var).to_bytes());
            let j = quake_c_sys::COM_CheckParm(buff.as_ptr() as *const c_char);
            let argc = core::ptr::addr_of!(quake_c_sys::com_argc).read();
            if j != 0 && j < argc - 1 {
                let argv = core::ptr::addr_of!(quake_c_sys::com_argv).read();
                let next = *argv.add(j as usize + 1);
                let first = *next as u8;
                if first != b'-' && first != b'+' {
                    quake_c_sys::Cvar_Set(var, next);
                }
            }
        }
    }
}
