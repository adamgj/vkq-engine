//! Differential test: `quake_rs::keys` vs the original `Quake/keys.c`. Phase 7
//! M10b.
//!
//! `keys.c` is composed into `stubs/keys_ref.c` behind a TU-local rename block
//! (rather than listed in `build.rs`'s `C_SOURCES`) because four of the symbols
//! it owns already have link doubles that `tests/host_differential.rs` asserts
//! on; the reasoning is written out at the top of that file. What matters here
//! is the consequence: the oracle answers to `c_ref_*` and the port answers to
//! the plain names, and the two halves own **disjoint** copies of every datum
//! `keys.c` used to define -- `key_lines`, `key_linepos`, `key_insert`,
//! `edit_line`, `history_line`, `keybindings`, `consolekeys`, `menubound`,
//! `keydown`, `chat_team`, `keynames`, plus the chat buffer and input-grab
//! statics inside each half.
//!
//! Three groups of state are NOT disjoint and every test must treat them as
//! shared:
//!
//!  * `key_dest` (`stubs.c:2717`), because nineteen files read it and
//!    `keys.c` is the only writer;
//!  * the console mirror (`con_text`, `con_current`, ..., `glheight`), which
//!    `stubs/keys_ref.c` defines once;
//!  * the call recorder behind `Con_TabComplete`, `Con_Scroll`, `M_Keydown`,
//!    `VID_Toggle`, `SCR_UpdateScreen`, `IN_*`, ... -- console.c, menu.c,
//!    in_sdl.c and gl_vidsdl.c are not oracle sources, so BOTH sides call the
//!    same double.
//!
//! Because of that, every test below follows the `run_both_probed` shape: for
//! each side in turn, seed the shared state, reset the recorder, invoke, then
//! read the observation and the recorder **immediately**, before the other
//! side runs. Reading a shared fixture after both sides have run is the defect
//! that made seven M9f tests vacuous.
//!
//! Cbuf and Cmd are per-side by construction (the oracle's `cmd.c` registry
//! versus quake-capi's), the same disjoint-registry arrangement
//! `cvar_cmd_differential.rs` documents, so `+`/`-` command synthesis is
//! observed by registering a probe command on each registry and executing that
//! side's buffer.
//!
//! ADR-009: the raising entry points are called through their plain names, so
//! the port goes port core -> `Host_Reraise` exactly as `Quake/keys_glue.c`
//! does in the engine build. None of the scenarios here make a guarded callee
//! raise (the doubles only record), so `Host_Guard` always answers "no raise"
//! on both sides.

use core::ffi::{c_char, c_double, c_float, c_int, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_c_sys as c;
use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_rs::cmd as rcmd;
use quake_rs::cvar as rcvar;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `stubs/keys_ref.c`'s side convention: 1 = the `c_ref_*` oracle, 0 = the
/// Rust port.
const C: c_int = 1;
const R: c_int = 0;
const SIDES: [c_int; 2] = [C, R];

// keys.h:32-129 -- the keycodes the scenarios below name.
const K_TAB: c_int = 9;
const K_ENTER: c_int = 13;
const K_ESCAPE: c_int = 27;
const K_BACKSPACE: c_int = 127;
const K_UPARROW: c_int = 128;
const K_DOWNARROW: c_int = 129;
const K_LEFTARROW: c_int = 130;
const K_RIGHTARROW: c_int = 131;
const K_ALT: c_int = 132;
const K_CTRL: c_int = 133;
const K_SHIFT: c_int = 134;
const K_F1: c_int = 135;
const K_INS: c_int = 147;
const K_DEL: c_int = 148;
const K_PGUP: c_int = 150;
const K_HOME: c_int = 151;
const K_END: c_int = 152;
const K_MOUSE1: c_int = 200;

// keys.h:135-140 -- keydest_t
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
const KEY_MESSAGE: c_int = 2;
const KEY_MENU: c_int = 3;

const MAXCMDLINE: usize = 256;
const MAX_KEYS: c_int = 256;

// ---------------------------------------------------------------------------
// FFI

/// `stubs/keys_ref.c`'s `ctest_keys_state_t`.
#[repr(C)]
#[derive(Clone, Copy)]
struct KeysState {
    line: [c_char; MAXCMDLINE],
    tabhint: [c_char; MAXCMDLINE],
    chat: [c_char; MAXCMDLINE],
    lpos: c_int,
    ins: c_int,
    eline: c_int,
    hline: c_int,
    dest: c_int,
    team: c_int,
    clen: c_int,
    textentry: c_int,
    grabkey: c_int,
    grabchar: c_int,
    blink: c_double,
}

impl Default for KeysState {
    fn default() -> Self {
        KeysState {
            line: [0; MAXCMDLINE],
            tabhint: [0; MAXCMDLINE],
            chat: [0; MAXCMDLINE],
            lpos: 0,
            ins: 0,
            eline: 0,
            hline: 0,
            dest: 0,
            team: 0,
            clen: 0,
            textentry: 0,
            grabkey: 0,
            grabchar: 0,
            blink: 0.0,
        }
    }
}

/// A `KeysState` rendered as comparable, printable Rust values.
#[derive(Debug, PartialEq, Clone)]
struct State {
    line: String,
    tabhint: String,
    chat: String,
    lpos: c_int,
    ins: c_int,
    eline: c_int,
    hline: c_int,
    dest: c_int,
    team: c_int,
    clen: c_int,
    textentry: c_int,
    grabkey: c_int,
    grabchar: c_int,
    blink: c_double,
}

/// `stubs/keys_ref.c`'s `ctest_keys_calls_t`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct KeysCalls {
    updatescreen_calls: c_int,
    tabcomplete_calls: c_int,
    tabcomplete_mode: c_int,
    scroll_calls: c_int,
    scroll_lines: c_int,
    forcemousemove_calls: c_int,
    selectall_calls: c_int,
    copyselection_calls: c_int,
    toggleconsole_calls: c_int,
    menukeydown_calls: c_int,
    menukeydown_key: c_int,
    menucharinput_calls: c_int,
    menucharinput_key: c_int,
    togglemenu_calls: c_int,
    vidtoggle_calls: c_int,
    updateinputmode_calls: c_int,
    activate_calls: c_int,
    deactivateforconsole_calls: c_int,
    clipboard_calls: c_int,
}

