//! C ABI shims for `Quake/cmd.c` (Rust migration Phase 7 M2).
//!
//! Near-transliteration. `cmd_functions` / `cmd_alias` / `cmd_text` /
//! `cmd_source` stay C-visible globals owned by `Quake/cvar_cmd_glue.c`
//! (console.c walks both registries for tab completion), so this module
//! manipulates them through `quake-c-sys` externs. cmd.c's private state
//! (`cmd_wait`, `cmd_argc`, `cmd_argv`, `cmd_args`) becomes module statics.
//!
//! ADR-009 audit: `cmd->function()` may `Host_Error`, and so may
//! `SZ_GetSpace` behind `SZ_Write`/`SZ_Print`. Handlers are dispatched only
//! through `CvarCmd_Glue_CallXCommand`; the buffer writes that can actually
//! overflow go through `CvarCmd_Glue_SzWrite` / `CvarCmd_Glue_ForwardPrint`.
//! Rust-implemented handlers cannot return a status, so they park theirs in
//! `PENDING_RAISE`, which the dispatcher drains right after the guard.

use crate::cvar::{
    con_print, cstr, cvar_command_core, cvar_register_variable_core, strcmp, Cvar_FindVarAfter,
    Cvar_VariableString, Raise,
};
use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use quake_c_sys as c;
use quake_c_sys::cvar_cmd as g;

const MAX_ALIAS_NAME: usize = 32;
const MAX_ARGS: usize = 80;
const CMDLINE_LENGTH: usize = 256;
const CONFIG_NAME: &CStr = c"vkQuake.cfg";

const SRC_CLIENT: c::cmd_source_t = c::cmd_source_t_src_client;
const SRC_COMMAND: c::cmd_source_t = c::cmd_source_t_src_command;
const SRC_SERVER: c::cmd_source_t = c::cmd_source_t_src_server;

/// `typedef void (*xtabcommand_t) (const char *partial);`
type XTabCommand = Option<unsafe extern "C" fn(*const c_char)>;

/// ADR-011 mirror of `cmd_function_t` (cmd.h:92-101). bindgen emits the type
/// as opaque because no C prototype exposes its fields, so the layout is
/// transcribed field-for-field from the header rather than inferred.
#[repr(C)]
struct CmdFunction {
    next: *mut CmdFunction,
    name: *const c_char,
    function: c::xcommand_t,
    completion: XTabCommand,
    srctype: c::cmd_source_t,
    dynamic: c::qboolean,
    qcinterceptable: c::qboolean,
}

/// ADR-011 mirror of cmd.c's private `cmdalias_t` (cmd.c:35-40); the same
/// struct is transcribed into `Quake/cvar_cmd_glue.c` and console.c.
#[repr(C)]
struct CmdAlias {
    next: *mut CmdAlias,
    name: [c_char; MAX_ALIAS_NAME],
    value: *mut c_char,
}

extern "C" {
    /// `cmd_function_t *cmd_functions;` (Quake/cvar_cmd_glue.c)
    static mut cmd_functions: *mut CmdFunction;
    /// `cmdalias_t *cmd_alias;` (Quake/cvar_cmd_glue.c)
    static mut cmd_alias: *mut CmdAlias;
}

/// `static qboolean cmd_wait;`
static mut CMD_WAIT: c::qboolean = false;
/// `static int cmd_argc;`
static mut CMD_ARGC: c_int = 0;
/// `static char cmd_argv[MAX_ARGS][1024];`
static mut CMD_ARGV: [[c_char; 1024]; MAX_ARGS] = [[0; 1024]; MAX_ARGS];
/// `static const char *cmd_args = NULL;` -- a borrowed pointer into the text
/// passed to Cmd_TokenizeString, exactly as in C.
static mut CMD_ARGS: *const c_char = ptr::null();
/// `static char cmd_null_string[] = "";`
const CMD_NULL_STRING: &CStr = c"";

/// ADR-009: a raise caught inside a Rust command handler. Handlers are
/// `extern "C" fn()` with no return value, so the status is parked here and
/// drained by the dispatcher immediately after the guard helper returns.
static mut PENDING_RAISE: c_int = 0;

pub(crate) fn set_pending_raise(raised: c_int) {
    if raised != 0 {
        // SAFETY: single-threaded host
        unsafe { PENDING_RAISE = raised };
    }
}

fn take_pending_raise() -> c_int {
    // SAFETY: single-threaded host
    unsafe {
        let r = PENDING_RAISE;
        PENDING_RAISE = 0;
        r
    }
}

/// `cmd_argv[i]` as a C pointer (the dispatcher compares argv[0] directly).
pub(crate) fn cmd_argv_ptr(i: usize) -> *const c_char {
    // SAFETY: single-threaded host; i < MAX_ARGS
    unsafe { (&raw const CMD_ARGV[i]) as *const c_char }
}

fn q_strcasecmp(a: *const c_char, b: *const c_char) -> c_int {
    // SAFETY: both are NUL-terminated C strings
    unsafe { g::q_strcasecmp(a, b) }
}

fn streq(a: *const c_char, b: *const c_char) -> bool {
    cstr(a) == cstr(b)
}

// ---------------------------------------------------------------------------

/// C: `void Cmd_Wait_f (void)`
extern "C" fn cmd_wait_f() {
    // SAFETY: single-threaded host
    unsafe { CMD_WAIT = true };
}

