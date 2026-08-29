//! C ABI shims for `Quake/cvar.c` (Rust migration Phase 7 M2).
//!
//! Near-transliteration. The registry stays a linked list of C-layout
//! `cvar_t` nodes allocated from the engine allocator -- engine C files
//! declare `cvar_t` statics and read `.value`/`.string` directly -- so only
//! the logic moved. `cvar_vars` was `static` in C and is private here.
//!
//! ADR-009 audit: `var->callback(var)` and `PR_AutoCvarChanged` can
//! `Host_Error`, and so can the serverinfo/userinfo replication blocks
//! (`MSG_Write*` -> `SZ_GetSpace`). None of them are called directly; each
//! goes through a `CvarCmd_Glue_*` helper that wraps it in `Host_Guard`, and
//! the caught status is propagated to `Quake/cvar_cmd_glue.c`, which
//! re-issues the jump from a pure C frame. Every entry point that can reach
//! one of those is a `quake_rs_*` status core rather than a direct export.
//! `Mem_Alloc`/`Sys_Error` only abort the process, so they are called
//! directly (Phase 5/6 precedent).

use crate::cmd::{set_pending_raise, Cmd_Argc, Cmd_Argv, Cmd_Exists};
use core::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void, CStr};
use core::ptr;
use quake_c_sys as c;
use quake_c_sys::cvar_cmd as g;

const CVAR_ARCHIVE: c::cvarflags_t = c::cvarflags_t_CVAR_ARCHIVE;
const CVAR_NOTIFY: c::cvarflags_t = c::cvarflags_t_CVAR_NOTIFY;
const CVAR_SERVERINFO: c::cvarflags_t = c::cvarflags_t_CVAR_SERVERINFO;
const CVAR_USERINFO: c::cvarflags_t = c::cvarflags_t_CVAR_USERINFO;
const CVAR_CHANGED: c::cvarflags_t = c::cvarflags_t_CVAR_CHANGED;
const CVAR_ROM: c::cvarflags_t = c::cvarflags_t_CVAR_ROM;
const CVAR_LOCKED: c::cvarflags_t = c::cvarflags_t_CVAR_LOCKED;
const CVAR_REGISTERED: c::cvarflags_t = c::cvarflags_t_CVAR_REGISTERED;
const CVAR_CALLBACK: c::cvarflags_t = c::cvarflags_t_CVAR_CALLBACK;
const CVAR_USERDEFINED: c::cvarflags_t = c::cvarflags_t_CVAR_USERDEFINED;
const CVAR_AUTOCVAR: c::cvarflags_t = c::cvarflags_t_CVAR_AUTOCVAR;
const CVAR_SETA: c::cvarflags_t = c::cvarflags_t_CVAR_SETA;

/// `static cvar_t *cvar_vars;`
static mut CVAR_VARS: *mut c::cvar_t = ptr::null_mut();

/// `static char cvar_null_string[] = "";`
const CVAR_NULL_STRING: &CStr = c"";

/// Guard status carried back to the reraising C wrappers: 0 == no raise.
pub(crate) type Raise = c_int;

// ---------------------------------------------------------------------------
// small helpers

#[inline]
pub(crate) fn cstr(p: *const c_char) -> &'static [u8] {
    if p.is_null() {
        return b"";
    }
    // SAFETY: engine strings are NUL-terminated; callers pass C string pointers
    unsafe { CStr::from_ptr(p) }.to_bytes()
}

/// `Con_Printf`/`Con_Warning`/... with a pre-formatted Rust byte string. The
/// engine console entry points are variadic; the "%s" call-through is the
/// established capi pattern (net_main.rs).
pub(crate) fn con_print(f: unsafe extern "C" fn(*const c_char, ...), mut bytes: Vec<u8>) {
    bytes.push(0);
    // SAFETY: bytes is NUL-terminated and outlives the call
    unsafe { f(c"%s".as_ptr(), bytes.as_ptr()) };
}

#[inline]
fn atof(s: *const c_char) -> c_double {
    // COMPAT: libc atof semantics (leading whitespace, hex floats, inf/nan)
    // are observable in cvar values; call through instead of parsing in Rust.
    // SAFETY: s is a NUL-terminated C string
    unsafe { g::atof(s) }
}

// ---------------------------------------------------------------------------
//
//  USER COMMANDS
//
// ---------------------------------------------------------------------------