#[allow(dead_code)]
extern "C" {
    // ---- the oracle (Quake/keys.c, renamed inside stubs/keys_ref.c) -------
    fn c_ref_Key_Console(key: c_int);
    fn c_ref_Char_Console(key: c_int);
    fn c_ref_Key_Event(key: c_int, down: bool);
    fn c_ref_Key_EventWithKeycode(key: c_int, down: bool, keycode: c_int);
    fn c_ref_Char_Event(key: c_int);
    fn c_ref_Key_Message(key: c_int);
    fn c_ref_Char_Message(key: c_int);
    fn c_ref_Key_EndChat();
    fn c_ref_Key_StringToKeynum(str_: *const c_char) -> c_int;
    fn c_ref_Key_KeynumToString(keynum: c_int) -> *const c_char;
    fn c_ref_Key_SetBinding(keynum: c_int, binding: *const c_char);
    fn c_ref_Key_Unbind_f();
    fn c_ref_Key_Unbindall_f();
    fn c_ref_Key_Bindlist_f();
    fn c_ref_Key_Bind_f();
    fn c_ref_Key_WriteBindings(f: *mut c::FILE);
    fn c_ref_Key_Init();
    fn c_ref_History_Init();
    fn c_ref_History_Shutdown();
    fn c_ref_Key_TextEntry() -> bool;
    fn c_ref_Key_ClearStates();
    fn c_ref_Key_BeginInputGrab();
    fn c_ref_Key_EndInputGrab();
    fn c_ref_Key_UpdateForDest();

    // ---- the port (quake-capi/src/keys.rs + stubs/keys_ref.c's glue half) -
    fn Key_Console(key: c_int);
    fn Char_Console(key: c_int);
    fn Key_Event(key: c_int, down: bool);
    fn Key_EventWithKeycode(key: c_int, down: bool, keycode: c_int);
    fn Char_Event(key: c_int);
    fn Key_Message(key: c_int);
    fn Char_Message(key: c_int);
    fn Key_EndChat();
    fn Key_StringToKeynum(str_: *const c_char) -> c_int;
    fn Key_KeynumToString(keynum: c_int) -> *const c_char;
    fn Key_SetBinding(keynum: c_int, binding: *const c_char);
    fn Key_Unbind_f();
    fn Key_Unbindall_f();
    fn Key_Bindlist_f();
    fn Key_Bind_f();
    fn Key_WriteBindings(f: *mut c::FILE);
    fn Key_Init();
    fn History_Init();
    fn History_Shutdown();
    fn Key_TextEntry() -> bool;
    fn Key_ClearStates();
    fn Key_BeginInputGrab();
    fn Key_EndInputGrab();
    fn Key_UpdateForDest();

    // ---- the fixture (stubs/keys_ref.c) ----------------------------------
    fn ctest_keys_reset(side: c_int);
    fn ctest_keys_set_line(side: c_int, line: c_int, text: *const c_char);
    fn ctest_keys_set_edit(side: c_int, eline: c_int, hline: c_int, linepos: c_int, insert: c_int);
    fn ctest_keys_snapshot(side: c_int, out: *mut KeysState);
    fn ctest_keys_get_line(side: c_int, line: c_int, out: *mut c_char, cap: c_int);
    fn ctest_keys_binding(side: c_int, keynum: c_int) -> *const c_char;
    fn ctest_keys_flags(side: c_int, keynum: c_int) -> c_int;
    fn ctest_keys_set_keydown(side: c_int, keynum: c_int, value: c_int);
    fn ctest_keys_set_consolekey(side: c_int, keynum: c_int, value: c_int);
    fn ctest_keys_set_menubound(side: c_int, keynum: c_int, value: c_int);
    fn ctest_keys_set_cls_state(side: c_int, state: c_int);
    fn ctest_keys_set_demo(side: c_int, playback: c_int, paused: c_int, speed: c_float);
    fn ctest_keys_demospeed(side: c_int) -> c_float;
    fn ctest_keys_demopaused(side: c_int) -> c_int;
    fn ctest_keys_set_dest(dest: c_int);
    fn ctest_keys_get_dest() -> c_int;
    #[allow(clippy::too_many_arguments)]
    fn ctest_keys_set_con(
        text: *mut c_char,
        current: c_int,
        linewidth: c_int,
        vislines: c_int,
        totallines: c_int,
        backscroll: c_int,
        forcedup: c_int,
        height: c_int,
    );
    fn ctest_keys_con_backscroll() -> c_int;
    fn ctest_keys_probe_reset();
    fn ctest_keys_probe_get(out: *mut KeysCalls);
    fn ctest_keys_set_menu(text_entry: c_int, waiting_for_key_binding: c_int, is_quitting: c_int);
    fn ctest_keys_set_copyselection_result(value: c_int);
    fn ctest_keys_set_clipboard(text: *const c_char);
    fn ctest_keys_set_gamedir(side: c_int, dir: *const c_char);
    fn ctest_keys_set_cfg_unbindall(side: c_int, value: c_float);
    fn ctest_keys_keyname_count(side: c_int) -> c_int;
    fn ctest_keys_keyname(side: c_int, i: c_int) -> *const c_char;
    fn ctest_keys_keyname_num(side: c_int, i: c_int) -> c_int;

    // ---- cmd/cbuf, per-side registries -----------------------------------
    fn c_ref_Cvar_Init();
    fn c_ref_Cmd_Init();
    fn c_ref_Cbuf_Init();
    fn c_ref_Cmd_TokenizeString(text: *const c_char);
    fn c_ref_Cmd_Argc() -> c_int;
    fn c_ref_Cmd_Argv(arg: c_int) -> *const c_char;
    fn c_ref_Cmd_AddCommand2(
        cmd_name: *const c_char,
        function: c::xcommand_t,
        srctype: c::cmd_source_t,
        qcinterceptable: c::qboolean,
    ) -> *mut c::cmd_function_t;
    fn c_ref_Cbuf_Execute();
    fn Cbuf_Execute();

    fn tmpfile() -> *mut c::FILE;
    fn rewind(f: *mut c::FILE);
}

// ---------------------------------------------------------------------------
// helpers

fn cs(s: &str) -> Vec<c_char> {
    let mut v: Vec<c_char> = s.bytes().map(|b| b as c_char).collect();
    v.push(0);
    v
}

fn leak_str(s: &str) -> *const c_char {
    Box::leak(cs(s).into_boxed_slice()).as_ptr()
}

/// SAFETY: `p` must be NUL-terminated or null.
unsafe fn to_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: caller guarantees NUL termination.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

