//! Differential test: `quake_capi::{cvar, cmd}` vs the original `Quake/cvar.c`
//! / `Quake/cmd.c` (compiled as `c_ref_*`). Phase 7 M2.
//!
//! Both sides keep their OWN, entirely separate registries for the lifetime
//! of this test binary (Rust's `CVAR_VARS`/`cmd_functions`/`cmd_alias` are
//! process statics owned by quake-capi and the glue-owned C globals; the
//! `c_ref_*` build has its own `static cvar_t *cvar_vars`/`cmd_functions` /
//! `cmd_alias` inside the renamed cvar.c/cmd.c). Every test therefore drives
//! BOTH sides through the SAME sequence of calls with distinct, per-test
//! name prefixes (so unrelated tests sharing the one process-wide registry
//! never collide), and diffs the observable results after each step. All
//! tests serialize on one mutex, matching the established
//! `net_msg_differential.rs` idiom, since both registries are global mutable
//! state.
//!
//! The raise-capable ABI (`Cvar_RegisterVariable`, `Cvar_Set*`, `Cvar_Create`,
//! `Cvar_Command`, `Cvar_Reset`, `Cbuf_Execute`, `Cbuf_InsertText`,
//! `Cmd_ExecuteString`, `Cmd_ForwardToServer`) is called directly (not
//! through `ctest_try_host`) throughout: none of the scenarios below register
//! a `CVAR_SERVERINFO`/`CVAR_USERINFO`/`CVAR_AUTOCVAR`/callback cvar or an
//! `xcommand_t` that errors, so `Host_Guard` always returns "no raise" on
//! both sides and `Host_Reraise` is a no-op. A dedicated Host_Error-path
//! differential (ROM/LOCKED write attempts, which do NOT raise; a
//! callback that DOES raise) is out of scope for this pass -- see the final
//! report for the gap.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

// quake-capi's crate/lib name is `quake_rs` (see rust/quake-capi/Cargo.toml's
// [lib] name), not `quake_capi` (that's only the package/path name).
use quake_c_sys as c;
use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_rs::cmd as rcmd;
use quake_rs::cvar as rcvar;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// c_ref_* oracle surface (Quake/cvar.c + Quake/cmd.c, renamed by the shared
// prelude force-include -- see rust/quake-ctest/include/c_ref_prelude.h).

// Some of these oracle declarations round out the full cvar.c/cmd.c public
// surface for completeness/documentation even though this pass's test cases
// don't happen to call every one (e.g. Cvar_SetValueQuick, the ROM setters,
// Cvar_Command/Cvar_Reset as free functions, Cmd_FindCommand, Cmd_Exists,
// Cbuf_Init) -- allow dead_code rather than trim the declarations.
#[allow(dead_code)]
extern "C" {
    fn c_ref_Cvar_Init();
    fn c_ref_Cvar_FindVar(var_name: *const c_char) -> *mut c::cvar_t;
    fn c_ref_Cvar_FindVarAfter(prev_name: *const c_char, with_flags: c_uint) -> *mut c::cvar_t;
    fn c_ref_Cvar_VariableValue(var_name: *const c_char) -> c_double;
    fn c_ref_Cvar_VariableString(var_name: *const c_char) -> *const c_char;
    fn c_ref_Cvar_CompleteVariable(partial: *const c_char) -> *const c_char;
    fn c_ref_Cvar_RegisterVariable(variable: *mut c::cvar_t);
    fn c_ref_Cvar_SetQuick(var: *mut c::cvar_t, value: *const c_char);
    fn c_ref_Cvar_SetValueQuick(var: *mut c::cvar_t, value: c_float);
    fn c_ref_Cvar_Set(var_name: *const c_char, value: *const c_char);
    fn c_ref_Cvar_SetValue(var_name: *const c_char, value: c_float);
    fn c_ref_Cvar_SetROM(var_name: *const c_char, value: *const c_char);
    fn c_ref_Cvar_SetValueROM(var_name: *const c_char, value: c_float);
    fn c_ref_Cvar_Create(name: *const c_char, value: *const c_char) -> *mut c::cvar_t;
    fn c_ref_Cvar_Command() -> c::qboolean;
    fn c_ref_Cvar_Reset(name: *const c_char);
    fn c_ref_Cvar_WriteVariables(f: *mut c::FILE);
    fn c_ref_Cvar_LockVar(var_name: *const c_char);
    fn c_ref_Cvar_UnlockVar(var_name: *const c_char);

    fn c_ref_Cmd_Init();
    fn c_ref_Cmd_Argc() -> c_int;
    fn c_ref_Cmd_Argv(arg: c_int) -> *const c_char;
    fn c_ref_Cmd_Args() -> *const c_char;
    fn c_ref_Cmd_TokenizeString(text: *const c_char);
    fn c_ref_Cmd_AddCommand2(
        cmd_name: *const c_char,
        function: c::xcommand_t,
        srctype: c::cmd_source_t,
        qcinterceptable: c::qboolean,
    ) -> *mut c::cmd_function_t;
    fn c_ref_Cmd_FindCommand(cmd_name: *const c_char) -> *mut c::cmd_function_t;
    fn c_ref_Cmd_Exists(cmd_name: *const c_char) -> c::qboolean;
    fn c_ref_Cmd_CompleteCommand(partial: *const c_char) -> *const c_char;
    fn c_ref_Cmd_ExecuteString(text: *const c_char, src: c::cmd_source_t) -> c::qboolean;
    fn c_ref_Cmd_CheckParm(parm: *const c_char) -> c_int;
    fn c_ref_Cmd_AliasExists(aliasname: *const c_char) -> c::qboolean;
    fn c_ref_Cbuf_Init();
    fn c_ref_Cbuf_AddText(text: *const c_char);
    fn c_ref_Cbuf_Execute();
    fn c_ref_Cbuf_InsertText(text: *const c_char);

    static mut c_ref_cmd_alias: *mut CCmdAlias;
    static mut cmd_alias: *mut CCmdAlias; // quake-capi's registry (stubs.c-owned global)

    fn tmpfile() -> *mut c::FILE;
    fn rewind(f: *mut c::FILE);
}

/// ADR-011 mirror of cmd.c's private `cmdalias_t` -- identical layout to
/// `rust/quake-capi/src/cmd.rs`'s own private `CmdAlias` (both transcribed
/// from the same cmd.c source).
#[repr(C)]
struct CCmdAlias {
    next: *mut CCmdAlias,
    name: [c_char; 32],
    value: *mut c_char,
}

// ---------------------------------------------------------------------------
// small helpers

fn cs(s: &str) -> Vec<c_char> {
    let mut v: Vec<c_char> = s.bytes().map(|b| b as c_char).collect();
    v.push(0);
    v
}