/*
=============================================================================
                        COMMAND BUFFER
=============================================================================
*/

/// C: `void Cbuf_Init (void)`
#[no_mangle]
pub extern "C" fn Cbuf_Init() {
    // space for commands and script files
    // SAFETY: cmd_text is the glue-owned sizebuf; SZ_Alloc only allocates
    unsafe { g::SZ_Alloc(&raw mut g::cmd_text, 1 << 18) };
}

/// C: `void Cbuf_AddText (const char *text)`
#[no_mangle]
pub extern "C" fn Cbuf_AddText(text: *const c_char) {
    let l = cstr(text).len() as c_int;
    // SAFETY: cmd_text is the glue-owned sizebuf; the pre-check below is what
    // keeps SZ_Write off its Host_Error path, so it is called directly.
    unsafe {
        if g::cmd_text.cursize + l >= g::cmd_text.maxsize {
            con_print(c::Con_Printf, b"Cbuf_AddText: overflow\n".to_vec());
            return;
        }
        g::SZ_Write(&raw mut g::cmd_text, text as *const c_void, l);
    }
}

/// C: `void Cbuf_AddTextLen (const char *text, int l)`
#[no_mangle]
pub extern "C" fn Cbuf_AddTextLen(text: *const c_char, l: c_int) {
    // SAFETY: cmd_text is the glue-owned sizebuf; caller guarantees l bytes
    unsafe {
        if g::cmd_text.cursize + l >= g::cmd_text.maxsize {
            // COMPAT: the message says "Cbuf_AddText" here too (cmd.c:108)
            con_print(c::Con_Printf, b"Cbuf_AddText: overflow\n".to_vec());
            return;
        }
        g::SZ_Write(&raw mut g::cmd_text, text as *const c_void, l);
    }
}

/// `void Cbuf_InsertText (const char *text)`
pub(crate) fn cbuf_insert_text_core(text: *const c_char) -> Raise {
    // SAFETY: cmd_text is the glue-owned sizebuf; temp is a Mem_Alloc block
    unsafe {
        // copy off any commands still remaining in the exec buffer
        let templen = g::cmd_text.cursize;
        let temp = if templen != 0 {
            let t = c::Mem_Alloc(templen as usize) as *mut u8;
            ptr::copy_nonoverlapping(g::cmd_text.data, t, templen as usize);
            g::SZ_Clear(&raw mut g::cmd_text);
            t
        } else {
            ptr::null_mut() // shut up compiler
        };

        // add the entire text of the file
        Cbuf_AddText(text);
        // COMPAT: the extra newline, and its position before the copied-off
        // remainder, are both observable (cmd.c:142-148)
        let raised =
            g::CvarCmd_Glue_SzWrite(&raw mut g::cmd_text, c"\n".as_ptr() as *const c_void, 1);
        if raised != 0 {
            return raised;
        }
        // add the copied off data
        if templen != 0 {
            let raised =
                g::CvarCmd_Glue_SzWrite(&raw mut g::cmd_text, temp as *const c_void, templen);
            if raised != 0 {
                return raised;
            }
            c::Mem_Free(temp as *const c_void);
        }
        0
    }
}

/// C: `void Cbuf_Waited (void)` -- Spike: for renderer/server isolation
#[no_mangle]
pub extern "C" fn Cbuf_Waited() {
    // SAFETY: single-threaded host
    unsafe { CMD_WAIT = false };
}

/// `void Cbuf_Execute (void)`
fn cbuf_execute_core() -> Raise {
    let mut line = [0u8; 1024];

    // SAFETY: cmd_text is the glue-owned sizebuf with a live data allocation
    unsafe {
        while g::cmd_text.cursize != 0 && !CMD_WAIT {
            // find a \n or ; line break
            let text = g::cmd_text.data;
            let cursize = g::cmd_text.cursize as usize;

            // COMPAT: the C `text[i + 1]` check reads one byte past cursize at
            // i == cursize - 1. That byte lives inside the 256 KB SZ_Alloc, so
            // the slice is extended by one where maxsize permits; line_break
            // treats a missing byte as 0.
            let span = (cursize + 1).min(g::cmd_text.maxsize as usize);
            let i = quake_cvar::line_break(core::slice::from_raw_parts(text, span), cursize);

            // COMPAT: longer lines are silently truncated to 1023 bytes
            if i > line.len() - 1 {
                ptr::copy_nonoverlapping(text, line.as_mut_ptr(), line.len() - 1);
                line[line.len() - 1] = 0;
            } else {
                ptr::copy_nonoverlapping(text, line.as_mut_ptr(), i);
                line[i] = 0;
            }

            // delete the text from the command buffer and move remaining
            // commands down; this is necessary because commands (exec, alias)
            // can insert data at the beginning of the text buffer
            if i == cursize {
                g::cmd_text.cursize = 0;
            } else {
                let i = i + 1;
                g::cmd_text.cursize -= i as c_int;
                ptr::copy(text.add(i), text, g::cmd_text.cursize as usize);
            }

            // execute the command line
            let (_, raised) = cmd_execute_string_core(line.as_ptr() as *const c_char, SRC_COMMAND);
            if raised != 0 {
                return raised;
            }
        }
    }
    0
}

/*
==============================================================================
                        SCRIPT COMMANDS
==============================================================================
*/