/// C: `void Cvar_List_f (void)`
extern "C" fn cvar_list_f() {
    let (partial, len): (*const c_char, usize) = if Cmd_Argc() > 1 {
        let p = Cmd_Argv(1);
        (p, cstr(p).len())
    } else {
        (ptr::null(), 0)
    };

    let mut count = 0;
    // SAFETY: single-threaded host; the registry is a well-formed C list
    unsafe {
        let mut cvar = CVAR_VARS;
        while !cvar.is_null() {
            if !partial.is_null() && strncmp(partial, (*cvar).name, len) != 0 {
                cvar = (*cvar).next;
                continue;
            }
            let mut line = Vec::new();
            line.extend_from_slice(if (*cvar).flags & CVAR_ARCHIVE != 0 {
                b"*"
            } else {
                b" "
            });
            line.extend_from_slice(if (*cvar).flags & CVAR_NOTIFY != 0 {
                b"s"
            } else {
                b" "
            });
            line.push(b' ');
            line.extend_from_slice(cstr((*cvar).name));
            line.extend_from_slice(b" \"");
            line.extend_from_slice(cstr((*cvar).string));
            line.extend_from_slice(b"\"\n");
            con_print(c::Con_SafePrintf, line);
            count += 1;
            cvar = (*cvar).next;
        }
    }

    con_print(c::Con_SafePrintf, format!("{count} cvars").into_bytes());
    if !partial.is_null() {
        let mut b = b" beginning with \"".to_vec();
        b.extend_from_slice(cstr(partial));
        b.push(b'"');
        con_print(c::Con_SafePrintf, b);
    }
    con_print(c::Con_SafePrintf, b"\n".to_vec());
}

/// C: `void Cvar_Inc_f (void)`
extern "C" fn cvar_inc_f() {
    match Cmd_Argc() {
        2 => {
            let raised = cvar_set_value_core(
                Cmd_Argv(1),
                Cvar_VariableValue(Cmd_Argv(1)) as c_float + 1.0,
            );
            set_pending_raise(raised);
        }
        3 => {
            let v = Cvar_VariableValue(Cmd_Argv(1)) + atof(Cmd_Argv(2));
            let raised = cvar_set_value_core(Cmd_Argv(1), v as c_float);
            set_pending_raise(raised);
        }
        // COMPAT: `default:` and `case 1:` share the usage message
        _ => con_print(
            c::Con_Printf,
            b"inc <cvar> [amount] : increment cvar\n".to_vec(),
        ),
    }
}

/// C: `void Cvar_Set_f (void)` -- both the `set`, `seta` and `setfl` commands
extern "C" fn cvar_set_f() {
    let varname = Cmd_Argv(1);
    let varvalue = Cmd_Argv(2);
    let mut fl: c::cvarflags_t = 0;

    if Cmd_Argc() < 3 {
        let mut b = cstr(Cmd_Argv(0)).to_vec();
        b.extend_from_slice(b" <cvar> <value>\n");
        con_print(c::Con_Printf, b);
        return;
    }

    if cstr(Cmd_Argv(0)) == b"setfl" && Cmd_Argc() == 4 {
        for &ch in cstr(Cmd_Argv(3)) {
            match ch {
                // COMPAT: 'a' forgets the other flags -- see cvar.c:133
                b'a' => fl |= CVAR_ARCHIVE | CVAR_SETA,
                b'u' => fl |= CVAR_USERINFO,
                b's' => fl |= CVAR_SERVERINFO,
                _ => {
                    let mut b = cstr(Cmd_Argv(0)).to_vec();
                    b.extend_from_slice(b" \"");
                    b.extend_from_slice(cstr(varname));
                    b.extend_from_slice(b"\" unknown cvar flag '");
                    b.push(ch);
                    b.extend_from_slice(b"'\n");
                    con_print(c::Con_Warning, b);
                    return;
                }
            }
        }
    } else if Cmd_Argc() > 3 {
        let mut b = cstr(Cmd_Argv(0)).to_vec();
        b.extend_from_slice(b" \"");
        b.extend_from_slice(cstr(varname));
        b.extend_from_slice(b"\" command with extra args\n");
        con_print(c::Con_Warning, b);
        return;
    }

    let (var, raised) = cvar_create_core(varname, varvalue);
    if raised != 0 {
        set_pending_raise(raised);
        return;
    }
    // COMPAT: cvar.c dereferences the Cvar_Create result unconditionally; it
    // returns NULL when the name is already a command. Preserved as a crash
    // in C, but a NULL write is not expressible here -- bail out instead.
    if var.is_null() {
        return;
    }
    // SAFETY: var is a live registry node
    unsafe { (*var).flags |= fl };
    let raised = cvar_set_quick_core(var, varvalue);
    if raised != 0 {
        set_pending_raise(raised);
        return;
    }

    if cstr(Cmd_Argv(0)) == b"seta" {
        // SAFETY: var is a live registry node
        unsafe { (*var).flags |= CVAR_ARCHIVE | CVAR_SETA };
    }
}