/// Leaks a NUL-terminated buffer so the returned pointer stays valid for the
/// life of the process. Required for `Cmd_AddCommand2`'s `cmd_name` argument:
/// per `Quake/cmd.c`, `Cmd_AddCommand2` only copies `cmd_name` into the
/// `cmd_function_t` allocation when `host_initialized` is true; this test
/// harness never sets `host_initialized`, so `cmd->name = cmd_name;` aliases
/// the raw pointer we pass in. A temporary `cs("...").as_ptr()` would dangle
/// the instant the backing `Vec<c_char>` drops at the end of the call
/// statement -- this is what a UB-diagnosis in this test suite (nondeterministic
/// results, intermittent access violations) tracked down to. Use this helper
/// for every `cmd_name` argument passed to either side's `Cmd_AddCommand2`.
fn leak_str(s: &str) -> *const c_char {
    Box::leak(cs(s).into_boxed_slice()).as_ptr()
}

/// SAFETY: `p` must be NUL-terminated (or null).
unsafe fn to_str(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: p is non-null (checked above) and NUL-terminated per this
    // function's contract.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// A heap-allocated `cvar_t`, leaked so its `name`/`string` pointers stay
/// valid for the life of the process (mirrors how the engine declares
/// `static cvar_t foo = {"foo", "0"};`).
fn new_cvar(name: &str, value: &str, flags: c::cvarflags_t) -> *mut c::cvar_t {
    let name = Box::leak(cs(name).into_boxed_slice()).as_ptr();
    let value = Box::leak(cs(value).into_boxed_slice()).as_ptr();
    let cvar = Box::new(c::cvar_t {
        name,
        string: value,
        flags,
        value: 0.0,
        default_string: core::ptr::null(),
        callback: None,
        completion: None,
        next: core::ptr::null_mut(),
    });
    Box::leak(cvar) as *mut c::cvar_t
}

/// Ensures `Cvar_Init`/`Cmd_Init` ran on both sides exactly once for this
/// binary (idempotent registration: re-running is harmless, it just prints
/// "already defined" for the built-in commands on both sides identically).
fn init_once() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: the init functions take no arguments and only touch their
        // own side's global registry; every caller holds TEST_LOCK.
        unsafe {
            c_ref_Cvar_Init();
            c_ref_Cmd_Init();
            c_ref_Cbuf_Init();
            rcvar::Cvar_Init();
            rcmd::Cmd_Init();
            rcmd::Cbuf_Init();
        }
    });
}

extern "C" fn noop_handler() {}

// ---------------------------------------------------------------------------
// 1. Registration order: sorted insert, duplicates, cvar/cmd name collisions

#[test]
fn cvar_register_sorted_order() {
    let _g = lock();
    init_once();

    let names = ["t1_zzz", "t1_aaa", "t1_mmm", "t1_bbb"];
    for n in names {
        // SAFETY: new_cvar leaks the cvar_t and its name/string buffers, so
        // the registry entries stay valid for the process lifetime.
        unsafe {
            c_ref_Cvar_RegisterVariable(new_cvar(n, "0", 0));
            Cvar_RegisterVariable(new_cvar(n, "0", 0) as *mut c::cvar_t);
        }
    }

    // walk both registries via FindVarAfter("", 0) and collect only the
    // t1_-prefixed names, in registry order
    let walk = |find_after: unsafe extern "C" fn(*const c_char, c_uint) -> *mut c::cvar_t| {
        // SAFETY: prev is always a NUL-terminated buffer that outlives each
        // call, and non-null results are live (leaked) registry nodes.
        unsafe {
            let mut out = Vec::new();
            let mut prev = cs("");
            loop {
                let v = find_after(prev.as_ptr(), 0);
                if v.is_null() {
                    break;
                }
                let name = to_str((*v).name);
                prev = cs(&name);
                if name.starts_with("t1_") {
                    out.push(name);
                }
            }
            out
        }
    };

    let c_order = walk(c_ref_Cvar_FindVarAfter);
    let r_order = walk(rcvar::Cvar_FindVarAfter);
    assert_eq!(c_order, r_order, "sorted registration order must match");
    assert_eq!(c_order, vec!["t1_aaa", "t1_bbb", "t1_mmm", "t1_zzz"]);
}

#[test]
fn cvar_register_duplicate_is_noop() {
    let _g = lock();
    init_once();

    // SAFETY: new_cvar/cs allocations are leaked or outlive the calls;
    // TEST_LOCK serializes access to both global registries.
    unsafe {
        c_ref_Cvar_RegisterVariable(new_cvar("t2_dup", "1", 0));
        c_ref_Cvar_RegisterVariable(new_cvar("t2_dup", "2", 0)); // ignored: already defined
        Cvar_RegisterVariable(new_cvar("t2_dup", "1", 0));
        Cvar_RegisterVariable(new_cvar("t2_dup", "2", 0));

        assert_eq!(
            c_ref_Cvar_VariableValue(cs("t2_dup").as_ptr()),
            rcvar::Cvar_VariableValue(cs("t2_dup").as_ptr())
        );
        assert_eq!(c_ref_Cvar_VariableValue(cs("t2_dup").as_ptr()), 1.0);
    }
}