/// C: `void Cmd_StuffCmds_f (void)`
extern "C" fn cmd_stuffcmds_f() {
    let mut cmds = [0u8; CMDLINE_LENGTH];
    let mut plus = false; // On Unix, argv[0] is command name

    // SAFETY: cmdline is a registered engine cvar with a live string
    let line = unsafe { cstr(c::cmdline.string) };

    let mut j = 0usize;
    for i in 0..line.len() {
        if line[i] == b'+' {
            plus = true;
            if j > 0 {
                cmds[j - 1] = b';';
                cmds[j] = b' ';
                j += 1;
            }
        } else if line[i] == b'-' && (i == 0 || line[i - 1] == b' ') {
            // johnfitz -- allow hypenated map names with +map
            plus = false;
        } else if plus {
            cmds[j] = line[i];
            j += 1;
        }
    }
    cmds[j] = 0;

    let raised = cbuf_insert_text_core(cmds.as_ptr() as *const c_char);
    set_pending_raise(raised);
}

/// C: `void Cmd_Exec_f (void)`
extern "C" fn cmd_exec_f() {
    if Cmd_Argc() != 2 {
        con_print(
            c::Con_Printf,
            b"exec <filename> : execute a script file\n".to_vec(),
        );
        return;
    }

    let path = Cmd_Argv(1);
    let mut display_path = path;
    // SAFETY: path is a NUL-terminated argv slot
    let legacy_config_alias = unsafe {
        g::q_strcasecmp(path, c"config.cfg".as_ptr()) == 0
            || g::q_strcasecmp(path, CONFIG_NAME.as_ptr()) == 0
    };

    let mut buf: *mut u8 = ptr::null_mut();

    // SAFETY: engine file APIs; buf is a Mem_Alloc block sized from the file
    unsafe {
        if legacy_config_alias {
            // "exec config.cfg" executes vkQuake.cfg from the user config directory.
            let f = c::COM_FOpenPrefFile(CONFIG_NAME.as_ptr(), c"rb".as_ptr());
            if !f.is_null() {
                let length = c::Sys_filelength(f);
                buf = c::Mem_Alloc(length as usize + 1) as *mut u8;
                if c::stdio::fread(buf as *mut c_void, 1, length as usize, f) != length as usize {
                    c::Mem_Free(buf as *const c_void);
                    buf = ptr::null_mut();
                } else {
                    *buf.add(length as usize) = 0;
                    display_path = CONFIG_NAME.as_ptr();
                }
                c::stdio::fclose(f);
            }
        } else {
            buf = c::COM_LoadFile(path, ptr::null_mut());
        }

        if buf.is_null() {
            if warncmd_value() != 0.0 {
                let mut b = b"couldn't exec ".to_vec();
                b.extend_from_slice(cstr(path));
                b.push(b'\n');
                con_print(c::Con_Printf, b);
            }
            return;
        }

        if warncmd_value() != 0.0 {
            let mut b = b"execing ".to_vec();
            b.extend_from_slice(cstr(display_path));
            b.push(b'\n');
            con_print(c::Con_Printf, b);
        }

        // COMPAT: the trailing newline is inserted FIRST, so it ends up after
        // the file contents in the buffer (cmd.c:325-326)
        let raised = cbuf_insert_text_core(c"\n".as_ptr());
        if raised != 0 {
            set_pending_raise(raised);
            return;
        }
        let raised = cbuf_insert_text_core(buf as *const c_char);
        if raised != 0 {
            set_pending_raise(raised);
            return;
        }

        c::Mem_Free(buf as *const c_void);
    }
}

fn warncmd_value() -> f32 {
    // SAFETY: cmd_warncmd is the glue-owned cvar registered by Cmd_Init
    unsafe { g::cmd_warncmd.value }
}

/// C: `void Cmd_Echo_f (void)`
extern "C" fn cmd_echo_f() {
    for i in 1..Cmd_Argc() {
        let mut b = cstr(Cmd_Argv(i)).to_vec();
        b.push(b' ');
        con_print(c::Con_Printf, b);
    }
    con_print(c::Con_Printf, b"\n".to_vec());
}