fn cbuf_to_string(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn init_once() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: each init only touches its own side's registry; every caller
        // holds TEST_LOCK.
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

/// Puts both sides back to a known state and clears the shared console mirror,
/// the shared `key_dest` and the shared call recorder.
fn reset_all() {
    // SAFETY: fixture calls; no arguments outlive the call.
    unsafe {
        for side in SIDES {
            ctest_keys_reset(side);
            ctest_keys_set_cls_state(side, 0); // ca_dedicated: not ca_disconnected
            ctest_keys_set_demo(side, 0, 0, 1.0);
            ctest_keys_set_cfg_unbindall(side, 0.0);
        }
        ctest_keys_set_dest(KEY_GAME);
        ctest_keys_set_menu(0, 0, 0);
        ctest_keys_set_copyselection_result(0);
        ctest_keys_set_clipboard(std::ptr::null());
        ctest_keys_set_con(std::ptr::null_mut(), 0, 0, 0, 0, 0, 0, 0);
        ctest_keys_probe_reset();
    }
    quake_ctest::fs::clear_logs();
}

fn snapshot(side: c_int) -> State {
    let mut raw = KeysState::default();
    // SAFETY: `raw` is a live, correctly-shaped ctest_keys_state_t.
    unsafe { ctest_keys_snapshot(side, &mut raw) };
    State {
        line: cbuf_to_string(&raw.line),
        tabhint: cbuf_to_string(&raw.tabhint),
        chat: cbuf_to_string(&raw.chat),
        lpos: raw.lpos,
        ins: raw.ins,
        eline: raw.eline,
        hline: raw.hline,
        dest: raw.dest,
        team: raw.team,
        clen: raw.clen,
        textentry: raw.textentry,
        grabkey: raw.grabkey,
        grabchar: raw.grabchar,
        blink: raw.blink,
    }
}

fn probe() -> KeysCalls {
    let mut out = KeysCalls::default();
    // SAFETY: `out` is a live, correctly-shaped ctest_keys_calls_t.
    unsafe { ctest_keys_probe_get(&mut out) };
    out
}

fn con_log() -> Vec<String> {
    quake_ctest::fs::con_log()
}

fn set_line(side: c_int, line: c_int, text: &str) {
    let t = cs(text);
    // SAFETY: NUL-terminated; the fixture copies.
    unsafe { ctest_keys_set_line(side, line, t.as_ptr()) };
}

fn get_line(side: c_int, line: c_int) -> String {
    let mut buf = [0 as c_char; MAXCMDLINE];
    // SAFETY: `buf` is MAXCMDLINE bytes and the cap says so.
    unsafe { ctest_keys_get_line(side, line, buf.as_mut_ptr(), MAXCMDLINE as c_int) };
    cbuf_to_string(&buf)
}

fn binding(side: c_int, keynum: c_int) -> Option<String> {
    // SAFETY: the fixture returns the live keybindings[] entry or NULL.
    unsafe { to_str(ctest_keys_binding(side, keynum)) }
}

fn set_binding(side: c_int, keynum: c_int, value: Option<&str>) {
    let v = value.map(cs);
    let p = v.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
    // SAFETY: NUL-terminated or NULL; keys.c q_strdup's it.
    unsafe {
        if side == C {
            c_ref_Key_SetBinding(keynum, p)
        } else {
            Key_SetBinding(keynum, p)
        }
    }
}

/// Tokenizes into the given side's `cmd_argv`, then runs that side's command.
fn run_cmd(side: c_int, text: &str, f: fn(c_int)) {
    let t = cs(text);
    // SAFETY: NUL-terminated; the tokenizer copies into that side's argv.
    unsafe {
        if side == C {
            c_ref_Cmd_TokenizeString(t.as_ptr())
        } else {
            rcmd::Cmd_TokenizeString(t.as_ptr())
        }
    }
    f(side);
}

fn bind_f(side: c_int) {
    // SAFETY: reads that side's tokenized argv only.
    unsafe {
        if side == C {
            c_ref_Key_Bind_f()
        } else {
            Key_Bind_f()
        }
    }
}

fn unbind_f(side: c_int) {
    // SAFETY: reads that side's tokenized argv only.
    unsafe {
        if side == C {
            c_ref_Key_Unbind_f()
        } else {
            Key_Unbind_f()
        }
    }
}

fn key_console(side: c_int, key: c_int) {
    // SAFETY: the plain entry point is the glue's re-raising wrapper.
    unsafe {
        if side == C {
            c_ref_Key_Console(key)
        } else {
            Key_Console(key)
        }
    }
}

fn char_console(side: c_int, key: c_int) {
    // SAFETY: as above.
    unsafe {
        if side == C {
            c_ref_Char_Console(key)
        } else {
            Char_Console(key)
        }
    }
}

fn key_event(side: c_int, key: c_int, down: bool) {
    // SAFETY: as above.
    unsafe {
        if side == C {
            c_ref_Key_Event(key, down)
        } else {
            Key_Event(key, down)
        }
    }
}

/// Shared `key_dest` (`stubs.c:2717`) -- both sides write the one object.
fn dest() -> c_int {
    // SAFETY: fixture read of a plain int.
    unsafe { ctest_keys_get_dest() }
}

/// bit 0 keydown, bit 1 consolekeys, bit 2 menubound, for that side's tables.
fn flags(side: c_int, key: c_int) -> c_int {
    // SAFETY: fixture read; `key` is bounded by MAX_KEYS at every call site.
    unsafe { ctest_keys_flags(side, key) }
}

fn demospeed(side: c_int) -> c_float {
    // SAFETY: fixture read of that side's cls.demospeed.
    unsafe { ctest_keys_demospeed(side) }
}

fn demopaused(side: c_int) -> c_int {
    // SAFETY: fixture read of that side's cls.demopaused.
    unsafe { ctest_keys_demopaused(side) }
}

fn set_keydown(side: c_int, key: c_int, down: bool) {
    // SAFETY: fixture write into that side's keydown[].
    unsafe { ctest_keys_set_keydown(side, key, i32::from(down)) };
}

fn set_consolekey(side: c_int, key: c_int) {
    // SAFETY: fixture write into that side's consolekeys[].
    unsafe { ctest_keys_set_consolekey(side, key, 1) };
}

fn set_menubound(side: c_int, key: c_int) {
    // SAFETY: fixture write into that side's menubound[].
    unsafe { ctest_keys_set_menubound(side, key, 1) };
}

/// Runs `body` for one side with the shared state seeded first and the
/// recorder cleared, then hands back what `body` produced together with the
/// recorder reading taken immediately afterwards -- the M9f
/// `run_both_probed` shape. Reading either after the other side has run would
/// observe the other side's writes to the shared doubles.
fn run_probed<T>(side: c_int, seed: impl Fn(c_int), body: impl Fn(c_int) -> T) -> (T, KeysCalls) {
    seed(side);
    // SAFETY: fixture call.
    unsafe { ctest_keys_probe_reset() };
    quake_ctest::fs::clear_logs();
    let out = body(side);
    (out, probe())
}

// ===========================================================================
// 1. keynames[] -- the table itself, and both conversions

#[test]
fn keynames_tables_agree_entry_for_entry() {
    let _g = lock();
    // SAFETY: fixture reads of the two tables.
    unsafe {
        let n_c = ctest_keys_keyname_count(C);
        let n_r = ctest_keys_keyname_count(R);
        assert_eq!(n_c, n_r, "keynames[] length");
        // keys.c:54-142 holds 74 live entries (the KP_NUMLOCK line is
        // commented out); a truncated table on both sides would still compare
        // equal, so pin the ends of it too.
        assert!(n_c > 60, "keynames[] looks truncated: {n_c}");
        assert_eq!(to_str(ctest_keys_keyname(C, 0)).as_deref(), Some("TAB"));
        assert_eq!(
            to_str(ctest_keys_keyname(C, n_c - 1)).as_deref(),
            Some("TOUCHPAD")
        );
        for i in 0..n_c {
            assert_eq!(
                to_str(ctest_keys_keyname(C, i)),
                to_str(ctest_keys_keyname(R, i)),
                "keynames[{i}].name"
            );
            assert_eq!(
                ctest_keys_keyname_num(C, i),
                ctest_keys_keyname_num(R, i),
                "keynames[{i}].keynum"
            );
        }
    }
}

#[test]
fn keynum_to_string_matches_over_the_table_ascii_and_out_of_range() {
    let _g = lock();
    let mut keynums: Vec<c_int> = vec![-1000, -2, -1, 0, 1, 31, 32, 33, 126, 127, 128];
    keynums.extend(33..=126); // printable ascii, the tinystr path
                              // SAFETY: fixture read.
    let n = unsafe { ctest_keys_keyname_count(C) };
    // SAFETY: index bounded by the count just read.
    keynums.extend((0..n).map(|i| unsafe { ctest_keys_keyname_num(C, i) }));
    keynums.extend([199, 200, 255, 256, 257, 1000, c_int::MAX, c_int::MIN]);

    for k in keynums {
        // SAFETY: both return a static or table-owned NUL-terminated string.
        let (a, b) = unsafe {
            (
                to_str(c_ref_Key_KeynumToString(k)),
                to_str(Key_KeynumToString(k)),
            )
        };
        assert_eq!(a, b, "Key_KeynumToString({k})");
        assert!(a.is_some(), "Key_KeynumToString({k}) returned NULL");
    }
}

#[test]
fn string_to_keynum_matches_over_every_name_ascii_and_junk() {
    let _g = lock();
    let mut names: Vec<String> = Vec::new();
    // SAFETY: fixture reads; the table entries are NUL-terminated.
    unsafe {
        let n = ctest_keys_keyname_count(C);
        for i in 0..n {
            let name = to_str(ctest_keys_keyname(C, i)).unwrap();
            names.push(name.to_ascii_uppercase());
            names.push(name.to_ascii_lowercase());
            names.push(name);
        }
    }
    // every single printable character, incl. the ones that ARE key names
    for ch in 32u8..=126 {
        names.push((ch as char).to_string());
    }
    names.extend(
        [
            "",
            "SEMICOLON",
            "semicolon",
            "SeMiCoLoN",
            "NOT_A_KEY",
            "K_ENTER",
            "ENTER ",
            " ENTER",
            "MOUSE9",
            "AUX1",
            "\u{7f}",
        ]
        .iter()
        .map(|s| s.to_string()),
    );

    for name in &names {
        let s = cs(name);
        // SAFETY: NUL-terminated.
        let (a, b) = unsafe {
            (
                c_ref_Key_StringToKeynum(s.as_ptr()),
                Key_StringToKeynum(s.as_ptr()),
            )
        };
        assert_eq!(a, b, "Key_StringToKeynum({name:?})");
    }

    // NULL is a documented early-out (keys.c:605)
    // SAFETY: both sides check for NULL before dereferencing.
    let (a, b) = unsafe {
        (
            c_ref_Key_StringToKeynum(std::ptr::null()),
            Key_StringToKeynum(std::ptr::null()),
        )
    };
    assert_eq!(a, b);
    assert_eq!(a, -1);
}

#[test]
fn string_to_keynum_round_trips_every_table_entry() {
    let _g = lock();
    // SAFETY: fixture reads.
    unsafe {
        let n = ctest_keys_keyname_count(C);
        for i in 0..n {
            let name = to_str(ctest_keys_keyname(C, i)).unwrap();
            let s = cs(&name);
            let num = Key_StringToKeynum(s.as_ptr());
            assert_eq!(
                num,
                c_ref_Key_StringToKeynum(s.as_ptr()),
                "round trip for {name:?}"
            );
            // COMPAT: a single-character name (keys.c:607 `if (!str[1]) return
            // str[0];`) never reaches the table walk, so "'" resolves to 0x27
            // rather than to its keynames[] entry. Both sides must agree on
            // that, which is what the two calls above pin.
            assert_eq!(
                to_str(Key_KeynumToString(num)),
                to_str(c_ref_Key_KeynumToString(num)),
                "back conversion for {name:?}"
            );
        }
    }
}

// ===========================================================================
// 2. Key_SetBinding / bind / unbind / unbindall / bindlist

#[test]
fn set_binding_matches_including_the_minus_one_guard() {
    let _g = lock();
    reset_all();

    for side in SIDES {
        // keys.c:657's `if (keynum == -1) return;` is exercised here but not
        // directly asserted: the only observable difference if it were
        // dropped is a write to keybindings[-1], and reading that back would
        // itself be out of bounds. What the comparison below does pin is the
        // free-and-replace path and the NULL clear.
        set_binding(side, -1, Some("ignored"));
        set_binding(side, 'a' as c_int, Some("+attack"));
        set_binding(side, K_MOUSE1, Some("impulse 10"));
        set_binding(side, K_F1, Some("")); // empty is stored, not NULL
        set_binding(side, 'a' as c_int, Some("+forward")); // frees and replaces
        set_binding(side, K_MOUSE1, None); // frees to NULL
    }

    for k in 0..MAX_KEYS {
        assert_eq!(binding(C, k), binding(R, k), "keybindings[{k}]");
    }
    assert_eq!(binding(C, 'a' as c_int).as_deref(), Some("+forward"));
    assert_eq!(binding(C, K_F1).as_deref(), Some(""));
    assert_eq!(binding(C, K_MOUSE1), None);
}

#[test]
fn bind_f_matches_output_and_bindings() {
    let _g = lock();
    init_once();
    reset_all();

    let script = [
        "bind",                        // argc 1 -> usage
        "bind nosuchkey",              // invalid key name
        "bind a",                      // valid, unbound
        "bind a +attack",              // set
        "bind a",                      // valid, bound -> echo
        "bind SPACE +jump and more",   // argv 2.. joined with single spaces
        "bind ; \"impulse 9\"",        // ';' is a key name AND a separator
        "bind \"MOUSE1\" \"+attack\"", // quoted key name
        "bind UPARROW",                // multi-word name lookup
    ];

    for line in script {
        let mut per_side: Vec<(Vec<String>, Vec<Option<String>>)> = Vec::new();
        for side in SIDES {
            let (out, _calls) = run_probed(
                side,
                |_| {},
                |side| {
                    run_cmd(side, line, bind_f);
                    let bindings: Vec<Option<String>> = [
                        'a' as c_int,
                        ' ' as c_int,
                        ';' as c_int,
                        K_MOUSE1,
                        K_UPARROW,
                    ]
                    .iter()
                    .map(|&k| binding(side, k))
                    .collect();
                    (con_log(), bindings)
                },
            );
            per_side.push(out);
        }
        assert_eq!(per_side[0], per_side[1], "bind_f for {line:?}");
    }

    // and the text is the text keys.c writes, not just "equal to itself"
    let (log, _) = run_probed(
        C,
        |_| {},
        |side| {
            run_cmd(side, "bind", bind_f);
            (con_log(), ())
        },
    );
    assert_eq!(
        log.0,
        vec!["[con] bind <key> [command] : attach a command to a key\n"]
    );
}

#[test]
fn unbind_f_matches_output_and_bindings() {
    let _g = lock();
    init_once();
    reset_all();

    let script = [
        "unbind",           // argc 1 -> usage
        "unbind a b",       // argc 3 -> usage too
        "unbind nosuchkey", // invalid
        "unbind a",         // bound -> cleared
        "unbind a",         // already clear -> still fine
    ];

    for line in script {
        let mut per_side: Vec<(Vec<String>, Option<String>)> = Vec::new();
        for side in SIDES {
            let (out, _) = run_probed(
                side,
                |side| set_binding(side, 'a' as c_int, Some("+attack")),
                |side| {
                    run_cmd(side, line, unbind_f);
                    (con_log(), binding(side, 'a' as c_int))
                },
            );
            per_side.push(out);
        }
        assert_eq!(per_side[0], per_side[1], "unbind_f for {line:?}");
    }

    let (log, _) = run_probed(
        C,
        |_| {},
        |side| {
            run_cmd(side, "unbind nosuchkey", unbind_f);
            (con_log(), ())
        },
    );
    assert_eq!(log.0, vec!["[con] \"nosuchkey\" isn't a valid key\n"]);
}

#[test]
fn unbindall_f_clears_every_binding_on_both_sides() {
    let _g = lock();
    reset_all();

    for side in SIDES {
        for k in [0, 5, 'a' as c_int, K_F1, K_MOUSE1, MAX_KEYS - 1] {
            set_binding(side, k, Some("+attack"));
        }
        // SAFETY: reads/writes that side's keybindings[] only.
        unsafe {
            if side == C {
                c_ref_Key_Unbindall_f()
            } else {
                Key_Unbindall_f()
            }
        }
    }

    for k in 0..MAX_KEYS {
        assert_eq!(
            binding(C, k),
            binding(R, k),
            "keybindings[{k}] after unbindall"
        );
        assert_eq!(binding(C, k), None);
    }
}

#[test]
fn bindlist_f_matches_output_and_iteration_order() {
    let _g = lock();
    reset_all();

    let fixture = |side: c_int| {
        ctest_keys_reset_side(side);
        set_binding(side, K_MOUSE1, Some("+attack")); // 200, listed last
        set_binding(side, 'z' as c_int, Some("say hi")); // 122
        set_binding(side, 'a' as c_int, Some("+forward")); // 97
        set_binding(side, K_F1, Some("")); // 135, skipped (*kb == 0)
        set_binding(side, 1, Some("weird")); // 1 -> "<UNKNOWN KEYNUM>"
    };

    let mut per_side = Vec::new();
    for side in SIDES {
        let (log, _) = run_probed(side, fixture, |side| {
            // SAFETY: reads that side's keybindings[] and prints via the
            // shared Con_SafePrintf capture, read back immediately below.
            unsafe {
                if side == C {
                    c_ref_Key_Bindlist_f()
                } else {
                    Key_Bindlist_f()
                }
            }
            con_log()
        });
        per_side.push(log);
    }
    assert_eq!(per_side[0], per_side[1]);
    assert_eq!(
        per_side[0],
        vec![
            "[safe]    <UNKNOWN KEYNUM> \"weird\"\n",
            "[safe]    a \"+forward\"\n",
            "[safe]    z \"say hi\"\n",
            "[safe]    MOUSE1 \"+attack\"\n",
            "[safe] 4 bindings\n",
        ]
    );
}

/// `ctest_keys_reset` clears everything for one side; wrapping it keeps the
/// closures above free of `unsafe`.
fn ctest_keys_reset_side(side: c_int) {
    // SAFETY: fixture call; frees and zeroes that side's tables only.
    unsafe { ctest_keys_reset(side) };
}

// ===========================================================================
// 3. Key_WriteBindings -- a config.cfg byte-diff subject

/// SAFETY: reads the whole tmpfile back into a Vec<u8>.
unsafe fn slurp(f: *mut c::FILE) -> Vec<u8> {
    // SAFETY: `f` is a live FILE* per this function's contract.
    unsafe {
        rewind(f);
        let mut out = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            let n = c::stdio::fread(
                chunk.as_mut_ptr() as *mut core::ffi::c_void,
                1,
                chunk.len(),
                f,
            );
            if n == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..n]);
        }
        out
    }
}