/// C: `void Cvar_Toggle_f (void)`
extern "C" fn cvar_toggle_f() {
    if Cmd_Argc() < 2 {
        con_print(
            c::Con_Printf,
            b"toggle <cvar> [value] [altvalue]: toggle cvar\n".to_vec(),
        );
        return;
    }
    let v = Cvar_FindVar(Cmd_Argv(1));
    if v.is_null() {
        let mut b = b"variable \"".to_vec();
        b.extend_from_slice(cstr(Cmd_Argv(1)));
        b.extend_from_slice(b"\" not found\n");
        con_print(c::Con_Printf, b);
        return;
    }

    // SAFETY: v is a live registry node
    let raised = unsafe {
        if Cmd_Argc() >= 3 {
            let newval = Cmd_Argv(2);
            let mut defval = if Cmd_Argc() > 3 {
                Cmd_Argv(3)
            } else {
                (*v).default_string
            };
            if defval.is_null() {
                defval = c"0".as_ptr();
            }
            if cstr(newval) == cstr((*v).string) {
                cvar_set_quick_core(v, defval)
            } else {
                cvar_set_quick_core(v, newval)
            }
        } else if (*v).value != 0.0 {
            cvar_set_quick_core(v, c"0".as_ptr())
        } else {
            cvar_set_quick_core(v, c"1".as_ptr())
        }
    };
    set_pending_raise(raised);
}

/// C: `void Cvar_Cycle_f (void)`
extern "C" fn cvar_cycle_f() {
    if Cmd_Argc() < 3 {
        con_print(
            c::Con_Printf,
            b"cycle <cvar> <value list>: cycle cvar through a list of values\n".to_vec(),
        );
        return;
    }

    // loop through the args until you find one that matches the current cvar value.
    let mut i = 2;
    while i < Cmd_Argc() {
        // COMPAT: zero is assumed to be a string (cvar.c:221-223)
        if atof(Cmd_Argv(i)) == 0.0 {
            if cstr(Cmd_Argv(i)) == cstr(Cvar_VariableString(Cmd_Argv(1))) {
                break;
            }
        } else if atof(Cmd_Argv(i)) == Cvar_VariableValue(Cmd_Argv(1)) {
            break;
        }
        i += 1;
    }

    let raised = if i == Cmd_Argc() || i + 1 == Cmd_Argc() {
        cvar_set_core(Cmd_Argv(1), Cmd_Argv(2))
    } else {
        cvar_set_core(Cmd_Argv(1), Cmd_Argv(i + 1))
    };
    set_pending_raise(raised);
}

/// C: `void Cvar_Reset_f (void)`
extern "C" fn cvar_reset_f() {
    match Cmd_Argc() {
        2 => {
            let raised = cvar_reset_core(Cmd_Argv(1));
            set_pending_raise(raised);
        }
        // COMPAT: `default:` falls into `case 1:`
        _ => con_print(
            c::Con_Printf,
            b"reset <cvar> : reset cvar to default\n".to_vec(),
        ),
    }
}

/// C: `void Cvar_ResetAll_f (void)`
extern "C" fn cvar_resetall_f() {
    // SAFETY: single-threaded host; the registry is a well-formed C list.
    // COMPAT: the walk reads var->next AFTER Cvar_Reset ran the callbacks,
    // exactly as the C for-loop does.
    unsafe {
        let mut var = CVAR_VARS;
        while !var.is_null() {
            let raised = cvar_reset_core((*var).name);
            if raised != 0 {
                set_pending_raise(raised);
                return;
            }
            var = (*var).next;
        }
    }
}

