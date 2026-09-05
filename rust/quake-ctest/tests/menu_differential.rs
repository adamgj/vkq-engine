//! Differential test: `quake_rs::menu` vs the original `Quake/menu.c`.
//! Phase 7 M10e.
//!
//! `menu.c` is composed into `stubs/menu_ref.c` behind a TU-local rename block
//! rather than listed in `build.rs`'s `C_SOURCES`; the two reasons are written
//! out at the top of that file. The one that shapes this suite: **no plain
//! `M_*` name comes from `menu_ref.c` at all**, because six of them are
//! already link doubles that ~1200 existing assertions hold `console.c`,
//! `draw`, `host.c` and `keys.c` to. So the oracle half is driven through
//! `c_ref_M_*` and the port half through `quake_rs_menu_*`, and
//! `Quake/menu_glue.c`'s plain-name layer is verified by the engine link,
//! not here.
//!
//! Everything `menu.c` defines is per side -- roughly eighty statics: every
//! cursor, every scroll offset, the savegame table, the setup/lanconfig
//! scratch buffers. That is the point: a divergence in one key handler shows
//! up in every later draw.
//!
//! What both halves share, and what `seed_shared` therefore re-seeds before
//! **each** side runs: `vid`, `glwidth`, `glheight`, `scr_con_current`,
//! `realtime`, `host_rawframetime`, `key_dest`, `keydown[]`, `slistInProgress`,
//! `multiuser`, `vulkan_globals` (behind `ctest_menu_set_caps`), the mouse
//! position, the `modlist` / `extralevels_sorted` heads, the draw log in
//! `stubs/draw_ref.c` and the console capture in `stubs/stubs.c`.
//!
//! Split per side, and seeded per side: `cl` / `cls`, `sv` / `svs`,
//! `hostCacheCount` / `net_hostport` / `ipv4Available` / `ipv6Available`, the
//! command buffer behind `cmd_text`, the eight objects `Quake/menu_glue.c`
//! owns, and all 52 writable cvars `menu.c` touches.
//!
//! THE OBSERVATION. A menu function's entire observable behaviour is four
//! ordered records plus the glue-owned state: the `Draw_*` log, the console
//! log, the text it appended to the command buffer, and the cvars it wrote.
//! `Obs` captures all four plus `m_state`, `m_return_state`, `m_entersound`,
//! `m_is_quitting`, `m_return_onerror`, `m_return_reason`, `key_dest` and
//! `cls.demonum`, and every assertion compares the whole thing. `assert_drawn`
//! additionally refuses an empty C draw log: two empty buffers compared equal
//! is a defect, not a pass.
//!
//! ADR-009. `menu.c` names no longjmp-capable function itself; every raise it
//! can propagate comes back out of an engine callee, through one of the ten
//! `Menu_Glue_*` guards. `stubs/stubs.c`'s `ctest_set_menu_raise_mask` arms
//! `SCR_ModalMessage`, `M_Menu_Video_f`, `M_Video_Draw` and `M_Video_Key` to
//! `Host_Error`, which is what lets `raises_*` below drive a real raise out of
//! `M_Keydown` (three ways: the reset-config modal at `menu.c:2274`, the
//! options row that opens the video menu at `:2284`, and `M_Video_Key`) and
//! out of `M_Draw`, and check that both halves report the same status, the
//! same message and the same state left behind.
//!
//! NOT COVERED, stated rather than papered over:
//!
//!  * `S_LocalSound`. `snd_dma.c` is an oracle TU, so `c_ref_S_LocalSound` is
//!    the real function and the plain name is quake-capi's port -- but
//!    `snd_dma.c:S_LocalSound` returns before any output when `sound_started`
//!    is false, which it is here. The three `misc/menu*.wav` calls the menu
//!    makes are therefore silent no-ops on both sides and this suite cannot
//!    tell a missing one from a present one.
//!  * `registered`. One object both halves read, because it is `CVAR_ROM` and
//!    `menu.c` only reads it (`:638`).
//!  * The video menu itself. `M_Menu_Video_f` / `M_Video_Draw` / `M_Video_Key`
//!    live in `gl_vidsdl.c` and are not part of M10e; the doubles in
//!    `stubs/stubs.c` record the call and its argument, which is exactly the
//!    marshalling under comparison.
//!  * The body of `NET_Slist_f`. There is one implementation in this link and
//!    both halves call it, so it cannot diverge -- and it cannot be entered
//!    twice per process either (see `seed_shared`), so `slistInProgress` is
//!    seeded true and every menu path that reaches it takes the early return
//!    on both sides. The menu's own contribution -- the scope it selects and
//!    the state it writes around the call -- is compared.
//!  * What the real `NET_Poll` would put in the host cache. The only
//!    `NET_Poll` in this link is `stubs/host_ref.c`'s double, so
//!    `M_Search_Draw`'s `slistInProgress` branch (`menu.c:4436`) is compared
//!    for how it propagates a raise out of that double -- which is the one
//!    place `Host_Glue_NET_Poll`'s re-raise is observable -- and not for what
//!    a real poll would have changed underneath it.
//!  * `CL_NextDemo`'s body (`menu.c:673`, `:4692`). It reaches `CL_Disconnect`
//!    and then the aborting `CDAudio_Stop`, so every test runs with
//!    `cl_startdemos` 0 and `main_menu_demo_loop_guard_matches` drives the
//!    three guard conjuncts instead.
//!  * `COM_Rand`'s shared PRNG. `menu.c:3571` advances it into `msg_number`,
//!    which nothing then reads -- the quit box draws three fixed strings
//!    (`menu.c:3637-3639`) -- so the two halves consuming two different draws
//!    from one shared generator is unobservable here. If a later change gives
//!    `msg_number` a reader, this stops being true.
//!  * The other two re-raising wrappers in `Quake/menu_glue.c`,
//!    `M_ToggleMenu_f` (:310) and `M_UpdateMouse` (:318). Neither has an
//!    armable raise in this link: `M_ToggleMenu_f` can only raise through
//!    `Con_ToggleConsole_f`, whose double is `stubs/keys_ref.c:613` and is
//!    load-bearing for the pre-existing keys suite, and `M_UpdateMouse` can
//!    only raise through a slider-drag cvar write. Both wrappers are the same
//!    three lines as `M_Draw` (:326) and `M_Keydown` (:334), which are
//!    covered; the gap is in the harness, not in the port.
//!  * `M_Menu_Quit_f` is NOT a raise site. `menu.c:3559-3578` writes state and
//!    at most queues a `game` command; the `cl_confirmquit` check that reads
//!    like a confirmation prompt is in `M_Quit_Draw` (`:3643`) and draws a
//!    text box rather than calling `SCR_ModalMessage`. No `menu.c` *command*
//!    entry point reaches a `Menu_Glue_*` guard that this harness can arm.
//!  * `shadow_modes[]` (`menu.c:2062`) indexed by an out-of-range
//!    `r_rtshadows`. The C reads past a four-element stack array there, so the
//!    differential stays inside `0..=3`; the port reproduces the same index
//!    arithmetic and is marked `// COMPAT:` against ADR-004. Recorded, not
//!    laundered.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint, CStr};
use std::sync::{Mutex, MutexGuard, Once, OnceLock};

use quake_ctest::fs as ctfs; // also links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `stubs/menu_ref.c`'s side convention: 1 = the `c_ref_*` oracle, 0 = the port.
const C: c_int = 1;
const R: c_int = 0;

// menu.h:26-49
const M_NONE: c_int = 0;
const M_MAIN: c_int = 1;
const M_SINGLEPLAYER: c_int = 2;
const M_LOAD: c_int = 3;
const M_SAVE: c_int = 4;
const M_MULTIPLAYER: c_int = 5;
const M_SETUP: c_int = 6;
const M_NET: c_int = 7;
const M_OPTIONS: c_int = 8;
const M_GAME: c_int = 9;
const M_SOUND: c_int = 10;
const M_VIDEO: c_int = 11;
const M_GRAPHICS: c_int = 12;
const M_KEYS: c_int = 13;
const M_HELP: c_int = 14;
const M_QUIT: c_int = 15;
const M_LANCONFIG: c_int = 16;
const M_MPGAMEOPTIONS: c_int = 17;
const M_SEARCH: c_int = 18;
const M_SLIST: c_int = 19;
const M_MODS: c_int = 20;
const M_MAPS: c_int = 21;
const M_SKILL: c_int = 22;

const ALL_STATES: [c_int; 23] = [
    M_NONE,
    M_MAIN,
    M_SINGLEPLAYER,
    M_LOAD,
    M_SAVE,
    M_MULTIPLAYER,
    M_SETUP,
    M_NET,
    M_OPTIONS,
    M_GAME,
    M_SOUND,
    M_VIDEO,
    M_GRAPHICS,
    M_KEYS,
    M_HELP,
    M_QUIT,
    M_LANCONFIG,
    M_MPGAMEOPTIONS,
    M_SEARCH,
    M_SLIST,
    M_MODS,
    M_MAPS,
    M_SKILL,
];

// keys.h:31-131
const K_TAB: c_int = 9;
const K_ENTER: c_int = 13;
const K_ESCAPE: c_int = 27;
const K_SPACE: c_int = 32;
const K_BACKSPACE: c_int = 127;
const K_UPARROW: c_int = 128;
const K_DOWNARROW: c_int = 129;
const K_LEFTARROW: c_int = 130;
const K_RIGHTARROW: c_int = 131;
const K_CTRL: c_int = 133;
const K_PGDN: c_int = 149;
const K_PGUP: c_int = 150;
const K_HOME: c_int = 151;
const K_END: c_int = 152;
const K_MOUSE1: c_int = 200;
const K_MOUSE2: c_int = 201;
const K_MWHEELUP: c_int = 206;
const K_MWHEELDOWN: c_int = 207;

// keys.h:136-142
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
const KEY_MENU: c_int = 3;

// client.h:102-107
/// `menu.c:2204-2213`. Only the two rows the tests name are spelled out.
const OPT_VIDEO: c_int = 2;
const OPT_DEFAULTS: c_int = 6;

const CA_DISCONNECTED: c_int = 1;
const CA_CONNECTED: c_int = 2;
const SIGNONS: c_int = 4;

// quakedef.h:424-451
const MAPTYPE_MOD_START: c_uint = 4;
const MAPTYPE_MOD_LEVEL: c_uint = 5;
const MAPTYPE_ID_START: c_uint = 13;
const MAPTYPE_ID_EP1_LEVEL: c_uint = 14;
const MAPTYPE_ID_DM: c_uint = 19;

/// menu.h:105-112
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Crosshair {
    crosshair_char: c_char,
    viewport_x_offset: c_float,
    viewport_y_offset: c_float,
    menu_x_offset: c_int,
    menu_y_offset: c_int,
}

