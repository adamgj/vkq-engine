//! Differential test: `quake_rs::console` vs the original `Quake/console.c`.
//! Phase 7 M10c.
//!
//! `console.c` is composed into `stubs/console_ref.c` behind a TU-local rename
//! block rather than listed in `build.rs`'s `C_SOURCES`, for the same reason
//! `keys.c` is: the plain `Con_*` names already have link doubles that older
//! suites assert on. The consequence here is the usual one -- the oracle
//! answers to `c_ref_*` and the port answers to the plain names, and the two
//! halves own **disjoint** copies of everything `console.c` defines
//! (`con_text`, `con_linewidth`, `con_current`, `con_times`, the cvars,
//! `key_tabpartial`, `tablist`, the selection/hot-link statics, `log_file`).
//!
//! State that is NOT disjoint and that every test therefore re-seeds before
//! each side runs:
//!
//!  * `vid` / `glwidth` / `glheight` / `scr_con_current`, `realtime`,
//!    `host_rawframetime`, `scr_disabled_for_loading`;
//!  * the keys mirror -- `key_dest`, `key_lines`, `key_linepos`, `key_insert`,
//!    `edit_line`, `history_line`, `key_tabhint`, `keydown`, `chat_team` --
//!    which quake-capi's keys port owns once for this link;
//!  * `cl` / `cls`, owned once by quake-capi's cl_main port;
//!  * the call recorder and the draw log in `stubs/console_ref.c`, and the
//!    `[con]`/`[safe]`/`[link]` capture in `stubs/stubs.c`.
//!
//! Because of that every test follows the `both()` shape: for each side in
//! turn, seed, clear the recorders, invoke, and read the observation
//! **immediately**, before the other side runs.
//!
//! ADR-009: `Con_TabComplete` and `Con_ToggleConsole_f` are the two entry
//! points that can reach a raise, so the port is driven through
//! `quake_rs_con_tab_complete` / `quake_rs_con_toggle_console_f` +
//! `Host_Reraise` exactly as `Quake/console_glue.c` does; the fixture does
//! that, so a divergence in the raise status shows up as an abort here rather
//! than as a silent difference. None of the scenarios below make a guarded
//! callee raise.
//!
//! Not covered, and why: `Con_Warning`, `Con_DWarning`, `Con_DPrintf` and
//! `Con_DPrintf2` stay 100% C in `Quake/console_glue.c` (decision 1 of the
//! milestone contract), so there is no ported half to compare them against;
//! `Con_NotifyBox` stays C too and spins on `SCR_UpdateScreen` until a key
//! arrives, so it is unreachable from a test.

use core::ffi::{c_char, c_double, c_float, c_int, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, OnceLock};

use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_types::fs::MAX_OSPATH;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `stubs/console_ref.c`'s side convention: 1 = the `c_ref_*` oracle, 0 = the
/// Rust port.
const C: c_int = 1;
const R: c_int = 0;

// console.h:59-61 -- tabcomplete_t
const TABCOMPLETE_AUTOHINT: c_int = 0;
const TABCOMPLETE_USER: c_int = 1;

// keys.h:135-140 -- keydest_t
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
const KEY_MESSAGE: c_int = 2;

// keys.h -- the two keys Con_TabComplete / Con_UpdateMouseState read.
const K_SHIFT: c_int = 134;
const K_MOUSE1: c_int = 200;

// client.h -- cactive_t
const CA_DISCONNECTED: c_int = 1;
const CA_CONNECTED: c_int = 2;

// protocol/client: cl.gametype
const GAME_COOP: c_int = 0;
const GAME_DEATHMATCH: c_int = 1;

/// `stubs/stubs.c`'s `SIGNONS`; anything else makes `Con_Printf` reach the
/// `SCR_UpdateScreen` tail that stays C in `Quake/console_glue.c`.
const SIGNONS: c_int = 4;

const NUM_CON_TIMES: usize = 4;

/// The scrollback geometry `fresh` installs. 64 > (480 >> 3) + 1, so
/// Con_Linefeed's clamp is a no-op at con_backscroll == 0.
const CON_COLS: c_int = 20;
const CON_LINES: c_int = 64;
const MAXCMDLINE: usize = 256;

// ---------------------------------------------------------------------------
// FFI

/// `stubs/console_ref.c`'s `ctest_console_state_t`.
#[repr(C)]
#[derive(Clone, Copy)]
struct ConStateRaw {
    linewidth: c_int,
    buffersize: c_int,
    totallines: c_int,
    backscroll: c_int,
    current: c_int,
    x: c_int,
    vislines: c_int,
    initialized: c_int,
    debuglog: c_int,
    forcedup: c_int,
    redirected: c_int,
    tablistlen: c_int,
    times: [c_float; NUM_CON_TIMES],
    lastcenter: [c_char; 1024],
    redirect: [c_char; 8192],
    tabpartial: [c_char; MAXCMDLINE],
}

/// `stubs/console_ref.c`'s `ctest_console_calls_t`.
#[repr(C)]
#[derive(Clone, Copy)]
struct ConCallsRaw {
    updatescreen_calls: c_int,
    menumain_calls: c_int,
    explore_calls: c_int,
    clipboard_calls: c_int,
    setcursor_calls: c_int,
    cursor: c_int,
    sleep_calls: c_int,
    explore_path: [c_char; MAX_OSPATH],
    clipboard: [c_char; 4096],
    canvascolor: [c_float; 4],
}

#[allow(dead_code)]
extern "C" {
    // ---- per-side entry points -------------------------------------------
    fn ctest_console_print(side: c_int, txt: *const c_char);
    fn ctest_console_printf(side: c_int, txt: *const c_char);
    fn ctest_console_safeprintf(side: c_int, txt: *const c_char);
    fn ctest_console_linkprintf(side: c_int, addr: *const c_char, txt: *const c_char);
    fn ctest_console_centerprintf(side: c_int, linewidth: c_int, txt: *const c_char);
    fn ctest_console_strip(side: c_int, txt: *const c_char, out: *mut c_char, cap: c_int);
    fn ctest_console_checkresize(side: c_int);
    fn ctest_console_clear(side: c_int);
    fn ctest_console_dump(side: c_int);
    fn ctest_console_clearnotify(side: c_int);
    fn ctest_console_scroll(side: c_int, lines: c_int);
    fn ctest_console_selectall(side: c_int);
    fn ctest_console_mousemove(side: c_int, x: c_int, y: c_int);
    fn ctest_console_updatemousestate(side: c_int);
    fn ctest_console_copyselection(side: c_int) -> bool;
    fn ctest_console_quakebar(side: c_int, len: c_int, out: *mut c_char, cap: c_int);
    fn ctest_console_logcenterprint(side: c_int, str_: *const c_char);
    fn ctest_console_match(side: c_int, str_: *const c_char, partial: *const c_char) -> bool;
    fn ctest_console_addtotablist(
        side: c_int,
        name: *const c_char,
        partial: *const c_char,
        type_: *const c_char,
    );
    fn ctest_console_tabcomplete(side: c_int, mode: c_int);
    fn ctest_console_toggleconsole(side: c_int);
    fn ctest_console_messagemode(side: c_int, team: c_int);
    fn ctest_console_debuglog(side: c_int, msg: *const c_char);
    fn ctest_console_log_init(side: c_int, basedir: *const c_char, session: *const c_char);
    fn ctest_console_log_close(side: c_int);
    fn ctest_console_init(side: c_int);
    fn ctest_console_drawnotify(side: c_int);
    fn ctest_console_drawinput(side: c_int);
    fn ctest_console_drawconsole(side: c_int, lines: c_int, drawinput: bool);

    // ---- redirect --------------------------------------------------------
    fn ctest_console_redirect(side: c_int, on: bool);
    fn ctest_console_is_redirected(side: c_int) -> bool;
    fn ctest_console_redirect_output(side: c_int) -> *const c_char;
    fn ctest_console_clear_redirect_output();

    // ---- tab list --------------------------------------------------------
    fn ctest_console_tablist_count(side: c_int) -> c_int;
    fn ctest_console_tablist_entry(
        side: c_int,
        idx: c_int,
        name: *mut c_char,
        namecap: c_int,
        type_: *mut c_char,
        typecap: c_int,
    ) -> c_int;

    // ---- seeding and observation ----------------------------------------
    fn ctest_console_setup(side: c_int, buffersize: c_int, linewidth: c_int);
    fn ctest_console_reset(side: c_int);
    fn ctest_console_set_cvars(
        side: c_int,
        notifytime: *const c_char,
        logcenterprint: *const c_char,
        notifycenter: *const c_char,
        notifyfade: *const c_char,
        notifyfadetime: *const c_char,
        maxcols: *const c_char,
    );
    fn ctest_console_set_notify_time(side: c_int, index: c_int, t: c_float);
    fn ctest_console_set_forcedup(side: c_int, v: bool);
    fn ctest_console_set_tabpartial(side: c_int, text: *const c_char);
    fn ctest_console_snapshot(side: c_int, out: *mut ConStateRaw);
    fn ctest_console_buffer_size(side: c_int) -> c_int;
    fn ctest_console_get_line(side: c_int, line: c_int, out: *mut c_char, cap: c_int);

    // ---- shared engine state --------------------------------------------
    fn ctest_console_set_vid(conwidth: c_int, conheight: c_int, width: c_int, height: c_int);
    fn ctest_console_set_gl(w: c_int, h: c_int, concurrent: c_float);
    fn ctest_console_set_keydest(dest: c_int);
    fn ctest_console_get_keydest() -> c_int;
    fn ctest_console_set_editline(text: *const c_char, linepos: c_int, insert: c_int);
    fn ctest_console_get_editline(
        out: *mut c_char,
        cap: c_int,
        linepos: *mut c_int,
        hist: *mut c_int,
    );
    fn ctest_console_get_tabhint(out: *mut c_char, cap: c_int);
    fn ctest_console_set_chat_team(v: bool);
    fn ctest_console_get_chat_team() -> bool;
    fn ctest_console_set_cls(state: c_int, signon: c_int, demoplayback: bool, demoseeking: bool);
    fn ctest_console_set_cl_gametype(gametype: c_int);
    fn ctest_console_set_time(now: c_double, rawframetime: c_double);
    fn ctest_console_set_scr_disabled(v: bool);
    fn ctest_console_set_gamedir(side: c_int, dir: *const c_char);
    fn ctest_console_tokenize(side: c_int, text: *const c_char);
    fn ctest_console_set_keydown(key: c_int, down: bool);

    // ---- recorders -------------------------------------------------------
    fn ctest_console_reset_calls();
    fn ctest_console_get_calls(out: *mut ConCallsRaw);
    fn ctest_console_set_mouse(x: c_int, y: c_int);
    fn ctest_console_set_explore_result(v: bool);
    fn ctest_console_clipboard() -> *const c_char;
    fn ctest_console_clear_draw_log();
    fn ctest_console_draw_log() -> *const c_char;
    fn ctest_get_last_link_addr() -> *const c_char;

    // ---- stubs.c ---------------------------------------------------------
    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(index: c_int) -> *const c_char;
    fn ctest_set_args(argc: c_int, argv: *mut *mut c_char);
}