/// C: `void Cvar_ResetCfg_f (void)`
extern "C" fn cvar_resetcfg_f() {
    // SAFETY: single-threaded host; the registry is a well-formed C list
    unsafe {
        let mut var = CVAR_VARS;
        while !var.is_null() {
            if (*var).flags & CVAR_ARCHIVE != 0 {
                let raised = cvar_reset_core((*var).name);
                if raised != 0 {
                    set_pending_raise(raised);
                    return;
                }
            }
            var = (*var).next;
        }
    }
}

// ---------------------------------------------------------------------------
//
//  INIT
//
// ---------------------------------------------------------------------------

/// C: `void Cvar_Init (void)`
#[no_mangle]
pub extern "C" fn Cvar_Init() {
    let add = |name: &CStr, f: extern "C" fn()| {
        // C: Cmd_AddCommand (name, f) -- the macro's src_command/false
        crate::cmd::Cmd_AddCommand2(
            name.as_ptr(),
            Some(f as unsafe extern "C" fn()),
            c::cmd_source_t_src_command,
            false,
        );
    };
    add(c"cvarlist", cvar_list_f);
    add(c"toggle", cvar_toggle_f);
    add(c"cycle", cvar_cycle_f);
    add(c"inc", cvar_inc_f);
    add(c"reset", cvar_reset_f);
    add(c"resetall", cvar_resetall_f);
    add(c"resetcfg", cvar_resetcfg_f);
    add(c"set", cvar_set_f);
    add(c"seta", cvar_set_f);
}

// ---------------------------------------------------------------------------
//
//  CVAR FUNCTIONS
//
// ---------------------------------------------------------------------------

/// `int strcmp (a, b)` restricted to the equality test the C makes
#[inline]
fn streq(a: *const c_char, b: *const c_char) -> bool {
    cstr(a) == cstr(b)
}

/// `int strncmp (a, b, n)` -- returns 0 when the first `n` bytes match, using
/// C semantics (the comparison stops at a NUL in either operand).
fn strncmp(a: *const c_char, b: *const c_char, n: usize) -> c_int {
    let (a, b) = (cstr(a), cstr(b));
    for i in 0..n {
        let ca = a.get(i).copied().unwrap_or(0);
        let cb = b.get(i).copied().unwrap_or(0);
        if ca != cb {
            return ca as c_int - cb as c_int;
        }
        if ca == 0 {
            return 0;
        }
    }
    0
}

/// C `strcmp` sign, used for the alphabetical registry inserts
pub(crate) fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    let (a, b) = (cstr(a), cstr(b));
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return a[i] as c_int - b[i] as c_int;
        }
    }
    a.len() as c_int - b.len() as c_int
}

/// C: `cvar_t *Cvar_FindVar (const char *var_name)`
#[no_mangle]
pub extern "C" fn Cvar_FindVar(var_name: *const c_char) -> *mut c::cvar_t {
    // SAFETY: single-threaded host; the registry is a well-formed C list
    unsafe {
        let mut var = CVAR_VARS;
        while !var.is_null() {
            if streq(var_name, (*var).name) {
                return var;
            }
            var = (*var).next;
        }
    }
    ptr::null_mut()
}

/// C: `cvar_t *Cvar_FindVarAfter (const char *prev_name, unsigned int with_flags)`
#[no_mangle]
pub extern "C" fn Cvar_FindVarAfter(
    prev_name: *const c_char,
    with_flags: c_uint,
) -> *mut c::cvar_t {
    // SAFETY: single-threaded host; the registry is a well-formed C list
    unsafe {
        let mut var = if !cstr(prev_name).is_empty() {
            let v = Cvar_FindVar(prev_name);
            if v.is_null() {
                return ptr::null_mut();
            }
            (*v).next
        } else {
            CVAR_VARS
        };

        // search for the next cvar matching the needed flags
        while !var.is_null() {
            // COMPAT: with_flags == 0 matches everything (cvar.c:356)
            if ((*var).flags & with_flags) != 0 || with_flags == 0 {
                break;
            }
            var = (*var).next;
        }
        var
    }
}

/// C: `void Cvar_LockVar (const char *var_name)`
#[no_mangle]
pub extern "C" fn Cvar_LockVar(var_name: *const c_char) {
    let var = Cvar_FindVar(var_name);
    if !var.is_null() {
        // SAFETY: var is a live registry node
        unsafe { (*var).flags |= CVAR_LOCKED };
    }
}