extern "C" {
    // stubs/draw_ref.c -- the shared recorder
    fn ctest_draw_clear_log();
    fn ctest_draw_log() -> *const c_char;
    fn ctest_draw_set_pic_newline(on: bool);
    fn ctest_draw_set_trycache_missing(missing: bool);
    fn ctest_set_map_description(desc: *const c_char, ret: bool);
    fn ctest_set_loading_plaque_recording(on: bool);
    fn ctest_clear_net_addresses();
    fn ctest_add_net_address(addr: *const c_char);

    // stubs/stubs.c -- the shared console capture and the Host_Error trap
    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;
    fn ctest_set_modal_message_answer(answer: c_int);
    fn ctest_set_menu_raise_mask(mask: c_int);

    // stubs/menu_ref.c -- cvars
    fn ctest_menu_cvar_count() -> c_int;
    fn ctest_menu_cvar_name(idx: c_int) -> *const c_char;
    fn ctest_menu_register_cvars();
    fn ctest_menu_cvar_registered(side: c_int, idx: c_int) -> c_int;
    fn ctest_menu_reset_cvars();
    fn ctest_menu_cvar_string(side: c_int, idx: c_int) -> *const c_char;
    fn ctest_menu_cvar_value(side: c_int, idx: c_int) -> c_float;
    fn ctest_menu_cvar_set(side: c_int, idx: c_int, value: *const c_char);

    // stubs/menu_ref.c -- shared world state
    fn ctest_menu_set_screen(vid_w: c_int, vid_h: c_int, glw: c_int, glh: c_int, con: c_float);
    fn ctest_menu_set_time(now: c_double, rawframetime: c_double);
    fn ctest_menu_set_key_dest(dest: c_int);
    fn ctest_menu_get_key_dest() -> c_int;
    fn ctest_menu_set_key_down(key: c_int, down: bool);
    fn ctest_menu_clear_key_down();
    fn ctest_menu_bind(keynum: c_int, command: *const c_char);
    fn ctest_menu_clear_binds();
    fn ctest_menu_set_misc(slist_in_progress: bool, multiuser: bool);
    fn ctest_menu_set_mouse(x: c_int, y: c_int);
    fn ctest_menu_set_caps(ray_query: bool, sample_rate_shading: bool, max_aniso: c_float);
    fn ctest_menu_clear_levels();
    fn ctest_menu_add_level(name: *const c_char, type_: c_uint, message: *const c_char);
    fn ctest_menu_clear_mods();
    fn ctest_menu_add_mod(name: *const c_char, full_name: *const c_char);

    // stubs/menu_ref.c -- per-side world state
    fn ctest_menu_set_server(side: c_int, active: bool, maxclients: c_int, limit: c_int);
    fn ctest_menu_set_client(
        side: c_int,
        state: c_int,
        demonum: c_int,
        demoplayback: bool,
        signon: c_int,
        intermission: c_int,
        mapname: *const c_char,
    );
    fn ctest_menu_get_demonum(side: c_int) -> c_int;
    fn ctest_menu_get_slist_scope() -> c_int;
    fn ctest_menu_get_slist_silent() -> bool;
    fn ctest_menu_set_net(side: c_int, cachecount: c_int, v4: bool, v6: bool, port: c_int);
    fn ctest_menu_set_gamedir(side: c_int, dir: *const c_char);
    fn ctest_menu_cbuf_init();
    fn ctest_menu_cbuf_clear(side: c_int);
    fn ctest_menu_cbuf_text(side: c_int) -> *const c_char;
    fn ctest_menu_tokenize(side: c_int, text: *const c_char);

    // stubs/menu_ref.c -- the glue-owned objects
    fn ctest_menu_set_state(side: c_int, v: c_int);
    fn ctest_menu_get_state(side: c_int) -> c_int;
    fn ctest_menu_set_return_state(side: c_int, v: c_int);
    fn ctest_menu_get_return_state(side: c_int) -> c_int;
    fn ctest_menu_set_entersound(side: c_int, v: bool);
    fn ctest_menu_get_entersound(side: c_int) -> bool;
    fn ctest_menu_set_is_quitting(side: c_int, v: bool);
    fn ctest_menu_get_is_quitting(side: c_int) -> bool;
    fn ctest_menu_set_return_onerror(side: c_int, v: bool);
    fn ctest_menu_get_return_onerror(side: c_int) -> bool;
    fn ctest_menu_set_return_reason(side: c_int, s: *const c_char);
    fn ctest_menu_get_return_reason(side: c_int) -> *const c_char;

    // stubs/menu_ref.c -- the four raise-capable entry points
    fn ctest_menu_toggle_menu_f(side: c_int) -> c_int;
    fn ctest_menu_update_mouse(side: c_int) -> c_int;
    fn ctest_menu_draw(side: c_int) -> c_int;
    fn ctest_menu_keydown(side: c_int, key: c_int) -> c_int;

    // stubs/menu_ref.c -- the non-raising surface
    fn ctest_menu_charinput(side: c_int, key: c_int);
    fn ctest_menu_text_entry(side: c_int) -> bool;
    fn ctest_menu_waiting_for_key_binding(side: c_int) -> bool;
    fn ctest_menu_get_scale(side: c_int) -> c_float;
    fn ctest_menu_get_crosshair_def(side: c_int, value: c_float, out: *mut Crosshair);
    fn ctest_menu_print(side: c_int, cx: c_int, cy: c_int, str_: *const c_char);
    fn ctest_menu_draw_pic(side: c_int, x: c_int, y: c_int, pic: *const c_char);
    fn ctest_menu_draw_trans_pic(side: c_int, x: c_int, y: c_int, pic: *const c_char);
    fn ctest_menu_menu_changed(side: c_int);
    fn ctest_menu_handle_scroll_bar_keys(
        side: c_int,
        key: c_int,
        cursor: *mut c_int,
        first: *mut c_int,
        total: c_int,
        max_on_screen: c_int,
    ) -> bool;
    fn ctest_menu_mouse_update_cursor(
        side: c_int,
        cursor: *mut c_int,
        left: c_int,
        right: c_int,
        top: c_int,
        item_height: c_int,
        index: c_int,
    );
    fn ctest_menu_check_mods(side: c_int);
    fn ctest_menu_new_game(side: c_int);

    // stubs/menu_ref.c -- the command entry points
    fn ctest_menu_menu_main_f(side: c_int) -> c_int;
    fn ctest_menu_menu_options_f(side: c_int) -> c_int;
    fn ctest_menu_menu_quit_f(side: c_int) -> c_int;
    fn ctest_menu_menu_singleplayer_f(side: c_int) -> c_int;
    fn ctest_menu_menu_load_f(side: c_int) -> c_int;
    fn ctest_menu_menu_save_f(side: c_int) -> c_int;
    fn ctest_menu_menu_maps_cmd_f(side: c_int) -> c_int;
    fn ctest_menu_menu_multiplayer_f(side: c_int) -> c_int;
    fn ctest_menu_menu_setup_f(side: c_int) -> c_int;
    fn ctest_menu_menu_keys_f(side: c_int) -> c_int;
    fn ctest_menu_menu_help_f(side: c_int) -> c_int;
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: every producer in the two stub files returns a NUL-terminated buffer.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// One side's complete observable result.
#[derive(PartialEq, Eq, Debug, Clone)]
struct Obs {
    draw: String,
    con: Vec<String>,
    cbuf: String,
    state: c_int,
    ret_state: c_int,
    entersound: bool,
    quitting: bool,
    onerror: bool,
    reason: String,
    key_dest: c_int,
    demonum: c_int,
    /// `slist_scope` / `slist_silent` are shared objects `menu.c:4417-4418`
    /// writes on both halves; snapshotting them per side is what makes the
    /// search menu's scope selection comparable rather than invisible.
    slist_scope: c_int,
    slist_silent: bool,
    /// name, string, `f32::to_bits` of the value -- bits so a NaN a defect
    /// produced compares unequal to a NaN it did not.
    cvars: Vec<(String, String, u32)>,
}

fn snapshot(side: c_int) -> Obs {
    // SAFETY: plain readers over the two stub files' statics.
    unsafe {
        let mut con = Vec::new();
        for i in 0..ctest_con_log_len() {
            con.push(cstr(ctest_con_log_get(i)));
        }
        let mut cvars = Vec::new();
        for i in 0..ctest_menu_cvar_count() {
            cvars.push((
                cstr(ctest_menu_cvar_name(i)),
                cstr(ctest_menu_cvar_string(side, i)),
                ctest_menu_cvar_value(side, i).to_bits(),
            ));
        }
        Obs {
            draw: cstr(ctest_draw_log()),
            con,
            cbuf: cstr(ctest_menu_cbuf_text(side)),
            state: ctest_menu_get_state(side),
            ret_state: ctest_menu_get_return_state(side),
            entersound: ctest_menu_get_entersound(side),
            quitting: ctest_menu_get_is_quitting(side),
            onerror: ctest_menu_get_return_onerror(side),
            reason: cstr(ctest_menu_get_return_reason(side)),
            key_dest: ctest_menu_get_key_dest(),
            demonum: ctest_menu_get_demonum(side),
            slist_scope: ctest_menu_get_slist_scope(),
            slist_silent: ctest_menu_get_slist_silent(),
            cvars,
        }
    }
}

fn once() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: both are idempotent one-shot initialisers in stubs/menu_ref.c.
        unsafe {
            ctest_menu_cbuf_init();
            ctest_menu_register_cvars();
        }
    });
}

/// Everything both halves share, re-seeded before each side runs.
fn seed_shared() {
    // SAFETY: plain setters over statics in the two stub files.
    unsafe {
        ctest_menu_set_screen(640, 480, 640, 480, 0.0);
        ctest_menu_set_time(10.0, 0.05);
        ctest_menu_set_key_dest(KEY_GAME);
        ctest_menu_clear_key_down();
        // keybindings is shared: c_ref_prelude.h renames neither the array
        // nor Key_SetBinding, so the composed menu.c and the port read the
        // one table stubs/keys_ref.c:220 defines. Clearing it here keeps a
        // bind left behind by one case out of the next one.
        ctest_menu_clear_binds();
        // slistInProgress is seeded TRUE, not false. net_main.c is not an
        // oracle source here: there is ONE NET_Slist_f in this link
        // (stubs/net_main_glue_ref.c:180 -> rust_net_Slist_f) and both
        // halves call it. Its body schedules two file-static
        // PollProcedures into a shared list, and with the fixed 0.0 clock
        // at stubs/stubs.c:3003 a SECOND scheduling of the same node makes
        // that list point at itself (quake-capi/src/net_main.rs:1342-1359
        // takes the `next_time >= next_time` break with prev still NULL),
        // after which every later walk spins forever. Running each case on
        // two sides means every NET_Slist_f path would be entered twice, so
        // the flag is seeded true and the call early-returns on BOTH sides.
        // What menu.c contributes to that path -- the scope argument and
        // the state writes around the call -- is still compared; the body
        // of NET_Slist_f is shared code and is NOT COVERED here. Tests that
        // need the other branch set the flag false in their own setup.
        ctest_menu_set_misc(true, false);
        ctest_menu_set_mouse(0, 0);
        ctest_menu_set_caps(false, false, 1.0);
        ctest_menu_clear_levels();
        ctest_menu_clear_mods();
        ctest_set_modal_message_answer(1);
        ctest_set_menu_raise_mask(0);
        ctest_draw_set_pic_newline(true);
        ctest_draw_set_trycache_missing(false);
        ctest_set_map_description(c"".as_ptr(), false);
        // stubs/stubs.c's SCR_BeginLoadingPlaque aborts unless a caller
        // opts in -- tests/cl_main_differential.rs:1291 depends on that
        // abort. menu.c:952 and :4387 reach it, so this suite opts in.
        ctest_set_loading_plaque_recording(true);
        ctest_clear_net_addresses();
        ctest_menu_reset_cvars();
    }
}

/// The per-side objects: the eight `menu_glue.c` owns, `cl`/`cls`, `sv`/`svs`,
/// the net-search globals and the command buffer.
fn seed_side(side: c_int) {
    // SAFETY: per-side setters over the split objects.
    unsafe {
        ctest_menu_set_state(side, M_NONE);
        ctest_menu_set_return_state(side, M_NONE);
        ctest_menu_set_entersound(side, false);
        ctest_menu_set_is_quitting(side, false);
        ctest_menu_set_return_onerror(side, false);
        ctest_menu_set_return_reason(side, c"".as_ptr());
        ctest_menu_set_client(side, CA_DISCONNECTED, -1, false, 0, 0, c"".as_ptr());
        ctest_menu_set_server(side, false, 1, 1);
        ctest_menu_set_net(side, 0, true, false, 26000);
        // M_ScanSaves (menu.c:846) is menu.c's only com_gamedir reader, and it
        // opens a plain path under it. Cleared here so a test that points it at
        // a directory of real saves cannot leak that into the next slot table.
        ctest_menu_set_gamedir(side, c"".as_ptr());
        ctest_menu_cbuf_clear(side);
        // The argument vector is a global too, and menu.c reads it without
        // ever writing it (menu.c:3025 Cmd_Argc, :3029/:3036 Cmd_Argv). A
        // test that tokenizes an argument would otherwise leave it visible
        // to whichever test cargo runs next, so every side starts from a
        // one-token vector: Cmd_Argc()==1, Cmd_Argv(1)=="".
        ctest_menu_tokenize(side, c"menu_maps".as_ptr());
    }
    // menu.c:673 and :4692 call CL_NextDemo when the main menu closes with a
    // demo queued. CL_NextDemo with no demos listed reaches CL_Disconnect and
    // then CDAudio_Stop, which stubs/stubs.c:7811 still aborts on -- cd_sdl.c
    // is not an oracle source and making it one is not M10e work. Every test
    // therefore runs with the demo loop off and
    // `main_menu_demo_loop_guard_matches` drives the guard itself; the
    // CL_NextDemo call is recorded as NOT COVERED at the top of this file.
    set_cvar(side, "cl_startdemos", "0");
}

/// Run both sides. For each side: seed, run `setup`, clear the three ordered
/// recorders, invoke, and snapshot **immediately** -- anything read after the
/// loop would see only the last side's shared state.
fn both<T>(setup: impl Fn(c_int), run: impl Fn(c_int) -> T) -> ((Obs, T), (Obs, T)) {
    once();
    let mut out = Vec::new();
    for side in [C, R] {
        seed_shared();
        seed_side(side);
        setup(side);
        // SAFETY: clears the shared recorders and this side's command buffer.
        unsafe {
            ctest_draw_clear_log();
            ctest_clear_con_log();
            ctest_menu_cbuf_clear(side);
        }
        let value = run(side);
        out.push((snapshot(side), value));
    }
    let rust = out.pop().unwrap();
    let c = out.pop().unwrap();
    (c, rust)
}