#[test]
fn write_bindings_is_byte_identical() {
    let _g = lock();
    reset_all();

    // bindings deliberately containing quotes, semicolons and spaces, plus an
    // empty binding (skipped) and a key with no keynames[] entry.
    let fixture: &[(c_int, &str)] = &[
        (1, "weird"),
        ('a' as c_int, "+attack"),
        (';' as c_int, "impulse 9; impulse 10"),
        ('q' as c_int, "say \"hello there\""),
        (' ' as c_int, "+jump"),
        (K_F1, ""),
        (K_MOUSE1, "impulse 10 ; wait ; +attack"),
        (MAX_KEYS - 1, "bind_the_last_key"),
    ];

    for unbindall in [0.0f32, 1.0f32] {
        let mut bytes: Vec<Vec<u8>> = Vec::new();
        for side in SIDES {
            // SAFETY: tmpfile() hands back an owned FILE*; both writers only
            // fprintf into it.
            unsafe {
                ctest_keys_reset(side);
                ctest_keys_set_cfg_unbindall(side, unbindall);
                for &(k, v) in fixture {
                    set_binding(side, k, Some(v));
                }
                let f = tmpfile();
                assert!(!f.is_null(), "tmpfile()");
                if side == C {
                    c_ref_Key_WriteBindings(f)
                } else {
                    Key_WriteBindings(f)
                }
                bytes.push(slurp(f));
                c::stdio::fclose(f);
            }
        }
        assert_eq!(
            String::from_utf8_lossy(&bytes[0]),
            String::from_utf8_lossy(&bytes[1]),
            "Key_WriteBindings text (cfg_unbindall={unbindall})"
        );
        assert_eq!(bytes[0], bytes[1], "Key_WriteBindings bytes");
        assert!(
            bytes[0].starts_with(if unbindall == 0.0 {
                b"bind \"<UNKNOWN".as_slice()
            } else {
                b"unbindall\n".as_slice()
            }),
            "unexpected prologue: {:?}",
            String::from_utf8_lossy(&bytes[0])
        );
        assert!(
            String::from_utf8_lossy(&bytes[0]).contains("bind \"q\" \"say \"hello there\"\"\n"),
            "quote handling changed: {:?}",
            String::from_utf8_lossy(&bytes[0])
        );
    }
}

// ===========================================================================
// 4. Key_Console line editing

/// Drives one key sequence through both sides from the same seeded line and
/// compares the state and the recorder after each side.
fn console_scenario(name: &str, seed: impl Fn(c_int), keys: &[c_int]) {
    let mut per_side: Vec<(State, KeysCalls, Vec<String>, c_int, String)> = Vec::new();
    for side in SIDES {
        let (out, calls) = run_probed(side, &seed, |side| {
            for &k in keys {
                key_console(side, k);
            }
            (
                snapshot(side),
                con_log(),
                // SAFETY: fixture read of the shared console mirror.
                unsafe { ctest_keys_con_backscroll() },
                get_line(side, snapshot(side).eline),
            )
        });
        per_side.push((out.0, calls, out.1, out.2, out.3));
    }
    assert_eq!(per_side[0], per_side[1], "console scenario {name:?}");
}

fn seed_line(text: &'static str, linepos: c_int) -> impl Fn(c_int) {
    move |side: c_int| {
        ctest_keys_reset_side(side);
        set_line(side, 0, text);
        // SAFETY: fixture write.
        unsafe { ctest_keys_set_edit(side, 0, 0, linepos, 1) };
    }
}

#[test]
fn console_cursor_motion_matches() {
    let _g = lock();
    reset_all();

    console_scenario("home", seed_line("]hello world", 12), &[K_HOME]);
    console_scenario("end", seed_line("]hello world", 1), &[K_END]);
    console_scenario(
        "left past the prompt",
        seed_line("]ab", 3),
        &[K_LEFTARROW, K_LEFTARROW, K_LEFTARROW, K_LEFTARROW],
    );
    console_scenario(
        "right past the end",
        seed_line("]ab", 1),
        &[K_RIGHTARROW, K_RIGHTARROW, K_RIGHTARROW, K_RIGHTARROW],
    );
}