/// C: `void Cvar_UnlockVar (const char *var_name)`
#[no_mangle]
pub extern "C" fn Cvar_UnlockVar(var_name: *const c_char) {
    let var = Cvar_FindVar(var_name);
    if !var.is_null() {
        // SAFETY: var is a live registry node
        unsafe { (*var).flags &= !CVAR_LOCKED };
    }
}

/// C: `void Cvar_UnlockAll (void)`
#[no_mangle]
pub extern "C" fn Cvar_UnlockAll() {
    // SAFETY: single-threaded host; the registry is a well-formed C list
    unsafe {
        let mut var = CVAR_VARS;
        while !var.is_null() {
            (*var).flags &= !CVAR_LOCKED;
            var = (*var).next;
        }
    }
}

/// C: `double Cvar_VariableValue (const char *var_name)`
#[no_mangle]
pub extern "C" fn Cvar_VariableValue(var_name: *const c_char) -> c_double {
    let var = Cvar_FindVar(var_name);
    if var.is_null() {
        return 0.0;
    }
    // SAFETY: var is a live registry node
    atof(unsafe { (*var).string })
}

/// C: `const char *Cvar_VariableString (const char *var_name)`
#[no_mangle]
pub extern "C" fn Cvar_VariableString(var_name: *const c_char) -> *const c_char {
    let var = Cvar_FindVar(var_name);
    if var.is_null() {
        return CVAR_NULL_STRING.as_ptr();
    }
    // SAFETY: var is a live registry node
    unsafe { (*var).string }
}

/// C: `const char *Cvar_CompleteVariable (const char *partial)`
#[no_mangle]
pub extern "C" fn Cvar_CompleteVariable(partial: *const c_char) -> *const c_char {
    let len = cstr(partial).len();
    if len == 0 {
        return ptr::null();
    }

    // SAFETY: single-threaded host; the registry is a well-formed C list
    unsafe {
        let mut cvar = CVAR_VARS;
        while !cvar.is_null() {
            // COMPAT: completion is case-SENSITIVE (strncmp), unlike dispatch
            if strncmp(partial, (*cvar).name, len) == 0 {
                return (*cvar).name;
            }
            cvar = (*cvar).next;
        }
    }
    ptr::null()
}

// ---------------------------------------------------------------------------
// status cores (ADR-009): every path below can reach a guarded C callback

/// `void Cvar_Reset (const char *name)`
pub(crate) fn cvar_reset_core(name: *const c_char) -> Raise {
    let var = Cvar_FindVar(name);
    if var.is_null() {
        let mut b = b"variable \"".to_vec();
        b.extend_from_slice(cstr(name));
        b.extend_from_slice(b"\" not found\n");
        con_print(c::Con_Printf, b);
        0
    } else {
        // SAFETY: var is a live registry node
        cvar_set_quick_core(var, unsafe { (*var).default_string })
    }
}

/// `void Cvar_SetQuick (cvar_t *var, const char *value)`
pub(crate) fn cvar_set_quick_core(var: *mut c::cvar_t, value: *const c_char) -> Raise {
    // SAFETY: var is a live registry node; value is a NUL-terminated string
    unsafe {
        if (*var).flags & (CVAR_ROM | CVAR_LOCKED) != 0 {
            return 0;
        }
        if (*var).flags & CVAR_REGISTERED == 0 {
            return 0;
        }

        if (*var).string.is_null() {
            (*var).string = g::q_strdup(value);
        } else {
            if streq((*var).string, value) {
                return 0; // no change
            }

            (*var).flags |= CVAR_CHANGED;
            let len = cstr(value).len();
            // COMPAT: the existing allocation is reused when the new value has
            // the same length; only a length change reallocates (cvar.c:480).
            if len != cstr((*var).string).len() {
                c::Mem_Free((*var).string as *const c_void);
                (*var).string = c::Mem_Alloc(len + 1) as *const c_char;
            }
            ptr::copy_nonoverlapping(value as *const u8, (*var).string as *mut u8, len + 1);
        }

        (*var).value = atof((*var).string) as c_float;

        // johnfitz -- save initial value for "reset" command
        if (*var).default_string.is_null() {
            (*var).default_string = g::q_strdup((*var).string);
        }
        // johnfitz -- during initialization, update default too
        else if !g::host_initialized {
            c::Mem_Free((*var).default_string as *const c_void);
            (*var).default_string = g::q_strdup((*var).string);
        }

        if let Some(cb) = (*var).callback {
            // ADR-009: the callback may Host_Error; after a caught raise the
            // rest of this function must not run.
            let raised = g::CvarCmd_Glue_CallCvarCallback(Some(cb), var);
            if raised != 0 {
                return raised;
            }
        }
        if (*var).flags & CVAR_AUTOCVAR != 0 {
            let raised = g::CvarCmd_Glue_AutoCvarChanged(var);
            if raised != 0 {
                return raised;
            }
        }

        if (*var).flags & CVAR_SERVERINFO != 0 {
            let raised = g::CvarCmd_Glue_ServerinfoChanged(var);
            if raised != 0 {
                return raised;
            }
        }
        if (*var).flags & CVAR_USERINFO != 0 {
            let raised = g::CvarCmd_Glue_UserinfoChanged(var);
            if raised != 0 {
                return raised;
            }
        }
        0
    }
}