/// C: `void Cmd_Alias_f (void)`
extern "C" fn cmd_alias_f() {
    // SAFETY: single-threaded host; cmd_alias is a well-formed C list
    unsafe {
        match Cmd_Argc() {
            1 => {
                // list all aliases
                let mut a = cmd_alias;
                let mut i = 0;
                while !a.is_null() {
                    let mut b = b"   ".to_vec();
                    b.extend_from_slice(cstr((*a).name.as_ptr()));
                    b.extend_from_slice(b": ");
                    b.extend_from_slice(cstr((*a).value));
                    con_print(c::Con_SafePrintf, b);
                    a = (*a).next;
                    i += 1;
                }
                if i != 0 {
                    con_print(
                        c::Con_SafePrintf,
                        format!("{i} alias command(s)\n").into_bytes(),
                    );
                } else {
                    con_print(c::Con_SafePrintf, b"no alias commands found\n".to_vec());
                }
            }
            2 => {
                // output current alias string
                let mut a = cmd_alias;
                while !a.is_null() {
                    if streq(Cmd_Argv(1), (*a).name.as_ptr()) {
                        let mut b = b"   ".to_vec();
                        b.extend_from_slice(cstr((*a).name.as_ptr()));
                        b.extend_from_slice(b": ");
                        b.extend_from_slice(cstr((*a).value));
                        con_print(c::Con_Printf, b);
                    }
                    a = (*a).next;
                }
            }
            _ => {
                // set alias string
                let s = Cmd_Argv(1);
                if cstr(s).len() >= MAX_ALIAS_NAME {
                    con_print(c::Con_Printf, b"Alias name is too long\n".to_vec());
                    return;
                }

                // if the alias allready exists, reuse it
                let mut a = cmd_alias;
                while !a.is_null() {
                    if streq(s, (*a).name.as_ptr()) {
                        c::Mem_Free((*a).value as *const c_void);
                        break;
                    }
                    a = (*a).next;
                }

                if a.is_null() {
                    // COMPAT: new aliases go on the HEAD of the list, unsorted
                    a = c::Mem_Alloc(core::mem::size_of::<CmdAlias>()) as *mut CmdAlias;
                    (*a).next = cmd_alias;
                    cmd_alias = a;
                }
                let name = cstr(s);
                ptr::copy_nonoverlapping(
                    name.as_ptr(),
                    (*a).name.as_mut_ptr() as *mut u8,
                    name.len(),
                );
                *((*a).name.as_mut_ptr() as *mut u8).add(name.len()) = 0;

                // copy the rest of the command line
                let mut cmd = [0u8; 1024];
                let c_argc = Cmd_Argc();
                for i in 2..c_argc {
                    quake_util::strl::strlcat(&mut cmd, cstr(Cmd_Argv(i)));
                    if i != c_argc - 1 {
                        quake_util::strl::strlcat(&mut cmd, b" ");
                    }
                }
                if quake_util::strl::strlcat(&mut cmd, b"\n") >= cmd.len() {
                    con_print(c::Con_Printf, b"alias value too long!\n".to_vec());
                    cmd[0] = b'\n'; // nullify the string
                    cmd[1] = 0;
                }

                (*a).value = g::q_strdup(cmd.as_ptr() as *const c_char);
            }
        }
    }
}

/// C: `void Cmd_Unalias_f (void)`
extern "C" fn cmd_unalias_f() {
    // SAFETY: single-threaded host; cmd_alias is a well-formed C list
    unsafe {
        match Cmd_Argc() {
            2 => {
                let mut prev: *mut CmdAlias = ptr::null_mut();
                let mut a = cmd_alias;
                while !a.is_null() {
                    if streq(Cmd_Argv(1), (*a).name.as_ptr()) {
                        if !prev.is_null() {
                            (*prev).next = (*a).next;
                        } else {
                            cmd_alias = (*a).next;
                        }

                        c::Mem_Free((*a).value as *const c_void);
                        c::Mem_Free(a as *const c_void);
                        return;
                    }
                    prev = a;
                    a = (*a).next;
                }
                let mut b = b"No alias named ".to_vec();
                b.extend_from_slice(cstr(Cmd_Argv(1)));
                b.push(b'\n');
                con_print(c::Con_Printf, b);
            }
            // COMPAT: `default:` falls into `case 1:`
            _ => con_print(c::Con_Printf, b"unalias <name> : delete alias\n".to_vec()),
        }
    }
}

/// C: `qboolean Cmd_AliasExists (const char *aliasname)`
#[no_mangle]
pub extern "C" fn Cmd_AliasExists(aliasname: *const c_char) -> c::qboolean {
    // SAFETY: single-threaded host; cmd_alias is a well-formed C list
    unsafe {
        let mut a = cmd_alias;
        while !a.is_null() {
            if q_strcasecmp(aliasname, (*a).name.as_ptr()) == 0 {
                return true;
            }
            a = (*a).next;
        }
    }
    false
}

/// C: `void Cmd_Unaliasall_f (void)`
extern "C" fn cmd_unaliasall_f() {
    // SAFETY: single-threaded host; cmd_alias is a well-formed C list
    unsafe {
        while !cmd_alias.is_null() {
            let blah = (*cmd_alias).next;
            c::Mem_Free((*cmd_alias).value as *const c_void);
            c::Mem_Free(cmd_alias as *const c_void);
            cmd_alias = blah;
        }
    }
}

/*
=============================================================================
                    COMMAND EXECUTION
=============================================================================
*/

/// C: `void Cmd_List_f (void)`
extern "C" fn cmd_list_f() {
    // C keeps `len = strlen (partial)` for the strncmp; a prefix test is the
    // same comparison.
    let partial: *const c_char = if Cmd_Argc() > 1 {
        Cmd_Argv(1)
    } else {
        ptr::null()
    };

    let mut count = 0;
    // SAFETY: single-threaded host; cmd_functions is a well-formed C list
    unsafe {
        let mut cmd = cmd_functions;
        while !cmd.is_null() {
            if !partial.is_null() && !cstr((*cmd).name).starts_with(cstr(partial)) {
                cmd = (*cmd).next;
                continue;
            }
            let mut b = b"   ".to_vec();
            b.extend_from_slice(cstr((*cmd).name));
            b.push(b'\n');
            con_print(c::Con_SafePrintf, b);
            count += 1;
            cmd = (*cmd).next;
        }
    }

    con_print(c::Con_SafePrintf, format!("{count} commands").into_bytes());
    if !partial.is_null() {
        let mut b = b" beginning with \"".to_vec();
        b.extend_from_slice(cstr(partial));
        b.push(b'"');
        con_print(c::Con_SafePrintf, b);
    }
    con_print(c::Con_SafePrintf, b"\n".to_vec());
}