#[test]
fn cvar_cmd_name_collision_both_directions() {
    let _g = lock();
    init_once();

    // SAFETY: leak_str/new_cvar/cs buffers are leaked or outlive the calls;
    // TEST_LOCK serializes access to both global registries.
    unsafe {
        // cmd registered first -> cvar registration with the same name fails
        c_ref_Cmd_AddCommand2(
            leak_str("t3_cmdfirst"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        rcmd::Cmd_AddCommand2(
            leak_str("t3_cmdfirst"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );

        c_ref_Cvar_RegisterVariable(new_cvar("t3_cmdfirst", "5", 0));
        Cvar_RegisterVariable(new_cvar("t3_cmdfirst", "5", 0));

        assert!(c_ref_Cvar_FindVar(cs("t3_cmdfirst").as_ptr()).is_null());
        assert!(rcvar::Cvar_FindVar(cs("t3_cmdfirst").as_ptr()).is_null());

        // cvar registered first -> Cmd_AddCommand2 with the same name fails
        c_ref_Cvar_RegisterVariable(new_cvar("t3_varfirst", "5", 0));
        Cvar_RegisterVariable(new_cvar("t3_varfirst", "5", 0));

        let c_cmd = c_ref_Cmd_AddCommand2(
            leak_str("t3_varfirst"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        let r_cmd = rcmd::Cmd_AddCommand2(
            leak_str("t3_varfirst"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        assert!(c_cmd.is_null());
        assert!(r_cmd.is_null());
    }
}

#[test]
fn cmd_addcommand_srctype_collision_and_dup() {
    let _g = lock();
    init_once();

    // SAFETY: leak_str names live for the process (required by
    // Cmd_AddCommand2's pointer-aliasing when !host_initialized); TEST_LOCK
    // serializes access to both global registries.
    unsafe {
        // same name + same srctype + same function pointer -> silently ignored
        let c1 = c_ref_Cmd_AddCommand2(
            leak_str("t4_dup"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        let c2 = c_ref_Cmd_AddCommand2(
            leak_str("t4_dup"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        let r1 = rcmd::Cmd_AddCommand2(
            leak_str("t4_dup"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        let r2 = rcmd::Cmd_AddCommand2(
            leak_str("t4_dup"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        assert!(!c1.is_null() && c2.is_null());
        assert!(!r1.is_null() && r2.is_null());

        // same name + DIFFERENT srctype -> distinct registration (not a collision)
        let c3 = c_ref_Cmd_AddCommand2(
            leak_str("t4_dup"),
            Some(noop_handler),
            c::cmd_source_t_src_client,
            false,
        );
        let r3 = rcmd::Cmd_AddCommand2(
            leak_str("t4_dup"),
            Some(noop_handler),
            c::cmd_source_t_src_client,
            false,
        );
        assert_eq!(c3.is_null(), r3.is_null());
        assert!(!c3.is_null());
    }
}

// ---------------------------------------------------------------------------
// 2. FindVarAfter flag filtering + CompleteVariable / CompleteCommand

#[test]
fn cvar_findvarafter_flag_filter() {
    let _g = lock();
    init_once();

    const ARCHIVE: c::cvarflags_t = c::cvarflags_t_CVAR_ARCHIVE;
    // SAFETY: new_cvar/cs buffers are leaked or outlive the calls, and the
    // non-null FindVarAfter results are live registry nodes.
    unsafe {
        c_ref_Cvar_RegisterVariable(new_cvar("t5_plain", "0", 0));
        c_ref_Cvar_RegisterVariable(new_cvar("t5_arch", "0", ARCHIVE));
        Cvar_RegisterVariable(new_cvar("t5_plain", "0", 0));
        Cvar_RegisterVariable(new_cvar("t5_arch", "0", ARCHIVE));

        let c_hit = c_ref_Cvar_FindVarAfter(cs("").as_ptr(), ARCHIVE);
        let r_hit = rcvar::Cvar_FindVarAfter(cs("").as_ptr(), ARCHIVE);
        // COMPAT: with_flags==0 matches everything; ARCHIVE only matches
        // ARCHIVE-flagged cvars, and the first hit may be any pre-existing
        // ARCHIVE cvar from earlier tests/built-ins, so just check both
        // sides land on the exact same name.
        assert_eq!(to_str((*c_hit).name), to_str((*r_hit).name));
    }
}

#[test]
fn cvar_complete_variable_and_cmd_complete_command() {
    let _g = lock();
    init_once();

    // SAFETY: new_cvar/leak_str/cs buffers are leaked or outlive the calls;
    // completion results are static registry strings (or NULL, handled).
    unsafe {
        c_ref_Cvar_RegisterVariable(new_cvar("t6_uniq_prefix_var", "0", 0));
        Cvar_RegisterVariable(new_cvar("t6_uniq_prefix_var", "0", 0));
        c_ref_Cmd_AddCommand2(
            leak_str("t6_uniq_prefix_cmd"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        rcmd::Cmd_AddCommand2(
            leak_str("t6_uniq_prefix_cmd"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );

        let c_v = c_ref_Cvar_CompleteVariable(cs("t6_uniq_prefix_v").as_ptr());
        let r_v = rcvar::Cvar_CompleteVariable(cs("t6_uniq_prefix_v").as_ptr());
        assert_eq!(to_str(c_v), to_str(r_v));
        assert_eq!(to_str(c_v), "t6_uniq_prefix_var");

        let c_c = c_ref_Cmd_CompleteCommand(cs("t6_uniq_prefix_c").as_ptr());
        let r_c = rcmd::Cmd_CompleteCommand(cs("t6_uniq_prefix_c").as_ptr());
        assert_eq!(to_str(c_c), to_str(r_c));
        assert_eq!(to_str(c_c), "t6_uniq_prefix_cmd");

        // empty partial -> NULL on both sides
        assert!(c_ref_Cvar_CompleteVariable(cs("").as_ptr()).is_null());
        assert!(rcvar::Cvar_CompleteVariable(cs("").as_ptr()).is_null());
        assert!(c_ref_Cmd_CompleteCommand(cs("").as_ptr()).is_null());
        assert!(rcmd::Cmd_CompleteCommand(cs("").as_ptr()).is_null());
    }
}

// ---------------------------------------------------------------------------
// 3. Alias define / redefine / unalias / expansion (via Cbuf + Cmd_Init's
// built-in `alias`/`unalias` commands, present from init_once()).

fn alias_lookup(head: *mut CCmdAlias, name: &str) -> Option<String> {
    // SAFETY: head is either null or points into a live alias list whose
    // nodes have NUL-terminated name/value strings (cmd.c invariant).
    unsafe {
        let mut a = head;
        while !a.is_null() {
            let n = CStr::from_ptr((*a).name.as_ptr()).to_string_lossy();
            if n == name {
                return Some(to_str((*a).value));
            }
            a = (*a).next;
        }
        None
    }
}

#[test]
fn alias_define_redefine_unalias() {
    let _g = lock();
    init_once();

    // SAFETY: cs buffers outlive the calls; alias_lookup walks each side's
    // own live alias list; TEST_LOCK serializes registry access.
    unsafe {
        // define
        c_ref_Cbuf_AddText(cs("alias t7_a \"echo hi\"\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("alias t7_a \"echo hi\"\n").as_ptr());
        Cbuf_Execute();

        assert_eq!(
            alias_lookup(c_ref_cmd_alias, "t7_a"),
            alias_lookup(cmd_alias, "t7_a")
        );
        assert_eq!(alias_lookup(c_ref_cmd_alias, "t7_a").unwrap(), "echo hi\n");
        assert_eq!(
            c_ref_Cmd_AliasExists(cs("t7_a").as_ptr()),
            rcmd::Cmd_AliasExists(cs("t7_a").as_ptr())
        );

        // redefine (same name, reuses the node -- value is freed and replaced)
        c_ref_Cbuf_AddText(cs("alias t7_a \"echo bye\"\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("alias t7_a \"echo bye\"\n").as_ptr());
        Cbuf_Execute();
        assert_eq!(
            alias_lookup(c_ref_cmd_alias, "t7_a"),
            alias_lookup(cmd_alias, "t7_a")
        );
        assert_eq!(alias_lookup(c_ref_cmd_alias, "t7_a").unwrap(), "echo bye\n");

        // unalias
        c_ref_Cbuf_AddText(cs("unalias t7_a\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("unalias t7_a\n").as_ptr());
        Cbuf_Execute();
        assert_eq!(
            c_ref_Cmd_AliasExists(cs("t7_a").as_ptr()),
            rcmd::Cmd_AliasExists(cs("t7_a").as_ptr())
        );
        assert!(!rcmd::Cmd_AliasExists(cs("t7_a").as_ptr()));
    }
}

#[test]
fn alias_expansion_via_cbuf_insert_text() {
    let _g = lock();
    init_once();

    // SAFETY: new_cvar/cs buffers are leaked or outlive the calls; TEST_LOCK
    // serializes access to both registries and command buffers.
    unsafe {
        // aliases insert their body via Cbuf_InsertText when executed; observe
        // that the aliased command actually ran by giving it a distinguishable
        // side effect (a cvar set).
        c_ref_Cvar_RegisterVariable(new_cvar("t8_target", "0", 0));
        Cvar_RegisterVariable(new_cvar("t8_target", "0", 0));

        c_ref_Cbuf_AddText(cs("alias t8_alias \"set t8_target 42\"\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("alias t8_alias \"set t8_target 42\"\n").as_ptr());
        Cbuf_Execute();

        c_ref_Cbuf_AddText(cs("t8_alias\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("t8_alias\n").as_ptr());
        Cbuf_Execute();

        assert_eq!(
            c_ref_Cvar_VariableValue(cs("t8_target").as_ptr()),
            rcvar::Cvar_VariableValue(cs("t8_target").as_ptr())
        );
        assert_eq!(c_ref_Cvar_VariableValue(cs("t8_target").as_ptr()), 42.0);
    }
}

// ---------------------------------------------------------------------------
// 4. Tokenizer edge cases (Cmd_TokenizeString / Cmd_Argc / Cmd_Argv / Cmd_Args)

fn tokenize_and_dump(
    tokenize: unsafe extern "C" fn(*const c_char),
    argc: unsafe extern "C" fn() -> c_int,
    argv: unsafe extern "C" fn(c_int) -> *const c_char,
    args: unsafe extern "C" fn() -> *const c_char,
    text: &[u8],
) -> (Vec<String>, String) {
    let mut buf = text.to_vec();
    buf.push(0);
    // SAFETY: buf is NUL-terminated and outlives the calls; argv indices are
    // bounded by argc; Cmd_Args returns a NUL-terminated (or null) pointer.
    unsafe {
        tokenize(buf.as_ptr() as *const c_char);
        let n = argc();
        let mut v = Vec::new();
        for i in 0..n {
            v.push(to_str(argv(i)));
        }
        (v, to_str(args()))
    }
}

fn tokenize_c(text: &[u8]) -> (Vec<String>, String) {
    tokenize_and_dump(
        c_ref_Cmd_TokenizeString,
        c_ref_Cmd_Argc,
        c_ref_Cmd_Argv,
        c_ref_Cmd_Args,
        text,
    )
}

fn tokenize_r(text: &[u8]) -> (Vec<String>, String) {
    unsafe extern "C" fn tok(t: *const c_char) {
        // SAFETY: caller (tokenize_and_dump) passes a NUL-terminated buffer
        // that outlives this call, matching Cmd_TokenizeString's contract.
        unsafe { rcmd::Cmd_TokenizeString(t) };
    }
    tokenize_and_dump(tok, rcmd::Cmd_Argc, rcmd::Cmd_Argv, rcmd::Cmd_Args, text)
}

#[test]
fn tokenizer_quotes_and_plain_tokens() {
    let _g = lock();
    init_once();
    for input in [
        &b"foo bar baz"[..],
        b"foo \"bar baz\" qux",
        b"  leading   whitespace",
        b"trailing whitespace   ",
        b"\"unterminated quote",
        b"",
        b"   ",
    ] {
        assert_eq!(tokenize_c(input), tokenize_r(input), "input: {:?}", input);
    }
}

#[test]
fn tokenizer_comment_latch_and_newline_stop() {
    let _g = lock();
    init_once();
    for input in [
        &b"foo // this is a comment"[..],
        b"foo // comment ; not a split\nbar",
        b"foo\nbar",
        b"// only a comment",
        b"a/b", // single slash: not a comment, ordinary token char
        b"a/",  // buffer ending in a single '/'
    ] {
        assert_eq!(tokenize_c(input), tokenize_r(input), "input: {:?}", input);
    }
}

#[test]
fn tokenizer_arg_clamp_and_token_truncation() {
    let _g = lock();
    init_once();

    // 90 single-char tokens: MAX_ARGS (80) clamps Cmd_Argc/Argv identically
    let many = (0..90)
        .map(|i| format!("a{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(tokenize_c(many.as_bytes()), tokenize_r(many.as_bytes()));

    // a single token > 1024 bytes: silently truncated by Cmd_AddArg's strlcpy
    let long_token = "x".repeat(2000);
    assert_eq!(
        tokenize_c(long_token.as_bytes()),
        tokenize_r(long_token.as_bytes())
    );
}

#[test]
fn tokenizer_cmd_args_pointer_suffix() {
    let _g = lock();
    init_once();

    // cmd_args borrows into the ORIGINAL buffer starting at the second
    // token; both sides must agree on the resulting suffix bytes.
    let text = b"first second third fourth";
    let (c_argv, c_args) = tokenize_c(text);
    let (r_argv, r_args) = tokenize_r(text);
    assert_eq!(c_argv, r_argv);
    assert_eq!(c_args, r_args);
    assert_eq!(c_args, "second third fourth");
}

// ---------------------------------------------------------------------------
// 5. Cbuf_Execute line splitting + Cbuf_InsertText ordering + wait

/// Trace hooks: each side gets its own registered command writing into its
/// own trace vector (the two registries are disjoint, so the SAME name can
/// be reused by each side's own handler without cross-talk). `TEST_LOCK`
/// already serializes every test in this file, so a plain `Mutex` (rather
/// than anything fancier) is enough to avoid `static mut` references.
static C_TRACE: Mutex<Vec<i32>> = Mutex::new(Vec::new());
static R_TRACE: Mutex<Vec<i32>> = Mutex::new(Vec::new());

fn c_trace() -> Vec<i32> {
    C_TRACE.lock().unwrap_or_else(|p| p.into_inner()).clone()
}
fn r_trace() -> Vec<i32> {
    R_TRACE.lock().unwrap_or_else(|p| p.into_inner()).clone()
}
fn clear_traces() {
    C_TRACE.lock().unwrap_or_else(|p| p.into_inner()).clear();
    R_TRACE.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

extern "C" fn c_trace_1() {
    C_TRACE.lock().unwrap_or_else(|p| p.into_inner()).push(1);
}
extern "C" fn c_trace_2() {
    C_TRACE.lock().unwrap_or_else(|p| p.into_inner()).push(2);
}
extern "C" fn r_trace_1() {
    R_TRACE.lock().unwrap_or_else(|p| p.into_inner()).push(1);
}
extern "C" fn r_trace_2() {
    R_TRACE.lock().unwrap_or_else(|p| p.into_inner()).push(2);
}

#[test]
fn cbuf_line_splitting_and_insert_text_ordering() {
    let _g = lock();
    init_once();

    clear_traces();
    // SAFETY: leak_str names live for the process; the trace handlers only
    // touch their own Mutex-guarded vectors; TEST_LOCK serializes the rest.
    unsafe {
        c_ref_Cmd_AddCommand2(
            leak_str("t9_one"),
            Some(c_trace_1),
            c::cmd_source_t_src_command,
            false,
        );
        c_ref_Cmd_AddCommand2(
            leak_str("t9_two"),
            Some(c_trace_2),
            c::cmd_source_t_src_command,
            false,
        );
        rcmd::Cmd_AddCommand2(
            leak_str("t9_one"),
            Some(r_trace_1),
            c::cmd_source_t_src_command,
            false,
        );
        rcmd::Cmd_AddCommand2(
            leak_str("t9_two"),
            Some(r_trace_2),
            c::cmd_source_t_src_command,
            false,
        );

        // ';' splits within a line, '\n' also splits
        c_ref_Cbuf_AddText(cs("t9_one;t9_two\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("t9_one;t9_two\n").as_ptr());
        Cbuf_Execute();
        assert_eq!(c_trace(), r_trace());
        assert_eq!(c_trace(), vec![1, 2]);

        // Cbuf_InsertText prepends ahead of whatever is still buffered
        clear_traces();
        c_ref_Cbuf_AddText(cs("t9_two\n").as_ptr());
        c_ref_Cbuf_InsertText(cs("t9_one\n").as_ptr());
        rcmd::Cbuf_AddText(cs("t9_two\n").as_ptr());
        Cbuf_InsertText(cs("t9_one\n").as_ptr());
        c_ref_Cbuf_Execute();
        Cbuf_Execute();
        assert_eq!(c_trace(), r_trace());
        assert_eq!(c_trace(), vec![1, 2]);
    }
}

// ---------------------------------------------------------------------------
// 6. Cvar_WriteVariables byte-compare (ARCHIVE cvars, seta/USERDEFINED prefix)

/// SAFETY: reads the whole tmpfile back into a Vec<u8>.
unsafe fn slurp(f: *mut c::FILE) -> Vec<u8> {
    // SAFETY: f is a live FILE* per this function's contract; fread writes at
    // most chunk.len() bytes into the chunk buffer.
    unsafe {
        rewind(f);
        let mut out = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            let n = c::stdio::fread(chunk.as_mut_ptr() as *mut c_void, 1, chunk.len(), f);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..n]);
        }
        out
    }
}

#[test]
fn cvar_write_variables_byte_compare() {
    let _g = lock();
    init_once();

    // SAFETY: new_cvar/cs buffers are leaked or outlive the calls; tmpfile
    // handles are checked non-null before use and closed exactly once.
    unsafe {
        c_ref_Cvar_RegisterVariable(new_cvar("t10_archived", "0", c::cvarflags_t_CVAR_ARCHIVE));
        Cvar_RegisterVariable(new_cvar("t10_archived", "0", c::cvarflags_t_CVAR_ARCHIVE));
        c_ref_Cvar_Set(cs("t10_archived").as_ptr(), cs("hello").as_ptr());
        Cvar_Set(cs("t10_archived").as_ptr(), cs("hello").as_ptr());

        // seta: an ARCHIVE cvar with CVAR_SETA gets the "seta " prefix line
        c_ref_Cbuf_AddText(cs("seta t10_seta 7\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("seta t10_seta 7\n").as_ptr());
        Cbuf_Execute();

        // a non-archived cvar must NOT appear in the output at all
        c_ref_Cvar_RegisterVariable(new_cvar("t10_plain", "9", 0));
        Cvar_RegisterVariable(new_cvar("t10_plain", "9", 0));

        let cf = tmpfile();
        let rf = tmpfile();
        assert!(!cf.is_null() && !rf.is_null(), "tmpfile failed");
        c_ref_Cvar_WriteVariables(cf);
        rcvar::Cvar_WriteVariables(rf);

        let c_bytes = slurp(cf);
        let r_bytes = slurp(rf);
        c::stdio::fclose(cf);
        c::stdio::fclose(rf);

        // Both dumps contain the full registry (built-ins + earlier tests'
        // archived cvars too), so filter to just the lines this test cares
        // about rather than byte-comparing the whole file.
        let c_text = String::from_utf8_lossy(&c_bytes);
        let r_text = String::from_utf8_lossy(&r_bytes);
        fn filt(s: &str) -> Vec<&str> {
            s.lines().filter(|l| l.contains("t10_")).collect()
        }
        assert_eq!(filt(&c_text), filt(&r_text));
        assert!(filt(&c_text).contains(&"t10_archived \"hello\""));
        assert!(filt(&c_text)
            .iter()
            .any(|l| l.starts_with("seta t10_seta \"7\"")));
        assert!(!filt(&c_text).iter().any(|l| l.contains("t10_plain")));
    }
}

// ---------------------------------------------------------------------------
// 7. Cvar_SetQuick semantics: ROM/LOCKED gates, CHANGED flag, alloc reuse,
// default_string toggle

#[test]
fn cvar_set_quick_rom_and_locked_gates() {
    let _g = lock();
    init_once();

    // SAFETY: new_cvar/cs buffers are leaked or outlive the calls; FindVar
    // results are live registry nodes (registered just above).
    unsafe {
        let c_var = new_cvar("t11_rom", "orig", c::cvarflags_t_CVAR_ROM);
        let r_var = new_cvar("t11_rom", "orig", c::cvarflags_t_CVAR_ROM);
        c_ref_Cvar_RegisterVariable(c_var);
        Cvar_RegisterVariable(r_var);

        // CVAR_ROM blocks a plain SetQuick write
        c_ref_Cvar_SetQuick(
            c_ref_Cvar_FindVar(cs("t11_rom").as_ptr()),
            cs("changed").as_ptr(),
        );
        Cvar_SetQuick(
            rcvar::Cvar_FindVar(cs("t11_rom").as_ptr()),
            cs("changed").as_ptr(),
        );
        assert!(!c_ref_Cvar_VariableString(cs("t11_rom").as_ptr()).is_null());
        assert_eq!(
            to_str(c_ref_Cvar_VariableString(cs("t11_rom").as_ptr())),
            to_str(rcvar::Cvar_VariableString(cs("t11_rom").as_ptr()))
        );
        assert_eq!(
            to_str(c_ref_Cvar_VariableString(cs("t11_rom").as_ptr())),
            "orig"
        );

        // CVAR_LOCKED also blocks
        let c_locked = new_cvar("t11_locked", "orig", 0);
        let r_locked = new_cvar("t11_locked", "orig", 0);
        c_ref_Cvar_RegisterVariable(c_locked);
        Cvar_RegisterVariable(r_locked);
        c_ref_Cvar_LockVar(cs("t11_locked").as_ptr());
        rcvar::Cvar_LockVar(cs("t11_locked").as_ptr());
        c_ref_Cvar_SetQuick(
            c_ref_Cvar_FindVar(cs("t11_locked").as_ptr()),
            cs("changed").as_ptr(),
        );
        Cvar_SetQuick(
            rcvar::Cvar_FindVar(cs("t11_locked").as_ptr()),
            cs("changed").as_ptr(),
        );
        assert_eq!(
            to_str(c_ref_Cvar_VariableString(cs("t11_locked").as_ptr())),
            to_str(rcvar::Cvar_VariableString(cs("t11_locked").as_ptr()))
        );
        assert_eq!(
            to_str(c_ref_Cvar_VariableString(cs("t11_locked").as_ptr())),
            "orig"
        );

        // unlocking allows the write through again
        c_ref_Cvar_UnlockVar(cs("t11_locked").as_ptr());
        rcvar::Cvar_UnlockVar(cs("t11_locked").as_ptr());
        c_ref_Cvar_SetQuick(
            c_ref_Cvar_FindVar(cs("t11_locked").as_ptr()),
            cs("now").as_ptr(),
        );
        Cvar_SetQuick(
            rcvar::Cvar_FindVar(cs("t11_locked").as_ptr()),
            cs("now").as_ptr(),
        );
        assert_eq!(
            to_str(c_ref_Cvar_VariableString(cs("t11_locked").as_ptr())),
            to_str(rcvar::Cvar_VariableString(cs("t11_locked").as_ptr()))
        );
        assert_eq!(
            to_str(c_ref_Cvar_VariableString(cs("t11_locked").as_ptr())),
            "now"
        );
    }
}

#[test]
fn cvar_set_quick_changed_flag_and_default_string() {
    let _g = lock();
    init_once();

    const CHANGED: c::cvarflags_t = c::cvarflags_t_CVAR_CHANGED;
    // SAFETY: new_cvar/cs buffers are leaked or outlive the calls; FindVar
    // results are live registry nodes; ctest_set_host_initialized only
    // toggles a global flag and is restored before the test ends.
    unsafe {
        let c_var = new_cvar("t12_flag", "0", 0);
        let r_var = new_cvar("t12_flag", "0", 0);
        c_ref_Cvar_RegisterVariable(c_var);
        Cvar_RegisterVariable(r_var);

        // no change: same value string -> CHANGED flag NOT set
        c_ref_Cvar_SetQuick(c_ef_findvar_c("t12_flag"), cs("0").as_ptr());
        Cvar_SetQuick(
            rcvar::Cvar_FindVar(cs("t12_flag").as_ptr()),
            cs("0").as_ptr(),
        );
        assert_eq!((*c_ef_findvar_c("t12_flag")).flags & CHANGED, 0);
        assert_eq!(
            (*rcvar::Cvar_FindVar(cs("t12_flag").as_ptr())).flags & CHANGED,
            0
        );

        // actual change -> CHANGED flag set on both sides
        c_ref_Cvar_SetQuick(c_ef_findvar_c("t12_flag"), cs("1").as_ptr());
        Cvar_SetQuick(
            rcvar::Cvar_FindVar(cs("t12_flag").as_ptr()),
            cs("1").as_ptr(),
        );
        assert_ne!((*c_ef_findvar_c("t12_flag")).flags & CHANGED, 0);
        assert_ne!(
            (*rcvar::Cvar_FindVar(cs("t12_flag").as_ptr())).flags & CHANGED,
            0
        );

        // default_string tracks the latest Set while !host_initialized (per
        // cvar.c's "during initialization, update default too"); it is only
        // captured once and frozen after host_initialized becomes true.
        assert_eq!(
            to_str((*c_ef_findvar_c("t12_flag")).default_string),
            to_str((*rcvar::Cvar_FindVar(cs("t12_flag").as_ptr())).default_string)
        );
        assert_eq!(to_str((*c_ef_findvar_c("t12_flag")).default_string), "1");

        // once host_initialized flips true, default_string freezes even
        // though further Sets still update ->string and flip CVAR_CHANGED
        ctest_set_host_initialized(true);
        c_ref_Cvar_SetQuick(c_ef_findvar_c("t12_flag"), cs("2").as_ptr());
        Cvar_SetQuick(
            rcvar::Cvar_FindVar(cs("t12_flag").as_ptr()),
            cs("2").as_ptr(),
        );
        assert_eq!(to_str((*c_ef_findvar_c("t12_flag")).string), "2");
        assert_eq!(
            to_str((*rcvar::Cvar_FindVar(cs("t12_flag").as_ptr())).string),
            "2"
        );
        assert_eq!(to_str((*c_ef_findvar_c("t12_flag")).default_string), "1");
        assert_eq!(
            to_str((*rcvar::Cvar_FindVar(cs("t12_flag").as_ptr())).default_string),
            "1"
        );
        ctest_set_host_initialized(false);
    }
}

unsafe fn c_ef_findvar_c(name: &str) -> *mut c::cvar_t {
    // SAFETY: cs() yields a NUL-terminated buffer that outlives the call.
    unsafe { c_ref_Cvar_FindVar(cs(name).as_ptr()) }
}

// ---------------------------------------------------------------------------
// 8. Cvar_SetValue formatting parity + Cvar_Create

#[test]
fn cvar_set_value_formatting_parity() {
    let _g = lock();
    init_once();

    // SAFETY: new_cvar/cs buffers are leaked or outlive the calls; TEST_LOCK
    // serializes access to both global registries.
    unsafe {
        c_ref_Cvar_RegisterVariable(new_cvar("t13_val", "0", 0));
        Cvar_RegisterVariable(new_cvar("t13_val", "0", 0));

        for v in [0.0f32, 1.0, -1.0, 3.5, 0.1, 100000.0, -0.0001, 123456.79] {
            c_ref_Cvar_SetValue(cs("t13_val").as_ptr(), v);
            Cvar_SetValue(cs("t13_val").as_ptr(), v);
            assert_eq!(
                to_str(c_ref_Cvar_VariableString(cs("t13_val").as_ptr())),
                to_str(rcvar::Cvar_VariableString(cs("t13_val").as_ptr())),
                "value: {v}"
            );
        }
    }
}

#[test]
fn cvar_create_existing_vs_new_vs_command_collision() {
    let _g = lock();
    init_once();

    // SAFETY: cs/leak_str buffers outlive the calls; non-null Cvar_Create
    // results are live (leaked) registry nodes safe to dereference.
    unsafe {
        // brand new
        let c_new = c_ref_Cvar_Create(cs("t14_new").as_ptr(), cs("5").as_ptr());
        let r_new = Cvar_Create(cs("t14_new").as_ptr(), cs("5").as_ptr());
        assert_eq!(c_new.is_null(), r_new.is_null());
        assert!(!c_new.is_null());
        assert_eq!(
            (*c_new).flags & c::cvarflags_t_CVAR_USERDEFINED,
            (*r_new).flags & c::cvarflags_t_CVAR_USERDEFINED
        );

        // already exists -> returns the existing node, no new registration
        let c_again = c_ref_Cvar_Create(cs("t14_new").as_ptr(), cs("999").as_ptr());
        let r_again = Cvar_Create(cs("t14_new").as_ptr(), cs("999").as_ptr());
        assert_eq!(
            to_str(c_ref_Cvar_VariableString(cs("t14_new").as_ptr())),
            "5"
        );
        assert_eq!(
            to_str(rcvar::Cvar_VariableString(cs("t14_new").as_ptr())),
            "5"
        );
        assert!(!c_again.is_null() && !r_again.is_null());

        // name collides with an existing command -> NULL
        c_ref_Cmd_AddCommand2(
            leak_str("t14_cmd"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        rcmd::Cmd_AddCommand2(
            leak_str("t14_cmd"),
            Some(noop_handler),
            c::cmd_source_t_src_command,
            false,
        );
        let c_collide = c_ref_Cvar_Create(cs("t14_cmd").as_ptr(), cs("1").as_ptr());
        let r_collide = Cvar_Create(cs("t14_cmd").as_ptr(), cs("1").as_ptr());
        assert!(c_collide.is_null() && r_collide.is_null());
    }
}

// ---------------------------------------------------------------------------
// 9. Cmd_ExecuteString srctype filtering

#[test]
fn cmd_execute_string_srctype_filtering() {
    let _g = lock();
    init_once();

    clear_traces();
    // SAFETY: leak_str names live for the process; host_client is pointed at
    // a real leaked client_t before the src_client path dereferences its
    // name, and reset to null before the test ends.
    unsafe {
        c_ref_Cmd_AddCommand2(
            leak_str("t15_srv"),
            Some(c_trace_1),
            c::cmd_source_t_src_server,
            false,
        );
        rcmd::Cmd_AddCommand2(
            leak_str("t15_srv"),
            Some(r_trace_1),
            c::cmd_source_t_src_server,
            false,
        );

        // src_command may not run a src_server command
        let c_ok = c_ref_Cmd_ExecuteString(cs("t15_srv").as_ptr(), c::cmd_source_t_src_command);
        let r_ok = Cmd_ExecuteString(cs("t15_srv").as_ptr(), c::cmd_source_t_src_command);
        assert_eq!(c_ok, r_ok);
        assert_eq!(c_trace(), r_trace());
        assert!(c_trace().is_empty());

        // src_server may run it
        let c_ok = c_ref_Cmd_ExecuteString(cs("t15_srv").as_ptr(), c::cmd_source_t_src_server);
        let r_ok = Cmd_ExecuteString(cs("t15_srv").as_ptr(), c::cmd_source_t_src_server);
        assert_eq!(c_ok, r_ok);
        assert_eq!(c_trace(), r_trace());
        assert_eq!(c_trace(), vec![1]);

        // src_client running anything but a src_client command still runs it,
        // but first prints a "tried to" warning via `host_client->name` --
        // point host_client at a real allocation so that dereference is safe.
        let client = ctest_make_client_with_name(leak_str("t15_client"));
        ctest_set_host_client(client);

        // src_client running anything but a src_client command still runs it
        // (after printing a DPrintf, which this test doesn't capture)
        clear_traces();
        c_ref_Cmd_AddCommand2(
            leak_str("t15_cmd"),
            Some(c_trace_2),
            c::cmd_source_t_src_command,
            false,
        );
        rcmd::Cmd_AddCommand2(
            leak_str("t15_cmd"),
            Some(r_trace_2),
            c::cmd_source_t_src_command,
            false,
        );
        let c_ok = c_ref_Cmd_ExecuteString(cs("t15_cmd").as_ptr(), c::cmd_source_t_src_client);
        let r_ok = Cmd_ExecuteString(cs("t15_cmd").as_ptr(), c::cmd_source_t_src_client);
        assert_eq!(c_ok, r_ok);
        assert_eq!(c_trace(), r_trace());
        assert_eq!(c_trace(), vec![2]);

        // unknown command -> false on both, and Cvar_Command is consulted
        // (falls through to "unknown command" when it's not a cvar either)
        let c_ok =
            c_ref_Cmd_ExecuteString(cs("t15_nonexistent").as_ptr(), c::cmd_source_t_src_command);
        let r_ok = Cmd_ExecuteString(cs("t15_nonexistent").as_ptr(), c::cmd_source_t_src_command);
        assert_eq!(c_ok, r_ok);

        // leave host_client null again for any later test
        ctest_set_host_client(core::ptr::null_mut());
    }
}

// ---------------------------------------------------------------------------
// 10. Cmd_CheckParm

#[test]
fn cmd_checkparm_matches() {
    let _g = lock();
    init_once();

    // SAFETY: cs buffers are NUL-terminated and outlive the calls; TEST_LOCK
    // serializes access to both sides' tokenizer state.
    unsafe {
        c_ref_Cmd_TokenizeString(cs("cmd -foo bar -baz").as_ptr());
        rcmd::Cmd_TokenizeString(cs("cmd -foo bar -baz").as_ptr());

        for parm in ["-foo", "-baz", "-missing", "bar"] {
            let c_idx = c_ref_Cmd_CheckParm(cs(parm).as_ptr());
            let r_idx = rcmd::Cmd_CheckParm(cs(parm).as_ptr());
            assert_eq!(c_idx, r_idx, "parm: {parm}");
        }
    }
}

// ---------------------------------------------------------------------------
// extern declarations for the raise-capable plain wrappers (stubs.c-owned;
// see rust/quake-ctest/stubs/stubs.c's "plain Cvar_/Cbuf_/Cmd_ wrapper"
// block). These are distinct linker symbols from the c_ref_* ones above:
// the shared force-include renames Quake/cvar.c's and Quake/cmd.c's OWN
// definitions to c_ref_*, leaving the plain names free for these
// hand-written wrappers that reraise from a pure C frame per ADR-009.
extern "C" {
    fn Cbuf_Execute();
    fn Cbuf_InsertText(text: *const c_char);
    fn Cmd_ExecuteString(text: *const c_char, src: c::cmd_source_t) -> c::qboolean;
    fn Cvar_RegisterVariable(variable: *mut c::cvar_t);
    fn Cvar_SetQuick(var: *mut c::cvar_t, value: *const c_char);
    fn Cvar_Set(var_name: *const c_char, value: *const c_char);
    fn Cvar_SetValue(var_name: *const c_char, value: c_float);
    fn Cvar_Create(name: *const c_char, value: *const c_char) -> *mut c::cvar_t;
    /// Allocates (and leaks) a `client_t` with `->name` set, and returns it.
    /// `cmd.c`'s `Cmd_ExecuteString` dereferences `host_client->name`
    /// unconditionally on the `src_client` "tried to" warning path, so any
    /// test exercising that path must point `host_client` (via
    /// `ctest_set_host_client`) at a real allocation first, or it segfaults.
    fn ctest_make_client_with_name(name: *const c_char) -> *mut c_void;
    fn ctest_set_host_client(c: *mut c_void);
    /// Toggles `host_initialized`, which gates `Cvar_RegisterVariable`'s
    /// name-copy strategy and `Cvar_SetQuick`'s default_string freeze.
    fn ctest_set_host_initialized(v: c::qboolean);
}

// ---------------------------------------------------------------------------
// 11. Console command handlers, driven end-to-end through Cbuf_AddText /
// Cbuf_Execute.
//
// Everything above tests the cvar/cmd *API* directly; the `xcommand_t`
// handlers that cvar.c's Cvar_Init and cmd.c's Cmd_Init register (inc,
// toggle, cycle, reset, resetall, resetcfg, set/seta, cvarlist, cmdlist,
// apropos, echo, wait, exec, stuffcmds, alias, unalias, unaliasall) were
// entirely uncovered. They are the surface users actually reach, they are
// where the Rust port routes its raises through PENDING_RAISE rather than
// returning a status, and the `inc` double->float narrowing bug found in
// review lived here. Each line is fed to both sides in the same order, and
// after every line both the console output and the tracked cvar values are
// compared.

extern "C" {
    fn c_ref_Cbuf_Waited();
    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
}

/// SAFETY: caller holds TEST_LOCK; the log holds NUL-terminated C strings
/// owned by the stub.
unsafe fn con_snapshot() -> Vec<String> {
    // SAFETY: see the function contract.
    unsafe {
        let n = ctest_con_log_len();
        (0..n).map(|i| to_str(ctest_con_log_get(i))).collect()
    }
}

const T18_CVARS: &[&str] = &["t18_a", "t18_b", "t18_big", "t18_new", "t18_seta"];

#[test]
fn console_command_handlers_match() {
    let _g = lock();
    init_once();

    // SAFETY: leaked cvar_t storage stays valid for the process; TEST_LOCK
    // serializes both registries and the shared console log; every string
    // passed in is NUL-terminated and outlives its call.
    unsafe {
        c_ref_Cvar_RegisterVariable(new_cvar("t18_a", "0", 0));
        Cvar_RegisterVariable(new_cvar("t18_a", "0", 0));
        c_ref_Cvar_RegisterVariable(new_cvar("t18_b", "1", c::cvarflags_t_CVAR_ARCHIVE));
        Cvar_RegisterVariable(new_cvar("t18_b", "1", c::cvarflags_t_CVAR_ARCHIVE));
        c_ref_Cvar_RegisterVariable(new_cvar("t18_big", "0", 0));
        Cvar_RegisterVariable(new_cvar("t18_big", "0", 0));

        // stuffcmds reads the one shared `cmdline` cvar_t (stubs.c-owned), so
        // both sides see the same input; give it something with a '+' run, a
        // '-' terminator and a leading argv[0] to exercise the whole loop.
        let saved_cmdline = c::cmdline.string;
        c::cmdline.string = leak_str("vkquake +echo one +echo two -window +echo three");

        let lines: &[&str] = &[
            // echo / usage lines
            "echo hello world\n",
            "echo\n",
            // inc: usage, unit step, explicit amount, and the review
            // regression -- 16777217 is the first integer f32 cannot hold, so
            // a double->float narrowing that happens twice diverges here.
            "inc\n",
            "inc t18_a\n",
            "inc t18_a 2.5\n",
            "set t18_big 16777217\n",
            "inc t18_big\n",
            "inc t18_missing\n",
            // toggle: usage, missing var, numeric flip, explicit value pair
            "toggle\n",
            "toggle t18_missing\n",
            "toggle t18_a\n",
            "toggle t18_a\n",
            "toggle t18_a on off\n",
            "toggle t18_a on off\n",
            "toggle t18_a on\n",
            // cycle: usage, no match, match in the middle, match at the end
            "cycle t18_b\n",
            "cycle t18_b 1 2 3\n",
            "cycle t18_b 1 2 3\n",
            "cycle t18_b 1 2 3\n",
            "cycle t18_b 1 2 3\n",
            "cycle t18_b zero one\n",
            // set / seta: usage, extra args, creation, archive flagging
            "set t18_new\n",
            "set t18_new 5\n",
            "set t18_new a b c\n",
            "seta t18_seta 7\n",
            // listings (scoped by prefix so unrelated tests' registrations
            // cannot perturb the output)
            "cvarlist t18_\n",
            "cmdlist t18_\n",
            "cmdlist unalias\n",
            "apropos t18_\n",
            "apropos t18_nothing_matches_this\n",
            "apropos\n",
            // aliases through the buffer
            "alias t18_al \"inc t18_a\"\n",
            "t18_al\n",
            "alias t18_al \"inc t18_a 10\"\n",
            "t18_al\n",
            "unalias t18_al\n",
            "t18_al\n",
            "unalias\n",
            "unalias t18_al\n",
            "unaliasall\n",
            // exec of a file that does not exist
            "exec t18_missing.cfg\n",
            "exec\n",
            // stuffcmds expands cmdline into the buffer
            "stuffcmds\n",
            // wait defers the rest of the buffer to the next Cbuf_Execute
            "echo before; wait; echo after\n",
            // resets
            "reset\n",
            "reset t18_a\n",
            "reset t18_missing\n",
            "resetcfg\n",
            "resetall\n",
        ];

        for line in lines {
            let text = cs(line);

            ctest_clear_con_log();
            c_ref_Cbuf_AddText(text.as_ptr());
            c_ref_Cbuf_Execute();
            // `wait` latches cmd_wait, which Cbuf_Execute never clears --
            // host.c does, once per frame, via Cbuf_Waited. Emulate one frame
            // boundary so the rest of the buffer drains (and so a stuck
            // cmd_wait cannot leak into the next line or the next test).
            c_ref_Cbuf_Waited();
            c_ref_Cbuf_Execute();
            let c_log = con_snapshot();
            let c_state: Vec<String> = T18_CVARS
                .iter()
                .map(|n| to_str(c_ref_Cvar_VariableString(cs(n).as_ptr())))
                .collect();

            ctest_clear_con_log();
            rcmd::Cbuf_AddText(text.as_ptr());
            Cbuf_Execute();
            rcmd::Cbuf_Waited();
            Cbuf_Execute();
            let r_log = con_snapshot();
            let r_state: Vec<String> = T18_CVARS
                .iter()
                .map(|n| to_str(rcvar::Cvar_VariableString(cs(n).as_ptr())))
                .collect();

            assert_eq!(c_log, r_log, "console output for {line:?}");
            assert_eq!(c_state, r_state, "cvar state after {line:?}");
        }

        // The regression the review found: C evaluates
        // Cvar_SetValue (name, Cvar_VariableValue (name) + 1) with a double
        // add and ONE narrowing at the call. 16777217 + 1 narrowed once is
        // 16777218; narrowing to f32 first would give 16777216 and then
        // 16777216 again, so the value would never move.
        c_ref_Cvar_Set(cs("t18_big").as_ptr(), cs("16777217").as_ptr());
        Cvar_Set(cs("t18_big").as_ptr(), cs("16777217").as_ptr());
        c_ref_Cbuf_AddText(cs("inc t18_big\n").as_ptr());
        c_ref_Cbuf_Execute();
        rcmd::Cbuf_AddText(cs("inc t18_big\n").as_ptr());
        Cbuf_Execute();
        let c_big = to_str(c_ref_Cvar_VariableString(cs("t18_big").as_ptr()));
        let r_big = to_str(rcvar::Cvar_VariableString(cs("t18_big").as_ptr()));
        assert_eq!(c_big, r_big, "inc narrowing");
        assert_ne!(c_big, "16777217", "inc must move the value at all");

        c::cmdline.string = saved_cmdline;
    }
}