/// The `char val[32]` float expansion shared by `Cvar_SetValueQuick` and
/// `Cvar_SetValue` (cvar.c:541-558 / cvar.c:584-601).
fn value_string(value: c_float) -> [u8; 32] {
    // COMPAT: ADR-005 -- %f goes through the engine printf formatter, and the
    // trailing-zero kill loop runs exactly as the C wrote it.
    quake_cvar::value_string(value)
}

/// `void Cvar_SetValueQuick (cvar_t *var, const float value)`
pub(crate) fn cvar_set_value_quick_core(var: *mut c::cvar_t, value: c_float) -> Raise {
    let val = value_string(value);
    cvar_set_quick_core(var, val.as_ptr() as *const c_char)
}

/// `void Cvar_Set (const char *var_name, const char *value)`
pub(crate) fn cvar_set_core(var_name: *const c_char, value: *const c_char) -> Raise {
    let var = Cvar_FindVar(var_name);
    if var.is_null() {
        // there is an error in C code if this happens
        let mut b = b"Cvar_Set: variable ".to_vec();
        b.extend_from_slice(cstr(var_name));
        b.extend_from_slice(b" not found\n");
        con_print(c::Con_Printf, b);
        return 0;
    }
    cvar_set_quick_core(var, value)
}

/// `void Cvar_SetValue (const char *var_name, const float value)`
pub(crate) fn cvar_set_value_core(var_name: *const c_char, value: c_float) -> Raise {
    let val = value_string(value);
    cvar_set_core(var_name, val.as_ptr() as *const c_char)
}

/// `void Cvar_SetROM (const char *var_name, const char *value)`
fn cvar_set_rom_core(var_name: *const c_char, value: *const c_char) -> Raise {
    let var = Cvar_FindVar(var_name);
    if var.is_null() {
        return 0;
    }
    // SAFETY: var is a live registry node
    unsafe { (*var).flags &= !CVAR_ROM };
    let raised = cvar_set_quick_core(var, value);
    if raised != 0 {
        // COMPAT: the C longjmp skips the flag restore too
        return raised;
    }
    // SAFETY: var is a live registry node
    unsafe { (*var).flags |= CVAR_ROM };
    0
}

/// `void Cvar_SetValueROM (const char *var_name, const float value)`
fn cvar_set_value_rom_core(var_name: *const c_char, value: c_float) -> Raise {
    let var = Cvar_FindVar(var_name);
    if var.is_null() {
        return 0;
    }
    // SAFETY: var is a live registry node
    unsafe { (*var).flags &= !CVAR_ROM };
    let raised = cvar_set_value_quick_core(var, value);
    if raised != 0 {
        return raised;
    }
    // SAFETY: var is a live registry node
    unsafe { (*var).flags |= CVAR_ROM };
    0
}