/// `static char *Cmd_TintSubstring (const char *in, const char *substr, char *out, size_t outsize)`
fn cmd_tint_substring(
    input: *const c_char,
    substr: *const c_char,
    out: &mut [u8; 256],
) -> *const c_char {
    quake_util::strl::strlcpy(out, cstr(input));
    // SAFETY: out is NUL-terminated by strlcpy; q_strcasestr walks within it
    unsafe {
        let mut m = g::q_strcasestr(out.as_mut_ptr() as *const c_char, substr) as *mut u8;
        while !m.is_null() {
            let l = cstr(substr).len();
            // COMPAT: `while (l-- > 0) if (*m >= ' ' && *m < 127) *m++ |= 0x80;`
            // -- `m` only advances inside the `if`, so a byte outside the
            // printable range makes the loop re-test the same byte.
            for _ in 0..l {
                if *m >= b' ' && *m < 127 {
                    *m |= 0x80;
                    m = m.add(1);
                }
            }
            m = g::q_strcasestr(out.as_mut_ptr() as *const c_char, substr) as *mut u8;
        }
    }
    out.as_ptr() as *const c_char
}

/// C: `void Cmd_Apropos_f (void)`
extern "C" fn cmd_apropos_f() {
    let mut tmpbuf = [0u8; 256];
    let mut hits = 0;
    let substr = Cmd_Argv(1);
    if cstr(substr).is_empty() {
        let mut b = cstr(Cmd_Argv(0)).to_vec();
        b.extend_from_slice(
            b" <substring> : search through commands and cvars for the given substring\n",
        );
        con_print(c::Con_SafePrintf, b);
        return;
    }

    // SAFETY: single-threaded host; both registries are well-formed C lists
    unsafe {
        let mut cmd = cmd_functions;
        while !cmd.is_null() {
            if !g::q_strcasestr((*cmd).name, substr).is_null() && (*cmd).srctype != SRC_SERVER {
                hits += 1;
                let tinted = cmd_tint_substring((*cmd).name, substr, &mut tmpbuf);
                let mut b = cstr(tinted).to_vec();
                b.push(b'\n');
                con_print(c::Con_SafePrintf, b);
            }
            cmd = (*cmd).next;
        }

        let mut var = Cvar_FindVarAfter(c"".as_ptr(), 0);
        while !var.is_null() {
            if !g::q_strcasestr((*var).name, substr).is_null() {
                hits += 1;
                let tinted = cmd_tint_substring((*var).name, substr, &mut tmpbuf);
                let mut b = cstr(tinted).to_vec();
                b.extend_from_slice(b" (current value: \"");
                b.extend_from_slice(cstr((*var).string));
                b.extend_from_slice(b"\")\n");
                con_print(c::Con_SafePrintf, b);
            }
            var = (*var).next;
        }
    }
    if hits == 0 {
        con_print(
            c::Con_SafePrintf,
            b"no cvars nor commands contain that substring\n".to_vec(),
        );
    }
}

/// C: `void Cmd_Init (void)`
#[no_mangle]
pub extern "C" fn Cmd_Init() {
    let add = |name: &CStr, f: extern "C" fn()| {
        Cmd_AddCommand2(
            name.as_ptr(),
            Some(f as unsafe extern "C" fn()),
            SRC_COMMAND,
            false,
        );
    };
    add(c"cmdlist", cmd_list_f); // johnfitz
    add(c"unalias", cmd_unalias_f); // johnfitz
    add(c"unaliasall", cmd_unaliasall_f); // johnfitz

    add(c"stuffcmds", cmd_stuffcmds_f);
    add(c"exec", cmd_exec_f);
    add(c"echo", cmd_echo_f);
    add(c"alias", cmd_alias_f);
    add(c"cmd", cmd_forward_to_server_handler);
    add(c"wait", cmd_wait_f);

    add(c"apropos", cmd_apropos_f);
    add(c"find", cmd_apropos_f);

    // Neither cvar has a callback or a replicated flag, so registration
    // cannot reach a guarded path; the status is discarded like the C's
    // void-returning Cvar_RegisterVariable.
    let _ = cvar_register_variable_core(&raw mut g::cl_nopext);
    let _ = cvar_register_variable_core(&raw mut g::cmd_warncmd);
}

/// C: `int Cmd_Argc (void)`
#[no_mangle]
pub extern "C" fn Cmd_Argc() -> c_int {
    // SAFETY: single-threaded host
    unsafe { CMD_ARGC }
}

/// C: `const char *Cmd_Argv (int arg)`
#[no_mangle]
pub extern "C" fn Cmd_Argv(arg: c_int) -> *const c_char {
    // SAFETY: single-threaded host
    unsafe {
        if arg < 0 || arg >= CMD_ARGC {
            return CMD_NULL_STRING.as_ptr();
        }
    }
    cmd_argv_ptr(arg as usize)
}

/// C: `const char *Cmd_Args (void)`
#[no_mangle]
pub extern "C" fn Cmd_Args() -> *const c_char {
    // SAFETY: single-threaded host
    unsafe {
        if CMD_ARGS.is_null() {
            return c"".as_ptr();
        }
        CMD_ARGS
    }
}