// ---------------------------------------------------------------------------
// Rust-side views

/// `ConStateRaw` rendered as comparable, printable values.
#[derive(Debug, PartialEq, Clone)]
struct Snap {
    linewidth: c_int,
    buffersize: c_int,
    totallines: c_int,
    backscroll: c_int,
    current: c_int,
    x: c_int,
    vislines: c_int,
    initialized: c_int,
    debuglog: c_int,
    forcedup: c_int,
    redirected: c_int,
    tablistlen: c_int,
    times: [c_float; NUM_CON_TIMES],
    lastcenter: String,
    redirect: String,
    tabpartial: String,
}

#[derive(Debug, PartialEq, Clone)]
struct Calls {
    updatescreen: c_int,
    menumain: c_int,
    explore: c_int,
    clipboard: c_int,
    setcursor: c_int,
    cursor: c_int,
    explore_path: String,
    canvascolor: [c_float; 4],
}

/// Everything a print-shaped scenario can observe: the geometry and the raw
/// scrollback bytes, masks included.
///
/// The `ctest_con_log` capture stream is deliberately NOT part of this: only
/// the plain half of stubs/console_ref.c calls `ctest_console_emit`, because
/// the oracle half's Con_Printf is console.c's own and its Sys_Printf echo is
/// a no-op there (stubs/console_ref.c:778). The log is therefore an
/// observation of the port alone, asserted after `both` has run the R side
/// last; the differential on console output is `lines` (con_text), which is
/// strictly stronger.
#[derive(PartialEq, Clone)]
struct Obs {
    snap: Snap,
    lines: Vec<Vec<u8>>,
}

/// Prints only the non-blank scrollback rows, escaped, so an assertion failure
/// is readable instead of a wall of 32s. Escaping keeps the colour masks
/// visible.
impl std::fmt::Debug for Obs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rows: Vec<String> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.iter().any(|&b| b != b' '))
            .map(|(i, l)| format!("{i}:{}", l.escape_ascii()))
            .collect();
        f.debug_struct("Obs")
            .field("snap", &self.snap)
            .field("lines", &rows)
            .finish()
    }
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// `ctest_console_set_cvars` stores the pointer it is handed in `cvar_t::string`,
/// so the storage has to outlive the call.
fn leak(s: &str) -> *const c_char {
    Box::leak(cs(s).into_boxed_c_str()).as_ptr()
}