fn compare<T: PartialEq + std::fmt::Debug>(what: &str, c: &(Obs, T), r: &(Obs, T)) {
    assert_eq!(c.0.draw, r.0.draw, "{what}: draw log");
    assert_eq!(c.0.con, r.0.con, "{what}: console log");
    assert_eq!(c.0.cbuf, r.0.cbuf, "{what}: command buffer");
    assert_eq!(c.0.state, r.0.state, "{what}: m_state");
    assert_eq!(c.0.ret_state, r.0.ret_state, "{what}: m_return_state");
    assert_eq!(c.0.entersound, r.0.entersound, "{what}: m_entersound");
    assert_eq!(c.0.quitting, r.0.quitting, "{what}: m_is_quitting");
    assert_eq!(c.0.onerror, r.0.onerror, "{what}: m_return_onerror");
    assert_eq!(c.0.reason, r.0.reason, "{what}: m_return_reason");
    assert_eq!(c.0.key_dest, r.0.key_dest, "{what}: key_dest");
    assert_eq!(c.0.demonum, r.0.demonum, "{what}: cls.demonum");
    assert_eq!(c.0.slist_scope, r.0.slist_scope, "{what}: slist_scope");
    assert_eq!(c.0.slist_silent, r.0.slist_silent, "{what}: slist_silent");
    assert_eq!(c.0.cvars, r.0.cvars, "{what}: cvars");
    assert_eq!(c.1, r.1, "{what}: result");
}

/// The general assertion: everything observable agrees.
fn assert_same<T: PartialEq + std::fmt::Debug>(
    what: &str,
    setup: impl Fn(c_int),
    run: impl Fn(c_int) -> T,
) -> Obs {
    let _g = lock();
    let (c, r) = both(setup, run);
    compare(what, &c, &r);
    c.0
}

/// For the drawing entry points. The non-empty check is deliberate: comparing
/// two empty buffers is not a test.
fn assert_drawn<T: PartialEq + std::fmt::Debug>(
    what: &str,
    setup: impl Fn(c_int),
    run: impl Fn(c_int) -> T,
) {
    let obs = assert_same(what, setup, run);
    assert!(
        !obs.draw.is_empty(),
        "{what}: the C oracle drew nothing (m_state={}, key_dest={})",
        obs.state,
        obs.key_dest
    );
}

/// For the pure helpers, which must draw nothing at all.
fn assert_quiet<T: PartialEq + std::fmt::Debug>(
    what: &str,
    setup: impl Fn(c_int),
    run: impl Fn(c_int) -> T,
) {
    let obs = assert_same(what, setup, run);
    assert_eq!(
        obs.draw, "",
        "{what}: the C oracle drew something unexpected"
    );
}

fn noop(_side: c_int) {}

/// Press a key sequence, ignoring the raise status (no test in this file arms
/// a raise inside a `setup`).
fn press(side: c_int, keys: &[c_int]) {
    for &k in keys {
        // SAFETY: dispatched through ctest_try_host in stubs/menu_ref.c.
        unsafe { ctest_menu_keydown(side, k) };
    }
}

/// Look a cvar up by the name it is registered under.
fn cvar_index(name: &str) -> c_int {
    once();
    // SAFETY: a bounded read over the fixture's own table.
    unsafe {
        for i in 0..ctest_menu_cvar_count() {
            if cstr(ctest_menu_cvar_name(i)) == name {
                return i;
            }
        }
    }
    panic!("no cvar named {name} in stubs/menu_ref.c's table");
}

fn set_cvar(side: c_int, name: &str, value: &str) {
    let idx = cvar_index(name);
    let v = std::ffi::CString::new(value).unwrap();
    // SAFETY: `v` outlives the call; Cvar_Set copies.
    unsafe { ctest_menu_cvar_set(side, idx, v.as_ptr()) };
}

// ---------------------------------------------------------------------------
// The fixture itself. If the two registries ever stop agreeing about which
// object a menu cvar name resolves to, every cvar assertion below turns into a
// comparison of two "variable not found" warnings -- symmetric, and worth
// nothing. This test is what stops that from happening silently.

#[test]
fn every_menu_cvar_resolves_to_the_object_menu_c_reads() {
    let _g = lock();
    once();
    // SAFETY: bounded reads over the fixture's table and the two registries.
    unsafe {
        let n = ctest_menu_cvar_count();
        assert_eq!(n, 53, "the cvar table lost or gained an entry");
        let mut unregistered = Vec::new();
        for i in 0..n {
            let name = cstr(ctest_menu_cvar_name(i));
            let (c, r) = (
                ctest_menu_cvar_registered(C, i),
                ctest_menu_cvar_registered(R, i),
            );
            if c == 0 && r == 0 {
                unregistered.push(name);
                continue;
            }
            assert_eq!(c, 1, "{name}: the oracle registry resolves it elsewhere");
            assert_eq!(r, 1, "{name}: the port registry resolves it elsewhere");
        }
        // `registered` is CVAR_ROM and read-only at menu.c:638; it is the one
        // entry both slots deliberately share.
        assert_eq!(unregistered, vec!["registered".to_string()]);
    }
}

// ---------------------------------------------------------------------------
// M_GetScale (menu.c:229)