/// `void Cvar_RegisterVariable (cvar_t *variable)`
pub(crate) fn cvar_register_variable_core(variable: *mut c::cvar_t) -> Raise {
    // SAFETY: `variable` is a caller-owned cvar_t (usually a C static)
    unsafe {
        // first check to see if it has already been defined
        if !Cvar_FindVar((*variable).name).is_null() {
            let mut b = b"Can't register variable ".to_vec();
            b.extend_from_slice(cstr((*variable).name));
            b.extend_from_slice(b", already defined\n");
            con_print(c::Con_Printf, b);
            return 0;
        }

        // check for overlap with a command
        if Cmd_Exists((*variable).name) {
            let mut b = b"Cvar_RegisterVariable: ".to_vec();
            b.extend_from_slice(cstr((*variable).name));
            b.extend_from_slice(b" is a command\n");
            con_print(c::Con_Printf, b);
            return 0;
        }

        // johnfitz -- insert each entry in alphabetical order
        if CVAR_VARS.is_null() || strcmp((*variable).name, (*CVAR_VARS).name) < 0 {
            (*variable).next = CVAR_VARS;
            CVAR_VARS = variable;
        } else {
            let mut prev = CVAR_VARS;
            let mut cursor = (*CVAR_VARS).next;
            while !cursor.is_null() && strcmp((*variable).name, (*cursor).name) > 0 {
                prev = cursor;
                cursor = (*cursor).next;
            }
            (*variable).next = (*prev).next;
            (*prev).next = variable;
        }
        (*variable).flags |= CVAR_REGISTERED;

        // copy the value off, because future sets will Mem_Free it
        // COMPAT: silent truncation at 512 bytes (cvar.c:644/685)
        let mut value = [0u8; 512];
        quake_util::strl::strlcpy(&mut value, cstr((*variable).string));
        (*variable).string = ptr::null();
        (*variable).default_string = ptr::null();

        if (*variable).flags & CVAR_CALLBACK == 0 {
            (*variable).callback = None;
        }

        // set it through the function to be consistent
        let set_rom = (*variable).flags & CVAR_ROM;
        (*variable).flags &= !CVAR_ROM;
        let raised = cvar_set_quick_core(variable, value.as_ptr() as *const c_char);
        if raised != 0 {
            // COMPAT: the C longjmp skips the CVAR_ROM restore as well
            return raised;
        }
        if set_rom != 0 {
            (*variable).flags |= CVAR_ROM;
        }
        0
    }
}

/// `cvar_t *Cvar_Create (const char *name, const char *value)`
pub(crate) fn cvar_create_core(
    name: *const c_char,
    value: *const c_char,
) -> (*mut c::cvar_t, Raise) {
    let existing = Cvar_FindVar(name);
    if !existing.is_null() {
        return (existing, 0); // already exists.
    }
    if Cmd_Exists(name) {
        return (ptr::null_mut(), 0); // error! panic! oh noes!
    }

    // COMPAT: one allocation, with the name copied in-line after the struct
    // SAFETY: Mem_Alloc zeroes; the trailing bytes hold the NUL-terminated name
    unsafe {
        let name_bytes = cstr(name);
        let newvar = c::Mem_Alloc(core::mem::size_of::<c::cvar_t>() + name_bytes.len() + 1)
            as *mut c::cvar_t;
        let name_dst = newvar.add(1) as *mut u8;
        ptr::copy_nonoverlapping(name_bytes.as_ptr(), name_dst, name_bytes.len());
        *name_dst.add(name_bytes.len()) = 0;
        (*newvar).name = name_dst as *const c_char;
        (*newvar).flags = CVAR_USERDEFINED;

        (*newvar).string = value;
        let raised = cvar_register_variable_core(newvar);
        (newvar, raised)
    }
}

/// `qboolean Cvar_Command (void)`
pub(crate) fn cvar_command_core() -> (c::qboolean, Raise) {
    // check variables
    let v = Cvar_FindVar(Cmd_Argv(0));
    if v.is_null() {
        return (false, 0);
    }

    // perform a variable print or set
    if Cmd_Argc() == 1 {
        // SAFETY: v is a live registry node
        unsafe {
            let mut b = b"\"".to_vec();
            b.extend_from_slice(cstr((*v).name));
            b.extend_from_slice(b"\" is \"");
            b.extend_from_slice(cstr((*v).string));
            b.extend_from_slice(b"\"\n");
            con_print(c::Con_Printf, b);
        }
        return (true, 0);
    }

    // SAFETY: v is a live registry node
    let raised = cvar_set_core(unsafe { (*v).name }, Cmd_Argv(1));
    (true, raised)
}

// ---------------------------------------------------------------------------
// non-raising exports