#[test]
fn console_word_motion_over_mixed_separators_matches() {
    let _g = lock();
    reset_all();

    // ' ', '_', '\t' and ';' are the separators (keys.c:200-212)
    let line = "]alpha  beta_gamma\tdelta;;eps";
    for (name, key, from) in [
        ("ctrl-left from end", K_LEFTARROW, line.len() as c_int),
        ("ctrl-left mid-word", K_LEFTARROW, 12),
        ("ctrl-left in run of seps", K_LEFTARROW, 8),
        ("ctrl-left at prompt", K_LEFTARROW, 1),
        ("ctrl-right from prompt", K_RIGHTARROW, 1),
        ("ctrl-right mid-word", K_RIGHTARROW, 12),
        ("ctrl-right in run of seps", K_RIGHTARROW, 24),
        ("ctrl-right at end", K_RIGHTARROW, line.len() as c_int),
    ] {
        console_scenario(
            name,
            move |side| {
                ctest_keys_reset_side(side);
                set_line(side, 0, line);
                // SAFETY: fixture writes.
                unsafe {
                    ctest_keys_set_edit(side, 0, 0, from, 1);
                    ctest_keys_set_keydown(side, K_CTRL, 1);
                }
            },
            &[key],
        );
    }
}

#[test]
fn console_backspace_and_delete_match_at_both_ends() {
    let _g = lock();
    reset_all();

    console_scenario(
        "backspace at the prompt is a no-op",
        seed_line("]abc", 1),
        &[K_BACKSPACE, K_BACKSPACE],
    );
    console_scenario(
        "backspace from the end",
        seed_line("]abc", 4),
        &[K_BACKSPACE, K_BACKSPACE, K_BACKSPACE, K_BACKSPACE],
    );
    console_scenario("del at the end is a no-op", seed_line("]abc", 4), &[K_DEL]);
    console_scenario(
        "del from the prompt",
        seed_line("]abc", 1),
        &[K_DEL, K_DEL, K_DEL, K_DEL],
    );

    let line = "]one two_three  four";
    console_scenario(
        "ctrl-backspace eats a word",
        move |side| {
            ctest_keys_reset_side(side);
            set_line(side, 0, line);
            // SAFETY: fixture writes.
            unsafe {
                ctest_keys_set_edit(side, 0, 0, line.len() as c_int, 1);
                ctest_keys_set_keydown(side, K_CTRL, 1);
            }
        },
        &[
            K_BACKSPACE,
            K_BACKSPACE,
            K_BACKSPACE,
            K_BACKSPACE,
            K_BACKSPACE,
        ],
    );
    console_scenario(
        "ctrl-del eats a word",
        move |side| {
            ctest_keys_reset_side(side);
            set_line(side, 0, line);
            // SAFETY: fixture writes.
            unsafe {
                ctest_keys_set_edit(side, 0, 0, 1, 1);
                ctest_keys_set_keydown(side, K_CTRL, 1);
            }
        },
        &[K_DEL, K_DEL, K_DEL, K_DEL, K_DEL],
    );
}

#[test]
fn console_insert_toggle_and_char_entry_match() {
    let _g = lock();
    reset_all();

    // K_INS with no modifier flips key_insert (keys.c:452)
    console_scenario(
        "insert toggle",
        seed_line("]abc", 2),
        &[K_INS, K_INS, K_INS],
    );

    // Char_Console in insert mode vs overwrite mode, in the middle and at the
    // end of the line.
    for (name, insert, linepos) in [
        ("insert mid-line", 1, 2),
        ("overwrite mid-line", 0, 2),
        ("insert at end", 1, 4),
        ("overwrite at end", 0, 4),
    ] {
        let mut per_side: Vec<(State, KeysCalls)> = Vec::new();
        for side in SIDES {
            let (st, calls) = run_probed(
                side,
                |side| {
                    ctest_keys_reset_side(side);
                    set_line(side, 0, "]abc");
                    // SAFETY: fixture write.
                    unsafe { ctest_keys_set_edit(side, 0, 0, linepos, insert) };
                },
                |side| {
                    for ch in "XY".chars() {
                        char_console(side, ch as c_int);
                    }
                    snapshot(side)
                },
            );
            per_side.push((st, calls));
        }
        assert_eq!(per_side[0], per_side[1], "char_console {name:?}");
    }
}

#[test]
fn char_console_respects_the_line_length_limit_identically() {
    let _g = lock();
    reset_all();

    let long: String = std::iter::once(']')
        .chain(std::iter::repeat_n('x', MAXCMDLINE - 3))
        .collect();
    let mut per_side: Vec<State> = Vec::new();
    for side in SIDES {
        let long = long.clone();
        let (st, _) = run_probed(
            side,
            move |side| {
                ctest_keys_reset_side(side);
                let t = cs(&long);
                // SAFETY: NUL-terminated, shorter than MAXCMDLINE.
                unsafe {
                    ctest_keys_set_line(side, 0, t.as_ptr());
                    ctest_keys_set_edit(side, 0, 0, long.len() as c_int, 1);
                }
            },
            |side| {
                for _ in 0..8 {
                    char_console(side, 'Z' as c_int);
                }
                snapshot(side)
            },
        );
        per_side.push(st);
    }
    assert_eq!(per_side[0], per_side[1]);
    assert_eq!(per_side[0].lpos, MAXCMDLINE as c_int - 1);
}

#[test]
fn console_history_walk_matches_past_both_ends() {
    let _g = lock();
    reset_all();

    let seed = |side: c_int| {
        ctest_keys_reset_side(side);
        set_line(side, 0, "]first");
        set_line(side, 1, "]second");
        set_line(side, 2, "]third");
        set_line(side, 3, "]"); // the live edit line
                                // SAFETY: fixture write.
        unsafe { ctest_keys_set_edit(side, 3, 3, 1, 1) };
    };

    // Walk up past the oldest entry (the do/while at keys.c:396 stops when it
    // wraps back to edit_line), then back down past the newest.
    console_scenario(
        "up past the oldest",
        seed,
        &[K_UPARROW, K_UPARROW, K_UPARROW, K_UPARROW, K_UPARROW],
    );
    console_scenario(
        "down with no history walked yet",
        seed,
        &[K_DOWNARROW, K_DOWNARROW],
    );
    console_scenario(
        "up then back down to the saved current line",
        seed,
        &[
            K_UPARROW,
            K_UPARROW,
            K_DOWNARROW,
            K_DOWNARROW,
            K_DOWNARROW,
            K_DOWNARROW,
        ],
    );

    // The "current" line is a function-local static in each half, so this
    // scenario also pins that both halves remember the typed-but-unsubmitted
    // text across an up/down round trip.
    console_scenario(
        "typed text survives an up/down round trip",
        |side| {
            ctest_keys_reset_side(side);
            set_line(side, 0, "]older");
            set_line(side, 1, "]typing");
            // SAFETY: fixture write.
            unsafe { ctest_keys_set_edit(side, 1, 1, 7, 1) };
        },
        &[K_UPARROW, K_DOWNARROW],
    );
}

#[test]
fn console_enter_stores_history_and_prints_identically() {
    let _g = lock();
    init_once();
    reset_all();

    for (name, first, second) in [
        ("distinct lines advance edit_line", "]hello", "]world"),
        ("identical lines do not", "]same", "]same"),
    ] {
        let mut per_side: Vec<(State, Vec<String>, KeysCalls, String, String)> = Vec::new();
        for side in SIDES {
            let (out, calls) = run_probed(
                side,
                |side| {
                    ctest_keys_reset_side(side);
                    // ca_disconnected (== 0 is ca_dedicated; 1 is
                    // ca_disconnected) makes ENTER force a screen update
                    // (keys.c:284) -- exercise that branch.
                    // SAFETY: fixture writes.
                    unsafe { ctest_keys_set_cls_state(side, 1) };
                    set_line(side, 0, first);
                    // SAFETY: fixture write.
                    unsafe { ctest_keys_set_edit(side, 0, 0, first.len() as c_int, 1) };
                },
                |side| {
                    key_console(side, K_ENTER);
                    let after_first = snapshot(side);
                    set_line(side, after_first.eline, second);
                    // SAFETY: fixture write.
                    unsafe {
                        ctest_keys_set_edit(
                            side,
                            after_first.eline,
                            after_first.hline,
                            second.len() as c_int,
                            1,
                        )
                    };
                    key_console(side, K_ENTER);
                    // ENTER pushes the line into that side's command buffer;
                    // draining it here both observes the forwarded text (as
                    // an "Unknown command" echo) and keeps the buffer from
                    // leaking into a later test in this binary.
                    exec_cbuf(side);
                    (
                        snapshot(side),
                        con_log(),
                        get_line(side, 0),
                        get_line(side, 1),
                    )
                },
            );
            per_side.push((out.0, out.1, calls, out.2, out.3));
        }
        assert_eq!(per_side[0], per_side[1], "enter scenario {name:?}");
        assert_eq!(
            per_side[0].2.updatescreen_calls, 2,
            "the ca_disconnected screen-update branch was never taken"
        );
    }
}