fn c_field(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .map(|&c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn snap(side: c_int) -> Snap {
    // SAFETY: ADR-004. The fixture fills a caller-owned POD struct from one
    // side's globals; TEST_LOCK is held.
    let raw: Box<ConStateRaw> = unsafe {
        let mut raw: Box<ConStateRaw> = Box::new(core::mem::zeroed());
        ctest_console_snapshot(side, &mut *raw);
        raw
    };
    Snap {
        linewidth: raw.linewidth,
        buffersize: raw.buffersize,
        totallines: raw.totallines,
        backscroll: raw.backscroll,
        current: raw.current,
        x: raw.x,
        vislines: raw.vislines,
        initialized: raw.initialized,
        debuglog: raw.debuglog,
        forcedup: raw.forcedup,
        redirected: raw.redirected,
        tablistlen: raw.tablistlen,
        times: raw.times,
        lastcenter: c_field(&raw.lastcenter),
        redirect: c_field(&raw.redirect),
        tabpartial: c_field(&raw.tabpartial),
    }
}

fn calls() -> Calls {
    // SAFETY: ADR-004. Reads the fixture's recorder struct; TEST_LOCK is held.
    let raw: Box<ConCallsRaw> = unsafe {
        let mut raw: Box<ConCallsRaw> = Box::new(core::mem::zeroed());
        ctest_console_get_calls(&mut *raw);
        raw
    };
    Calls {
        updatescreen: raw.updatescreen_calls,
        menumain: raw.menumain_calls,
        explore: raw.explore_calls,
        clipboard: raw.clipboard_calls,
        setcursor: raw.setcursor_calls,
        cursor: raw.cursor,
        explore_path: c_field(&raw.explore_path),
        canvascolor: raw.canvascolor,
    }
}

/// One scrollback line as raw bytes, `con_linewidth` wide, high bits intact.
fn line_bytes(side: c_int, line: c_int, width: usize) -> Vec<u8> {
    let mut buf = vec![0 as c_char; 1024];
    // SAFETY: ADR-004. `buf` is 1024 bytes and the cap is passed as such; the
    // fixture memsets it and copies at most min(cap-1, con_linewidth) bytes.
    unsafe { ctest_console_get_line(side, line, buf.as_mut_ptr(), 1024) };
    buf[..width.min(1023)].iter().map(|&c| c as u8).collect()
}

/// The scrollback. Small buffers are read whole; for a `Con_Init`-sized buffer
/// only the 48 lines ending at `con_current` are read, which is where every
/// scenario below writes.
fn lines(side: c_int) -> Vec<Vec<u8>> {
    let s = snap(side);
    if s.totallines <= 0 || s.linewidth <= 0 {
        return Vec::new();
    }
    let w = s.linewidth as usize;
    if s.totallines <= 48 {
        (0..s.totallines).map(|i| line_bytes(side, i, w)).collect()
    } else {
        (s.current - 47..=s.current)
            .map(|i| line_bytes(side, i, w))
            .collect()
    }
}

fn con_log() -> Vec<String> {
    // SAFETY: ADR-004. Reads the stubs.c capture; TEST_LOCK is held.
    unsafe {
        (0..ctest_con_log_len())
            .map(|i| {
                CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

/// The port-side capture log with every colour mask stripped, so a tinted
/// echo (Con_FormatTabMatch, console.c:1919) reads back as ASCII.
fn con_log_unmasked() -> Vec<String> {
    // SAFETY: ADR-004. Reads the fixture's capture log; TEST_LOCK is held and
    // every entry is a NUL-terminated C string owned by stubs/stubs.c.
    unsafe {
        (0..ctest_con_log_len())
            .map(|i| {
                CStr::from_ptr(ctest_con_log_get(i))
                    .to_bytes()
                    .iter()
                    .map(|&b| (b & 0x7f) as char)
                    .collect()
            })
            .collect()
    }
}

fn draw_log() -> String {
    // SAFETY: ADR-004. Reads the fixture's draw stream; TEST_LOCK is held.
    unsafe {
        CStr::from_ptr(ctest_console_draw_log())
            .to_string_lossy()
            .into_owned()
    }
}

fn clipboard() -> String {
    // SAFETY: ADR-004. Reads the fixture's clipboard recorder.
    unsafe {
        CStr::from_ptr(ctest_console_clipboard())
            .to_string_lossy()
            .into_owned()
    }
}

fn editline() -> (String, c_int, c_int) {
    let mut buf = vec![0 as c_char; MAXCMDLINE];
    let mut pos: c_int = 0;
    let mut hist: c_int = 0;
    // SAFETY: ADR-004. `buf` holds MAXCMDLINE bytes, which is the cap passed.
    unsafe {
        ctest_console_get_editline(buf.as_mut_ptr(), MAXCMDLINE as c_int, &mut pos, &mut hist)
    };
    (c_field(&buf), pos, hist)
}

fn tabhint() -> String {
    let mut buf = vec![0 as c_char; MAXCMDLINE];
    // SAFETY: ADR-004. Same cap as the buffer's length.
    unsafe { ctest_console_get_tabhint(buf.as_mut_ptr(), MAXCMDLINE as c_int) };
    c_field(&buf)
}

fn observe(side: c_int) -> Obs {
    Obs {
        snap: snap(side),
        lines: lines(side),
    }
}

/// Runs `f` for the C oracle and then for the Rust port and asserts the two
/// observations agree, returning the (agreed) value so a test can also check
/// it against an independent expectation.
fn both<T: PartialEq + std::fmt::Debug>(mut f: impl FnMut(c_int) -> T) -> T {
    let c = f(C);
    let r = f(R);
    assert_eq!(c, r, "C oracle (left) vs Rust port (right)");
    c
}

/// The shared engine state every scenario starts from: a 640x480 canvas, a
/// disconnected client past the signon count (so `Con_Printf`'s C tail never
/// reaches `SCR_UpdateScreen`), no keys down, an empty edit line.
fn seed_shared() {
    // SAFETY: ADR-004. Plain stores into the harness-owned engine globals;
    // TEST_LOCK is held.
    unsafe {
        ctest_console_set_vid(640, 480, 640, 480);
        ctest_console_set_gl(640, 480, 480.0);
        ctest_console_set_cls(CA_DISCONNECTED, SIGNONS, false, false);
        ctest_console_set_cl_gametype(GAME_COOP);
        ctest_console_set_keydest(KEY_GAME);
        ctest_console_set_chat_team(false);
        ctest_console_set_time(0.0, 0.0);
        ctest_console_set_scr_disabled(false);
        ctest_console_set_keydown(K_SHIFT, false);
        ctest_console_set_keydown(K_MOUSE1, false);
        ctest_console_set_mouse(0, 0);
        ctest_console_set_explore_result(true);
        ctest_console_set_editline(cs("").as_ptr(), 1, 1);
        ctest_console_reset_calls();
        ctest_clear_con_log();
    }
}

/// Per-side reset: drops the buffer, links, selection, hot link, tab list and
/// redirect, then re-installs the shipped cvar defaults (console.c:70-75).
fn seed_side(side: c_int, buffersize: c_int, linewidth: c_int) {
    // SAFETY: ADR-004. Fixture setup on one side; TEST_LOCK is held.
    unsafe {
        ctest_console_reset(side);
        ctest_console_set_cvars(
            side,
            leak("3"),
            leak("1"),
            leak("0"),
            leak("0"),
            leak("0.5"),
            leak("0"),
        );
        if buffersize > 0 {
            ctest_console_setup(side, buffersize, linewidth);
        }
    }
}

/// `seed_shared` + `seed_side` + a cleared recorder, in the order every test
/// needs it. 1280 bytes over 20 columns is 64 lines: small enough to compare
/// whole, and -- this matters -- more than `(glheight >> 3) + 1`, so
/// Con_Linefeed's unconditional clamp (console.c:1099) leaves con_backscroll
/// at 0 instead of driving it to `con_totallines - 61`.
fn fresh(side: c_int) {
    seed_shared();
    seed_side(side, CON_LINES * CON_COLS, CON_COLS);
    // SAFETY: ADR-004. Recorder reset only.
    unsafe {
        ctest_console_reset_calls();
        ctest_clear_con_log();
    }
}

fn print(side: c_int, txt: &str) {
    let t = cs(txt);
    // SAFETY: ADR-004. `t` outlives the call; the callee only reads it.
    unsafe { ctest_console_print(side, t.as_ptr()) };
}

/// A scratch directory per side, so the two `Con_Dump_f` / `LOG_Init` runs
/// cannot see each other's file.
fn scratch(name: &str, side: c_int) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("vkq_ctest_console")
        .join(format!("{name}_{side}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scratch_str(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// `Con_Init` registers six cvars and five commands per side and can only run
/// once per registry, so it runs once for the whole binary and the geometry it
/// produced is remembered for `con_init_uses_fifty_columns`.
fn ensure_init() -> &'static (Snap, Snap) {
    static INIT: OnceLock<(Snap, Snap)> = OnceLock::new();
    INIT.get_or_init(|| {
        seed_shared();
        // SAFETY: ADR-004. Runs each side's Con_Init exactly once; TEST_LOCK is
        // held by the caller, and OnceLock makes it once per process.
        unsafe {
            ctest_console_reset(C);
            ctest_console_reset(R);
            ctest_clear_con_log();
            ctest_console_init(C);
            let c = snap(C);
            ctest_clear_con_log();
            ctest_console_init(R);
            let r = snap(R);
            (c, r)
        }
    })
}

// ---------------------------------------------------------------------------
// Con_Print: wrapping, control characters, scrollback

#[test]
fn con_print_wraps_at_linewidth() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // 46 characters with no space: longer than two 20-column lines.
        print(side, "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJ");
        observe(side)
    });
    assert_eq!(obs.snap.linewidth, 20);
    assert_eq!(obs.snap.x, 6, "46 % 20");
}

#[test]
fn con_print_word_wraps_at_a_boundary() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // "aaaa " fits; "bbbbbbbbbbbbbbbb" does not, so it starts a new line.
        print(side, "aaaa bbbbbbbbbbbbbbbb cc\n");
        observe(side)
    });
    let text: Vec<String> = obs
        .lines
        .iter()
        .map(|l| String::from_utf8_lossy(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        text.iter().any(|l| l == "aaaa"),
        "word wrap should leave \"aaaa\" alone on its line, got {text:?}"
    );
}

#[test]
fn con_print_newline_and_carriage_return() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        print(side, "first\nsecond\rrewritten\nthird\n");
        observe(side)
    });
    let text: Vec<String> = obs
        .lines
        .iter()
        .map(|l| String::from_utf8_lossy(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        text.contains(&"rewritten".to_string()),
        "the \\r line should have been overwritten in place, got {text:?}"
    );
    assert!(
        !text.contains(&"second".to_string()),
        "\\r + reprint should have replaced \"second\", got {text:?}"
    );
}

#[test]
fn con_print_colored_prefix_sets_the_high_bit() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        print(side, "\u{2}tinted\n");
        print(side, "plain\n");
        observe(side)
    });
    let tinted: Vec<u8> = obs
        .lines
        .iter()
        .flat_map(|l| l.iter().copied())
        .filter(|&b| b >= 0x80)
        .collect();
    assert_eq!(
        tinted,
        b"tinted".iter().map(|&b| b | 0x80).collect::<Vec<u8>>(),
        "only the \\x02-prefixed text should carry the colour mask"
    );
}

#[test]
fn con_print_talk_prefix_sets_the_high_bit() {
    let _g = lock();
    // console.c:1130 -- the \x01 prefix also plays misc/talk.wav; both sides
    // reach a real S_LocalSound (the port's, and snd_dma.c's under the
    // prelude's rename), which is silent with no sound device.
    let obs = both(|side| {
        fresh(side);
        print(side, "\u{1}chat\n");
        observe(side)
    });
    let tinted: Vec<u8> = obs
        .lines
        .iter()
        .flat_map(|l| l.iter().copied())
        .filter(|&b| b >= 0x80)
        .collect();
    assert_eq!(
        tinted,
        b"chat".iter().map(|&b| b | 0x80).collect::<Vec<u8>>()
    );
}

#[test]
fn con_print_wraps_around_totallines() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        for i in 0..130 {
            print(side, &format!("line{i}\n"));
        }
        observe(side)
    });
    // 64 lines of scrollback; the first 66 prints must have been overwritten.
    assert_eq!(obs.snap.totallines, CON_LINES);
    let text: Vec<String> = obs
        .lines
        .iter()
        .map(|l| String::from_utf8_lossy(l).trim_end().to_string())
        .collect();
    assert!(
        text.iter().any(|l| l == "line129"),
        "the newest line must survive, got {text:?}"
    );
    assert!(
        !text.iter().any(|l| l == "line0"),
        "the oldest line must have been overwritten, got {text:?}"
    );
}

#[test]
fn con_print_stamps_con_times() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. Seeds realtime, which Con_Print samples per line.
        unsafe { ctest_console_set_time(12.5, 0.0) };
        print(side, "a\nb\nc\n");
        observe(side)
    });
    assert!(
        obs.snap.times.contains(&12.5),
        "Con_Print should stamp con_times with realtime, got {:?}",
        obs.snap.times
    );
}

#[test]
fn con_linefeed_clamps_backscroll() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        for i in 0..30 {
            print(side, &format!("l{i}\n"));
        }
        // A short screen so both clamps are reachable: Con_Scroll's ceiling
        // (console.c:1032, vid.height) and Con_Linefeed's (console.c:1099,
        // glheight) are both con_totallines - 10 - 1 == 53.
        // SAFETY: ADR-004. Seeds vid.height and glheight.
        unsafe {
            ctest_console_set_vid(176, 80, 640, 80);
            ctest_console_set_gl(640, 80, 80.0);
        }
        ctest_scroll(side, 60);
        let scrolled = snap(side).backscroll;
        print(side, "more\n");
        (scrolled, observe(side))
    });
    assert_eq!(
        obs.0, 53,
        "Con_Scroll ceiling: totallines(64) - (80>>3) - 1"
    );
    assert_eq!(
        obs.1.snap.backscroll, 53,
        "Con_Linefeed bumps con_backscroll then re-clamps it (console.c:1097)"
    );
}