/// C: `void Cvar_SetCallback (cvar_t *var, cvarcallback_t func)`
///
/// # Safety
/// `var` must point at a live `cvar_t`.
#[no_mangle]
pub unsafe extern "C" fn Cvar_SetCallback(var: *mut c::cvar_t, func: c::cvarcallback_t) {
    // SAFETY: var points at a live cvar_t per the cvar.h contract
    unsafe {
        (*var).callback = func;
        if func.is_some() {
            (*var).flags |= CVAR_CALLBACK;
        } else {
            (*var).flags &= !CVAR_CALLBACK;
        }
    }
}

/// C: `void Cvar_SetCompletion (cvar_t *var, cvarcompletion_t func)`
///
/// # Safety
/// `var` must point at a live `cvar_t`.
#[no_mangle]
pub unsafe extern "C" fn Cvar_SetCompletion(var: *mut c::cvar_t, func: c::cvarcompletion_t) {
    // SAFETY: var points at a live cvar_t per the cvar.h contract
    unsafe { (*var).completion = func };
}

/// C: `void Cvar_WriteVariables (FILE *f)`
///
/// # Safety
/// `f` must be an open, writable `FILE *`.
#[no_mangle]
pub unsafe extern "C" fn Cvar_WriteVariables(f: *mut c::FILE) {
    // SAFETY: f is an open FILE*; the registry is a well-formed C list.
    // COMPAT: fprintf is used rather than a Rust-side buffer so that a NULL
    // var->string behaves exactly like the C (config.cfg is a byte gate).
    unsafe {
        let mut var = CVAR_VARS;
        while !var.is_null() {
            if (*var).flags & CVAR_ARCHIVE != 0 {
                if (*var).flags & (CVAR_USERDEFINED | CVAR_SETA) != 0 {
                    g::fprintf(f, c"seta ".as_ptr());
                }
                g::fprintf(f, c"%s \"%s\"\n".as_ptr(), (*var).name, (*var).string);
            }
            var = (*var).next;
        }
    }
}

// ---------------------------------------------------------------------------
// quake_rs_* status cores, called by the reraising wrappers in
// Quake/cvar_cmd_glue.c. Rust must never call these back through the C
// wrappers (that would longjmp across a Rust frame).

/// C: `void quake_rs_cvar_register_variable (cvar_t *variable, int *raised)`
///
/// # Safety
/// `variable` and `raised` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_register_variable(
    variable: *mut c::cvar_t,
    raised: *mut c_int,
) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_register_variable_core(variable) };
}

/// C: `void quake_rs_cvar_set_quick (cvar_t *var, const char *value, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_set_quick(
    var: *mut c::cvar_t,
    value: *const c_char,
    raised: *mut c_int,
) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_set_quick_core(var, value) };
}

/// C: `void quake_rs_cvar_set_value_quick (cvar_t *var, float value, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_set_value_quick(
    var: *mut c::cvar_t,
    value: c_float,
    raised: *mut c_int,
) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_set_value_quick_core(var, value) };
}

/// C: `void quake_rs_cvar_set (const char *name, const char *value, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_set(
    name: *const c_char,
    value: *const c_char,
    raised: *mut c_int,
) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_set_core(name, value) };
}

/// C: `void quake_rs_cvar_set_value (const char *name, float value, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_set_value(
    name: *const c_char,
    value: c_float,
    raised: *mut c_int,
) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_set_value_core(name, value) };
}

/// C: `void quake_rs_cvar_set_rom (const char *name, const char *value, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_set_rom(
    name: *const c_char,
    value: *const c_char,
    raised: *mut c_int,
) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_set_rom_core(name, value) };
}

/// C: `void quake_rs_cvar_set_value_rom (const char *name, float value, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_set_value_rom(
    name: *const c_char,
    value: c_float,
    raised: *mut c_int,
) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_set_value_rom_core(name, value) };
}

/// C: `cvar_t *quake_rs_cvar_create (const char *name, const char *value, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_create(
    name: *const c_char,
    value: *const c_char,
    raised: *mut c_int,
) -> *mut c::cvar_t {
    let (var, r) = cvar_create_core(name, value);
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = r };
    var
}

/// C: `qboolean quake_rs_cvar_command (int *raised)`
///
/// # Safety
/// `raised` must be a valid out-pointer.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_command(raised: *mut c_int) -> c::qboolean {
    let (handled, r) = cvar_command_core();
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = r };
    handled
}

/// C: `void quake_rs_cvar_reset (const char *name, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cvar_reset(name: *const c_char, raised: *mut c_int) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cvar_reset_core(name) };
}