#[test]
fn console_ctrl_shortcuts_match() {
    let _g = lock();
    reset_all();

    let ctrl_seed = |line: &'static str, pos: c_int| {
        move |side: c_int| {
            ctest_keys_reset_side(side);
            set_line(side, 0, line);
            // SAFETY: fixture writes.
            unsafe {
                ctest_keys_set_edit(side, 0, 0, pos, 1);
                ctest_keys_set_keydown(side, K_CTRL, 1);
            }
        }
    };

    console_scenario("ctrl-a selects all", ctrl_seed("]abc", 4), &['a' as c_int]);
    console_scenario("ctrl-A selects all", ctrl_seed("]abc", 4), &['A' as c_int]);

    // Ctrl-C: when Con_CopySelectionToClipboard answers true the line is left
    // alone; when it answers false the line is aborted (keys.c:461-471).
    for (name, copied) in [("ctrl-c with a selection", 1), ("ctrl-c with none", 0)] {
        // SAFETY: shared fixture write, applied before either side runs and
        // not disturbed by either.
        unsafe { ctest_keys_set_copyselection_result(copied) };
        console_scenario(name, ctrl_seed("]abort me", 9), &['c' as c_int]);
    }
    // SAFETY: restore the shared default.
    unsafe { ctest_keys_set_copyselection_result(0) };

    // Ctrl-Ins copies; Shift-Ins pastes the clipboard through PasteToConsole.
    let clip = cs("pasted;line");
    // SAFETY: NUL-terminated; the fixture copies.
    unsafe { ctest_keys_set_clipboard(clip.as_ptr()) };
    console_scenario(
        "shift-ins pastes",
        |side| {
            ctest_keys_reset_side(side);
            set_line(side, 0, "]");
            // SAFETY: fixture writes.
            unsafe {
                ctest_keys_set_edit(side, 0, 0, 1, 1);
                ctest_keys_set_keydown(side, K_SHIFT, 1);
            }
        },
        &[K_INS],
    );
    console_scenario(
        "ctrl-v pastes",
        |side| {
            ctest_keys_reset_side(side);
            set_line(side, 0, "]x");
            // SAFETY: fixture writes.
            unsafe {
                ctest_keys_set_edit(side, 0, 0, 2, 1);
                ctest_keys_set_keydown(side, K_CTRL, 1);
            }
        },
        &['v' as c_int],
    );
    // SAFETY: restore the shared default.
    unsafe { ctest_keys_set_clipboard(std::ptr::null()) };
}

#[test]
fn console_scroll_keys_match() {
    let _g = lock();
    reset_all();

    // SAFETY: seeds the shared console mirror; both sides read the same one.
    unsafe { ctest_keys_set_con(std::ptr::null_mut(), 0, 0, 96, 0, 0, 0, 480) };
    console_scenario("pgup", seed_line("]", 1), &[K_PGUP]);
    console_scenario(
        "ctrl-pgup",
        |side| {
            ctest_keys_reset_side(side);
            set_line(side, 0, "]");
            // SAFETY: fixture writes.
            unsafe {
                ctest_keys_set_edit(side, 0, 0, 1, 1);
                ctest_keys_set_keydown(side, K_CTRL, 1);
            }
        },
        &[K_PGUP],
    );
    console_scenario(
        "ctrl-end resets backscroll",
        |side| {
            ctest_keys_reset_side(side);
            set_line(side, 0, "]");
            // SAFETY: fixture writes.
            unsafe {
                ctest_keys_set_edit(side, 0, 0, 1, 1);
                ctest_keys_set_keydown(side, K_CTRL, 1);
                ctest_keys_set_con(std::ptr::null_mut(), 0, 0, 96, 0, 7, 0, 480);
            }
        },
        &[K_END],
    );
    // SAFETY: restore the shared default.
    unsafe { ctest_keys_set_con(std::ptr::null_mut(), 0, 0, 0, 0, 0, 0, 0) };
}

// ===========================================================================
// 5. Key_EventWithKeycode -- keydown[] bookkeeping and +/- synthesis

/// What `key_event_demo_playback_seek_and_speed_match` compares per side:
/// the commands the synthesized text reached, `cls.demospeed`,
/// `cls.demopaused`, the call recorder and the console echo.
type DemoObservation = (Vec<String>, c_float, c_int, KeysCalls, Vec<String>);