fn ctest_scroll(side: c_int, lines: c_int) {
    // SAFETY: ADR-004. One fixture call; TEST_LOCK is held.
    unsafe { ctest_console_scroll(side, lines) };
}

// ---------------------------------------------------------------------------
// Con_CheckResize / Con_RecalcOffset

#[test]
fn con_checkresize_reflows_narrower_and_wider() {
    let _g = lock();
    for conwidth in [336_i32, 640, 1024, 88] {
        let obs = both(|side| {
            fresh(side);
            for i in 0..12 {
                print(side, &format!("row{i:02} xyzzy\n"));
            }
            // SAFETY: ADR-004. Seeds vid.conwidth, the only input to the width
            // computation at console.c:964.
            unsafe { ctest_console_set_vid(conwidth, 480, 640, 480) };
            // SAFETY: ADR-004. Runs one side's Con_CheckResize.
            unsafe { ctest_console_checkresize(side) };
            observe(side)
        });
        assert_eq!(
            obs.snap.linewidth,
            (conwidth >> 3) - 2,
            "console.c:964 -- (vid.conwidth >> 3) - 2"
        );
        assert_eq!(
            obs.snap.totallines,
            (CON_LINES * CON_COLS) / obs.snap.linewidth
        );
        assert_eq!(obs.snap.current, obs.snap.totallines - 1);
        assert_eq!(obs.snap.backscroll, 0);
    }
}

#[test]
fn con_checkresize_is_a_noop_when_the_width_is_unchanged() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        print(side, "kept\n");
        // Con_Scroll's ceiling here is 64 - (480 >> 3) - 1 == 3.
        ctest_scroll(side, 3);
        // (176 >> 3) - 2 == 20 == the width `fresh` installed.
        // SAFETY: ADR-004. Seeds vid; then one Con_CheckResize.
        unsafe {
            ctest_console_set_vid(176, 480, 640, 480);
            ctest_console_checkresize(side);
        }
        observe(side)
    });
    assert_eq!(
        obs.snap.backscroll, 3,
        "the early return at console.c:966 must not clear con_backscroll"
    );
}

#[test]
fn con_checkresize_recalcs_link_offsets() {
    let _g = lock();
    // Con_RecalcOffset (console.c:940) only shows up through the links, and the
    // only handle on a link is the hand cursor Con_Mousemove sets when the
    // pointer is over one.
    let seen = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. A wide canvas so the link lands at a stable pixel.
        unsafe { ctest_console_set_vid(640, 480, 640, 480) };
        let addr = cs("/tmp/target.txt");
        let txt = cs("target.txt");
        // SAFETY: ADR-004. One fixture call per side.
        unsafe { ctest_console_linkprintf(side, addr.as_ptr(), txt.as_ptr()) };
        // SAFETY: ADR-004. Reflow to 78 columns, then lay out the console.
        unsafe {
            ctest_console_checkresize(side);
            ctest_console_drawconsole(side, 480, false);
            ctest_console_reset_calls();
        }
        let mut hits = Vec::new();
        for y in 0..60 {
            // SAFETY: ADR-004. Hover; the cursor answer is recorded.
            unsafe { ctest_console_mousemove(side, 40, y * 8) };
            hits.push(calls().cursor);
        }
        hits
    });
    assert!(
        seen.contains(&1),
        "MOUSECURSOR_HAND (vid.h:96) should be reached over the reflowed link, got {seen:?}"
    );
}

// ---------------------------------------------------------------------------
// Con_Clear_f, Con_ClearNotify, Con_Scroll

#[test]
fn con_clear_f_blanks_the_buffer_and_the_backscroll() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        for i in 0..10 {
            print(side, &format!("junk{i}\n"));
        }
        ctest_scroll(side, 3);
        // SAFETY: ADR-004. One fixture call per side.
        unsafe { ctest_console_clear(side) };
        observe(side)
    });
    assert_eq!(obs.snap.backscroll, 0);
    assert!(
        obs.lines.iter().all(|l| l.iter().all(|&b| b == b' ')),
        "Con_Clear_f must memset the whole buffer to ' '"
    );
}

#[test]
fn con_clearnotify_zeroes_the_times_ring() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        for i in 0..NUM_CON_TIMES {
            // SAFETY: ADR-004. Seeds one slot of the per-side ring.
            unsafe { ctest_console_set_notify_time(side, i as c_int, 1.0 + i as c_float) };
        }
        // SAFETY: ADR-004. One fixture call per side.
        unsafe { ctest_console_clearnotify(side) };
        observe(side)
    });
    assert_eq!(obs.snap.times, [0.0; NUM_CON_TIMES]);
}

#[test]
fn con_scroll_clamps_in_both_directions() {
    let _g = lock();
    for (height, delta) in [(480_i32, 100_i32), (480, -100), (80, 7), (80, -1), (480, 0)] {
        let obs = both(|side| {
            fresh(side);
            for i in 0..18 {
                print(side, &format!("s{i}\n"));
            }
            // SAFETY: ADR-004. Con_Scroll's upper clamp reads vid.height.
            unsafe { ctest_console_set_vid(176, height, 640, height) };
            ctest_scroll(side, delta);
            observe(side)
        });
        let ceiling = CON_LINES - (height >> 3) - 1;
        let expected = if delta > 0 {
            delta.min(ceiling)
        } else {
            0.max(delta)
        };
        assert_eq!(
            obs.snap.backscroll, expected,
            "height={height} delta={delta}"
        );
    }
}

// ---------------------------------------------------------------------------
// Con_Dump_f

#[test]
fn con_dump_f_writes_masked_bytes() {
    let _g = lock();
    let (bytes, log, linkaddr) = {
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut logs: Vec<Vec<String>> = Vec::new();
        let mut rel: Vec<String> = Vec::new();
        for side in [C, R] {
            fresh(side);
            let dir = scratch("dump", side);
            let dirs = cs(&scratch_str(&dir));
            // SAFETY: ADR-004. Seeds the per-side gamedir and argv.
            unsafe {
                ctest_console_set_gamedir(side, dirs.as_ptr());
                ctest_console_tokenize(side, cs("condump").as_ptr());
            }
            print(side, "\u{2}colored\n");
            print(side, "plain\n");
            // SAFETY: ADR-004. One fixture call per side.
            unsafe {
                ctest_clear_con_log();
                ctest_console_dump(side);
            }
            out.push(std::fs::read(dir.join("condump.txt")).unwrap());
            logs.push(con_log());
            // SAFETY: ADR-004. Reads the recorded link target.
            let addr = unsafe {
                CStr::from_ptr(ctest_get_last_link_addr())
                    .to_string_lossy()
                    .into_owned()
            };
            rel.push(addr);
        }
        assert_eq!(out[0], out[1], "C oracle (left) vs Rust port (right)");
        // logs[1] / rel[1] are the port's: only the plain half of
        // stubs/console_ref.c records them (see Obs).
        (out.remove(0), logs.remove(1), rel.remove(1))
    };
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    assert!(
        text.contains("colored"),
        "console.c:906 masks each byte with 0x7f, so the tinted line must read \
         back as plain ASCII; got {text:?}"
    );
    assert!(text.contains("plain"));
    assert!(
        !bytes.iter().any(|&b| b >= 0x80),
        "no byte in the dump may keep its colour mask"
    );
    assert_eq!(
        log,
        vec![
            "[safe] Dumped console text to ".to_string(),
            "[link] condump.txt".to_string(),
            "[safe] .\n".to_string(),
        ]
    );
    assert!(
        linkaddr.ends_with("/condump.txt"),
        "link target was {linkaddr:?}"
    );
}

#[test]
fn con_dump_f_takes_the_name_from_argv_and_adds_the_extension() {
    let _g = lock();
    let log = {
        let mut logs: Vec<Vec<String>> = Vec::new();
        for side in [C, R] {
            fresh(side);
            let dir = scratch("dumpname", side);
            let dirs = cs(&scratch_str(&dir));
            // SAFETY: ADR-004. Seeds the per-side gamedir and argv.
            unsafe {
                ctest_console_set_gamedir(side, dirs.as_ptr());
                ctest_console_tokenize(side, cs("condump mylog").as_ptr());
            }
            print(side, "hello\n");
            // SAFETY: ADR-004. One fixture call per side.
            unsafe {
                ctest_clear_con_log();
                ctest_console_dump(side);
            }
            assert!(
                dir.join("mylog.txt").exists(),
                "COM_AddExtension should have appended .txt (side {side})"
            );
            logs.push(con_log());
        }
        // The port's log; see Obs.
        logs.remove(1)
    };
    assert_eq!(log[1], "[link] mylog.txt");
}

// ---------------------------------------------------------------------------
// Redirection