/// C: `void Cmd_TokenizeString (const char *text)`
///
/// # Safety
/// `text` must be a NUL-terminated C string that outlives the tokenize, since
/// `cmd_args` borrows into it exactly as the C does.
#[no_mangle]
pub unsafe extern "C" fn Cmd_TokenizeString(mut text: *const c_char) {
    // SAFETY: guaranteed by the caller (see above)
    unsafe {
        // clear the args from the last string
        let argv = (&raw mut CMD_ARGV) as *mut [c_char; 1024];
        for i in 0..CMD_ARGC as usize {
            (*argv.add(i))[0] = 0;
        }

        CMD_ARGC = 0;
        CMD_ARGS = ptr::null();

        loop {
            // COMPAT: `*text <= ' '` compares a plain C `char`, so on
            // signed-char targets bytes >= 0x80 count as whitespace.
            while *text != 0 && *text <= b' ' as c_char && *text != b'\n' as c_char {
                text = text.add(1);
            }

            if *text == b'\n' as c_char {
                // a newline seperates commands in the buffer
                break;
            }

            if *text == 0 {
                return;
            }

            if CMD_ARGC == 1 {
                CMD_ARGS = text;
            }

            text = c::COM_Parse(text);
            if text.is_null() {
                return;
            }

            Cmd_AddArg(c::COM_ThreadToken());
        }
    }
}

/// C: `void Cmd_AddArg (const char *arg)`
#[no_mangle]
pub extern "C" fn Cmd_AddArg(arg: *const c_char) {
    // SAFETY: single-threaded host; arg is a NUL-terminated C string
    unsafe {
        if (CMD_ARGC as usize) < MAX_ARGS {
            let slot = ((&raw mut CMD_ARGV) as *mut [c_char; 1024]).add(CMD_ARGC as usize);
            let dst = core::slice::from_raw_parts_mut(slot as *mut u8, 1024);
            quake_util::strl::strlcpy(dst, cstr(arg));
            CMD_ARGC += 1;
        }
    }
}

/// C: `cmd_function_t *Cmd_AddCommand2 (const char *cmd_name, xcommand_t function, cmd_source_t srctype, qboolean qcinterceptable)`
#[no_mangle]
pub extern "C" fn Cmd_AddCommand2(
    cmd_name: *const c_char,
    function: c::xcommand_t,
    srctype: c::cmd_source_t,
    qcinterceptable: c::qboolean,
) -> *mut c::cmd_function_t {
    // fail if the command is a variable name
    if !cstr(Cvar_VariableString(cmd_name)).is_empty() {
        let mut b = b"Cmd_AddCommand: ".to_vec();
        b.extend_from_slice(cstr(cmd_name));
        b.extend_from_slice(b" already defined as a var\n");
        con_print(c::Con_Printf, b);
        return ptr::null_mut();
    }

    // SAFETY: single-threaded host; cmd_functions is a well-formed C list
    unsafe {
        // fail if the command already exists
        let mut cmd = cmd_functions;
        while !cmd.is_null() {
            if streq(cmd_name, (*cmd).name) && (*cmd).srctype == srctype {
                let existing = (*cmd).function.map(|f| f as usize);
                let incoming = function.map(|f| f as usize);
                if existing != incoming && function.is_some() {
                    let mut b = b"Cmd_AddCommand: ".to_vec();
                    b.extend_from_slice(cstr(cmd_name));
                    b.extend_from_slice(b" already defined\n");
                    con_print(c::Con_Printf, b);
                }
                return ptr::null_mut();
            }
            cmd = (*cmd).next;
        }

        let name_bytes = cstr(cmd_name);
        let cmd: *mut CmdFunction = if g::host_initialized {
            let p = c::Mem_Alloc(core::mem::size_of::<CmdFunction>() + name_bytes.len() + 1)
                as *mut CmdFunction;
            let dst = p.add(1) as *mut u8;
            ptr::copy_nonoverlapping(name_bytes.as_ptr(), dst, name_bytes.len());
            *dst.add(name_bytes.len()) = 0;
            (*p).name = dst as *const c_char;
            (*p).dynamic = true;
            p
        } else {
            let p = c::Mem_Alloc(core::mem::size_of::<CmdFunction>()) as *mut CmdFunction;
            // COMPAT: the static path aliases the caller's name pointer
            (*p).name = cmd_name;
            (*p).dynamic = false;
            p
        };
        (*cmd).function = function;
        (*cmd).srctype = srctype;
        (*cmd).qcinterceptable = qcinterceptable;

        // johnfitz -- insert each entry in alphabetical order
        if cmd_functions.is_null() || strcmp((*cmd).name, (*cmd_functions).name) < 0 {
            (*cmd).next = cmd_functions;
            cmd_functions = cmd;
        } else {
            let mut prev = cmd_functions;
            let mut cursor = (*cmd_functions).next;
            while !cursor.is_null() && strcmp((*cmd).name, (*cursor).name) > 0 {
                prev = cursor;
                cursor = (*cursor).next;
            }
            (*cmd).next = (*prev).next;
            (*prev).next = cmd;
        }

        cmd as *mut c::cmd_function_t
    }
}

/// C: `void Cmd_RemoveCommand (cmd_function_t *cmd)`
///
/// # Safety
/// `cmd` must be a node previously returned by `Cmd_AddCommand2`.
#[no_mangle]
pub unsafe extern "C" fn Cmd_RemoveCommand(cmd: *mut c::cmd_function_t) {
    let cmd = cmd as *mut CmdFunction;
    // SAFETY: single-threaded host; cmd_functions is a well-formed C list
    unsafe {
        let mut link: *mut *mut CmdFunction = &raw mut cmd_functions;
        while !(*link).is_null() {
            if *link == cmd {
                *link = (*cmd).next;
                c::Mem_Free(cmd as *const c_void);
                return;
            }
            link = &raw mut (**link).next;
        }
        c::Sys_Error(
            c"Cmd_RemoveCommand unable to remove command %s".as_ptr(),
            (*cmd).name,
        );
    }
}