static C_TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static R_TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn trace(side: c_int) -> Vec<String> {
    let m = if side == C { &C_TRACE } else { &R_TRACE };
    m.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn clear_trace(side: c_int) {
    let m = if side == C { &C_TRACE } else { &R_TRACE };
    m.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

/// Records `argv[0] argv[1] ...` so the synthesized keynum parameter is
/// visible, not just the command name.
extern "C" fn c_probe_plus() {
    record(C, "+probe", true)
}
extern "C" fn c_probe_minus() {
    record(C, "-probe", true)
}
extern "C" fn c_probe_plain() {
    record(C, "probe", true)
}
extern "C" fn r_probe_plus() {
    record(R, "+probe", false)
}
extern "C" fn r_probe_minus() {
    record(R, "-probe", false)
}
extern "C" fn r_probe_plain() {
    record(R, "probe", false)
}

fn record(side: c_int, name: &str, oracle: bool) {
    // SAFETY: called from inside that side's Cbuf_Execute, so that side's argv
    // is live.
    let args = unsafe {
        let argc = if oracle {
            c_ref_Cmd_Argc()
        } else {
            rcmd::Cmd_Argc()
        };
        (0..argc)
            .map(|i| {
                to_str(if oracle {
                    c_ref_Cmd_Argv(i)
                } else {
                    rcmd::Cmd_Argv(i)
                })
                .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert!(args.starts_with(name), "probe {name} saw {args:?}");
    let m = if side == C { &C_TRACE } else { &R_TRACE };
    m.lock().unwrap_or_else(|p| p.into_inner()).push(args);
}

fn register_probes() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: leak_str names live for the process; the handlers only touch
        // their own Mutex-guarded vector; TEST_LOCK serializes callers.
        unsafe {
            for (name, cf, rf) in [
                (
                    "+probe",
                    c_probe_plus as extern "C" fn(),
                    r_probe_plus as extern "C" fn(),
                ),
                (
                    "-probe",
                    c_probe_minus as extern "C" fn(),
                    r_probe_minus as extern "C" fn(),
                ),
                (
                    "probe",
                    c_probe_plain as extern "C" fn(),
                    r_probe_plain as extern "C" fn(),
                ),
            ] {
                c_ref_Cmd_AddCommand2(leak_str(name), Some(cf), c::cmd_source_t_src_command, false);
                rcmd::Cmd_AddCommand2(leak_str(name), Some(rf), c::cmd_source_t_src_command, false);
            }
        }
    });
}

fn exec_cbuf(side: c_int) {
    // SAFETY: each side executes its own buffer; the registered handlers are
    // the only commands the scenarios below can reach.
    unsafe {
        if side == C {
            c_ref_Cbuf_Execute()
        } else {
            Cbuf_Execute()
        }
    }
}

#[test]
fn key_event_button_command_synthesis_matches() {
    let _g = lock();
    init_once();
    register_probes();
    reset_all();

    for (name, bind) in [
        ("button command", "+probe"),
        ("plain command", "probe one two"),
    ] {
        let mut per_side: Vec<(Vec<String>, c_int, c_int)> = Vec::new();
        for side in SIDES {
            let (out, _) = run_probed(
                side,
                |side| {
                    ctest_keys_reset_side(side);
                    set_binding(side, K_MOUSE1, Some(bind));
                    clear_trace(side);
                },
                |side| {
                    key_event(side, K_MOUSE1, true);
                    key_event(side, K_MOUSE1, true); // autorepeat: suppressed
                    key_event(side, K_MOUSE1, false);
                    key_event(side, K_MOUSE1, false); // stray up: ignored
                    exec_cbuf(side);
                    (trace(side), flags(side, K_MOUSE1), dest())
                },
            );
            per_side.push(out);
        }
        assert_eq!(per_side[0], per_side[1], "synthesis for {name:?}");
    }

    // and the synthesized text is the text keys.c:1113/1160 writes
    clear_trace(C);
    // SAFETY: seeds one side and runs its own buffer.
    unsafe { ctest_keys_reset(C) };
    set_binding(C, K_MOUSE1, Some("+probe"));
    key_event(C, K_MOUSE1, true);
    key_event(C, K_MOUSE1, false);
    exec_cbuf(C);
    assert_eq!(
        trace(C),
        vec![format!("+probe {K_MOUSE1}"), format!("-probe {K_MOUSE1}")]
    );
}

#[test]
fn key_event_autorepeat_and_keydown_bookkeeping_match() {
    let _g = lock();
    init_once();
    register_probes();
    reset_all();

    // keys.c:1050 only suppresses autorepeats while key_dest == key_game and
    // the console is not forced up.
    for (name, dest, forcedup) in [
        ("game, console down", KEY_GAME, 0),
        ("game, console forced up", KEY_GAME, 1),
        ("console", KEY_CONSOLE, 0),
    ] {
        let mut per_side: Vec<(Vec<String>, Vec<c_int>)> = Vec::new();
        for side in SIDES {
            let (out, _) = run_probed(
                side,
                |side| {
                    ctest_keys_reset_side(side);
                    set_binding(side, 'b' as c_int, Some("+probe"));
                    clear_trace(side);
                    // SAFETY: shared writes, re-applied for each side.
                    unsafe {
                        ctest_keys_set_dest(dest);
                        ctest_keys_set_con(std::ptr::null_mut(), 0, 0, 0, 0, 0, forcedup, 0);
                    }
                },
                |side| {
                    let mut seen = Vec::new();
                    for down in [true, true, true, false] {
                        key_event(side, 'b' as c_int, down);
                        seen.push(flags(side, 'b' as c_int));
                    }
                    exec_cbuf(side);
                    (trace(side), seen)
                },
            );
            per_side.push(out);
        }
        assert_eq!(per_side[0], per_side[1], "autorepeat for {name:?}");
    }
    // SAFETY: restore shared defaults.
    unsafe {
        ctest_keys_set_dest(KEY_GAME);
        ctest_keys_set_con(std::ptr::null_mut(), 0, 0, 0, 0, 0, 0, 0);
    }
}

#[test]
fn key_event_special_dispatch_matches() {
    let _g = lock();
    init_once();
    register_probes();
    reset_all();

    // alt-enter -> VID_Toggle; escape -> menu/console toggles; out of range
    // keynums are dropped (keys.c:1038).
    for (name, dest, pre, key, keycode, mb) in [
        ("alt-enter", KEY_GAME, vec![K_ALT], K_ENTER, 0, false),
        ("shift-escape", KEY_GAME, vec![K_SHIFT], K_ESCAPE, 0, false),
        ("escape in game", KEY_GAME, vec![], K_ESCAPE, 0, false),
        ("escape in menu", KEY_MENU, vec![], K_ESCAPE, 0, false),
        ("escape in message", KEY_MESSAGE, vec![], K_ESCAPE, 0, false),
        (
            "menu keydown",
            KEY_MENU,
            vec![],
            'q' as c_int,
            'q' as c_int,
            false,
        ),
        // menubound in the menu routes to the binding instead of M_Keydown
        (
            "menubound in menu",
            KEY_MENU,
            vec![],
            'q' as c_int,
            'q' as c_int,
            true,
        ),
        ("negative keynum", KEY_GAME, vec![], -1, 0, false),
        ("keynum past MAX_KEYS", KEY_GAME, vec![], MAX_KEYS, 0, false),
    ] {
        let mut per_side: Vec<(KeysCalls, State, Vec<String>)> = Vec::new();
        for side in SIDES {
            let pre = pre.clone();
            let (out, calls) = run_probed(
                side,
                |side| {
                    ctest_keys_reset_side(side);
                    for &k in &pre {
                        set_keydown(side, k, true);
                    }
                    if mb {
                        set_menubound(side, key);
                        set_binding(side, key, Some("probe from menu"));
                    }
                    clear_trace(side);
                    // SAFETY: shared write, re-applied for each side.
                    unsafe { ctest_keys_set_dest(dest) };
                },
                |side| {
                    // SAFETY: the plain entry point re-raises; nothing raises here.
                    unsafe {
                        if side == C {
                            c_ref_Key_EventWithKeycode(key, true, keycode)
                        } else {
                            Key_EventWithKeycode(key, true, keycode)
                        }
                    }
                    exec_cbuf(side);
                    (snapshot(side), trace(side))
                },
            );
            per_side.push((calls, out.0, out.1));
        }
        assert_eq!(per_side[0], per_side[1], "dispatch for {name:?}");
        match name {
            "alt-enter" => assert_eq!(per_side[0].0.vidtoggle_calls, 1),
            "shift-escape" => assert_eq!(per_side[0].0.toggleconsole_calls, 1),
            "escape in game" => assert_eq!(per_side[0].0.togglemenu_calls, 1),
            "escape in menu" => assert_eq!(per_side[0].0.menukeydown_calls, 1),
            "menu keydown" => assert_eq!(per_side[0].0.menukeydown_key, 'q' as c_int),
            "menubound in menu" => {
                assert_eq!(per_side[0].0.menukeydown_calls, 0);
                assert_eq!(per_side[0].2, vec!["probe from menu"]);
            }
            "negative keynum" | "keynum past MAX_KEYS" => {
                assert_eq!(per_side[0].0, KeysCalls::default())
            }
            _ => {}
        }
    }
    // SAFETY: restore the shared default.
    unsafe { ctest_keys_set_dest(KEY_GAME) };
}

#[test]
fn key_event_demo_playback_seek_and_speed_match() {
    let _g = lock();
    init_once();
    register_probes();
    reset_all();

    // keys.c:1117-1147: during demo playback most consolekeys drive seek /
    // speed instead of the binding.
    for (name, key, shift, paused, speed) in [
        ("seek back", K_LEFTARROW, false, 0, 1.0f32),
        ("seek back with shift", K_LEFTARROW, true, 0, 1.0),
        ("seek forward", K_RIGHTARROW, false, 0, 1.0),
        ("slow down", K_DOWNARROW, false, 0, 1.0),
        ("slow down to a pause", K_DOWNARROW, false, 0, 0.5),
        ("slow down while paused", K_DOWNARROW, false, 1, 1.0),
        ("speed up", K_UPARROW, false, 0, 1.0),
        ("speed up from paused zero", K_UPARROW, false, 1, 0.0),
        ("speed up past the clamp", K_UPARROW, false, 0, 64.0),
        ("other key opens the menu", 'x' as c_int, false, 0, 1.0),
        ("tab is exempt", K_TAB, false, 0, 1.0),
    ] {
        let mut per_side: Vec<DemoObservation> = Vec::new();
        for side in SIDES {
            let (out, calls) = run_probed(
                side,
                |side| {
                    ctest_keys_reset_side(side);
                    clear_trace(side);
                    // SAFETY: fixture writes.
                    unsafe {
                        ctest_keys_set_demo(side, 1, paused, speed);
                        ctest_keys_set_dest(KEY_GAME);
                    }
                    // consolekeys[] must be filled for the branch to be live
                    for k in [K_LEFTARROW, K_RIGHTARROW, K_UPARROW, K_DOWNARROW, K_TAB] {
                        set_consolekey(side, k);
                    }
                    set_consolekey(side, 'x' as c_int);
                    if shift {
                        set_keydown(side, K_SHIFT, true);
                    }
                },
                |side| {
                    key_event(side, key, true);
                    exec_cbuf(side);
                    (
                        trace(side),
                        demospeed(side),
                        demopaused(side),
                        // `seek` and `pause` are not registered on either
                        // registry, so Cbuf_Execute echoes the synthesized
                        // text back through the shared Con_Printf capture --
                        // which is exactly the string being compared.
                        con_log(),
                    )
                },
            );
            per_side.push((out.0, out.1, out.2, calls, out.3));
        }
        assert_eq!(per_side[0], per_side[1], "demo scenario {name:?}");
        if name == "seek back with shift" {
            assert_eq!(
                per_side[0].4,
                vec![
                    "[con] Unknown command \"seek\"
"
                ],
                "the seek command text is not reaching Cbuf_Execute"
            );
            assert_eq!(per_side[0].1, 1.0, "demospeed must not move on a seek");
        }
        if name == "slow down to a pause" {
            assert_eq!(per_side[0].1, 0.0, "the demospeed clamp-to-zero never ran");
        }
        if name == "speed up past the clamp" {
            assert_eq!(per_side[0].1, 64.0, "the CLAMP upper bound never ran");
        }
        if name == "other key opens the menu" {
            assert_eq!(per_side[0].3.togglemenu_calls, 1);
        }
    }
}

// ===========================================================================
// 6. Key_Init, History_Init / History_Shutdown, Key_TextEntry, input grab

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vkq_keys_ctest_{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn key_init_fills_the_tables_identically() {
    let _g = lock();
    init_once();
    reset_all();

    // History_Init (keys.c:814) opens history.txt through COM_FOpenPrefFile
    // while harness_active (stubs.c:797), which resolves under com_gamedir.
    // Point each side at its own empty directory so the read fails on both and
    // only the "]"-fill half of History_Init runs.
    for side in SIDES {
        let d = temp_dir(&format!("init_{side}"));
        let p = cs(d.to_str().unwrap());
        // SAFETY: NUL-terminated; the fixture copies into com_gamedir.
        unsafe { ctest_keys_set_gamedir(side, p.as_ptr()) };
        // SAFETY: registers into that side's own cmd registry.
        unsafe {
            if side == C {
                c_ref_Key_Init()
            } else {
                Key_Init()
            }
        }
    }

    for k in 0..MAX_KEYS {
        assert_eq!(
            flags(C, k),
            flags(R, k),
            "consolekeys/menubound/keydown for keynum {k}"
        );
    }
    // the tables must actually be populated -- a pair of empty arrays would
    // compare equal
    // SAFETY: fixture reads.
    unsafe {
        assert_eq!(ctest_keys_flags(C, 'a' as c_int) & 2, 2, "consolekeys['a']");
        assert_eq!(ctest_keys_flags(C, '`' as c_int) & 2, 0, "consolekeys['`']");
        assert_eq!(ctest_keys_flags(C, K_ESCAPE) & 4, 4, "menubound[K_ESCAPE]");
        assert_eq!(ctest_keys_flags(C, K_F1) & 4, 4, "menubound[K_F1]");
        assert_eq!(
            ctest_keys_flags(C, K_MOUSE1) & 2,
            2,
            "consolekeys[K_MOUSE1]"
        );
    }
    assert_eq!(snapshot(C).line, snapshot(R).line);
    assert_eq!(snapshot(C).lpos, snapshot(R).lpos);
    assert_eq!(snapshot(C).lpos, 1);
    assert_eq!(get_line(C, 700), get_line(R, 700));
    assert_eq!(get_line(C, 700), "]");
}

#[test]
fn history_file_round_trips_byte_identically() {
    let _g = lock();
    reset_all();

    let mut written: Vec<Vec<u8>> = Vec::new();
    let mut reread: Vec<Vec<String>> = Vec::new();
    for side in SIDES {
        let d = temp_dir(&format!("hist_{side}"));
        let p = cs(d.to_str().unwrap());
        // SAFETY: NUL-terminated; copied into com_gamedir.
        unsafe { ctest_keys_set_gamedir(side, p.as_ptr()) };

        ctest_keys_reset_side(side);
        set_line(side, 0, "]oldest");
        set_line(side, 1, "]middle \"quoted\"; and ; semis");
        set_line(side, 2, "]newest");
        set_line(side, 3, "]"); // the live edit line, never written
                                // SAFETY: fixture write.
        unsafe { ctest_keys_set_edit(side, 3, 3, 1, 1) };

        // SAFETY: writes history.txt under that side's com_gamedir.
        unsafe {
            if side == C {
                c_ref_History_Shutdown()
            } else {
                History_Shutdown()
            }
        }
        written.push(std::fs::read(d.join("history.txt")).unwrap_or_default());

        // now read it back the way Key_Init would
        ctest_keys_reset_side(side);
        // SAFETY: reads history.txt back into key_lines[].
        unsafe {
            if side == C {
                c_ref_History_Init()
            } else {
                History_Init()
            }
        }
        let st = snapshot(side);
        reread.push(vec![
            get_line(side, 0),
            get_line(side, 1),
            get_line(side, 2),
            get_line(side, 3),
            format!("edit_line={} history_line={}", st.eline, st.hline),
        ]);
    }

    assert_eq!(
        String::from_utf8_lossy(&written[0]),
        String::from_utf8_lossy(&written[1]),
        "history.txt text"
    );
    assert_eq!(written[0], written[1], "history.txt bytes");
    assert!(
        !written[0].is_empty(),
        "History_Shutdown wrote nothing -- the round trip proves nothing"
    );
    assert_eq!(reread[0], reread[1], "History_Init readback");
    assert!(
        reread[0][0].contains("oldest"),
        "History_Init read nothing back: {:?}",
        reread[0]
    );
}

#[test]
fn text_entry_and_update_for_dest_match() {
    let _g = lock();
    reset_all();

    for (want_dest, menu_text_entry) in [
        (KEY_GAME, 0),
        (KEY_CONSOLE, 0),
        (KEY_MESSAGE, 0),
        (KEY_MENU, 0),
        (KEY_MENU, 1),
    ] {
        let mut per_side: Vec<(bool, KeysCalls, c_int)> = Vec::new();
        for side in SIDES {
            let (out, calls) = run_probed(
                side,
                |_| {
                    // SAFETY: shared writes, re-applied for each side.
                    unsafe {
                        ctest_keys_set_dest(want_dest);
                        ctest_keys_set_menu(menu_text_entry, 0, 0);
                    }
                },
                |side| {
                    // SAFETY: reads shared key_dest and the menu doubles.
                    let te = unsafe {
                        if side == C {
                            c_ref_Key_TextEntry()
                        } else {
                            Key_TextEntry()
                        }
                    };
                    // SAFETY: drives IN_* through the shared recorder.
                    unsafe {
                        if side == C {
                            c_ref_Key_UpdateForDest()
                        } else {
                            Key_UpdateForDest()
                        }
                    }
                    (te, dest())
                },
            );
            per_side.push((out.0, calls, out.1));
        }
        assert_eq!(
            per_side[0], per_side[1],
            "dest={want_dest} menu_text_entry={menu_text_entry}"
        );
    }
    // SAFETY: restore shared defaults.
    unsafe {
        ctest_keys_set_dest(KEY_GAME);
        ctest_keys_set_menu(0, 0, 0);
    }
}

#[test]
fn input_grab_captures_the_same_key_and_char() {
    let _g = lock();
    reset_all();

    let mut per_side: Vec<(State, KeysCalls, State)> = Vec::new();
    for side in SIDES {
        let (out, calls) = run_probed(
            side,
            |side| {
                ctest_keys_reset_side(side);
                set_keydown(side, K_SHIFT, true);
            },
            |side| {
                // SAFETY: the plain entry points are the glue's re-raising
                // wrappers; nothing here raises.
                unsafe {
                    if side == C {
                        c_ref_Key_BeginInputGrab()
                    } else {
                        Key_BeginInputGrab()
                    }
                }
                key_event(side, 'k' as c_int, true);
                // SAFETY: keycode > 0 is what fills lastchar (keys.c:1065).
                unsafe {
                    if side == C {
                        c_ref_Key_EventWithKeycode(K_F1, true, 'Q' as c_int)
                    } else {
                        Key_EventWithKeycode(K_F1, true, 'Q' as c_int)
                    }
                }
                let grabbed = snapshot(side);
                // SAFETY: as above.
                unsafe {
                    if side == C {
                        c_ref_Key_EndInputGrab()
                    } else {
                        Key_EndInputGrab()
                    }
                }
                (grabbed, snapshot(side))
            },
        );
        per_side.push((out.0, calls, out.1));
    }
    assert_eq!(per_side[0], per_side[1]);
    assert_eq!(per_side[0].0.grabkey, K_F1);
    assert_eq!(per_side[0].0.grabchar, 'Q' as c_int);
    assert_eq!(
        per_side[0].1.updateinputmode_calls, 2,
        "IN_UpdateInputMode was never reached"
    );
}

#[test]
fn clear_states_zeroes_the_same_keys() {
    let _g = lock();
    init_once();
    register_probes();
    reset_all();

    let mut per_side: Vec<(Vec<c_int>, Vec<String>)> = Vec::new();
    for side in SIDES {
        let (out, _) = run_probed(
            side,
            |side| {
                ctest_keys_reset_side(side);
                clear_trace(side);
                for k in [3, 'a' as c_int, K_SHIFT, K_MOUSE1] {
                    set_keydown(side, k, true);
                }
                // a held button binding must emit its '-' on release
                set_binding(side, K_MOUSE1, Some("+probe"));
            },
            |side| {
                // SAFETY: the plain entry point re-raises; nothing raises here.
                unsafe {
                    if side == C {
                        c_ref_Key_ClearStates()
                    } else {
                        Key_ClearStates()
                    }
                }
                exec_cbuf(side);
                (
                    (0..MAX_KEYS).map(|k| flags(side, k)).collect::<Vec<_>>(),
                    trace(side),
                )
            },
        );
        per_side.push(out);
    }
    assert_eq!(
        per_side[0].0, per_side[1].0,
        "keydown[] after Key_ClearStates"
    );
    assert_eq!(
        per_side[0].1, per_side[1].1,
        "commands Key_ClearStates emitted"
    );
    assert!(
        per_side[0].0.iter().all(|&f| f & 1 == 0),
        "Key_ClearStates left a key down"
    );
}

// ===========================================================================
// 7. Chat buffer (Key_Message / Char_Message)

#[test]
fn chat_line_editing_matches() {
    let _g = lock();
    reset_all();

    let mut per_side: Vec<Vec<State>> = Vec::new();
    for side in SIDES {
        let (states, _) = run_probed(
            side,
            |side| {
                ctest_keys_reset_side(side);
                // SAFETY: shared write, re-applied for each side.
                unsafe { ctest_keys_set_dest(KEY_MESSAGE) };
            },
            |side| {
                let mut out = Vec::new();
                // SAFETY: Key_Message / Char_Message touch that side's chat
                // buffer and the shared key_dest only.
                unsafe {
                    for ch in "hi there".chars() {
                        if side == C {
                            c_ref_Char_Message(ch as c_int)
                        } else {
                            Char_Message(ch as c_int)
                        }
                    }
                    out.push(snapshot(side));
                    for _ in 0..3 {
                        if side == C {
                            c_ref_Key_Message(K_BACKSPACE)
                        } else {
                            Key_Message(K_BACKSPACE)
                        }
                    }
                    out.push(snapshot(side));
                    // ESCAPE aborts the chat line; ENTER would send it
                    if side == C {
                        c_ref_Key_Message(K_ESCAPE)
                    } else {
                        Key_Message(K_ESCAPE)
                    }
                    out.push(snapshot(side));
                }
                out
            },
        );
        per_side.push(states);
    }
    assert_eq!(per_side[0], per_side[1]);
    assert_eq!(per_side[0][0].chat, "hi there");
    assert_eq!(per_side[0][1].chat, "hi th");
    assert_eq!(per_side[0][2].clen, 0);
    // SAFETY: restore the shared default.
    unsafe { ctest_keys_set_dest(KEY_GAME) };
}