#[test]
fn con_redirect_captures_and_flushes() {
    let _g = lock();
    let (state, flushed) = {
        let mut states = Vec::new();
        let mut flushes = Vec::new();
        for side in [C, R] {
            fresh(side);
            // SAFETY: ADR-004. Installs, feeds and tears down the redirect.
            unsafe {
                ctest_console_clear_redirect_output();
                assert!(!ctest_console_is_redirected(side));
                ctest_console_redirect(side, true);
                let a = ctest_console_is_redirected(side);
                ctest_console_printf(side, cs("captured\n").as_ptr());
                let mid = snap(side);
                ctest_console_redirect(side, false);
                let b = ctest_console_is_redirected(side);
                states.push((a, mid.redirect.clone(), mid.redirected, b));
                flushes.push(
                    CStr::from_ptr(ctest_console_redirect_output(side))
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        assert_eq!(states[0], states[1], "C oracle (left) vs Rust port (right)");
        assert_eq!(
            flushes[0], flushes[1],
            "C oracle (left) vs Rust port (right)"
        );
        (states.remove(0), flushes.remove(0))
    };
    assert_eq!(state, (true, "captured\n".to_string(), 1, false));
    assert_eq!(flushed, "captured\n");
}

// ---------------------------------------------------------------------------
// Con_LogCenterPrint / Con_CenterPrintf / Con_Quakebar

#[test]
fn con_logcenterprint_bars_and_dedups() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. Two calls with the same text; the second is dropped
        // by the duplicate check at console.c:1506.
        unsafe {
            ctest_console_logcenterprint(side, cs("YOU GOT THE SHOTGUN").as_ptr());
            ctest_console_logcenterprint(side, cs("YOU GOT THE SHOTGUN").as_ptr());
        }
        observe(side)
    });
    assert_eq!(obs.snap.lastcenter, "YOU GOT THE SHOTGUN");
    let bars = obs
        .lines
        .iter()
        .filter(|l| l.contains(&0x1du8) || l.contains(&0x1eu8))
        .count();
    assert_eq!(
        bars, 2,
        "one bar above and one below, printed once: {obs:?}"
    );
    // `both` ran the port last, so con_log() is the port's; see Obs.
    let logbars = con_log()
        .iter()
        .filter(|l| l.contains('\u{1d}') || l.contains('\u{1e}'))
        .count();
    assert_eq!(logbars, 2);
}

#[test]
fn con_logcenterprint_is_suppressed_in_deathmatch() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. console.c:1509 -- deathmatch + con_logcenterprint != 2.
        unsafe {
            ctest_console_set_cl_gametype(GAME_DEATHMATCH);
            ctest_console_logcenterprint(side, cs("FRAGGED").as_ptr());
        }
        observe(side)
    });
    assert_eq!(
        obs.snap.lastcenter, "",
        "the early return is before the strcpy"
    );
    assert!(
        obs.lines.iter().all(|l| l.iter().all(|&b| b == b' ')),
        "nothing should reach the console: {obs:?}"
    );
    assert!(con_log().is_empty());
}

#[test]
fn con_logcenterprint_skips_while_demoseeking() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. console.c:1502 -- the demoseeking early return.
        unsafe {
            ctest_console_set_cls(CA_CONNECTED, SIGNONS, true, true);
            ctest_console_logcenterprint(side, cs("SEEKING").as_ptr());
        }
        observe(side)
    });
    assert_eq!(obs.snap.lastcenter, "");
    assert!(obs.lines.iter().all(|l| l.iter().all(|&b| b == b' ')));
    assert!(con_log().is_empty());
}

#[test]
fn con_centerprintf_pads_each_line() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. One fixture call per side.
        unsafe { ctest_console_centerprintf(side, 16, cs("ab\ncdef\n").as_ptr()) };
        observe(side)
    });
    let text: Vec<String> = obs
        .lines
        .iter()
        .map(|l| String::from_utf8_lossy(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        text,
        vec!["       ab".to_string(), "      cdef".to_string()],
        "console.c:1470 pads with (linewidth - len) / 2 spaces"
    );
    // `both` ran the port last, so the capture log is the port's; see Obs.
    assert_eq!(
        con_log(),
        vec![
            "[con]        ab\n".to_string(),
            "[con]       cdef\n".to_string(),
        ]
    );
}

#[test]
fn con_quakebar_at_many_widths() {
    let _g = lock();
    for len in [2_i32, 3, 8, 20, 40, 41, 64] {
        let bar = both(|side| {
            fresh(side);
            let mut buf = vec![0 as c_char; 128];
            // SAFETY: ADR-004. `buf` is 128 bytes and 128 is the cap passed.
            unsafe { ctest_console_quakebar(side, len, buf.as_mut_ptr(), 128) };
            c_field(&buf)
        });
        let width = len.min(40).min(20) as usize;
        assert_eq!(
            bar.trim_end_matches('\n').chars().count(),
            width,
            "len={len}: clamped to min(40, con_linewidth)"
        );
        assert!(bar.starts_with('\u{1d}'), "len={len}: {bar:?}");
    }
}

// ---------------------------------------------------------------------------
// Con_StripControlPrefixes

#[test]
fn con_strip_control_prefixes() {
    let _g = lock();
    for input in ["\u{1}talk", "\u{2}tint", "plain", "", "\u{3}other"] {
        let out = both(|side| {
            fresh(side);
            let t = cs(input);
            let mut buf = vec![0 as c_char; 128];
            // SAFETY: ADR-004. `t` outlives the call; cap matches `buf`.
            unsafe { ctest_console_strip(side, t.as_ptr(), buf.as_mut_ptr(), 128) };
            c_field(&buf)
        });
        let expected = match input.as_bytes().first() {
            Some(1) | Some(2) => &input[1..],
            _ => input,
        };
        assert_eq!(out, expected, "input {input:?}");
    }
}

// ---------------------------------------------------------------------------
// Con_Printf family

#[test]
fn con_safeprintf_and_printf_reach_the_console() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. Three fixture calls per side.
        unsafe {
            ctest_console_printf(side, cs("one\n").as_ptr());
            ctest_console_safeprintf(side, cs("two\n").as_ptr());
        }
        observe(side)
    });
    // `both` ran the port last, so the capture log is the port's; see Obs.
    assert_eq!(
        con_log(),
        vec!["[con] one\n".to_string(), "[safe] two\n".to_string()]
    );
    let text: Vec<String> = obs
        .lines
        .iter()
        .map(|l| String::from_utf8_lossy(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(text, vec!["one".to_string(), "two".to_string()]);
}

#[test]
fn con_linkprintf_skips_leading_spaces_in_the_link_range() {
    let _g = lock();
    // console.c:1414 walks link->begin forward past leading spaces; the
    // observable is again the hand cursor over the link's first character.
    let seen = both(|side| {
        fresh(side);
        // "pre " first, so the link does not start at column 0: begin is then
        // (con_current, 4) and the three spaces are really skipped. With
        // con_x == 0 the skip walks the previous, blank line instead and eats
        // the whole range (console.c:1413).
        print(side, "pre ");
        // SAFETY: ADR-004. One link, then lay the console out.
        unsafe {
            ctest_console_linkprintf(side, cs("/tmp/x").as_ptr(), cs("   spaced").as_ptr());
            ctest_console_drawconsole(side, 480, false);
            ctest_console_reset_calls();
        }
        let mut hits = Vec::new();
        // The link is on con_current, i.e. y 464; column c is at (c+1)*8.
        for col in 0..16 {
            // SAFETY: ADR-004. Hover; the cursor answer is recorded.
            unsafe { ctest_console_mousemove(side, (col + 1) * 8, 464) };
            hits.push(calls().cursor);
        }
        hits
    });
    // "pre " occupies columns 0..3, the three skipped spaces 4..6 and
    // "spaced" 7..12.
    assert_eq!(
        seen[4..7],
        [2, 2, 2],
        "the skipped spaces must not be part of the link: {seen:?}"
    );
    assert_eq!(
        seen[7..13],
        [1, 1, 1, 1, 1, 1],
        "MOUSECURSOR_HAND over the link text itself: {seen:?}"
    );
}

#[test]
fn con_linkprintf_skips_leading_spaces_across_the_ring_buffer_wrap() {
    let _g = lock();
    // `fresh` leaves con_current at con_totallines - 1 with con_x == 0, so
    // Con_Print's leading Con_Linefeed (console.c:1175) puts the text on
    // physical row 0 while link->begin still names the last physical row.
    // The skip loop then has to walk begin from row 63 to row 64, i.e. wrap
    // back to row 0 (console.c:1421). Walking `text` straight off the end of
    // con_text instead reads past the allocation and can collapse the link to
    // an empty range, making it un-clickable. The collapse depends on what the
    // bytes past the allocation happen to be, so a plain run only catches it by
    // luck; the deterministic detector is Guard Malloc
    // (DYLD_INSERT_LIBRARIES=/usr/lib/libgmalloc.dylib MALLOC_STRICT_SIZE=1),
    // under which the pre-fix engine dies here with SIGSEGV.
    let seen = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. One link on the wrap boundary, then lay it out.
        unsafe {
            ctest_console_linkprintf(side, cs("/tmp/x").as_ptr(), cs("wrapped").as_ptr());
            ctest_console_drawconsole(side, 480, false);
            ctest_console_reset_calls();
        }
        let mut hits = Vec::new();
        // The link is on con_current, i.e. y 464; column c is at (c+1)*8.
        for col in 0..10 {
            // SAFETY: ADR-004. Hover; the cursor answer is recorded.
            unsafe { ctest_console_mousemove(side, (col + 1) * 8, 464) };
            hits.push(calls().cursor);
        }
        hits
    });
    assert_eq!(
        seen[0..7],
        [1, 1, 1, 1, 1, 1, 1],
        "MOUSECURSOR_HAND over \"wrapped\" after the skip wraps to row 0: {seen:?}"
    );
}