/// C: `cmd_function_t *Cmd_FindCommand (const char *cmd_name)`
#[no_mangle]
pub extern "C" fn Cmd_FindCommand(cmd_name: *const c_char) -> *mut c::cmd_function_t {
    // SAFETY: single-threaded host; cmd_functions is a well-formed C list
    unsafe {
        let mut cmd = cmd_functions;
        while !cmd.is_null() {
            // COMPAT: lookup is case-INSENSITIVE here, unlike Cmd_Exists
            if q_strcasecmp(cmd_name, (*cmd).name) == 0 {
                return cmd as *mut c::cmd_function_t;
            }
            cmd = (*cmd).next;
        }
    }
    ptr::null_mut()
}

/// C: `qboolean Cmd_IsReservedName (const char *name)`
#[no_mangle]
pub extern "C" fn Cmd_IsReservedName(name: *const c_char) -> c::qboolean {
    let n = cstr(name);
    n.first() == Some(&b'_') && n.get(1) == Some(&b'_')
}

/// C: `qboolean Cmd_Exists (const char *cmd_name)`
#[no_mangle]
pub extern "C" fn Cmd_Exists(cmd_name: *const c_char) -> c::qboolean {
    // SAFETY: single-threaded host; cmd_functions is a well-formed C list
    unsafe {
        let mut cmd = cmd_functions;
        while !cmd.is_null() {
            if streq(cmd_name, (*cmd).name) {
                // these commands only exist in certain situations...
                // so pretend they don't exist here.
                if (*cmd).srctype != SRC_COMMAND {
                    cmd = (*cmd).next;
                    continue;
                }
                return true;
            }
            cmd = (*cmd).next;
        }
    }
    false
}

/// C: `const char *Cmd_CompleteCommand (const char *partial)`
#[no_mangle]
pub extern "C" fn Cmd_CompleteCommand(partial: *const c_char) -> *const c_char {
    let p = cstr(partial);
    if p.is_empty() {
        return ptr::null();
    }

    // SAFETY: single-threaded host; cmd_functions is a well-formed C list
    unsafe {
        let mut cmd = cmd_functions;
        while !cmd.is_null() {
            // COMPAT: completion is case-SENSITIVE (strncmp)
            if cstr((*cmd).name).starts_with(p) {
                return (*cmd).name;
            }
            cmd = (*cmd).next;
        }
    }
    ptr::null()
}

/// `qboolean Cmd_ExecuteString (const char *text, cmd_source_t src)`
pub(crate) fn cmd_execute_string_core(
    text: *const c_char,
    src: c::cmd_source_t,
) -> (c::qboolean, Raise) {
    // SAFETY: single-threaded host; both registries are well-formed C lists
    unsafe {
        g::cmd_source = src;
        Cmd_TokenizeString(text);

        // execute the command line
        if Cmd_Argc() == 0 {
            return (true, 0); // no tokens
        }

        // check functions
        let mut cmd = cmd_functions;
        while !cmd.is_null() {
            if q_strcasecmp(cmd_argv_ptr(0), (*cmd).name) == 0 {
                if src == SRC_CLIENT && (*cmd).srctype != SRC_CLIENT {
                    // COMPAT: this branch prints and then STILL runs the
                    // handler; only the two `continue` branches skip it.
                    let mut b = cstr(g::CvarCmd_Glue_HostClientName()).to_vec();
                    b.extend_from_slice(b" tried to ");
                    b.extend_from_slice(cstr(text));
                    b.push(b'\n');
                    con_print(c::Con_DPrintf, b);
                }
                // src_command can execute anything but server commands (which
                // it ignores, allowing for alternative behaviour); src_server
                // may only execute server commands. The C spells these as two
                // `else if ... continue` arms.
                else if (src == SRC_COMMAND && (*cmd).srctype == SRC_SERVER)
                    || (src == SRC_SERVER && (*cmd).srctype != SRC_SERVER)
                {
                    cmd = (*cmd).next;
                    continue;
                }
                // ADR-009: never call the handler directly
                let guard = g::CvarCmd_Glue_CallXCommand((*cmd).function);
                let pending = take_pending_raise();
                return (true, if guard != 0 { guard } else { pending });
            }
            cmd = (*cmd).next;
        }

        if src == SRC_CLIENT {
            // spike -- please don't execute similarly named aliases, nor custom cvars...
            let mut b = cstr(g::CvarCmd_Glue_HostClientName()).to_vec();
            b.extend_from_slice(b" tried to ");
            b.extend_from_slice(cstr(text));
            b.push(b'\n');
            con_print(c::Con_DPrintf, b);
            return (false, 0);
        }
        if src != SRC_COMMAND {
            return (false, 0);
        }

        // check alias
        let mut a = cmd_alias;
        while !a.is_null() {
            if q_strcasecmp(cmd_argv_ptr(0), (*a).name.as_ptr()) == 0 {
                let raised = cbuf_insert_text_core((*a).value);
                return (true, raised);
            }
            a = (*a).next;
        }

        // check cvars
        let (handled, raised) = cvar_command_core();
        if raised != 0 {
            return (true, raised);
        }
        if !handled && (warncmd_value() != 0.0 || c::developer.value != 0.0) {
            let mut b = b"Unknown command \"".to_vec();
            b.extend_from_slice(cstr(Cmd_Argv(0)));
            b.extend_from_slice(b"\"\n");
            con_print(c::Con_Printf, b);
        }

        (true, 0)
    }
}