#[test]
fn get_scale_matches_across_the_scale_cvars_and_screen_sizes() {
    for (menuscale, relative, conscale) in [
        ("1", "0", "1"),
        ("1", "1", "1"),
        ("1", "2", "1"),
        ("0", "2", "1"),
        ("3.5", "2", "2"),
        ("-1", "1", "1"),
        ("100", "2", "1"),
    ] {
        for (w, h) in [(320, 200), (640, 480), (1920, 1080), (800, 600)] {
            let what = format!("get_scale({menuscale},{relative},{conscale},{w}x{h})");
            assert_quiet(
                &what,
                |side| {
                    // SAFETY: setters over shared statics and this side's cvars.
                    unsafe { ctest_menu_set_screen(w, h, w, h, 0.0) };
                    set_cvar(side, "scr_menuscale", menuscale);
                    set_cvar(side, "scr_relativescale", relative);
                    set_cvar(side, "scr_conscale", conscale);
                },
                // SAFETY: a pure read of the cvars seeded above.
                |side| unsafe { ctest_menu_get_scale(side) }.to_bits(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// M_GetCrosshairDef (menu.c:250)

#[test]
fn get_crosshair_def_matches_over_the_defined_range_and_past_both_ends() {
    for v in [
        -1000.0f32, -1.5, -1.0, -0.5, 0.0, 0.4, 1.0, 1.9, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0,
        10.0, 100.0, 1.0e9,
    ] {
        assert_quiet(&format!("crosshair_def({v})"), noop, |side| {
            let mut out = Crosshair::default();
            // SAFETY: `out` is a live crosshair_t for the duration of the call.
            unsafe { ctest_menu_get_crosshair_def(side, v, &mut out) };
            (
                out.crosshair_char,
                out.viewport_x_offset.to_bits(),
                out.viewport_y_offset.to_bits(),
                out.menu_x_offset,
                out.menu_y_offset,
            )
        });
    }
}

// ---------------------------------------------------------------------------
// M_HandleScrollBarKeys (menu.c:2668)

#[test]
fn handle_scroll_bar_keys_matches_over_the_cursor_and_window_grid() {
    for key in [
        K_UPARROW,
        K_DOWNARROW,
        K_PGUP,
        K_PGDN,
        K_HOME,
        K_END,
        K_MWHEELUP,
        K_MWHEELDOWN,
        K_ENTER,
        K_LEFTARROW,
    ] {
        for (cursor, first, total, max) in [
            (0, 0, 0, 10),
            (0, 0, 1, 10),
            (0, 0, 40, 10),
            (5, 0, 40, 10),
            (9, 0, 40, 10),
            (39, 30, 40, 10),
            (-1, 0, 40, 10),
            (100, 0, 40, 10),
            (3, 3, 5, 10),
            (0, 0, 40, 0),
            (0, 0, 40, -3),
        ] {
            let what = format!("scrollbar(key={key},{cursor},{first},{total},{max})");
            assert_quiet(&what, noop, |side| {
                let mut cur = cursor;
                let mut fst = first;
                // SAFETY: both out-params are live `c_int`s.
                let handled = unsafe {
                    ctest_menu_handle_scroll_bar_keys(side, key, &mut cur, &mut fst, total, max)
                };
                (handled, cur, fst)
            });
        }
    }
}

/// `M_InScrollbar` (menu.c:498) gates the `K_MOUSE1` arm of
/// `M_HandleScrollBarKeys`, and its four comparisons are all inclusive. It
/// reads `scrollbar_x` / `scrollbar_y` / `scrollbar_size`, which only a draw
/// ever writes (menu.c:433-435), so the setup draws the keys menu: that calls
/// `M_DrawScrollbar (cbx, MENU_SCROLLBAR_X, 56, ..., BINDS_PER_PAGE - 2)`
/// (menu.c:2461), which leaves the bar at x=360, y=48, size=(17+2)*8=152.
/// Every edge of that box is probed exactly and one unit outside it.
#[test]
fn scroll_bar_mouse_grab_matches_on_every_edge_of_the_bar() {
    const BAR_X: c_int = 360;
    const BAR_Y: c_int = 48;
    const BAR_SIZE: c_int = 152;
    for cx in [BAR_X - 1, BAR_X, BAR_X + 4, BAR_X + 8, BAR_X + 9] {
        for cy in [
            BAR_Y - 1,
            BAR_Y,
            BAR_Y + 76,
            BAR_Y + BAR_SIZE,
            BAR_Y + BAR_SIZE + 1,
        ] {
            for (total, max) in [(40, 10), (10, 10)] {
                assert_quiet(
                    &format!("in_scrollbar(({cx},{cy}),total={total},max={max})"),
                    move |side| {
                        // SAFETY: a guarded command entry point.
                        unsafe {
                            ctest_menu_menu_keys_f(side);
                            ctest_menu_set_key_dest(KEY_MENU);
                        }
                        // The engine's frame order, and it matters: M_UpdateMouse
                        // ends by zeroing scrollbar_size (menu.c:4668), so the
                        // draw that republishes the bar has to come AFTER the
                        // mouse move, exactly as Host_Frame runs them.
                        place_mouse_canvas(side, cx, cy);
                        // SAFETY: dispatched through ctest_try_host.
                        unsafe { ctest_menu_draw(side) };
                    },
                    move |side| {
                        let mut cur = 20;
                        let mut fst = 5;
                        // SAFETY: both out-params are live `c_int`s.
                        let handled = unsafe {
                            ctest_menu_handle_scroll_bar_keys(
                                side, K_MOUSE1, &mut cur, &mut fst, total, max,
                            )
                        };
                        (handled, cur, fst)
                    },
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// M_Mouse_UpdateCursor (menu.c:4626)

#[test]
fn mouse_update_cursor_matches_over_the_hitbox_grid() {
    // Menu-canvas coordinates, not pixels: the hit box arguments below are in
    // canvas units, and `place_mouse_canvas` does the conversion. Every edge of
    // the (12,32)-(400,40) box is probed exactly and one unit outside it, since
    // the four comparisons in menu.c:4628-4629 are all inclusive.
    for (cx, cy) in [
        (0, 0),
        (100, 100),
        (160, 100),
        (12, 32),
        (400, 32),
        (401, 32),
        (11, 32),
        (200, 31),
        (200, 39),
        (200, 40),
        (200, 41),
        (479, 339),
    ] {
        for (left, right, top, height, index) in [
            (12, 400, 32, 8, 0),
            (12, 400, 32, 8, 3),
            (12, 400, 32, 0, 2),
            (400, 12, 32, 8, 1),
            (0, 640, 0, 480, 0),
        ] {
            let what = format!("mouse_cursor(({cx},{cy}),{left},{right},{top},{height},{index})");
            assert_quiet(
                &what,
                |side| place_mouse_canvas(side, cx, cy),
                |side| {
                    let mut cursor = -99;
                    // SAFETY: `cursor` is a live `c_int`.
                    unsafe {
                        ctest_menu_mouse_update_cursor(
                            side,
                            &mut cursor,
                            left,
                            right,
                            top,
                            height,
                            index,
                        )
                    };
                    cursor
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// M_Print / M_DrawPic / M_DrawTransPic (menu.c:288, :368, :350)

#[test]
fn print_matches_for_ascii_high_bit_and_empty_strings() {
    for s in [
        c"".as_ptr(),
        c"a".as_ptr(),
        c"OPTIONS".as_ptr(),
        c"the quick brown fox".as_ptr(),
        c"\x80\x81\xfe\xff".as_ptr(),
        c" leading and trailing ".as_ptr(),
    ] {
        let what = format!("print({:?})", cstr(s));
        assert_same(&what, noop, |side| {
            // SAFETY: `s` is a `'static` C string literal.
            unsafe { ctest_menu_print(side, 16, 32, s) }
        });
    }
}

#[test]
fn draw_pic_and_draw_trans_pic_match() {
    assert_drawn("draw_pic", noop, |side| {
        // SAFETY: the fixture resolves the name through the shared pic registry.
        unsafe { ctest_menu_draw_pic(side, 16, 4, c"gfx/qplaque.lmp".as_ptr()) }
    });
    assert_drawn("draw_trans_pic", noop, |side| {
        // SAFETY: as above.
        unsafe { ctest_menu_draw_trans_pic(side, 72, 32, c"gfx/ttl_main.lmp".as_ptr()) }
    });
}

// ---------------------------------------------------------------------------
// M_MenuChanged (menu.c:4586) and M_CheckMods (menu.c:626)

#[test]
fn menu_changed_matches() {
    assert_same("menu_changed", noop, |side| {
        // SAFETY: no arguments, no out-params.
        unsafe { ctest_menu_menu_changed(side) }
    });
}

/// FNV-1a-32 exactly as `COM_HashBlock` (`Quake/common_fs.c:1542`) computes it.
fn fnv1a(data: &[u8]) -> c_uint {
    data.iter().fold(0x811c_9dc5_u32, |h, &b| {
        (h ^ c_uint::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// `len` bytes whose FNV-1a-32 is exactly `want`.
///
/// `M_CheckCustomGfx` (`menu.c:4546`) only reaches its hash comparison when the
/// base pic is the exact length id's is and the custom pic sits in a lower
/// `path_id`, and the comparison only reports true when the hash is one of the
/// literals `menu.c:4583` carries. Without a file that actually hashes to one of
/// them the whole hash arm is unobservable, so the fixture constructs one rather
/// than shipping a 14856-byte binary blob.
///
/// FNV-1a is trivially invertible one byte at a time -- `h_next = (h_prev ^ b) *
/// P` and `P` is odd, so `h_prev = (h_next * P^-1) ^ b` for any `b` -- which
/// makes this a meet in the middle over the last four bytes: 2^16 forward from
/// the filler and 2^16 backward from `want`, expecting one collision in 2^32.
/// When the two halves miss, the filler changes and it tries again.
fn blob_hashing_to(len: usize, want: c_uint) -> Vec<u8> {
    // P^-1 mod 2^32 by Newton iteration; P is odd, so it exists.
    let mut inv: c_uint = 1;
    for _ in 0..5 {
        inv = inv.wrapping_mul(2_u32.wrapping_sub(0x0100_0193_u32.wrapping_mul(inv)));
    }
    assert_eq!(inv.wrapping_mul(0x0100_0193), 1);
    assert!(len >= 4);
    for filler in 0..=u16::MAX {
        let mut data = vec![(filler & 0xff) as u8; len];
        data[0] = (filler >> 8) as u8;
        let a = fnv1a(&data[..len - 4]);
        let mut fwd = std::collections::HashMap::with_capacity(1 << 16);
        for b1 in 0..=u8::MAX {
            let h1 = (a ^ c_uint::from(b1)).wrapping_mul(0x0100_0193);
            for b2 in 0..=u8::MAX {
                let h2 = (h1 ^ c_uint::from(b2)).wrapping_mul(0x0100_0193);
                fwd.insert(h2, (b1, b2));
            }
        }
        for b4 in 0..=u8::MAX {
            let h3 = want.wrapping_mul(inv) ^ c_uint::from(b4);
            for b3 in 0..=u8::MAX {
                let h2 = h3.wrapping_mul(inv) ^ c_uint::from(b3);
                if let Some(&(b1, b2)) = fwd.get(&h2) {
                    data[len - 4..].copy_from_slice(&[b1, b2, b3, b4]);
                    assert_eq!(fnv1a(&data), want);
                    return data;
                }
            }
        }
    }
    unreachable!("no four-byte tail hashes to {want:#x}")
}

/// The two-gamedir tree `M_CheckCustomGfx` walks. `a` is mounted first and gets
/// `path_id` 1, `b` second and gets 2 (`common_fs.c:645-648` doubles it per
/// gamedir), so a custom pic in `a` with its base pic in `b` is the
/// `id_custom < id_base` case that falls through to the length and hash checks,
/// and a custom pic in `b` is the `id_custom >= id_base` case that returns true
/// on the spot.
fn gfx_tree(kind: &str) -> std::path::PathBuf {
    const SP_HASH: c_uint = 0x86a6_f086;
    const SGL_HASH: c_uint = 0x7bba_813d;
    let root = std::env::temp_dir().join(format!(
        "quake-ctest-menu-gfx-{kind}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["a", "b"] {
        std::fs::create_dir_all(root.join(dir).join("gfx")).unwrap();
    }
    let customs = ["sp_maps.lmp", "skillmenu.lmp", "p_skill.lmp"];
    let (custom_dir, sp, sgl): (&str, Vec<u8>, Vec<u8>) = match kind {
        "absent" => return root,
        "custom_on_top" => ("b", vec![0; 16], vec![0; 16]),
        "hash_match" => (
            "a",
            blob_hashing_to(14856, SP_HASH),
            blob_hashing_to(6728, SGL_HASH),
        ),
        "hash_mismatch" => (
            "a",
            blob_hashing_to(14856, SP_HASH ^ 1),
            blob_hashing_to(6728, SGL_HASH ^ 1),
        ),
        "wrong_length" => (
            "a",
            blob_hashing_to(14855, SP_HASH),
            blob_hashing_to(6729, SGL_HASH),
        ),
        _ => unreachable!(),
    };
    for name in customs {
        std::fs::write(root.join(custom_dir).join("gfx").join(name), b"custom").unwrap();
    }
    std::fs::write(root.join("b").join("gfx").join("sp_menu.lmp"), sp).unwrap();
    std::fs::write(root.join("b").join("gfx").join("ttl_sgl.lmp"), sgl).unwrap();
    root
}

/// `M_CheckMods` (`menu.c:4581`) does not read the mod list at all -- it asks
/// `M_CheckCustomGfx` three questions about the mounted search path -- and its
/// three answers are file statics, so the only way to see them is to draw the
/// two menus that read them: the single-player menu, whose item count grows by
/// one when `m_singleplayer_showlevels` (`menu.c:731`, `:752`), and the skill
/// menu, which swaps its title pic on `m_skill_usecustomtitle` and its whole
/// body on `m_skill_usegfx` (`menu.c:3471`, `:3482`).
///
/// The mount stays behind after the test; `absent` runs last so what it leaves
/// is an empty one. Nothing else in this suite consults the search path --
/// `Draw_CachePic` is a link double and `M_ScanSaves` opens a plain path under
/// `com_gamedir` -- so a leftover mount is inert either way.
#[test]
fn check_mods_matches_across_the_custom_gfx_tree() {
    for kind in [
        "custom_on_top",
        "hash_match",
        "hash_mismatch",
        "wrong_length",
        "absent",
    ] {
        let root = gfx_tree(kind);
        assert_drawn(
            &format!("check_mods({kind})"),
            |side| {
                ctfs::set_host_dirs("/nonexistent-host-basedir", None);
                ctfs::setup(
                    if side == C {
                        ctfs::Side::C
                    } else {
                        ctfs::Side::Rust
                    },
                    &[&root],
                    0,
                    c"a;b",
                );
            },
            |side| {
                // SAFETY: guarded command entry points, then two draws
                // dispatched through ctest_try_host.
                unsafe {
                    ctest_menu_check_mods(side);
                    ctest_menu_menu_singleplayer_f(side);
                    let sp = ctest_menu_draw(side);
                    ctest_menu_set_state(side, M_SKILL);
                    (sp, ctest_menu_draw(side))
                }
            },
        );
    }
}
// ---------------------------------------------------------------------------
// M_NewGame (menu.c:672)

#[test]
fn new_game_restores_the_saved_demonum() {
    // menu.c:4617-4619 sends the map list's two menus back to the main menu
    // and leaves every other state alone, so the state the call starts in is
    // part of the case.
    for state in [M_MAIN, M_OPTIONS, M_MAPS, M_SKILL] {
        for demonum in [-1, 0, 3] {
            assert_same(
                &format!("new_game(state={state},demonum={demonum})"),
                move |side| {
                    // SAFETY: per-side setter over cls.
                    unsafe {
                        ctest_menu_set_client(
                            side,
                            CA_DISCONNECTED,
                            demonum,
                            false,
                            0,
                            0,
                            c"".as_ptr(),
                        )
                    };
                    // M_Menu_Main_f is what stashes m_save_demonum, so the
                    // restore has something to restore (menu.c:626-627).
                    press(side, &[]);
                    // SAFETY: a command entry point under ctest_try_host, then
                    // the state the case actually wants.
                    unsafe {
                        ctest_menu_menu_main_f(side);
                        ctest_menu_set_state(side, state);
                    };
                },
                // SAFETY: no arguments.
                |side| unsafe { ctest_menu_new_game(side) },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// M_TextEntry / M_WaitingForKeyBinding (menu.c:4924, :4941)

#[test]
fn text_entry_and_waiting_for_key_binding_match_in_every_state() {
    for state in ALL_STATES {
        assert_quiet(
            &format!("text_entry(state={state})"),
            // SAFETY: a per-side setter over the glue-owned m_state.
            move |side| unsafe { ctest_menu_set_state(side, state) },
            |side| {
                // SAFETY: two pure predicates.
                unsafe {
                    (
                        ctest_menu_text_entry(side),
                        ctest_menu_waiting_for_key_binding(side),
                    )
                }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// M_ToggleMenu_f (menu.c:4666)

#[test]
fn toggle_menu_matches_across_key_dest_state_and_connection() {
    for state in [M_NONE, M_MAIN, M_OPTIONS, M_QUIT, M_MAPS] {
        for dest in [KEY_GAME, KEY_CONSOLE, KEY_MENU] {
            for connected in [false, true] {
                let what = format!("toggle_menu(state={state},dest={dest},conn={connected})");
                assert_same(
                    &what,
                    move |side| {
                        // SAFETY: setters over the glue-owned state, key_dest and cls.
                        unsafe {
                            ctest_menu_set_state(side, state);
                            ctest_menu_set_key_dest(dest);
                            ctest_menu_set_client(
                                side,
                                if connected {
                                    CA_CONNECTED
                                } else {
                                    CA_DISCONNECTED
                                },
                                -1,
                                false,
                                SIGNONS,
                                0,
                                c"e1m1".as_ptr(),
                            );
                        }
                    },
                    // SAFETY: dispatched through ctest_try_host.
                    |side| unsafe { ctest_menu_toggle_menu_f(side) },
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The command entry points (menu.c:634, :862, :925, :1093, :1541, :2382,
// :2551, :2946, :3593, :4592)

/// Each of these is a `Cmd_AddCommand` target in `M_Init`, so each is a real
/// engine entry point and not just an internal transition.
#[test]
fn every_menu_command_matches_from_a_cold_start() {
    type Entry = (&'static str, unsafe extern "C" fn(c_int) -> c_int);
    let entries: [Entry; 11] = [
        ("menu_main", ctest_menu_menu_main_f),
        ("menu_singleplayer", ctest_menu_menu_singleplayer_f),
        ("menu_load", ctest_menu_menu_load_f),
        ("menu_save", ctest_menu_menu_save_f),
        ("menu_multiplayer", ctest_menu_menu_multiplayer_f),
        ("menu_setup", ctest_menu_menu_setup_f),
        ("menu_options", ctest_menu_menu_options_f),
        ("menu_keys", ctest_menu_menu_keys_f),
        ("menu_help", ctest_menu_menu_help_f),
        ("menu_quit", ctest_menu_menu_quit_f),
        ("menu_maps", ctest_menu_menu_maps_cmd_f),
    ];
    for (name, f) in entries {
        // SAFETY: every entry point is dispatched through ctest_try_host.
        assert_same(name, noop, |side| unsafe { f(side) });
    }
}

/// `M_Menu_Save_f` and `M_Menu_Load_f` refuse in the states `menu.c:872-887`
/// and `:925-936` name, and both refusals go to the console.
#[test]
fn save_and_load_refusals_match() {
    let cases = [
        ("not connected", CA_DISCONNECTED, 0, false, 0),
        ("connected, signon 0", CA_CONNECTED, 0, false, 0),
        ("connected, signed on", CA_CONNECTED, SIGNONS, false, 0),
        ("intermission", CA_CONNECTED, SIGNONS, false, 1),
        ("demo playback", CA_CONNECTED, SIGNONS, true, 0),
    ];
    for (label, state, signon, demo, inter) in cases {
        for (name, f) in [
            (
                "save",
                ctest_menu_menu_save_f as unsafe extern "C" fn(c_int) -> c_int,
            ),
            (
                "load",
                ctest_menu_menu_load_f as unsafe extern "C" fn(c_int) -> c_int,
            ),
        ] {
            assert_same(
                &format!("{name} ({label})"),
                move |side| {
                    // SAFETY: per-side setters over cl/cls and sv.
                    unsafe {
                        ctest_menu_set_client(
                            side,
                            state,
                            -1,
                            demo,
                            signon,
                            inter,
                            c"e1m1".as_ptr(),
                        );
                        ctest_menu_set_server(side, state == CA_CONNECTED, 1, 1);
                    }
                },
                // SAFETY: a command entry point under ctest_try_host.
                |side| unsafe { f(side) },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// M_Draw (menu.c:4772)

/// Forcing `m_state` reaches the eleven screens no `menu_*` command opens
/// (`m_net`, `m_lanconfig`, `m_mpgameoptions`, `m_search`, `m_slist`,
/// `m_skill`, `m_mods`, `m_game`, `m_sound`, `m_graphics`, `m_video`). Their
/// per-menu statics are whatever the previous test left, but that is the same
/// on both sides -- `both` applies the identical sequence to each -- so the
/// comparison stays honest even though the state is arbitrary.
#[test]
fn draw_matches_in_every_state() {
    for state in ALL_STATES {
        let what = format!("draw(state={state})");
        let f = move |side: c_int| {
            // SAFETY: a per-side setter over the glue-owned m_state.
            unsafe {
                // m_search draws through the !slistInProgress half; the other
                // one reaches the aborting NET_Poll double (see below).
                ctest_menu_set_misc(false, false);
                ctest_menu_set_state(side, state);
                ctest_menu_set_key_dest(KEY_MENU);
            }
        };
        // SAFETY: M_Draw is dispatched through ctest_try_host.
        let run = |side: c_int| unsafe { ctest_menu_draw(side) };
        if state == M_NONE {
            assert_quiet(&what, f, run);
        } else {
            assert_drawn(&what, f, run);
        }
    }
}

/// The main menu's plaque, its two title pics and the cursor blink all depend
/// on `realtime`, `registered` and whether a mod is loaded (`menu.c:638`,
/// `:700-716`).
#[test]
fn main_menu_draw_matches_across_the_blink_phases() {
    for now in [0.0f64, 0.1, 0.24, 0.25, 0.49, 0.5, 10.0, 1.0e6] {
        assert_drawn(
            &format!("main_draw(realtime={now})"),
            move |side| {
                // SAFETY: a shared setter plus a command entry point.
                unsafe {
                    ctest_menu_menu_main_f(side);
                    ctest_menu_set_time(now, 0.05);
                }
            },
            // SAFETY: dispatched through ctest_try_host.
            |side| unsafe { ctest_menu_draw(side) },
        );
    }
}

/// `M_Options_Draw` renders every slider and checkbox from the cvars, so a
/// wrong read or a wrong `va()` specifier shows up as a changed string in the
/// draw log. The values below are chosen to land on both sides of the
/// `menu.c:1514-1520`, `:1642-1656` and `:1598` branch points.
#[test]
fn options_menu_draw_matches_across_the_slider_value_branches() {
    let cases: [&[(&str, &str)]; 5] = [
        &[],
        &[
            ("viewsize", "100"),
            ("scr_style", "0"),
            ("scr_sbaralpha", "1"),
            ("gamma", "0.9"),
            ("contrast", "1.4"),
        ],
        &[
            ("viewsize", "110"),
            ("scr_style", "1"),
            ("scr_sbaralpha", "0"),
            ("gamma", "0.5"),
            ("contrast", "2"),
        ],
        &[
            ("viewsize", "120"),
            ("scr_style", "2"),
            ("scr_sbaralpha", "0.375"),
            ("sensitivity", "11"),
            ("m_pitch", "-0.022"),
        ],
        &[
            ("viewsize", "30"),
            ("scr_style", "9"),
            ("autoload", "1"),
            ("autofastload", "1"),
            ("cl_alwaysrun", "0"),
        ],
    ];
    for (i, sets) in cases.iter().enumerate() {
        assert_drawn(
            &format!("options_draw(case {i})"),
            move |side| {
                // SAFETY: a command entry point under ctest_try_host.
                unsafe { ctest_menu_menu_options_f(side) };
                for (name, value) in sets.iter() {
                    set_cvar(side, name, value);
                }
            },
            // SAFETY: dispatched through ctest_try_host.
            |side| unsafe { ctest_menu_draw(side) },
        );
    }
}

/// The graphics menu is where ADR-005's one `%g` lives (`menu.c:2038`) and
/// where the `shadow_modes[]` read at `:2062` can run past the end of the
/// table. Both are driven here.
#[test]
fn graphics_menu_draw_matches_across_the_renderer_cvars_and_caps() {
    let cases: [&[(&str, &str)]; 6] = [
        &[],
        &[("r_scale", "1"), ("vid_fsaa", "1"), ("r_rtshadows", "0")],
        &[("r_scale", "4"), ("vid_fsaa", "8"), ("r_rtshadows", "1")],
        &[("r_scale", "0"), ("vid_anisotropic", "16"), ("r_oit", "2")],
        &[
            ("r_rtshadows", "3"),
            ("r_particles", "2"),
            ("r_waterwarp", "2"),
        ],
        // r_rtshadows stops at 3 on purpose: menu.c:2062 indexes a four-entry
        // stack array with (int)r_rtshadows.value and does not clamp, so 99 is a
        // read ~380 bytes past the array that faults the C oracle outright. The
        // port keeps the same shape as a COMPAT deviation (ADR-004) rather than
        // committing the UB, which means this suite can pin every in-range value
        // and none of the out-of-range ones. Recorded, not laundered.
        &[
            ("r_rtshadows", "3"),
            ("host_maxfps", "0"),
            ("r_scale", "-1"),
        ],
    ];
    for (i, sets) in cases.iter().enumerate() {
        for (ray, srs, aniso) in [(false, false, 1.0f32), (true, true, 16.0)] {
            assert_drawn(
                &format!("graphics_draw(case {i}, ray={ray}, srs={srs}, aniso={aniso})"),
                move |side| {
                    // SAFETY: shared setters plus a per-side state write.
                    unsafe {
                        ctest_menu_set_caps(ray, srs, aniso);
                        ctest_menu_set_state(side, M_GRAPHICS);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    for (name, value) in sets.iter() {
                        set_cvar(side, name, value);
                    }
                },
                // SAFETY: dispatched through ctest_try_host.
                |side| unsafe { ctest_menu_draw(side) },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// M_Keydown (menu.c:4863)

/// Every key the menu dispatcher recognises, in every state, one press at a
/// time from the state the matching `menu_*` command (or a forced `m_state`)
/// leaves behind.
#[test]
fn keydown_matches_for_every_key_in_every_state() {
    let keys = [
        K_ESCAPE,
        K_ENTER,
        K_UPARROW,
        K_DOWNARROW,
        K_LEFTARROW,
        K_RIGHTARROW,
        K_SPACE,
        K_TAB,
        K_BACKSPACE,
        K_HOME,
        K_END,
        K_PGUP,
        K_PGDN,
        K_MOUSE1,
        K_MOUSE2,
        K_MWHEELUP,
        K_MWHEELDOWN,
        b'y' as c_int,
        b'n' as c_int,
        b'a' as c_int,
        b'1' as c_int,
    ];
    for state in ALL_STATES {
        for key in keys {
            assert_same(
                &format!("keydown(state={state}, key={key})"),
                move |side| {
                    // SAFETY: per-side and shared setters.
                    unsafe {
                        ctest_menu_set_state(side, state);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                },
                // SAFETY: dispatched through ctest_try_host.
                move |side| unsafe { ctest_menu_keydown(side, key) },
            );
        }
    }
}

/// Walking the main menu top to bottom and off both ends, then entering each
/// item. This is the test that would catch a cursor that wraps at the wrong
/// index or an entry that dispatches to the wrong submenu.
#[test]
fn main_menu_cursor_walk_and_enter_match() {
    for presses in 0..8 {
        for enter in [false, true] {
            let what = format!("main_walk(down x{presses}, enter={enter})");
            assert_drawn(
                &what,
                move |side| {
                    // SAFETY: command entry point plus keydown, both guarded.
                    unsafe {
                        ctest_menu_menu_main_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    for _ in 0..presses {
                        press(side, &[K_DOWNARROW]);
                    }
                    if enter {
                        press(side, &[K_ENTER]);
                    }
                },
                // SAFETY: dispatched through ctest_try_host.
                |side| unsafe { ctest_menu_draw(side) },
            );
        }
    }
    for presses in 0..8 {
        assert_drawn(
            &format!("main_walk(up x{presses})"),
            move |side| {
                // SAFETY: as above.
                unsafe {
                    ctest_menu_menu_main_f(side);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                for _ in 0..presses {
                    press(side, &[K_UPARROW]);
                }
            },
            // SAFETY: dispatched through ctest_try_host.
            |side| unsafe { ctest_menu_draw(side) },
        );
    }
}

/// The options menu's left/right adjust path: every cursor row, both
/// directions, with the resulting cvar writes compared. `Obs::cvars` carries
/// all 53, so a slider that moves the wrong cvar fails here even if its own
/// row still looks right.
#[test]
fn options_menu_adjust_matches_for_every_row_and_direction() {
    for row in 0..20 {
        for dir in [K_LEFTARROW, K_RIGHTARROW] {
            for repeats in [1, 3] {
                let what = format!("options_adjust(row={row}, dir={dir}, x{repeats})");
                assert_same(
                    &what,
                    move |side| {
                        // SAFETY: command entry point plus keydown, both guarded.
                        unsafe {
                            ctest_menu_menu_options_f(side);
                            ctest_menu_set_key_dest(KEY_MENU);
                        }
                        for _ in 0..row {
                            press(side, &[K_DOWNARROW]);
                        }
                    },
                    move |side| {
                        for _ in 0..repeats {
                            press(side, &[dir]);
                        }
                        // SAFETY: dispatched through ctest_try_host.
                        unsafe { ctest_menu_draw(side) }
                    },
                );
            }
        }
    }
}

/// The graphics submenu's adjust path, which is the one that reaches
/// `Cvar_SetValueQuick` (`menu.c:1784-1926`) rather than `Cvar_SetValue`.
#[test]
fn graphics_menu_adjust_matches_for_every_row_and_direction() {
    // The antialiasing and render-scale rows step through a ladder rather than
    // a slider (menu.c:1758 and :1784), so the value the row starts on decides
    // which rung is taken; a single starting value would leave most of both
    // ladders unexecuted.
    for start in ["0", "1", "2", "4", "8", "16"] {
        for row in 0..16 {
            for dir in [K_LEFTARROW, K_RIGHTARROW] {
                let what = format!("graphics_adjust(start={start}, row={row}, dir={dir})");
                assert_same(
                    &what,
                    move |side| {
                        // SAFETY: per-side and shared setters.
                        unsafe {
                            ctest_menu_set_state(side, M_GRAPHICS);
                            ctest_menu_set_key_dest(KEY_MENU);
                            ctest_menu_set_caps(true, true, 16.0);
                        }
                        set_cvar(side, "vid_fsaa", start);
                        set_cvar(side, "r_scale", start);
                        // The graphics menu has no command of its own; K_HOME
                        // puts the cursor at a known row (menu.c:2668
                        // M_HandleScrollBarKeys).
                        press(side, &[K_HOME]);
                        for _ in 0..row {
                            press(side, &[K_DOWNARROW]);
                        }
                    },
                    move |side| {
                        press(side, &[dir]);
                        // SAFETY: dispatched through ctest_try_host.
                        unsafe { ctest_menu_draw(side) }
                    },
                );
            }
        }
    }
}

/// The multiplayer game-options menu writes `skill`, `coop`, `teamplay`,
/// `fraglimit`, `timelimit` and `maxplayers` -- five cvars by name and one
/// `Cbuf_AddText`, all of which `Obs` carries.
#[test]
fn mpgameoptions_adjust_matches_for_every_row_and_direction() {
    for row in 0..12 {
        for dir in [K_LEFTARROW, K_RIGHTARROW, K_ENTER] {
            let what = format!("mpgameoptions_adjust(row={row}, dir={dir})");
            assert_same(
                &what,
                move |side| {
                    // SAFETY: per-side and shared setters.
                    unsafe {
                        ctest_menu_set_state(side, M_MPGAMEOPTIONS);
                        ctest_menu_set_key_dest(KEY_MENU);
                        ctest_menu_set_server(side, false, 4, 8);
                    }
                    press(side, &[K_HOME]);
                    for _ in 0..row {
                        press(side, &[K_DOWNARROW]);
                    }
                },
                move |side| {
                    press(side, &[dir]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
        }
    }
}

/// The skill menu is the one that pushes `skill %d` and `map %s` into the
/// command buffer (`menu.c:3543-3547`).
#[test]
fn skill_menu_selection_pushes_the_same_commands() {
    for row in 0..5 {
        assert_same(
            &format!("skill_select(row={row})"),
            move |side| {
                // SAFETY: per-side and shared setters.
                unsafe {
                    ctest_menu_set_state(side, M_SKILL);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                press(side, &[K_HOME]);
                for _ in 0..row {
                    press(side, &[K_DOWNARROW]);
                }
            },
            move |side| {
                press(side, &[K_ENTER]);
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_draw(side) }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// M_Charinput (menu.c:4900) -- the two text-entry menus

/// The setup menu's name field: every printable byte, plus backspace past the
/// start, plus the 16-character clamp at `menu.c:1116`.
#[test]
fn setup_menu_name_entry_matches() {
    let inputs: [&[u8]; 6] = [
        b"",
        b"a",
        b"player two",
        b"0123456789abcdefghij",
        b"\x80\xff",
        b" ",
    ];
    for text in inputs {
        assert_drawn(
            &format!("setup_name({:?})", String::from_utf8_lossy(text)),
            move |side| {
                // SAFETY: command entry point plus keydown, both guarded.
                unsafe {
                    ctest_menu_menu_setup_f(side);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                for &b in text {
                    // SAFETY: M_Charinput does not raise.
                    unsafe { ctest_menu_charinput(side, b as c_int) };
                }
            },
            |side| {
                press(side, &[K_BACKSPACE, K_BACKSPACE, K_DOWNARROW]);
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_draw(side) }
            },
        );
    }
}

/// The LAN-config menu's port field, which is digits-only and reads back
/// through `net_hostport` (`menu.c:3890-3930`), plus the address list
/// `M_LanConfig_Draw` pulls from `NET_ListAddresses` (`menu.c:3729`).
///
/// `lan_config_cursor` (`menu.c:3673`) has no absolute reset -- `M_Menu_LanConfig_f`
/// only initialises it when it is still -1 -- so the single `K_DOWNARROW` in
/// the setup advances it by one on every pass and the field the characters
/// land in rotates 0,1,2,3,0. That is deliberate and both halves rotate in
/// lockstep off the same static, so it widens what the five texts cover rather
/// than narrowing it. The committing key is `K_BACKSPACE` and not `K_ENTER`:
/// `K_ENTER` on rows 1 and 2 runs `M_Menu_Search_f`, which leaves `m_state`
/// at `m_search`, and the draw that follows would take `M_Search_Draw`'s
/// `NET_Poll` branch into `stubs/host_ref.c:276`. `K_ENTER` on all four rows
/// is covered by `keydown_matches_for_every_key_in_every_state`, which does
/// not draw afterwards; `K_BACKSPACE` runs the same `atoi` / 65535 clamp /
/// `q_snprintf` tail (`menu.c:3937-3942`) plus its own erase branch.
#[test]
fn lanconfig_port_entry_matches() {
    for text in ["", "26000", "99999999", "1a2b3c", "0"] {
        for addrs in [&[][..], &["192.168.0.2", "10.0.0.7", "fe80::1"][..]] {
            assert_drawn(
                &format!("lanconfig_port({text},addrs={})", addrs.len()),
                move |side| {
                    // SAFETY: per-side and shared setters.
                    unsafe {
                        ctest_menu_set_state(side, M_LANCONFIG);
                        ctest_menu_set_key_dest(KEY_MENU);
                        ctest_menu_set_net(side, 0, true, true, 26000);
                        ctest_clear_net_addresses();
                        for a in addrs {
                            let c = std::ffi::CString::new(*a).unwrap();
                            ctest_add_net_address(c.as_ptr());
                        }
                    }
                    press(side, &[K_HOME, K_DOWNARROW]);
                    for b in text.bytes() {
                        // SAFETY: M_Charinput does not raise.
                        unsafe { ctest_menu_charinput(side, b as c_int) };
                    }
                },
                |side| {
                    press(side, &[K_BACKSPACE]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The keys menu (menu.c:2382)

/// Binding and unbinding write `bind "%s" "%s"` and `unbind "%s"` into the
/// command buffer, and the bind mode also calls `IN_Deactivate`
/// (`menu.c:2483`), which the draw log records.
///
/// The three binding shapes for row 0's `+forward` are what separate
/// `menu.c:2511`'s `keys[1] != -1` from `keys[0] != -1`: entering bind mode on
/// a command that is bound to exactly ONE key must leave that key alone, and
/// the next row-0 draw is where that shows (the bound key's name, not `???`).
#[test]
fn keys_menu_bind_and_unbind_match() {
    let shapes: [(&str, &[c_int]); 3] = [
        ("unbound", &[]),
        ("one key", &[b'x' as c_int]),
        ("two keys", &[b'x' as c_int, b'y' as c_int]),
    ];
    for (shape, bound) in shapes {
        for row in [0, 1, 5, 12] {
            for action in ["bind", "unbind", "escape"] {
                assert_same(
                    &format!("keys_menu({action}, row={row}, {shape})"),
                    move |side| {
                        // SAFETY: command entry point plus keydown, both guarded.
                        unsafe {
                            for &k in bound {
                                ctest_menu_bind(k, c"+forward".as_ptr());
                            }
                            ctest_menu_menu_keys_f(side);
                            ctest_menu_set_key_dest(KEY_MENU);
                        }
                        // keys_cursor and first_key are file statics that
                        // M_Menu_Keys_f (menu.c:2361) does not reset, so
                        // `row` only means bindnames[row] once K_HOME has put
                        // both back to 0 (menu.c:537).
                        press(side, &[K_HOME]);
                        for _ in 0..row {
                            press(side, &[K_DOWNARROW]);
                        }
                    },
                    move |side| {
                        match action {
                            "bind" => press(side, &[K_ENTER, b'x' as c_int]),
                            "unbind" => press(side, &[K_BACKSPACE]),
                            _ => press(side, &[K_ESCAPE]),
                        }
                        // SAFETY: dispatched through ctest_try_host.
                        unsafe { ctest_menu_draw(side) }
                    },
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The maps and mods menus (menu.c:2946, :2551)

fn seed_levels(_side: c_int) {
    let levels: [(&CStr, c_uint, &CStr); 6] = [
        (c"start", MAPTYPE_ID_START, c"Introduction"),
        (c"e1m1", MAPTYPE_ID_EP1_LEVEL, c"the Slipgate Complex"),
        (c"e1m2", MAPTYPE_ID_EP1_LEVEL, c"Castle of the Damned"),
        (c"dm1", MAPTYPE_ID_DM, c""),
        (c"custom", MAPTYPE_MOD_LEVEL, c"A Custom Map"),
        (c"modstart", MAPTYPE_MOD_START, c"Mod Start"),
    ];
    for (name, ty, msg) in levels {
        // SAFETY: three `'static` C strings into the shared filelist fixture.
        unsafe { ctest_menu_add_level(name.as_ptr(), ty, msg.as_ptr()) };
    }
}

#[test]
fn maps_menu_draw_and_walk_match() {
    for row in 0..8 {
        assert_drawn(
            &format!("maps_menu(row={row})"),
            move |side| {
                seed_levels(side);
                // SAFETY: a command entry point under ctest_try_host.
                unsafe {
                    ctest_menu_menu_maps_cmd_f(side);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                for _ in 0..row {
                    press(side, &[K_DOWNARROW]);
                }
            },
            // SAFETY: dispatched through ctest_try_host.
            |side| unsafe { ctest_menu_draw(side) },
        );
    }
}

/// Entering a map from the maps menu pushes `map "%s"` -- or, when the map is
/// the one already running, nothing (`menu.c:2807`).
#[test]
fn maps_menu_enter_matches_connected_and_disconnected() {
    for (label, state, mapname) in [
        ("disconnected", CA_DISCONNECTED, c"".as_ptr()),
        ("on e1m1", CA_CONNECTED, c"e1m1".as_ptr()),
        ("on another map", CA_CONNECTED, c"e2m1".as_ptr()),
    ] {
        for row in 0..4 {
            assert_same(
                &format!("maps_enter({label}, row={row})"),
                move |side| {
                    seed_levels(side);
                    // SAFETY: per-side setters plus a guarded command.
                    unsafe {
                        ctest_menu_set_client(side, state, -1, false, SIGNONS, 0, mapname);
                        ctest_menu_menu_maps_cmd_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    for _ in 0..row {
                        press(side, &[K_DOWNARROW]);
                    }
                },
                move |side| {
                    press(side, &[K_ENTER]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
        }
    }
}

/// `M_SetSkillMenuMap` (`menu.c:3444-3448`) falls back to the raw map name
/// when `Mod_LoadMapDescription` returns false **or** leaves the buffer empty.
/// The fixture in `stubs/stubs.c` drives all three combinations, and the title
/// it produces is then read back through `M_PrintScroll` in `M_Skill_Draw`
/// (`menu.c:3478`).
#[test]
fn skill_menu_title_follows_the_map_description() {
    for (label, desc, ret) in [
        ("absent", c"".as_ptr(), false),
        ("empty", c"".as_ptr(), true),
        ("present", c"The Slipgate Complex".as_ptr(), true),
        (
            "longer than the scroll window",
            c"a very long map description that is wider than thirty characters".as_ptr(),
            true,
        ),
    ] {
        assert_drawn(
            &format!("skill_title({label})"),
            move |side| {
                seed_levels(side);
                // SAFETY: the shared fixture plus per-side setters and a
                // guarded command.
                unsafe {
                    ctest_set_map_description(desc, ret);
                    ctest_menu_set_client(side, CA_DISCONNECTED, -1, false, 0, 0, c"".as_ptr());
                    ctest_menu_menu_maps_cmd_f(side);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                press(side, &[K_ENTER]);
            },
            // SAFETY: dispatched through ctest_try_host.
            |side| unsafe { ctest_menu_draw(side) },
        );
    }
}

#[test]
fn mods_menu_draw_and_enter_match() {
    for row in 0..5 {
        assert_drawn(
            &format!("mods_menu(row={row})"),
            move |side| {
                let mods: [(&CStr, &CStr); 4] = [
                    (c"id1", c"Quake"),
                    (c"hipnotic", c"Scourge of Armagon"),
                    (c"mymod", c""),
                    (c"ad", c"Arcane Dimensions"),
                ];
                for (name, full) in mods {
                    // SAFETY: two `'static` C strings into the shared fixture.
                    unsafe { ctest_menu_add_mod(name.as_ptr(), full.as_ptr()) };
                }
                // SAFETY: per-side and shared setters.
                unsafe {
                    ctest_menu_set_state(side, M_MODS);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                press(side, &[K_HOME]);
                for _ in 0..row {
                    press(side, &[K_DOWNARROW]);
                }
            },
            |side| {
                press(side, &[K_ENTER]);
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_draw(side) }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// The net / server-list menus (menu.c:3593, :4464)

#[test]
fn net_and_serverlist_menus_match_across_the_availability_flags() {
    for (v4, v6) in [(false, false), (true, false), (false, true), (true, true)] {
        for count in [0, 1, 8] {
            assert_drawn(
                &format!("net_menu(v4={v4},v6={v6},count={count})"),
                move |side| {
                    // SAFETY: per-side and shared setters.
                    unsafe {
                        ctest_menu_set_net(side, count, v4, v6, 26000);
                        ctest_menu_set_misc(false, false);
                        ctest_menu_set_state(side, M_NET);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                },
                |side| {
                    press(side, &[K_HOME, K_ENTER]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
            assert_drawn(
                &format!("slist_menu(v4={v4},v6={v6},count={count})"),
                move |side| {
                    // SAFETY: as above.
                    unsafe {
                        ctest_menu_set_net(side, count, v4, v6, 26000);
                        ctest_menu_set_state(side, M_SLIST);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    press(side, &[K_HOME]);
                },
                |side| {
                    press(side, &[K_DOWNARROW, K_DOWNARROW]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
        }
    }
}

/// `M_Search_Draw` (`menu.c:4434-4446`) branches on `slistInProgress` and then
/// `hostCacheCount`. Only the `!slistInProgress` half is driven here: the other
/// one calls `NET_Poll`, and the single `NET_Poll` in this link is
/// `stubs/host_ref.c`'s double, which does not return. That branch is covered
/// instead by the mask-16 case in
/// `raises_out_of_draw_keydown_and_the_video_menu_key_match`, which arms that
/// double to raise and compares how the two halves propagate it. What stays
/// NOT COVERED is what the real `NET_Poll` would put in the host cache -- that
/// would need net_main.c as an oracle source, which is not M10e work.
#[test]
fn search_menu_draw_matches_across_the_poll_branches() {
    for in_progress in [false] {
        for count in [0, 3] {
            assert_drawn(
                &format!("search_draw(inprogress={in_progress},count={count})"),
                move |side| {
                    // SAFETY: shared and per-side setters.
                    unsafe {
                        ctest_menu_set_misc(in_progress, false);
                        ctest_menu_set_net(side, count, true, false, 26000);
                        ctest_menu_set_state(side, M_SEARCH);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                },
                // SAFETY: dispatched through ctest_try_host.
                |side| unsafe { ctest_menu_draw(side) },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The quit menu (menu.c:3593) and the modal prompt

#[test]
fn quit_menu_matches_across_confirmquit_and_the_modal_answer() {
    for confirm in ["0", "1"] {
        for answer in [0, 1] {
            for key in [b'y' as c_int, b'n' as c_int, K_ESCAPE, K_ENTER] {
                assert_same(
                    &format!("quit(confirm={confirm},answer={answer},key={key})"),
                    move |side| {
                        // SAFETY: shared setter plus a guarded command.
                        unsafe { ctest_set_modal_message_answer(answer) };
                        set_cvar(side, "cl_confirmquit", confirm);
                        // SAFETY: a command entry point under ctest_try_host.
                        unsafe {
                            ctest_menu_menu_quit_f(side);
                            ctest_menu_set_key_dest(KEY_MENU);
                        }
                    },
                    move |side| {
                        let status = press_one(side, key);
                        // SAFETY: dispatched through ctest_try_host.
                        (status, unsafe { ctest_menu_draw(side) })
                    },
                );
            }
        }
    }
}

/// Park `options_cursor` on an absolute row by pointing the mouse at it.
///
/// `options_cursor` (menu.c:2215) is a file static and no entry point resets
/// it -- `M_Menu_Options_f` (menu.c:2218) only sets `m_state` -- so a key walk
/// moves it *relative* to whatever the previous test left behind and cannot
/// name a row. `M_Options_Draw` re-derives it absolutely from the mouse
/// (menu.c:2243 -> M_Mouse_UpdateListCursor, menu.c:598), which needs
/// `m_mouse_moved`; that in turn needs `menu_changed` already consumed by an
/// earlier `M_UpdateMouse` (menu.c:4636-4639), hence two updates.
///
/// The pixel-to-canvas map is menu.c:188 with the seeded 640x480 screen and
/// `scr_menuscale` 1: `s = CLAMP (1, 1, min (640/320, 480/200)) = 1`, so
/// canvas x is `px - 160` and canvas y is `py - 140`. Row `n` occupies canvas
/// y `[40 + 8n, 40 + 8n + 7]`, and x must land in `[MENU_LABEL_X, 320]`.
fn hover_options_row(side: c_int, row: c_int) {
    place_mouse_canvas(side, 240, 40 + 8 * row + 4);
    // SAFETY: dispatched through ctest_try_host.
    unsafe { ctest_menu_draw(side) };
}

/// Put the cursor on a menu-canvas coordinate and make `m_mouse_moved` true.
///
/// `ctest_menu_set_mouse` only writes the pixel position `SDL_GetMouseState`
/// reports; `m_mouse_x` / `m_mouse_y` and `m_mouse_moved` are written by
/// `M_UpdateMouse` (menu.c:4634-4643), so a fixture that sets the pixels and
/// then calls something else entirely leaves the menu reading whatever the
/// previous case left behind. Two updates are needed rather than one:
/// `m_mouse_moved` is `!menu_changed && (pixels changed)` and the first
/// update is what clears `menu_changed`.
///
/// The seeded screen is 640x480 with `scr_menuscale` 1, so
/// `M_PixelToMenuCanvasCoord` (menu.c:188) is exactly `px - 160` / `py - 140`.
fn place_mouse_canvas(side: c_int, cx: c_int, cy: c_int) {
    let px = cx + 160;
    let py = cy + 140;
    // SAFETY: the mouse fixture plus a guarded entry point.
    unsafe {
        ctest_menu_set_mouse(px, py - 1);
        ctest_menu_update_mouse(side);
        ctest_menu_set_mouse(px, py);
        ctest_menu_update_mouse(side);
    }
}

fn press_one(side: c_int, key: c_int) -> c_int {
    // SAFETY: dispatched through ctest_try_host in stubs/menu_ref.c.
    unsafe { ctest_menu_keydown(side, key) }
}

// ---------------------------------------------------------------------------
// M_UpdateMouse (menu.c:4626)

#[test]
fn update_mouse_matches_across_key_dest_state_and_button_state() {
    for state in [M_NONE, M_MAIN, M_OPTIONS, M_KEYS, M_SLIST] {
        for dest in [KEY_GAME, KEY_MENU] {
            for (mx, my) in [(0, 0), (320, 240), (639, 479)] {
                for m1 in [false, true] {
                    assert_same(
                        &format!("update_mouse({state},{dest},({mx},{my}),m1={m1})"),
                        move |side| {
                            // SAFETY: shared and per-side setters.
                            unsafe {
                                ctest_menu_set_state(side, state);
                                ctest_menu_set_key_dest(dest);
                                ctest_menu_set_mouse(mx, my);
                                ctest_menu_set_key_down(K_MOUSE1, m1);
                            }
                        },
                        // SAFETY: dispatched through ctest_try_host.
                        |side| unsafe { ctest_menu_update_mouse(side) },
                    );
                }
            }
        }
    }
}

/// `M_UpdateMouse` and the scroll handlers read `keydown[K_CTRL]` and
/// `keydown[K_MOUSE1]` (`menu.c:2698`, `:4635`), which is shared state.
#[test]
fn keydown_array_reads_match_for_ctrl_and_mouse1() {
    for ctrl in [false, true] {
        for m1 in [false, true] {
            assert_same(
                &format!("modifier_reads(ctrl={ctrl},m1={m1})"),
                move |side| {
                    // SAFETY: shared and per-side setters.
                    unsafe {
                        ctest_menu_set_key_down(K_CTRL, ctrl);
                        ctest_menu_set_key_down(K_MOUSE1, m1);
                        ctest_menu_menu_keys_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                },
                |side| {
                    press(side, &[K_DOWNARROW, K_PGDN, K_MOUSE1]);
                    // SAFETY: dispatched through ctest_try_host.
                    (unsafe { ctest_menu_update_mouse(side) }, unsafe {
                        ctest_menu_draw(side)
                    })
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The demo-loop interaction (menu.c:626-627, :672, :675)

#[test]
fn main_menu_stashes_and_restores_cls_demonum() {
    for (demonum, playback, state) in [
        (-1, false, CA_DISCONNECTED),
        (0, false, CA_DISCONNECTED),
        (3, false, CA_DISCONNECTED),
        (3, true, CA_DISCONNECTED),
        (3, false, CA_CONNECTED),
    ] {
        assert_same(
            &format!("demonum({demonum},{playback},{state})"),
            move |side| {
                // SAFETY: a per-side setter over cls.
                unsafe {
                    ctest_menu_set_client(
                        side,
                        state,
                        demonum,
                        playback,
                        SIGNONS,
                        0,
                        c"e1m1".as_ptr(),
                    )
                };
            },
            move |side| {
                // SAFETY: two guarded entry points either side of a plain
                // per-side read of cls.demonum.
                unsafe {
                    let a = ctest_menu_menu_main_f(side);
                    let mid = ctest_menu_get_demonum(side);
                    let b = ctest_menu_toggle_menu_f(side);
                    (a, mid, b)
                }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// ADR-009: the raise topology

/// `M_Options_Key` reaches `SCR_ModalMessage` on `OPT_DEFAULTS`
/// (`menu.c:2274`). Arming the double to `Host_Error` drives a raise out of
/// the `M_Keydown` entry point: the oracle escapes through the harness trap,
/// the port returns a nonzero status out of `Menu_Glue_ModalMessage` and
/// `Quake/menu_glue.c:334` re-raises it. Both must report the same status,
/// the same message, and the same state left behind.
///
/// The unraised branch matters as much as the raised one: with the mask clear
/// the same key must queue `resetcfg` + `exec default.cfg` (menu.c:2280-2281)
/// on both sides, which is what the `answer` loop checks.
#[test]
fn raises_out_of_the_options_reset_config_row() {
    let _g = lock();
    once();
    let mut seen = Vec::new();
    for side in [C, R] {
        seed_shared();
        seed_side(side);
        // SAFETY: a guarded command entry point plus the absolute cursor park.
        unsafe { ctest_menu_menu_options_f(side) };
        hover_options_row(side, OPT_DEFAULTS);
        // SAFETY: arms the SCR_ModalMessage double in stubs/stubs.c.
        unsafe {
            ctest_set_menu_raise_mask(1);
            ctest_draw_clear_log();
            ctest_clear_con_log();
            ctest_menu_cbuf_clear(side);
        }
        let status = press_one(side, K_ENTER);
        // SAFETY: the trap keeps the message NUL-terminated.
        let msg = cstr(unsafe { ctest_host_error_message() });
        seen.push((status, msg, snapshot(side)));
        // SAFETY: disarm before the next side seeds.
        unsafe { ctest_set_menu_raise_mask(0) };
    }
    assert_eq!(seen[0].0, 1, "the C oracle did not raise");
    assert_eq!(seen[0].0, seen[1].0, "reset-config raise status");
    assert_eq!(seen[0].1, seen[1].1, "reset-config raise message");
    assert_eq!(seen[0].2, seen[1].2, "reset-config state after the raise");
}

/// The same row without the raise: both answers to the modal must produce the
/// same command buffer and the same drawn frame afterwards.
#[test]
fn options_reset_config_row_matches_for_both_modal_answers() {
    for answer in [0, 1] {
        assert_same(
            &format!("reset_config(answer={answer})"),
            move |side| {
                // SAFETY: a guarded command entry point plus the cursor park
                // and the SCR_ModalMessage return value.
                unsafe {
                    ctest_menu_menu_options_f(side);
                    ctest_set_modal_message_answer(answer);
                }
                hover_options_row(side, OPT_DEFAULTS);
            },
            |side| {
                let a = press_one(side, K_ENTER);
                // SAFETY: dispatched through ctest_try_host.
                (a, unsafe { ctest_menu_draw(side) })
            },
        );
    }
}

/// A raise case: label, `ctest_set_menu_raise_mask` bit, the entry point that
/// should propagate the raise, and the setup that puts the menu in reach of it.
type RaiseCase = (&'static str, c_int, fn(c_int) -> c_int, fn(c_int));

/// The same, out of `M_Draw` (mask 4, `M_Video_Draw`) and `M_Keydown`
/// (mask 8, `M_Video_Key`) with `m_state == m_video`, out of the options
/// menu key that opens the video menu (mask 2, `M_Menu_Video_f`), and out of
/// `M_Search_Draw`'s poll branch (mask 16, `NET_Poll`) -- the one case where
/// the raise comes back through a `Host_Guard` wrapper rather than straight
/// out of the callee, because `menu.c:4436` calls `NET_Poll` directly while
/// the port calls `Host_Glue_NET_Poll` (`Quake/host_glue.c:423`) and re-raises
/// its status. Nothing else in menu.c exercises that re-raise. Last, out of
/// `M_Load_Key`'s enter arm (mask 32, `SCR_BeginLoadingPlaque`), which is the
/// only raise `menu.c:952` can produce and so the only thing that tells
/// `M_Keydown`'s `m_load` dispatch apart from one that drops the status.
#[test]
fn raises_out_of_draw_keydown_and_the_video_menu_key_match() {
    let cases: [RaiseCase; 5] = [
        (
            "load_key",
            32,
            |side| {
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_keydown(side, K_ENTER) }
            },
            |side| {
                // SAFETY: a per-side setter, then a guarded command entry
                // point. Every slot in this directory is loadable, because
                // menu.c:947 gates the raise on loadable[load_cursor] and
                // load_cursor is a file static no entry point resets.
                unsafe {
                    ctest_menu_set_gamedir(side, savegame_dir(true).as_ptr());
                    ctest_menu_menu_load_f(side);
                    ctest_menu_set_state(side, M_LOAD);
                }
            },
        ),
        (
            "draw",
            4,
            |side| {
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_draw(side) }
            },
            |side| {
                // SAFETY: a per-side setter over the glue-owned m_state.
                unsafe { ctest_menu_set_state(side, M_VIDEO) };
            },
        ),
        (
            "keydown",
            8,
            |side| {
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_keydown(side, K_DOWNARROW) }
            },
            |side| {
                // SAFETY: as above.
                unsafe { ctest_menu_set_state(side, M_VIDEO) };
            },
        ),
        (
            "search_net_poll",
            16,
            |side| {
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_draw(side) }
            },
            |side| {
                // SAFETY: shared and per-side setters; slistInProgress is what
                // takes menu.c:4435's branch to NET_Poll.
                unsafe {
                    ctest_menu_set_misc(true, false);
                    ctest_menu_set_state(side, M_SEARCH);
                }
            },
        ),
        (
            "video_menu_key",
            2,
            |side| {
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_keydown(side, K_ENTER) }
            },
            |side| {
                // SAFETY: a guarded command entry point; the mouse then parks the
                // cursor on the "Video" row (menu.c:2237, :2271-2273).
                unsafe { ctest_menu_menu_options_f(side) };
                hover_options_row(side, OPT_VIDEO);
            },
        ),
    ];
    for (what, mask, run, setup) in cases {
        let _g = lock();
        once();
        let mut seen = Vec::new();
        for side in [C, R] {
            seed_shared();
            seed_side(side);
            setup(side);
            // SAFETY: arms the matching double in stubs/stubs.c.
            unsafe {
                ctest_menu_set_key_dest(KEY_MENU);
                ctest_set_menu_raise_mask(mask);
                ctest_draw_clear_log();
                ctest_clear_con_log();
            }
            let status = run(side);
            // SAFETY: the trap keeps the message NUL-terminated.
            let msg = cstr(unsafe { ctest_host_error_message() });
            seen.push((status, msg, snapshot(side)));
            // SAFETY: disarm before the next side seeds.
            unsafe { ctest_set_menu_raise_mask(0) };
        }
        assert_eq!(seen[0].0, 1, "{what}: the C oracle did not raise");
        assert_eq!(seen[0].0, seen[1].0, "{what}: raise status");
        assert_eq!(seen[0].1, seen[1].1, "{what}: raise message");
        assert_eq!(seen[0].2, seen[1].2, "{what}: state after the raise");
    }
}

// ---------------------------------------------------------------------------
// The command argument vector (menu.c:2958 -- `menu_maps` takes a filter)

#[test]
fn menu_maps_command_reads_its_argument_the_same_way() {
    for args in [
        "menu_maps",
        "menu_maps e1",
        "menu_maps start extra",
        "menu_maps \"\"",
    ] {
        let text = std::ffi::CString::new(args).unwrap();
        assert_same(
            &format!("menu_maps({args})"),
            |side| {
                seed_levels(side);
                // SAFETY: `text` outlives the call; Cmd_TokenizeString copies.
                unsafe { ctest_menu_tokenize(side, text.as_ptr()) };
            },
            // SAFETY: a command entry point under ctest_try_host.
            |side| unsafe { ctest_menu_menu_maps_cmd_f(side) },
        );
    }
}

// ---------------------------------------------------------------------------
// The load/save slot table (menu.c:846 -- M_ScanSaves)

/// A directory of real `s*.sav` files for `M_ScanSaves` (`menu.c:846`) to find.
/// `M_ScanSaves` opens `"%s/s%i.sav"` under `com_gamedir` with `Sys_fopen` --
/// a plain path, not a search-path lookup -- so a directory and
/// `ctest_menu_set_gamedir` are the whole fixture.
///
/// `all` fills every one of the 20 slots, which is what the raise case needs:
/// `load_cursor` is a `menu.c` file static that no entry point resets, so the
/// only way to be sure `menu.c:947`'s `loadable[load_cursor]` is true without
/// reaching into a static the port does not export is for every slot to be.
/// The mixed directory is the interesting one: slot 0 is well formed and its
/// comment carries the `_` that `menu.c:857-861` turns back into a space, slot
/// 3's comment is longer than `SAVEGAME_COMMENT_LENGTH` so `q_strlcpy`
/// truncates it, slot 5 stops after the version line so the second `fscanf`
/// fails and `menu.c:849` `continue`s with the `FILE *` still open (the leak
/// the port reproduces as a COMPAT quirk), and the remaining slots are absent
/// so the unused-slot placeholder stays under comparison too.
fn savegame_dir(all: bool) -> &'static CStr {
    static MIXED: OnceLock<std::ffi::CString> = OnceLock::new();
    static ALL: OnceLock<std::ffi::CString> = OnceLock::new();
    let cell = if all { &ALL } else { &MIXED };
    cell.get_or_init(|| {
        let tag = if all { "all" } else { "mixed" };
        let root = std::env::temp_dir().join(format!(
            "quake-ctest-menu-saves-{tag}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        for slot in 0..20 {
            let path = root.join(format!("s{slot}.sav"));
            let body = match (all, slot) {
                (false, 0) => "5\nthe_necropolis_kills_10\n".to_string(),
                (false, 3) => format!("5\n{}\n", "x".repeat(64)),
                (false, 5) => "5\n".to_string(),
                (false, _) => {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                (true, _) => format!("5\nslot_{slot}\n"),
            };
            std::fs::write(&path, body).unwrap();
        }
        std::ffi::CString::new(root.to_str().unwrap()).unwrap()
    })
}

/// The slot table as `M_ScanSaves` builds it, drawn and walked. Both the
/// present and the absent slots are in the directory `savegame_dir` lays down,
/// so the placeholder text, the `_`-to-space rewrite, the truncating copy and
/// the short-read `continue` are all in one comparison; pressing `K_ENTER` on
/// whichever row the walk lands on then separates a loadable slot (which
/// records a loading plaque and queues `load s%i`) from an unused one (which
/// returns after the sound).
#[test]
fn save_and_load_slot_tables_match() {
    for row in 0..14 {
        assert_drawn(
            &format!("load_slots(row={row})"),
            move |side| {
                // SAFETY: a per-side setter, then a guarded command entry point.
                unsafe {
                    ctest_menu_set_gamedir(side, savegame_dir(false).as_ptr());
                    ctest_menu_menu_load_f(side);
                    ctest_menu_set_state(side, M_LOAD);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                for _ in 0..row {
                    press(side, &[K_DOWNARROW]);
                }
            },
            |side| {
                press(side, &[K_ENTER]);
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_draw(side) }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// The help menu (menu.c:3450) -- gfx/help%i.lmp, page wrap in both directions

#[test]
fn help_menu_page_wrap_matches() {
    for presses in 0..8 {
        for key in [K_RIGHTARROW, K_LEFTARROW] {
            assert_drawn(
                &format!("help_pages(key={key} x{presses})"),
                move |side| {
                    // SAFETY: a guarded command entry point.
                    unsafe {
                        ctest_menu_menu_help_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    for _ in 0..presses {
                        press(side, &[key]);
                    }
                },
                // SAFETY: dispatched through ctest_try_host.
                |side| unsafe { ctest_menu_draw(side) },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The single-player and multiplayer menus, which branch on sv.active and
// svs.maxclients (menu.c:793-801, :2996)

#[test]
fn singleplayer_menu_matches_across_the_server_states() {
    for (active, maxclients, limit) in [(false, 1, 1), (true, 1, 8), (true, 4, 8)] {
        for row in 0..4 {
            assert_same(
                &format!("singleplayer(active={active},mc={maxclients},row={row})"),
                move |side| {
                    // SAFETY: per-side setters plus a guarded command.
                    unsafe {
                        ctest_menu_set_server(side, active, maxclients, limit);
                        ctest_menu_menu_singleplayer_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    for _ in 0..row {
                        press(side, &[K_DOWNARROW]);
                    }
                },
                move |side| {
                    press(side, &[K_ENTER]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
        }
    }
}

#[test]
fn multiplayer_menu_matches_across_the_availability_flags() {
    for (v4, v6) in [(false, false), (true, true)] {
        for row in 0..4 {
            assert_same(
                &format!("multiplayer(v4={v4},v6={v6},row={row})"),
                move |side| {
                    // SAFETY: per-side setters plus a guarded command.
                    unsafe {
                        ctest_menu_set_net(side, 0, v4, v6, 26000);
                        ctest_menu_menu_multiplayer_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    for _ in 0..row {
                        press(side, &[K_DOWNARROW]);
                    }
                },
                move |side| {
                    press(side, &[K_ENTER]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The setup menu's colour rows (menu.c:1093-1240), which write topcolor /
// bottomcolor and, when connected, push a `color %i %i` command.

#[test]
fn setup_menu_colour_rows_match() {
    for row in 0..6 {
        for dir in [K_LEFTARROW, K_RIGHTARROW] {
            for connected in [false, true] {
                assert_same(
                    &format!("setup_colours(row={row},dir={dir},conn={connected})"),
                    move |side| {
                        // SAFETY: per-side setters plus a guarded command.
                        unsafe {
                            ctest_menu_set_client(
                                side,
                                if connected {
                                    CA_CONNECTED
                                } else {
                                    CA_DISCONNECTED
                                },
                                -1,
                                false,
                                SIGNONS,
                                0,
                                c"e1m1".as_ptr(),
                            );
                            ctest_menu_menu_setup_f(side);
                            ctest_menu_set_key_dest(KEY_MENU);
                        }
                        for _ in 0..row {
                            press(side, &[K_DOWNARROW]);
                        }
                    },
                    move |side| {
                        press(side, &[dir, dir, K_ENTER]);
                        // SAFETY: dispatched through ctest_try_host.
                        unsafe { ctest_menu_draw(side) }
                    },
                );
            }
        }
    }
}

/// `menu.c:1096` -- `int setup_cursor_table[] = {40, 56, 80, 104, 140};`.
const SETUP_CURSOR_TABLE: [c_int; 5] = [40, 56, 80, 104, 140];

/// Park the setup-menu cursor on `row` through the mouse.
///
/// `M_Menu_Setup_f` does not reset `setup_cursor` (menu.c:1102-1117), so a
/// key walk would start from whatever the previous case left. `M_Setup_Draw`
/// re-derives the cursor from the mouse (menu.c:1160-1161), which makes the
/// placement absolute.
fn hover_setup_row(side: c_int, row: usize) {
    place_mouse_canvas(side, 200, SETUP_CURSOR_TABLE[row] + 4);
    // SAFETY: dispatched through ctest_try_host.
    unsafe { ctest_menu_draw(side) };
}

/// `menu.c:1216-1237` -- the "Accept Changes" row.
///
/// The three writes it can make are independent: the name goes through
/// `Cbuf_AddText`, the hostname through the guarded `Cvar_Set`, and the two
/// colours through a single `Cbuf_AddText` gated on **either** colour having
/// moved. Cases where exactly one colour moved are what separate that `||`
/// from an `&&`.
#[test]
fn setup_menu_accept_applies_only_the_changed_fields() {
    for (dtop, dbottom) in [(0, 0), (1, 0), (0, 1), (2, 3)] {
        assert_same(
            &format!("setup_accept(dtop={dtop},dbottom={dbottom})"),
            move |side| {
                // SAFETY: a guarded command entry point plus a shared setter.
                unsafe {
                    ctest_menu_menu_setup_f(side);
                    ctest_menu_set_key_dest(KEY_MENU);
                }
                hover_setup_row(side, 2);
                for _ in 0..dtop {
                    press(side, &[K_RIGHTARROW]);
                }
                hover_setup_row(side, 3);
                for _ in 0..dbottom {
                    press(side, &[K_RIGHTARROW]);
                }
                hover_setup_row(side, 4);
            },
            move |side| {
                press(side, &[K_ENTER]);
                // SAFETY: dispatched through ctest_try_host.
                unsafe { ctest_menu_draw(side) }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// The sound and game submenus (menu.c:2100-2130, :1440-1530)

#[test]
fn sound_and_game_submenu_adjusts_match() {
    for state in [M_SOUND, M_GAME] {
        for row in 0..10 {
            for dir in [K_LEFTARROW, K_RIGHTARROW] {
                assert_same(
                    &format!("submenu({state},row={row},dir={dir})"),
                    move |side| {
                        // SAFETY: per-side and shared setters.
                        unsafe {
                            ctest_menu_set_state(side, state);
                            ctest_menu_set_key_dest(KEY_MENU);
                        }
                        press(side, &[K_HOME]);
                        for _ in 0..row {
                            press(side, &[K_DOWNARROW]);
                        }
                    },
                    move |side| {
                        press(side, &[dir]);
                        // SAFETY: dispatched through ctest_try_host.
                        unsafe { ctest_menu_draw(side) }
                    },
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Get_Menu2 (menu.c:633) -- the item count of the main menu depends on whether
// gfx/mainmenu2.lmp cached, so both answers have to be driven.

#[test]
fn main_menu_item_count_follows_the_mainmenu2_pic() {
    for missing in [false, true] {
        for presses in 0..7 {
            assert_drawn(
                &format!("mainmenu2(missing={missing}, down x{presses})"),
                move |side| {
                    // SAFETY: a shared toggle in stubs/draw_ref.c plus a guarded command.
                    unsafe {
                        ctest_draw_set_trycache_missing(missing);
                        ctest_menu_menu_main_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    for _ in 0..presses {
                        press(side, &[K_DOWNARROW]);
                    }
                },
                |side| {
                    press(side, &[K_ENTER]);
                    // SAFETY: dispatched through ctest_try_host.
                    unsafe { ctest_menu_draw(side) }
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The demo-loop guard (menu.c:673-676, :4692)

/// `if (cls.demonum != -1 && !cls.demoplayback && cls.state != ca_connected)`
/// guards the one call this suite cannot make. Each of the three conjuncts is
/// driven false here with `cl_startdemos` on, so a port that dropped a
/// conjunct would call `CL_NextDemo` on a case where C does not -- and the
/// resulting `CDAudio_Stop` abort would take the binary down, which is a
/// louder failure than an assertion.
#[test]
fn main_menu_demo_loop_guard_matches() {
    let cases = [
        ("demonum -1", -1, false, CA_DISCONNECTED),
        ("demo playing", 2, true, CA_DISCONNECTED),
        ("connected", 2, false, CA_CONNECTED),
        ("connected and playing", 2, true, CA_CONNECTED),
    ];
    for (label, demonum, playback, state) in cases {
        for key in [K_ESCAPE, K_MOUSE2] {
            assert_same(
                &format!("demo_guard({label}, key={key})"),
                move |side| {
                    // SAFETY: per-side setters plus a guarded command.
                    unsafe {
                        ctest_menu_set_client(
                            side,
                            state,
                            demonum,
                            playback,
                            SIGNONS,
                            0,
                            c"e1m1".as_ptr(),
                        );
                        ctest_menu_menu_main_f(side);
                        ctest_menu_set_key_dest(KEY_MENU);
                    }
                    set_cvar(side, "cl_startdemos", "1");
                },
                move |side| press_one(side, key),
            );
        }
    }
}