// ---------------------------------------------------------------------------
// Tab completion

#[test]
fn con_match_is_a_case_insensitive_substring_test() {
    let _g = lock();
    for (s, partial) in [
        ("con_notifytime", "notify"),
        ("con_notifytime", "NOTIFY"),
        ("con_notifytime", "zzz"),
        ("map", ""),
        ("", "x"),
    ] {
        let got = both(|side| {
            fresh(side);
            let a = cs(s);
            let b = cs(partial);
            // SAFETY: ADR-004. Both strings outlive the call.
            unsafe { ctest_console_match(side, a.as_ptr(), b.as_ptr()) }
        });
        assert_eq!(
            got,
            s.to_lowercase().contains(&partial.to_lowercase()),
            "Con_Match({s:?}, {partial:?})"
        );
    }
}

#[test]
fn con_addtotablist_sorts_and_dedups() {
    let _g = lock();
    let entries = both(|side| {
        fresh(side);
        for (name, ty) in [
            ("zulu", "command"),
            ("alpha", "cvar"),
            ("mike", "command"),
            ("alpha", "cvar"),
            ("alpha2", "cvar"),
        ] {
            let n = cs(name);
            let p = cs("a");
            let t = cs(ty);
            // SAFETY: ADR-004. All three strings outlive the call; the fixture
            // copies them into the node it allocates.
            unsafe { ctest_console_addtotablist(side, n.as_ptr(), p.as_ptr(), t.as_ptr()) };
        }
        read_tablist(side)
    });
    assert_eq!(
        entries,
        vec![
            ("alpha".to_string(), "cvar".to_string(), 2),
            ("alpha2".to_string(), "cvar".to_string(), 1),
        ],
        "Con_Match filters on \"a\", the list is alphabetized and the repeat is \
         folded into count (console.c:1660)"
    );
}

fn read_tablist(side: c_int) -> Vec<(String, String, c_int)> {
    // SAFETY: ADR-004. Walks the per-side list through the fixture; the two
    // buffers are 256 bytes each and 256 is the cap passed for both.
    unsafe {
        let n = ctest_console_tablist_count(side);
        (0..n)
            .map(|i| {
                let mut name = vec![0 as c_char; 256];
                let mut ty = vec![0 as c_char; 256];
                let count = ctest_console_tablist_entry(
                    side,
                    i,
                    name.as_mut_ptr(),
                    256,
                    ty.as_mut_ptr(),
                    256,
                );
                (c_field(&name), c_field(&ty), count)
            })
            .collect()
    }
}