/// `void Cmd_ForwardToServer (void)`
fn cmd_forward_to_server_core() -> Raise {
    // SAFETY: the glue accessors read cls/cl_nopext in C
    unsafe {
        if !g::CvarCmd_Glue_ClsConnected() {
            let mut b = b"Can't \"".to_vec();
            b.extend_from_slice(cstr(Cmd_Argv(0)));
            b.extend_from_slice(b"\", not connected\n");
            con_print(c::Con_Printf, b);
            return 0;
        }

        if g::CvarCmd_Glue_ClsDemoPlayback() {
            return 0; // not really connected
        }

        let raised = g::CvarCmd_Glue_ForwardBegin();
        if raised != 0 {
            return raised;
        }

        if q_strcasecmp(Cmd_Argv(0), c"cmd".as_ptr()) != 0 {
            let raised = forward_print(cstr(Cmd_Argv(0)));
            if raised != 0 {
                return raised;
            }
            let raised = forward_print(b" ");
            if raised != 0 {
                return raised;
            }
        } else {
            // hack zone for compat.
            if cstr(Cmd_Args()) == b"protocols" {
                let (mut rmq, mut fitz, mut nq) = (0, 0, 0);
                g::CvarCmd_Glue_Protocols(&mut rmq, &mut fitz, &mut nq);
                let s = quake_util::printf::format(
                    b"protocols %i %i %i",
                    &[
                        quake_util::printf::Arg::I32(rmq),
                        quake_util::printf::Arg::I32(fitz),
                        quake_util::printf::Arg::I32(nq),
                    ],
                );
                return forward_print(&s);
            }
            if cstr(Cmd_Args()) == b"pext" && g::cl_nopext.value == 0.0 {
                let (mut p1, mut p1c, mut p2, mut p2c) = (0, 0, 0, 0);
                g::CvarCmd_Glue_PextNumbers(&mut p1, &mut p1c, &mut p2, &mut p2c);
                // COMPAT: ADR-005 -- C's `%#x` prints a bare `0` for zero
                let s = quake_util::printf::format(
                    b"pext %#x %#x %#x %#x",
                    &[
                        quake_util::printf::Arg::U32(p1),
                        quake_util::printf::Arg::U32(p1c),
                        quake_util::printf::Arg::U32(p2),
                        quake_util::printf::Arg::U32(p2c),
                    ],
                );
                return forward_print(&s);
            }
        }
        if Cmd_Argc() > 1 {
            forward_print(cstr(Cmd_Args()))
        } else {
            forward_print(b"\n")
        }
    }
}

/// SZ_Print into cls.message through the ADR-009 guard.
fn forward_print(bytes: &[u8]) -> Raise {
    let mut b = bytes.to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated and outlives the call
    unsafe { g::CvarCmd_Glue_ForwardPrint(b.as_ptr() as *const c_char) }
}

/// The "cmd" console command; the public `Cmd_ForwardToServer` lives in
/// `Quake/cvar_cmd_glue.c` so it can re-raise from a C frame.
extern "C" fn cmd_forward_to_server_handler() {
    let raised = cmd_forward_to_server_core();
    set_pending_raise(raised);
}

/// C: `int Cmd_CheckParm (const char *parm)`
#[no_mangle]
pub extern "C" fn Cmd_CheckParm(parm: *const c_char) -> c_int {
    if parm.is_null() {
        // SAFETY: Sys_Error is noreturn and does not longjmp
        unsafe { c::Sys_Error(c"Cmd_CheckParm: null input\n".as_ptr()) };
    }

    for i in 1..Cmd_Argc() {
        if q_strcasecmp(parm, Cmd_Argv(i)) == 0 {
            return i;
        }
    }

    0
}

// ---------------------------------------------------------------------------
// quake_rs_* status cores, called by the reraising wrappers in
// Quake/cvar_cmd_glue.c.

/// C: `int quake_rs_cbuf_execute (void)`
#[no_mangle]
pub extern "C" fn quake_rs_cbuf_execute() -> c_int {
    cbuf_execute_core()
}

/// C: `void quake_rs_cbuf_insert_text (const char *text, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cbuf_insert_text(text: *const c_char, raised: *mut c_int) {
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = cbuf_insert_text_core(text) };
}

/// C: `qboolean quake_rs_cmd_execute_string (const char *text, cmd_source_t src, int *raised)`
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cmd_execute_string(
    text: *const c_char,
    src: c::cmd_source_t,
    raised: *mut c_int,
) -> c::qboolean {
    let (result, r) = cmd_execute_string_core(text, src);
    // SAFETY: raised is a valid out-pointer from the glue wrapper
    unsafe { *raised = r };
    result
}

/// C: `int quake_rs_cmd_forward_to_server (void)`
#[no_mangle]
pub extern "C" fn quake_rs_cmd_forward_to_server() -> c_int {
    cmd_forward_to_server_core()
}