#[test]
fn con_tabcomplete_user_lists_every_match() {
    let _g = lock();
    ensure_init();
    let entries = {
        let mut all = Vec::new();
        for side in [C, R] {
            seed_shared();
            seed_side(side, CON_LINES * CON_COLS, CON_COLS);
            // SAFETY: ADR-004. Seeds the shared edit line, then completes.
            unsafe {
                ctest_console_set_editline(cs("]con_notify").as_ptr(), 11, 1);
                ctest_console_set_tabpartial(side, cs("").as_ptr());
                ctest_console_reset_calls();
                ctest_clear_con_log();
                ctest_console_tabcomplete(side, TABCOMPLETE_USER);
            }
            all.push((read_tablist(side), editline()));
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        // The port ran last, so the capture log is the port's; see Obs.
        (all.remove(0), con_log_unmasked())
    };
    let (entries, log) = entries;
    // "con_notify" is already the longest common prefix, so Con_TabComplete
    // prints the list and drops it again (console.c:2065): the observable is
    // the echo, not the surviving tablist, and both sides agree it is empty.
    assert!(entries.0.is_empty(), "{entries:?}");
    let printed = log.concat();
    for name in [
        "con_notifycenter",
        "con_notifyfade",
        "con_notifyfadetime",
        "con_notifytime",
    ] {
        assert!(
            printed.contains(name),
            "Con_PrintTabList should have echoed {name}: {log:?}"
        );
    }
    assert_eq!(
        entries.1 .0, "]con_notify",
        "the shared prefix is already on the line, so nothing is inserted"
    );
}

#[test]
fn con_tabcomplete_autohint_sets_the_hint_without_printing() {
    let _g = lock();
    ensure_init();
    let observed = {
        let mut all = Vec::new();
        for side in [C, R] {
            seed_shared();
            seed_side(side, CON_LINES * CON_COLS, CON_COLS);
            // SAFETY: ADR-004. Seeds the shared edit line, then hints.
            unsafe {
                ctest_console_set_editline(cs("]condu").as_ptr(), 6, 1);
                ctest_console_set_tabpartial(side, cs("").as_ptr());
                ctest_console_reset_calls();
                ctest_clear_con_log();
                ctest_console_tabcomplete(side, TABCOMPLETE_AUTOHINT);
            }
            all.push((tabhint(), editline()));
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        (all.remove(0), con_log())
    };
    let ((hint, line), log) = observed;
    assert_eq!(hint, "mp", "\"condu\" + hint == \"condump\"");
    assert!(log.is_empty(), "AUTOHINT must not print the list: {log:?}");
    assert_eq!(line.0, "]condu", "AUTOHINT must not rewrite the edit line");
}

#[test]
fn con_tabcomplete_user_inserts_the_unique_match() {
    let _g = lock();
    ensure_init();
    let line = {
        let mut all = Vec::new();
        for side in [C, R] {
            seed_shared();
            seed_side(side, CON_LINES * CON_COLS, CON_COLS);
            // SAFETY: ADR-004. Seeds the shared edit line, then completes.
            unsafe {
                ctest_console_set_editline(cs("]condu").as_ptr(), 6, 1);
                ctest_console_set_tabpartial(side, cs("").as_ptr());
                ctest_console_reset_calls();
                ctest_clear_con_log();
                ctest_console_tabcomplete(side, TABCOMPLETE_USER);
            }
            all.push(editline());
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert_eq!(
        line.0, "]condump ",
        "a single match is inserted and gets a trailing space (console.c:2093)"
    );
    assert_eq!(line.1, 9);
}

#[test]
fn con_tabcomplete_cycles_forwards_and_backwards() {
    let _g = lock();
    ensure_init();
    // The matches for "e" that Con_Init registered, alphabetized, are
    // ... con_notifytime, messagemode, messagemode2 ...; the cycle steps one
    // either way and, unlike the unique-match branch, appends no space.
    for (shift, expected) in [(false, "]messagemode2"), (true, "]con_notifytime")] {
        let line = {
            let mut all = Vec::new();
            for side in [C, R] {
                seed_shared();
                seed_side(side, CON_LINES * CON_COLS, CON_COLS);
                // SAFETY: ADR-004. key_tabpartial already set means the "cycle"
                // branch at console.c:2043; keydown[K_SHIFT] picks the
                // direction.
                //
                // That branch reuses Con_TabComplete's insert point instead of
                // recomputing it, and the insert point is a function-local
                // `static char *c` (console.c:1995), mirrored by TAB_C in the
                // port. No fixture can reach either one, so presetting
                // key_tabpartial is not enough on its own: the cycle state has
                // to be entered the way the engine enters it, with a
                // first-time-through completion. Otherwise `c` holds whatever
                // an earlier test left behind -- or NULL, if this test runs
                // first, and console.c:2023 dereferences it.
                unsafe {
                    ctest_console_set_editline(cs("]e").as_ptr(), 2, 1);
                    ctest_console_set_tabpartial(side, cs("").as_ptr());
                    ctest_console_tabcomplete(side, TABCOMPLETE_USER);

                    ctest_console_set_editline(cs("]messagemode").as_ptr(), 12, 1);
                    ctest_console_set_tabpartial(side, cs("e").as_ptr());
                    ctest_console_set_keydown(K_SHIFT, shift);
                    ctest_console_reset_calls();
                    ctest_clear_con_log();
                    ctest_console_tabcomplete(side, TABCOMPLETE_USER);
                }
                all.push(editline());
            }
            assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
            all.remove(0)
        };
        assert_eq!(line.0, expected, "shift={shift}");
    }
}

#[test]
fn con_tabcomplete_on_an_empty_line_does_nothing() {
    let _g = lock();
    ensure_init();
    let observed = {
        let mut all = Vec::new();
        for side in [C, R] {
            seed_shared();
            seed_side(side, CON_LINES * CON_COLS, CON_COLS);
            // SAFETY: ADR-004. console.c:2013 -- the empty-edit-line return.
            unsafe {
                ctest_console_set_editline(cs("]").as_ptr(), 1, 1);
                ctest_console_set_tabpartial(side, cs("").as_ptr());
                ctest_clear_con_log();
                ctest_console_tabcomplete(side, TABCOMPLETE_USER);
            }
            all.push((read_tablist(side), editline(), tabhint()));
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        (all.remove(0), con_log())
    };
    assert!(observed.0 .0.is_empty());
    assert_eq!(observed.0 .1 .0, "]");
    assert_eq!(observed.0 .2, "");
    assert!(observed.1.is_empty());
}

#[test]
fn con_tabcomplete_parses_the_command_before_the_cursor() {
    let _g = lock();
    ensure_init();
    // ParseCommand (console.c:1708) restarts at the last ';', so only the text
    // after it is completed.
    let entries = {
        let mut all = Vec::new();
        for side in [C, R] {
            seed_shared();
            seed_side(side, CON_LINES * CON_COLS, CON_COLS);
            // SAFETY: ADR-004. Seeds the shared edit line, then completes.
            unsafe {
                ctest_console_set_editline(cs("]echo hi;condu").as_ptr(), 14, 1);
                ctest_console_set_tabpartial(side, cs("").as_ptr());
                ctest_clear_con_log();
                ctest_console_tabcomplete(side, TABCOMPLETE_AUTOHINT);
            }
            all.push((tabhint(), editline()));
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert_eq!(
        entries.0, "mp",
        "\"condu\" after the ';' completes to condump"
    );
}

// ---------------------------------------------------------------------------
// Con_ToggleConsole_f / Con_MessageMode*

#[test]
fn con_toggleconsole_from_the_game_opens_the_console() {
    let _g = lock();
    let observed = {
        let mut all = Vec::new();
        for side in [C, R] {
            fresh(side);
            // SAFETY: ADR-004. key_dest != key_console -> the else branch.
            unsafe {
                ctest_console_set_keydest(KEY_GAME);
                ctest_console_toggleconsole(side);
                all.push((ctest_console_get_keydest(), calls(), snap(side).times));
            }
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert_eq!(observed.0, KEY_CONSOLE);
    assert_eq!(observed.2, [0.0; NUM_CON_TIMES]);
}

#[test]
fn con_toggleconsole_from_the_console_returns_to_the_game_when_connected() {
    let _g = lock();
    let observed = {
        let mut all = Vec::new();
        for side in [C, R] {
            fresh(side);
            // SAFETY: ADR-004. cls.state == ca_connected -> IN_Activate branch.
            unsafe {
                ctest_console_set_cls(CA_CONNECTED, SIGNONS, false, false);
                ctest_console_set_keydest(KEY_CONSOLE);
                ctest_console_set_editline(cs("]typing").as_ptr(), 7, 1);
                ctest_scroll(side, 3);
                ctest_console_toggleconsole(side);
                all.push((ctest_console_get_keydest(), editline(), calls(), snap(side)));
            }
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert_eq!(observed.0, KEY_GAME);
    assert_eq!(observed.1 .0, "]", "the typed line is cleared");
    assert_eq!(observed.1 .1, 1);
    assert_eq!(observed.2.menumain, 0, "connected -> no menu");
    assert_eq!(observed.3.backscroll, 0);
}

#[test]
fn con_toggleconsole_from_the_console_opens_the_menu_when_disconnected() {
    let _g = lock();
    let observed = {
        let mut all = Vec::new();
        for side in [C, R] {
            fresh(side);
            // SAFETY: ADR-004. cls.state != ca_connected -> M_Menu_Main_f.
            unsafe {
                ctest_console_set_cls(CA_DISCONNECTED, SIGNONS, false, false);
                ctest_console_set_keydest(KEY_CONSOLE);
                ctest_console_toggleconsole(side);
                all.push((ctest_console_get_keydest(), calls().menumain));
            }
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert_eq!(observed.1, 1, "M_Menu_Main_f (console.c:763)");
}

#[test]
fn con_messagemode_needs_a_live_connection() {
    let _g = lock();
    for (state, demo, team) in [
        (CA_CONNECTED, false, 0),
        (CA_CONNECTED, false, 1),
        (CA_CONNECTED, true, 0),
        (CA_DISCONNECTED, false, 0),
    ] {
        let observed = {
            let mut all = Vec::new();
            for side in [C, R] {
                fresh(side);
                // SAFETY: ADR-004. console.c:918/931 -- the two early returns.
                unsafe {
                    ctest_console_set_cls(state, SIGNONS, demo, false);
                    ctest_console_set_keydest(KEY_GAME);
                    ctest_console_set_chat_team(false);
                    ctest_console_messagemode(side, team);
                    all.push((ctest_console_get_keydest(), ctest_console_get_chat_team()));
                }
            }
            assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
            all.remove(0)
        };
        let live = state == CA_CONNECTED && !demo;
        assert_eq!(
            observed,
            (if live { KEY_MESSAGE } else { KEY_GAME }, live && team == 1),
            "state={state} demo={demo} team={team}"
        );
    }
}

// ---------------------------------------------------------------------------
// Selection, mouse state and the clipboard

#[test]
fn con_selectall_then_copy_puts_the_scrollback_on_the_clipboard() {
    let _g = lock();
    let (ok, text) = {
        let mut all = Vec::new();
        for side in [C, R] {
            fresh(side);
            print(side, "alpha\n");
            print(side, "bravo\n");
            // SAFETY: ADR-004. Selection needs a laid-out console first.
            unsafe {
                ctest_console_set_keydest(KEY_CONSOLE);
                ctest_console_drawconsole(side, 200, false);
                ctest_console_reset_calls();
                ctest_console_selectall(side);
                let ok = ctest_console_copyselection(side);
                all.push((ok, clipboard()));
            }
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert!(ok, "a non-empty selection copies");
    assert!(
        text.contains("alpha") && text.contains("bravo"),
        "clipboard was {text:?}"
    );
}

#[test]
fn con_copyselection_with_no_selection_returns_false() {
    let _g = lock();
    let ok = both(|side| {
        fresh(side);
        print(side, "alpha\n");
        // SAFETY: ADR-004. Con_GetNormalizedSelection fails with no selection.
        unsafe {
            ctest_console_set_keydest(KEY_CONSOLE);
            ctest_console_drawconsole(side, 200, false);
            ctest_console_copyselection(side)
        }
    });
    assert!(!ok, "console.c:806 -- the early return");
}

#[test]
fn con_mouse_drag_selects_a_range() {
    let _g = lock();
    let (ok, text, cursors) = {
        let mut all = Vec::new();
        for side in [C, R] {
            fresh(side);
            for i in 0..6 {
                print(side, &format!("row{i}\n"));
            }
            let mut cursors = Vec::new();
            // Con_ScreenToCanvas (console.c:222) is the identity here because
            // scr_con_current == glheight == vid.conheight == 480, so
            // Con_CanvasToOffset maps screen y to con_current - ((480-y)/8 - 2).
            // Con_Print linefeeds before each line, so row0..row5 are at
            // con_current-5..con_current; line con_current-k is at y 464-8k,
            // and column c is at x (c + CON_MARGIN) * 8 == (c+1)*8.
            // SAFETY: ADR-004. Press at one offset, drag to another, copy.
            unsafe {
                ctest_console_set_keydest(KEY_CONSOLE);
                ctest_console_drawconsole(side, 480, false);
                ctest_console_reset_calls();

                ctest_console_set_mouse(16, 424);
                ctest_console_set_keydown(K_MOUSE1, true);
                ctest_console_updatemousestate(side);
                cursors.push(calls().cursor);

                ctest_console_set_mouse(40, 448);
                ctest_console_mousemove(side, 40, 448);
                cursors.push(calls().cursor);

                let ok = ctest_console_copyselection(side);
                all.push((ok, clipboard(), cursors));
            }
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert!(ok, "the drag should have produced a non-empty selection");
    assert!(!text.is_empty(), "clipboard {text:?} cursors {cursors:?}");
    assert_eq!(
        cursors[1], 2,
        "MOUSECURSOR_IBEAM (vid.h:97) while dragging, got {cursors:?}"
    );
}

#[test]
fn con_updatemousestate_clears_the_selection_outside_the_console() {
    let _g = lock();
    let observed = {
        let mut all = Vec::new();
        for side in [C, R] {
            fresh(side);
            print(side, "text\n");
            // SAFETY: ADR-004. Select, then leave the console: console.c:681
            // clears the hot link, the mouse state and the selection.
            unsafe {
                ctest_console_set_keydest(KEY_CONSOLE);
                ctest_console_drawconsole(side, 200, false);
                ctest_console_selectall(side);
                ctest_console_set_keydest(KEY_GAME);
                ctest_console_reset_calls();
                ctest_console_updatemousestate(side);
                let copied = ctest_console_copyselection(side);
                all.push((copied, calls().cursor));
            }
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert!(!observed.0, "the selection must be gone");
    assert_eq!(observed.1, 0, "MOUSECURSOR_DEFAULT");
}

#[test]
fn con_double_click_selects_a_word() {
    let _g = lock();
    let text = {
        let mut all = Vec::new();
        for side in [C, R] {
            fresh(side);
            print(side, "alpha bravo charlie\n");
            // SAFETY: ADR-004. Two press/release cycles inside DOUBLECLICK_TIME
            // drive Con_TestWordBoundary (console.c:471).
            unsafe {
                ctest_console_set_keydest(KEY_CONSOLE);
                ctest_console_drawconsole(side, 480, false);
                ctest_console_set_time(0.0, 0.0);
                ctest_console_reset_calls();

                // The only text line is con_current itself, i.e. y 464; x 24
                // is column 2, the "p" of "alpha".
                ctest_console_set_mouse(24, 464);
                ctest_console_set_keydown(K_MOUSE1, true);
                ctest_console_updatemousestate(side);
                ctest_console_set_keydown(K_MOUSE1, false);
                ctest_console_updatemousestate(side);
                ctest_console_set_keydown(K_MOUSE1, true);
                ctest_console_updatemousestate(side);

                ctest_console_copyselection(side);
                all.push(clipboard());
            }
        }
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert!(
        ["alpha", "bravo", "charlie", " "].contains(&text.as_str()),
        "a double click should select exactly one word or the gap, got {text:?}"
    );
}

// ---------------------------------------------------------------------------
// Drawing

#[test]
fn con_drawnotify_matches() {
    let _g = lock();
    for centered in ["0", "1"] {
        let log = {
            let mut all = Vec::new();
            for side in [C, R] {
                seed_shared();
                seed_side(side, CON_LINES * CON_COLS, CON_COLS);
                // SAFETY: ADR-004. con_notifycenter picks the two layouts at
                // console.c:2166/2172.
                unsafe {
                    ctest_console_set_cvars(
                        side,
                        leak("3"),
                        leak("1"),
                        leak(centered),
                        leak("0"),
                        leak("0.5"),
                        leak("0"),
                    );
                    ctest_console_set_time(1.0, 0.0);
                }
                print(side, "notify one\n");
                print(side, "notify two\n");
                // SAFETY: ADR-004. One draw pass per side.
                unsafe {
                    ctest_console_clear_draw_log();
                    ctest_console_drawnotify(side);
                }
                all.push(draw_log());
            }
            assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
            all.remove(0)
        };
        assert!(
            log.contains("canvas ") && log.contains("char "),
            "centered={centered}: expected a canvas and characters, got {log:?}"
        );
    }
}

#[test]
fn con_drawnotify_draws_the_chat_prompt() {
    let _g = lock();
    for team in [false, true] {
        let log = {
            let mut all = Vec::new();
            for side in [C, R] {
                seed_shared();
                seed_side(side, CON_LINES * CON_COLS, CON_COLS);
                // SAFETY: ADR-004. key_message + chat_team pick the prompt at
                // console.c:2181.
                unsafe {
                    ctest_console_set_keydest(KEY_MESSAGE);
                    ctest_console_set_chat_team(team);
                    ctest_console_clear_draw_log();
                    ctest_console_drawnotify(side);
                }
                all.push(draw_log());
            }
            assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
            all.remove(0)
        };
        let want = if team { "|say_team:|" } else { "|say:|" };
        assert!(log.contains(want), "team={team}: got {log:?}");
    }
}

#[test]
fn con_drawinput_matches() {
    let _g = lock();
    for insert in [0, 1] {
        let log = {
            let mut all = Vec::new();
            for side in [C, R] {
                seed_shared();
                seed_side(side, CON_LINES * CON_COLS, CON_COLS);
                // SAFETY: ADR-004. key_insert picks pic_ins/pic_ovr.
                unsafe {
                    ctest_console_set_keydest(KEY_CONSOLE);
                    ctest_console_set_editline(cs("]impulse 9").as_ptr(), 6, insert);
                    ctest_console_set_time(0.25, 0.0);
                    ctest_console_drawconsole(side, 200, false);
                    ctest_console_clear_draw_log();
                    ctest_console_drawinput(side);
                }
                all.push(draw_log());
            }
            assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
            all.remove(0)
        };
        assert!(
            log.contains("char "),
            "insert={insert}: the edit line should be drawn, got {log:?}"
        );
    }
}

#[test]
fn con_drawinput_is_silent_outside_the_console() {
    let _g = lock();
    let log = both(|side| {
        seed_shared();
        seed_side(side, 400, 20);
        // SAFETY: ADR-004. console.c:2203 -- the key_dest early return.
        unsafe {
            ctest_console_set_keydest(KEY_GAME);
            ctest_console_clear_draw_log();
            ctest_console_drawinput(side);
        }
        draw_log()
    });
    assert_eq!(log, "", "nothing should be drawn");
}

#[test]
fn con_drawconsole_matches() {
    let _g = lock();
    for (lines, input, backscroll) in [
        (0_i32, false, 0_i32),
        (200, false, 0),
        (200, true, 0),
        (200, true, 3),
    ] {
        let log = {
            let mut all = Vec::new();
            for side in [C, R] {
                seed_shared();
                seed_side(side, CON_LINES * CON_COLS, CON_COLS);
                for i in 0..12 {
                    print(side, &format!("draw{i}\n"));
                }
                // SAFETY: ADR-004. One draw pass per side.
                unsafe {
                    ctest_console_set_keydest(KEY_CONSOLE);
                    ctest_console_set_editline(cs("]hello").as_ptr(), 6, 1);
                    ctest_scroll(side, -backscroll);
                    ctest_console_clear_draw_log();
                    ctest_console_drawconsole(side, lines, input);
                }
                all.push((draw_log(), snap(side).vislines));
            }
            assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
            all.remove(0)
        };
        if lines == 0 {
            assert_eq!(log.0, "", "console.c:2288 -- lines <= 0 draws nothing");
        } else {
            assert!(
                log.0.contains("conback"),
                "expected the background: {:?}",
                log.0
            );
            assert_eq!(log.1, lines, "con_vislines tracks the lines argument");
        }
    }
}

#[test]
fn con_drawconsole_highlights_the_selection() {
    let _g = lock();
    let log = both(|side| {
        seed_shared();
        seed_side(side, 400, 20);
        for i in 0..8 {
            print(side, &format!("sel{i}\n"));
        }
        // SAFETY: ADR-004. Con_DrawSelectionHighlight only runs with a
        // selection, which Con_SelectAll installs.
        unsafe {
            ctest_console_set_keydest(KEY_CONSOLE);
            ctest_console_drawconsole(side, 480, false);
            ctest_console_selectall(side);
            ctest_console_clear_draw_log();
            ctest_console_drawconsole(side, 480, false);
        }
        draw_log()
    });
    assert!(
        log.contains("fill "),
        "the highlight is a Draw_Fill, got {log:?}"
    );
}

// ---------------------------------------------------------------------------
// Con_Init, Con_DebugLog, LOG_Init / LOG_Close

#[test]
fn con_init_uses_fifty_columns() {
    let _g = lock();
    let (c, r) = ensure_init();
    assert_eq!(c, r, "C oracle (left) vs Rust port (right)");
    assert_eq!(c.linewidth, 50, "console.c:1068");
    assert_eq!(c.buffersize, 1024 * 1024, "CON_TEXTSIZE, no -consize");
    assert_eq!(c.totallines, (1024 * 1024) / 50);
    assert_eq!(c.current, c.totallines - 1);
    assert_eq!(c.backscroll, 0);
    assert_eq!(c.initialized, 1);
}

#[test]
fn log_init_debuglog_and_close_write_the_same_bytes() {
    let _g = lock();
    let bodies = {
        let mut all = Vec::new();
        // The oracle's LOG_Init gates on -condebug (console.c:2414); the port's
        // gate stays in Quake/console_glue.c, so only the oracle needs this.
        let parm = cs("-condebug");
        let mut argv = [cs("vkquake").into_raw(), parm.as_ptr() as *mut c_char];
        // SAFETY: ADR-004. `argv` and its strings outlive every call below.
        unsafe { ctest_set_args(2, argv.as_mut_ptr()) };

        for side in [C, R] {
            fresh(side);
            let dir = scratch("log", side);
            let dirs = cs(&scratch_str(&dir));
            let session = cs("01/02/2034 05:06:07");
            // SAFETY: ADR-004. Opens, writes and closes one side's log file.
            unsafe {
                ctest_console_log_init(side, dirs.as_ptr(), session.as_ptr());
                let opened = snap(side).debuglog;
                ctest_console_debuglog(side, cs("direct line\n").as_ptr());
                ctest_console_printf(side, cs("through Con_Printf\n").as_ptr());
                ctest_console_log_close(side);
                let body = std::fs::read_to_string(dir.join("qconsole.log")).unwrap();
                // The first line carries a wall-clock timestamp on the oracle
                // side and the fixture's fixed session string on the port side,
                // so only its shape is comparable.
                let (head, rest) = body.split_once('\n').unwrap();
                all.push((
                    opened,
                    head.starts_with("LOG started on: "),
                    rest.to_string(),
                ));
            }
        }
        // SAFETY: ADR-004. Drops the command line again.
        unsafe { ctest_set_args(0, argv.as_mut_ptr()) };
        // SAFETY: ADR-004. Reclaims the CString handed to into_raw above.
        unsafe { drop(CString::from_raw(argv[0])) };
        assert_eq!(all[0], all[1], "C oracle (left) vs Rust port (right)");
        all.remove(0)
    };
    assert_eq!(bodies.0, 1, "LOG_Init sets con_debuglog");
    assert!(bodies.1, "console.c:2442 -- the header line");
    assert_eq!(
        bodies.2.replace("\r\n", "\n"),
        "direct line\nthrough Con_Printf\n",
        "Sys_fopen opens a text stream, so the CRT translates \n on Windows; \
         the two sides are still compared byte for byte above"
    );
}

#[test]
fn con_debuglog_without_a_log_file_is_a_noop() {
    let _g = lock();
    let obs = both(|side| {
        fresh(side);
        // SAFETY: ADR-004. console.c:1218 -- the !log_file early return.
        unsafe { ctest_console_debuglog(side, cs("dropped\n").as_ptr()) };
        observe(side)
    });
    assert_eq!(obs.snap.debuglog, 0);
}
