//! Differential test: `quake_rs::sbar` vs the original `Quake/sbar.c`.
//! Phase 7 M10d.
//!
//! `sbar.c` is composed into `stubs/sbar_ref.c` behind a TU-local rename block
//! rather than listed in `build.rs`'s `C_SOURCES` (the reason is written out at
//! the top of that file). The consequence is the usual one: the oracle answers
//! to `c_ref_*` and the port answers to the plain names, and the two halves own
//! **disjoint** copies of everything `sbar.c` defines -- the ~150 `qpic_t *`
//! statics, `hipweapons[]`, `hudtype`, and the four objects
//! `Quake/sbar_glue.c` owns in the engine build (`sb_showscores`, `sb_lines`,
//! `fragsort[]`, `scoreboardlines`).
//!
//! State that is NOT disjoint and that every test therefore re-seeds before
//! each side runs (`seed_shared`):
//!
//!  * `vid` / `glwidth` / `glheight` / `scr_con_current` / `scr_viewsize`,
//!    `realtime`, `host_frametime`, `key_dest`, `draw_disc`;
//!  * `scr_style`, `scr_sbarscale`, `scr_sbaralpha`, `skill`, `teamplay`;
//!  * the wad directory both `W_GetLumpName` implementations look up in;
//!  * the draw log and the pic registry in `stubs/draw_ref.c`;
//!  * the ambient `qcvm` / `pr_global_struct` pair.
//!
//! Every drawing function's entire observable behaviour IS its sequence of
//! `Draw_*` / `M_*` / `GL_SetCanvas` calls, so the draw log is the primary
//! assertion. `assert_drawing` refuses an empty log: two empty buffers compared
//! equal is a defect, not a pass.
//!
//! ADR-009. `sbar.c`'s only longjmp-capable callee is `PR_ExecuteProgram`, at
//! `sbar.c:82` (`Sbar_CSQCCommand`), `:864`/`:870` (`Sbar_DrawCSCQ`) and
//! `:1590` (`Sbar_IntermissionOverlay`). The five entry points that reach it
//! run under `ctest_try_host` on both sides, with the Rust arm going through
//! `Host_Reraise (quake_rs_sbar_*())` exactly as `Quake/sbar_glue.c` does, so a
//! divergence in raise status is an assertion failure rather than a silent
//! difference. `raises_*` below drives that path with a QC function that calls
//! `Host_Error`, and checks the COMPAT consequence: C skips its
//! `PR_SwitchQCVM (NULL)` on the raise path and leaves the ambient qcvm
//! switched, so the port has to as well.
//!
//! Not covered, and why: `Sbar_Init`'s two `Cmd_AddCommand` registrations land
//! in two different command registries (cmd.c's for the oracle, quake-capi's
//! for the port), so only the `Sbar_LoadPics` tail is compared. `hudtype`
//! itself is a Rust-side private with no accessor; it is observed indirectly,
//! through the `picfromwad` lines `Sbar_LoadPics` emits, the pic
//! `Sbar_InventoryBarPic` returns, and the `Sbar_DrawInventory` draw log.

use core::ffi::{c_char, c_double, c_float, c_int, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `stubs/sbar_ref.c`'s side convention: 1 = the `c_ref_*` oracle, 0 = the port.
const C: c_int = 1;
const R: c_int = 0;

// client.h -- cl.gametype
const GAME_COOP: c_int = 0;
const GAME_DEATHMATCH: c_int = 1;

// keys.h:135-140 -- keydest_t
const KEY_GAME: c_int = 0;
const KEY_MENU: c_int = 3;

// quakedef.h:112-128 -- stat_t
const STAT_HEALTH: c_int = 0;
const STAT_AMMO: c_int = 3;
const STAT_ARMOR: c_int = 4;
const STAT_SHELLS: c_int = 6;
const STAT_NAILS: c_int = 7;
const STAT_ROCKETS: c_int = 8;
const STAT_CELLS: c_int = 9;
const STAT_ACTIVEWEAPON: c_int = 10;
const STAT_TOTALSECRETS: c_int = 11;
const STAT_TOTALMONSTERS: c_int = 12;
const STAT_SECRETS: c_int = 13;
const STAT_MONSTERS: c_int = 14;
const STAT_ITEMS: c_int = 15;

// quakedef.h:141-167 -- items_t
const IT_SHOTGUN: c_int = 1;
const IT_SUPER_SHOTGUN: c_int = 2;
const IT_NAILGUN: c_int = 4;
const IT_ROCKET_LAUNCHER: c_int = 32;
const IT_LIGHTNING: c_int = 64;
const IT_ARMOR2: c_int = 16384;
const IT_KEY1: c_int = 131072;
const IT_INVISIBILITY: c_int = 524288;
const IT_INVULNERABILITY: c_int = 1048576;
const IT_SUIT: c_int = 2097152;
const IT_QUAD: c_int = 4194304;
const IT_SIGIL1: c_int = 1 << 28;
const IT_SIGIL3: c_int = 1 << 30;
const IT_SUPER_NAILGUN: c_int = 8;
const IT_GRENADE_LAUNCHER: c_int = 16;
const IT_SHELLS: c_int = 256;
const IT_NAILS: c_int = 512;
const IT_ARMOR1: c_int = 8192;
const IT_ARMOR3: c_int = 32768;
const IT_KEY2: c_int = 262144;

// quakedef.h:171-192 -- rogueitems_t, the bits sbar.c reads when hudtype is 2
const RIT_LAVA_NAILGUN: c_int = 4096;
const RIT_ARMOR1: c_int = 8388608;
const RIT_ARMOR2: c_int = 16777216;
const RIT_ARMOR3: c_int = 33554432;
const RIT_LAVA_NAILS: c_int = 67108864;
const RIT_PLASMA_AMMO: c_int = 134217728;
const RIT_MULTI_ROCKETS: c_int = 268435456;
const RIT_SHIELD: c_int = 536870912;

// quakedef.h:196-206 -- hipnoticitems_t. sbar.c:60 indexes them by bit number
// through hipweapons[4] = {23, 7, 4, 16}.
const HIT_PROXIMITY_GUN: c_int = 1 << 16;
const HIT_MJOLNIR: c_int = 1 << 7;
const HIT_LASER_CANNON: c_int = 1 << 23;
const HIT_WETSUIT: c_int = 1 << 25;

const MAX_SCOREBOARD: c_int = 16;

// progs.h -- the OFS_ globals sbar.c marshals through, and the extglobals
// slots stubs/sbar_ref.c parks at fixed indices.
const OFS_RETURN: c_int = 1;
const OFS_PARM0: c_int = 4;
const OFS_PARM1: c_int = 7;
const G_CLTIME: c_int = 110;
const G_CLFRAME: c_int = 111;
const G_INTERM: c_int = 112;
const G_INTERMTIME: c_int = 113;
const G_LOCALENT: c_int = 114;
// progdefs.q1:3-9 -- globalvars_t is `int pad[28]; int self, other, world;
// float time; float frametime;`, so the two scalars sbar.c:851-852 and :1444
// write through pr_global_struct land at global 31 and 32.
const G_PR_TIME: c_int = 31;
const G_PR_FRAMETIME: c_int = 32;

/// `stubs/sbar_ref.c`'s `ctest_sbar_install_csqc` modes.
const CSQC_NONE: c_int = 0;
const CSQC_MARKER: c_int = 1;
const CSQC_RAISES: c_int = 2;

extern "C" {
    // stubs/draw_ref.c
    fn ctest_draw_clear_log();
    fn ctest_draw_log() -> *const c_char;
    fn ctest_draw_set_pic_newline(on: bool);

    // stubs/sbar_ref.c -- shared state
    fn ctest_sbar_set_screen(
        vid_w: c_int,
        vid_h: c_int,
        glw: c_int,
        glh: c_int,
        con_current: c_float,
        viewsize: c_float,
    );
    fn ctest_sbar_set_cvars(
        style: c_float,
        sbarscale: c_float,
        sbaralpha: c_float,
        skill: c_float,
        teamplay: c_float,
    );
    fn ctest_sbar_set_time(now: c_double, frametime: c_double);
    fn ctest_sbar_set_key_dest(dest: c_int);
    fn ctest_sbar_set_draw_disc(name: *const c_char);
    fn ctest_sbar_clear_lumps();
    fn ctest_sbar_seed_hud(mode: c_int);
    fn ctest_sbar_tokenize(side: c_int, text: *const c_char);

    // stubs/sbar_ref.c -- per-side client state
    #[allow(clippy::too_many_arguments)]
    fn ctest_sbar_set_client(
        side: c_int,
        time: c_double,
        oldtime: c_double,
        intermission: c_int,
        completed_time: c_int,
        faceanimtime: c_float,
        items: c_int,
        viewentity: c_int,
        maxclients: c_int,
        gametype: c_int,
        mapname: *const c_char,
        levelname: *const c_char,
    );
    fn ctest_sbar_set_stat(side: c_int, stat: c_int, value: c_int);
    fn ctest_sbar_get_stat(side: c_int, stat: c_int) -> c_int;
    fn ctest_sbar_get_item_gettime(side: c_int, index: c_int) -> c_float;
    fn ctest_sbar_exec_cmd(side: c_int, text: *const c_char) -> c_int;
    fn ctest_sbar_set_item_gettime(side: c_int, index: c_int, t: c_float);
    fn ctest_sbar_clear_scores(side: c_int);
    fn ctest_sbar_set_score(
        side: c_int,
        index: c_int,
        name: *const c_char,
        frags: c_int,
        colors: c_int,
        entertime: c_float,
        ping: c_int,
    );

    // stubs/sbar_ref.c -- the glue-owned objects
    fn ctest_sbar_set_sb_showscores(side: c_int, v: bool);
    fn ctest_sbar_get_sb_showscores(side: c_int) -> bool;
    fn ctest_sbar_set_sb_lines(side: c_int, v: c_int);
    fn ctest_sbar_get_sb_lines(side: c_int) -> c_int;
    fn ctest_sbar_get_scoreboardlines(side: c_int) -> c_int;
    fn ctest_sbar_get_fragsort(side: c_int, index: c_int) -> c_int;

    // stubs/sbar_ref.c -- the CSQC fixture
    fn ctest_sbar_install_csqc(
        side: c_int,
        consolecommand: c_int,
        drawhud: c_int,
        drawscores: c_int,
    );
    fn ctest_sbar_clear_csqc(side: c_int);
    fn ctest_sbar_get_qc_global_int(side: c_int, ofs: c_int) -> c_int;
    fn ctest_sbar_set_qc_global_int(side: c_int, ofs: c_int, v: c_int);
    fn ctest_sbar_builtin_calls() -> c_int;
    fn ctest_sbar_reset_builtin_calls();
    fn ctest_sbar_qcvm_active() -> bool;
    fn ctest_sbar_clear_qcvm();

    // stubs/sbar_ref.c -- entry points
    fn ctest_sbar_csqc_command(side: c_int, ret: *mut c_int) -> c_int;
    fn ctest_sbar_show_scores(side: c_int) -> c_int;
    fn ctest_sbar_dont_show_scores(side: c_int) -> c_int;
    fn ctest_sbar_draw(side: c_int) -> c_int;
    fn ctest_sbar_intermission_overlay(side: c_int) -> c_int;
    fn ctest_sbar_load_pics(side: c_int);
    fn ctest_sbar_init(side: c_int);
    fn ctest_sbar_draw_pic(side: c_int, x: c_int, y: c_int, pic: *const c_char);
    fn ctest_sbar_draw_pic_alpha(
        side: c_int,
        x: c_int,
        y: c_int,
        pic: *const c_char,
        alpha: c_float,
    );
    fn ctest_sbar_draw_character(side: c_int, x: c_int, y: c_int, num: c_int);
    fn ctest_sbar_draw_string(side: c_int, x: c_int, y: c_int, str_: *const c_char);
    fn ctest_sbar_draw_scroll_string(
        side: c_int,
        x: c_int,
        y: c_int,
        width: c_int,
        str_: *const c_char,
    );
    fn ctest_sbar_itoa(side: c_int, num: c_int, buf: *mut c_char) -> c_int;
    fn ctest_sbar_draw_num(
        side: c_int,
        x: c_int,
        y: c_int,
        num: c_int,
        digits: c_int,
        color: c_int,
    );
    fn ctest_sbar_draw_small_ammo_counter(side: c_int, x: c_int, y: c_int, val: c_int);
    fn ctest_sbar_sort_frags(side: c_int);
    fn ctest_sbar_color_for_map(side: c_int, m: c_int) -> c_int;
    fn ctest_sbar_solo_scoreboard(side: c_int);
    fn ctest_sbar_draw_scoreboard(side: c_int);
    fn ctest_sbar_inventory_bar_pic(side: c_int) -> *const c_char;
    fn ctest_sbar_calculate_flash_on(side: c_int, val: c_int) -> c_int;
    fn ctest_sbar_draw_inventory(side: c_int);
    fn ctest_sbar_draw_frags(side: c_int);
    fn ctest_sbar_draw_face(side: c_int, x: c_int, y: c_int, classic_style: bool);
    fn ctest_sbar_intermission_number(
        side: c_int,
        x: c_int,
        y: c_int,
        num: c_int,
        digits: c_int,
        color: c_int,
    );
    fn ctest_sbar_intermission_pic_for_char(side: c_int, ch: c_char, color: c_int)
        -> *const c_char;
    fn ctest_sbar_intermission_text_width(side: c_int, str_: *const c_char, color: c_int) -> c_int;
    fn ctest_sbar_intermission_text(
        side: c_int,
        x: c_int,
        y: c_int,
        str_: *const c_char,
        color: c_int,
    );
    fn ctest_sbar_deathmatch_overlay(side: c_int);
    fn ctest_sbar_mini_deathmatch_overlay(side: c_int);
    fn ctest_sbar_finale_overlay(side: c_int);
}

// ---------------------------------------------------------------------------
// harness

fn draw_log() -> String {
    // SAFETY: stubs/draw_ref.c keeps the buffer NUL-terminated.
    unsafe { CStr::from_ptr(ctest_draw_log()) }
        .to_string_lossy()
        .into_owned()
}

/// Everything both halves share. Re-seeded before each side runs.
fn seed_shared() {
    // SAFETY: plain setters over statics in stubs/sbar_ref.c and draw_ref.c.
    unsafe {
        if ctest_sbar_qcvm_active() {
            ctest_sbar_clear_qcvm();
        }
        ctest_sbar_set_screen(640, 480, 640, 480, 0.0, 100.0);
        ctest_sbar_set_cvars(0.0, 1.0, 0.75, 1.0, 0.0);
        ctest_sbar_set_time(10.0, 0.05);
        ctest_sbar_set_key_dest(KEY_GAME);
        ctest_sbar_set_draw_disc(c"disc".as_ptr());
        ctest_sbar_clear_lumps();
        ctest_draw_set_pic_newline(true);
        ctest_sbar_reset_builtin_calls();
    }
}

/// A quiet single-player client with no items and no CSQC.
fn seed_client(side: c_int) {
    // SAFETY: per-side setters over `cl` / the fixture's scoreboard array.
    unsafe {
        ctest_sbar_clear_csqc(side);
        ctest_sbar_set_client(
            side,
            12.0,
            11.95,
            0,
            0,
            0.0,
            0,
            1,
            1,
            GAME_COOP,
            c"e1m1".as_ptr(),
            c"the slipgate complex".as_ptr(),
        );
        for stat in 0..32 {
            ctest_sbar_set_stat(side, stat, 0);
        }
        for i in 0..32 {
            ctest_sbar_set_item_gettime(side, i, 0.0);
        }
        ctest_sbar_clear_scores(side);
        ctest_sbar_set_sb_showscores(side, false);
        ctest_sbar_set_sb_lines(side, 24);
    }
}

/// Run both sides. For each side in turn: seed the shared state, run `setup`,
/// clear the draw log, invoke, and read the observation **immediately** --
/// anything read after the loop would see only the last side's state.
fn both<T>(setup: impl Fn(c_int), run: impl Fn(c_int) -> T) -> ((String, T), (String, T)) {
    let mut out = Vec::new();
    for side in [C, R] {
        seed_shared();
        seed_client(side);
        setup(side);
        // SAFETY: clears the shared recorder in stubs/draw_ref.c.
        unsafe { ctest_draw_clear_log() };
        let value = run(side);
        out.push((draw_log(), value));
    }
    let rust = out.pop().unwrap();
    let c = out.pop().unwrap();
    (c, rust)
}

/// For entry points whose whole observable behaviour is the draw log. The
/// non-empty check is deliberate: comparing two empty buffers is not a test.
fn assert_drawing<T: PartialEq + std::fmt::Debug>(
    what: &str,
    setup: impl Fn(c_int),
    run: impl Fn(c_int) -> T,
) {
    let _g = lock();
    let (c, r) = both(setup, run);
    assert!(!c.0.is_empty(), "{what}: the C oracle drew nothing");
    assert_eq!(c.0, r.0, "{what}: draw log");
    assert_eq!(c.1, r.1, "{what}: result");
}

/// For the pure helpers, which must draw nothing at all.
fn assert_quiet<T: PartialEq + std::fmt::Debug>(
    what: &str,
    setup: impl Fn(c_int),
    run: impl Fn(c_int) -> T,
) {
    let _g = lock();
    let (c, r) = both(setup, run);
    assert_eq!(c.0, "", "{what}: the C oracle drew something unexpected");
    assert_eq!(c.0, r.0, "{what}: draw log");
    assert_eq!(c.1, r.1, "{what}: result");
}

fn noop(_side: c_int) {}

// ---------------------------------------------------------------------------
// Sbar_itoa (sbar.c:356)

/// Two `Sbar_itoa` inputs are out of range here, both because the C oracle
/// stops being observable on them rather than because the port disagrees.
///
/// `|num| >= 1000000000`: the `for (pow10 = 10; num >= pow10; pow10 *= 10)`
/// scan does not terminate, because `pow10` wraps past `10^9` and every
/// `10^k` with `k >= 32` is zero modulo 2^32, at which point `num >= pow10`
/// holds forever. That is a live defect in `Quake/sbar.c:362` -- reported,
/// not papered over -- and the port reproduces it. A hang cannot be
/// differentially compared.
///
/// `i32::MIN`: `num = -num` (`sbar.c:367`) is signed overflow, which clang
/// takes as licence to assume `num > 0` from there on. On the real `INT_MIN`
/// value the `while (pow10 != 1)` exit is then unreachable, and the digit
/// loop writes past the caller's buffer until it faults on the stack guard
/// page -- observed as SIGSEGV/SIGBUS in `c_ref_Sbar_itoa` on release
/// aarch64-apple-darwin, through this entry point and through the two
/// callers that inline it (`Sbar_DrawNum` at `sbar.c:400` and
/// `Sbar_IntermissionNumber` at `sbar.c:1317`). A crashing oracle is no more
/// comparable than a hanging one. The port itself is well-defined on
/// `i32::MIN` and keeps the wrapping arithmetic (COMPAT: ADR-010,
/// `quake-capi/src/sbar.rs`); there is simply no C side to diff it against.
#[test]
fn itoa_matches_across_the_sign_and_width_range() {
    for num in [
        0, 1, 9, 10, 99, 100, 999, 1000, 99999, 123456789, 999999999, -1, -9, -10, -999, -123456,
        -999999999,
    ] {
        assert_quiet(&format!("itoa({num})"), noop, |side| {
            let mut buf = [0i8; 64];
            // SAFETY: `buf` is 64 bytes; the longest output is 12.
            let len = unsafe { ctest_sbar_itoa(side, num, buf.as_mut_ptr()) };
            let bytes: Vec<u8> = buf[..len.max(0) as usize]
                .iter()
                .map(|&b| b as u8)
                .collect();
            (len, String::from_utf8_lossy(&bytes).into_owned())
        });
    }
}

/// The `i32::MIN` half of the exclusion above, pinned on the port alone so
/// the input keeps a regression test even though the C oracle cannot run it.
/// `num.wrapping_neg()` stays `INT_MIN`, so the `pow10` scan exits at once and
/// the single digit is `('0' + INT_MIN) as c_char`, which truncates back to
/// `'0'` -- output `"-0"`, length 2.
#[test]
fn itoa_of_int_min_is_well_defined_in_the_port() {
    let _g = lock();
    let mut buf = [0i8; 64];
    // SAFETY: `buf` is 64 bytes; this input writes 3 including the NUL.
    let len = unsafe { ctest_sbar_itoa(R, i32::MIN, buf.as_mut_ptr()) };
    let bytes: Vec<u8> = buf[..len.max(0) as usize]
        .iter()
        .map(|&b| b as u8)
        .collect();
    assert_eq!((len, String::from_utf8_lossy(&bytes)), (2, "-0".into()));
}

// ---------------------------------------------------------------------------
// Sbar_ColorForMap (sbar.c:475)

#[test]
fn color_for_map_matches_over_the_full_byte_range() {
    assert_quiet("color_for_map", noop, |side| -> Vec<c_int> {
        (-8..272)
            // SAFETY: a pure arithmetic helper.
            .map(|m| unsafe { ctest_sbar_color_for_map(side, m) })
            .collect()
    });
}

// ---------------------------------------------------------------------------
// Sbar_CalculateFlashOn (sbar.c:558)

#[test]
fn calculate_flash_on_matches_across_the_item_timings() {
    // 11.45 and 11.05 put (cl.time - time) at 0.55 and 0.95, the two deltas
    // whose *10 and *11 scalings land on different sides of the %5 wrap and
    // of the >= 10 cutoff (sbar.c:563).
    for (now, gettime, val) in [
        (12.0, 0.0, 0),
        (12.0, 11.9, 3),
        (12.0, 11.99, 5),
        (12.0, 12.0, 7),
        (12.0, 12.5, 1),
        (12.0, 11.45, 2),
        (12.0, 11.05, 6),
        (0.5, 0.0, 4),
        (100.0, 99.0, 31),
    ] {
        assert_quiet(
            &format!("calculate_flash_on({now}, {gettime}, {val})"),
            move |side| {
                // SAFETY: per-side seeders.
                unsafe {
                    ctest_sbar_set_client(
                        side,
                        now,
                        now - 0.05,
                        0,
                        0,
                        0.0,
                        0,
                        1,
                        1,
                        GAME_COOP,
                        c"e1m1".as_ptr(),
                        c"level".as_ptr(),
                    );
                    for i in 0..32 {
                        ctest_sbar_set_item_gettime(side, i, gettime);
                    }
                }
            },
            // SAFETY: a pure helper over cl.time, plus the cl.item_gettime
            // write-back at sbar.c:561, which is otherwise invisible: the
            // stamp it stores yields the same flashon either way.
            |side| unsafe {
                let on = ctest_sbar_calculate_flash_on(side, val);
                let stamps: Vec<u32> = (0..32)
                    .map(|i| ctest_sbar_get_item_gettime(side, i).to_bits())
                    .collect();
                (on, stamps)
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Sbar_SortFrags (sbar.c:446)

/// Seeds `maxclients` players, including a run of ties, so the ordering the
/// insertion sort produces for equal frags is compared too.
fn seed_players(side: c_int, frags: &[c_int]) {
    // SAFETY: per-side seeders.
    unsafe {
        ctest_sbar_set_client(
            side,
            12.0,
            11.95,
            0,
            0,
            0.0,
            0,
            1,
            frags.len() as c_int,
            GAME_DEATHMATCH,
            c"e1m1".as_ptr(),
            c"level".as_ptr(),
        );
        ctest_sbar_clear_scores(side);
        for (i, &f) in frags.iter().enumerate() {
            let name = std::ffi::CString::new(format!("player{i}")).unwrap();
            ctest_sbar_set_score(
                side,
                i as c_int,
                name.as_ptr(),
                f,
                (i as c_int * 0x11) & 0xff,
                i as c_float,
                20 + i as c_int,
            );
        }
    }
}

#[test]
fn sort_frags_matches_including_ties_and_empty_slots() {
    for frags in [
        vec![],
        vec![5],
        vec![1, 2, 3, 4],
        vec![4, 3, 2, 1],
        vec![7, 7, 7, 7],
        vec![0, 5, 0, 5, 0],
        vec![-3, 12, -3, 0, 12, 1],
        (0..MAX_SCOREBOARD).rev().collect::<Vec<_>>(),
    ] {
        let label = format!("sort_frags({frags:?})");
        let f = frags.clone();
        assert_quiet(
            &label,
            move |side| {
                if f.is_empty() {
                    // maxclients 0: nothing to sort, and scoreboardlines must
                    // still come out identical.
                    // SAFETY: per-side seeder.
                    unsafe {
                        ctest_sbar_set_client(
                            side,
                            12.0,
                            11.95,
                            0,
                            0,
                            0.0,
                            0,
                            1,
                            0,
                            GAME_DEATHMATCH,
                            c"e1m1".as_ptr(),
                            c"level".as_ptr(),
                        );
                        ctest_sbar_clear_scores(side);
                    }
                } else {
                    seed_players(side, &f);
                }
            },
            |side| {
                // SAFETY: the sort plus an immediate per-side readback.
                unsafe {
                    ctest_sbar_sort_frags(side);
                    let lines = ctest_sbar_get_scoreboardlines(side);
                    let order: Vec<c_int> = (0..MAX_SCOREBOARD)
                        .map(|i| ctest_sbar_get_fragsort(side, i))
                        .collect();
                    (lines, order)
                }
            },
        );
    }
}

/// Players with no name are skipped by the scan (`sbar.c:452`), which is what
/// makes `scoreboardlines` differ from `cl.maxclients`.
#[test]
fn sort_frags_skips_unnamed_slots() {
    assert_quiet(
        "sort_frags/unnamed",
        |side| {
            // SAFETY: per-side seeders.
            unsafe {
                ctest_sbar_set_client(
                    side,
                    12.0,
                    11.95,
                    0,
                    0,
                    0.0,
                    0,
                    1,
                    6,
                    GAME_DEATHMATCH,
                    c"e1m1".as_ptr(),
                    c"level".as_ptr(),
                );
                ctest_sbar_clear_scores(side);
                ctest_sbar_set_score(side, 0, c"alpha".as_ptr(), 3, 0x10, 1.0, 20);
                ctest_sbar_set_score(side, 2, c"".as_ptr(), 99, 0x20, 2.0, 30);
                ctest_sbar_set_score(side, 4, c"omega".as_ptr(), 9, 0x30, 3.0, 40);
            }
        },
        |side| {
            // SAFETY: sort plus immediate readback.
            unsafe {
                ctest_sbar_sort_frags(side);
                (
                    ctest_sbar_get_scoreboardlines(side),
                    (0..MAX_SCOREBOARD)
                        .map(|i| ctest_sbar_get_fragsort(side, i))
                        .collect::<Vec<_>>(),
                )
            }
        },
    );
}

// ---------------------------------------------------------------------------
// the primitive draw helpers (sbar.c:297-437)

#[test]
fn draw_pic_and_pic_alpha_match() {
    assert_drawing(
        "draw_pic",
        noop,
        // SAFETY: the fixture interns the pic name and forwards.
        |side| unsafe {
            ctest_sbar_draw_pic(side, 0, 0, c"sbar".as_ptr());
            ctest_sbar_draw_pic(side, -13, 200, c"ibar".as_ptr());
            ctest_sbar_draw_pic_alpha(side, 7, 3, c"face1".as_ptr(), 0.25);
            ctest_sbar_draw_pic_alpha(side, 7, 3, c"face1".as_ptr(), 1.0);
        },
    );
}

#[test]
fn draw_pic_honours_scr_sbaralpha() {
    for alpha in [0.0f32, 0.5, 0.999, 1.0, 2.0] {
        assert_drawing(
            &format!("draw_pic/sbaralpha={alpha}"),
            move |_side| {
                // SAFETY: shared cvar seeder.
                unsafe { ctest_sbar_set_cvars(0.0, 1.0, alpha, 1.0, 0.0) };
            },
            // SAFETY: as above.
            |side| unsafe { ctest_sbar_draw_pic(side, 4, 5, c"sbar".as_ptr()) },
        );
    }
}

#[test]
fn draw_character_and_string_match() {
    assert_drawing(
        "draw_character/string",
        noop,
        // SAFETY: as above.
        |side| unsafe {
            for num in [0, 32, 48, 65, 127, 128, 255] {
                ctest_sbar_draw_character(side, num, 8, num);
            }
            ctest_sbar_draw_string(side, 0, 0, c"".as_ptr());
            ctest_sbar_draw_string(side, 3, 4, c"HELLO quake".as_ptr());
        },
    );
}

#[test]
fn draw_scroll_string_matches_across_the_scroll_phase() {
    // Sbar_DrawScrollString's offset is ((int)(realtime * 30)) % len, so the
    // phase has to be swept to see the wraparound (sbar.c:337).
    for now in [0.0f64, 0.033, 0.5, 1.0, 2.7, 10.0, 123.456] {
        for width in [8, 40, 160, 320] {
            assert_drawing(
                &format!("draw_scroll_string({now}, {width})"),
                move |_side| {
                    // SAFETY: shared time seeder.
                    unsafe { ctest_sbar_set_time(now, 0.05) };
                },
                // SAFETY: as above.
                move |side| unsafe {
                    ctest_sbar_draw_scroll_string(
                        side,
                        0,
                        4,
                        width,
                        c"the quick brown fox jumps over the lazy dog".as_ptr(),
                    )
                },
            );
        }
    }
}

#[test]
fn draw_num_matches_across_digits_and_colours() {
    assert_drawing(
        "draw_num",
        noop,
        // SAFETY: as above.
        |side| unsafe {
            // i32::MIN excluded: see itoa_matches_across_the_sign_and_width_range.
            for &num in &[0, 5, 42, 999, 1000, -1, -42, -1000] {
                for digits in [1, 3, 5] {
                    for color in [0, 1] {
                        ctest_sbar_draw_num(side, 24, 0, num, digits, color);
                    }
                }
            }
        },
    );
}

#[test]
fn draw_small_ammo_counter_matches() {
    assert_drawing(
        "draw_small_ammo_counter",
        noop,
        // SAFETY: as above.
        |side| unsafe {
            for val in [0, 1, 9, 10, 99, 100, 255, 1000, -1] {
                ctest_sbar_draw_small_ammo_counter(side, 10, 20, val);
            }
        },
    );
}

// ---------------------------------------------------------------------------
// Sbar_LoadPics / Sbar_Init / hudtype autodetect (sbar.c:138, :283)

#[test]
fn load_pics_matches_for_every_hud_autodetect_outcome() {
    // 0 = no optional lumps (hudtype 0), 1 = the full hipnotic set (1),
    // 2 = the rogue set (2), 3 = hipnotic minus its last probe, which drops
    // back to 0 partway through the block, 4 = hipnotic minus its *first*
    // probe, the only arrangement that reaches Sbar_CheckPicFromWad's
    // hudtype == 0 early-out with the lump present (sbar.c:118).
    for mode in [0, 1, 2, 3, 4] {
        assert_drawing(
            &format!("load_pics/hud={mode}"),
            move |_side| {
                // SAFETY: seeds the shared wad directory.
                unsafe { ctest_sbar_seed_hud(mode) };
            },
            // SAFETY: as above.
            |side| unsafe { ctest_sbar_load_pics(side) },
        );
    }
}

#[test]
fn init_matches() {
    assert_drawing(
        "init",
        |_side| {
            // SAFETY: seeds the shared wad directory.
            unsafe { ctest_sbar_seed_hud(1) };
        },
        // SAFETY: as above. Only the Sbar_LoadPics tail is observable here;
        // the two Cmd_AddCommand calls land in two different registries.
        |side| unsafe { ctest_sbar_init(side) },
    );
}

/// `Sbar_Init`'s two `Cmd_AddCommand` calls land in two different registries,
/// so they are compared by running the commands and looking at what they did
/// (`sbar.c:285-286`). The first `Sbar_Init` in the process wins the
/// registration, so this observes whichever handler that call bound.
#[test]
fn init_registers_the_showscores_commands() {
    for (cmd, want) in [(c"+showscores", true), (c"-showscores", false)] {
        assert_quiet(
            &format!("init/{}", cmd.to_string_lossy()),
            |side| {
                // SAFETY: shared wad seed, then this side's registration.
                unsafe {
                    ctest_sbar_seed_hud(0);
                    ctest_sbar_init(side);
                    ctest_sbar_set_sb_showscores(side, !want);
                }
            },
            move |side| {
                // SAFETY: guarded dispatch into the side's own registry, then
                // an immediate readback.
                unsafe {
                    let raised = ctest_sbar_exec_cmd(side, cmd.as_ptr());
                    let got = ctest_sbar_get_sb_showscores(side);
                    assert_eq!(raised, 0, "running {cmd:?} raised");
                    assert_eq!(got, want, "{cmd:?} did not reach the right handler");
                    (raised, got)
                }
            },
        );
    }
}

#[test]
fn inventory_bar_pic_matches_for_every_hudtype() {
    for mode in [0, 1, 2] {
        for weapon in [0, 5, 6, 7, 8, 9, 10] {
            assert_quiet(
                &format!("inventory_bar_pic/hud={mode}/weapon={weapon}"),
                move |side| {
                    // SAFETY: shared wad seed, then this side's pic table.
                    unsafe {
                        ctest_sbar_seed_hud(mode);
                        ctest_sbar_load_pics(side);
                        ctest_sbar_set_stat(side, STAT_ACTIVEWEAPON, weapon);
                    }
                },
                |side| {
                    // SAFETY: returns an interned name owned by draw_ref.c.
                    unsafe {
                        CStr::from_ptr(ctest_sbar_inventory_bar_pic(side))
                            .to_string_lossy()
                            .into_owned()
                    }
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Sbar_DrawInventory / DrawFrags / DrawFace (sbar.c:581, :698, :740)

/// A loaded, well-stocked player, so the inventory/face branches all have
/// something to draw.
fn seed_loaded(side: c_int, hud: c_int, items: c_int, health: c_int) {
    // SAFETY: shared wad seed, then this side's pic table and client state.
    unsafe {
        ctest_sbar_seed_hud(hud);
        ctest_sbar_load_pics(side);
        ctest_sbar_set_client(
            side,
            12.0,
            11.95,
            0,
            0,
            12.3,
            items,
            1,
            1,
            GAME_COOP,
            c"e1m1".as_ptr(),
            c"the slipgate complex".as_ptr(),
        );
        ctest_sbar_set_stat(side, STAT_HEALTH, health);
        ctest_sbar_set_stat(side, STAT_ARMOR, 87);
        ctest_sbar_set_stat(side, STAT_SHELLS, 25);
        ctest_sbar_set_stat(side, STAT_NAILS, 100);
        ctest_sbar_set_stat(side, STAT_ROCKETS, 5);
        ctest_sbar_set_stat(side, STAT_CELLS, 60);
        ctest_sbar_set_stat(side, STAT_ACTIVEWEAPON, 6);
        ctest_sbar_set_stat(side, STAT_ITEMS, items);
        for i in 0..32 {
            ctest_sbar_set_item_gettime(side, i, 11.9);
        }
    }
}

const LOADOUT: c_int = IT_SHOTGUN
    | IT_SUPER_SHOTGUN
    | IT_NAILGUN
    | IT_ROCKET_LAUNCHER
    | IT_LIGHTNING
    | IT_ARMOR2
    | IT_KEY1
    | IT_QUAD
    | IT_SIGIL1
    | IT_SIGIL3;

/// Every weapon slot filled, with no powerup and no armour, so the bar and
/// face branches that `LOADOUT` short-circuits stay reachable.
const DRAW_BASE: c_int = IT_SHOTGUN
    | IT_SUPER_SHOTGUN
    | IT_NAILGUN
    | IT_SUPER_NAILGUN
    | IT_GRENADE_LAUNCHER
    | IT_ROCKET_LAUNCHER
    | IT_LIGHTNING
    | IT_SIGIL1
    | IT_SIGIL3;

/// `LOADOUT` carries `IT_QUAD`, so `Sbar_DrawFace` returns at its powerup
/// branch (`sbar.c:806`) before any of the health arithmetic runs. The face
/// table needs a powerup-free base to reach `sbar.c:818-829` at all.
const FACE_BASE: c_int = DRAW_BASE | IT_ARMOR2 | IT_KEY1;

#[test]
fn draw_inventory_matches_for_every_hudtype() {
    for hud in [0, 1, 2] {
        for items in [0, LOADOUT, DRAW_BASE, -1] {
            // STAT_ACTIVEWEAPON selects the highlighted slot, and in the rogue
            // set it also drives the "powered weapon" skip at sbar.c:600; the
            // hipnotic proximity gun occupies slot 4 (sbar.c:634).
            for weapon in [
                6,
                IT_NAILGUN,
                IT_LIGHTNING,
                RIT_LAVA_NAILGUN,
                RIT_LAVA_NAILGUN << 2,
                HIT_PROXIMITY_GUN,
            ] {
                assert_drawing(
                    &format!("draw_inventory/hud={hud}/items={items:#x}/weapon={weapon:#x}"),
                    move |side| {
                        seed_loaded(side, hud, items, 75);
                        // SAFETY: per-side stat seed.
                        unsafe { ctest_sbar_set_stat(side, STAT_ACTIVEWEAPON, weapon) };
                    },
                    // SAFETY: dispatches to the side's Sbar_DrawInventory.
                    |side| unsafe { ctest_sbar_draw_inventory(side) },
                );
            }
        }
    }
}

#[test]
fn draw_frags_matches() {
    assert_drawing(
        "draw_frags",
        |side| {
            seed_loaded(side, 0, LOADOUT, 75);
            seed_players(side, &[3, 9, 9, -2, 14, 0]);
        },
        // SAFETY: as above.
        |side| unsafe { ctest_sbar_draw_frags(side) },
    );
}

#[test]
fn draw_face_matches_across_health_powerups_and_style() {
    for classic in [false, true] {
        // 0/1, 19/20/21 and 99/100 straddle the health/20 bucket edges, the
        // f < 0 floor and the >= 100 cap (sbar.c:818-824).
        for health in [-5, 0, 1, 19, 20, 21, 40, 60, 75, 99, 100, 200] {
            for extra in [
                0,
                IT_INVISIBILITY,
                IT_INVULNERABILITY,
                IT_INVISIBILITY | IT_INVULNERABILITY,
                IT_QUAD,
                IT_SUIT,
            ] {
                // cl.faceanimtime on either side of cl.time (12.0) picks the
                // pain frame at sbar.c:826.
                for faceanim in [12.0f32, 11.0] {
                    let items = FACE_BASE | extra;
                    let what = format!(
                        "draw_face/classic={classic}/health={health}/extra={extra:#x}/anim={faceanim}"
                    );
                    assert_drawing(
                        &what,
                        move |side| {
                            seed_loaded(side, 0, items, health);
                            // SAFETY: per-side client seed; seed_loaded pins
                            // faceanimtime at 12.3, which this overrides.
                            unsafe {
                                ctest_sbar_set_client(
                                    side,
                                    12.0,
                                    11.95,
                                    0,
                                    0,
                                    faceanim,
                                    items,
                                    1,
                                    1,
                                    GAME_COOP,
                                    c"e1m1".as_ptr(),
                                    c"level".as_ptr(),
                                );
                                ctest_sbar_set_stat(side, STAT_HEALTH, health);
                                ctest_sbar_set_stat(side, STAT_ITEMS, items);
                            }
                        },
                        // SAFETY: as above.
                        move |side| unsafe { ctest_sbar_draw_face(side, 112, 0, classic) },
                    );
                }
            }
        }
    }
}

/// The rogue CTF swatch (`sbar.c:750`) is gated on five conditions at once.
/// The first loop walks each gate across its edge; the second exercises what
/// the branch draws once taken, including the `top == 8` glyph swap at
/// `sbar.c:786` and the coop/deathmatch `xofs` split at `sbar.c:772`.
#[test]
fn draw_face_matches_in_the_rogue_ctf_branch() {
    for classic in [false, true] {
        for teamplay in [0.0f32, 3.0, 3.5, 6.5, 7.0, 8.0] {
            for hud in [0, 2] {
                for maxclients in [1, 6] {
                    assert_drawing(
                        &format!("draw_face/ctf/gate/{classic}/{teamplay}/{hud}/{maxclients}"),
                        move |side| {
                            seed_loaded(side, hud, FACE_BASE, 75);
                            seed_players(side, &[3, 9, 9, -2, 14, 0]);
                            // SAFETY: shared cvar seed plus per-side client
                            // state. seed_players has to run first: it
                            // rewrites maxclients and gametype.
                            unsafe {
                                ctest_sbar_set_cvars(0.0, 1.0, 0.75, 1.0, teamplay);
                                ctest_sbar_set_client(
                                    side,
                                    12.0,
                                    11.95,
                                    0,
                                    0,
                                    12.3,
                                    FACE_BASE,
                                    1,
                                    maxclients,
                                    GAME_DEATHMATCH,
                                    c"e1m1".as_ptr(),
                                    c"level".as_ptr(),
                                );
                                ctest_sbar_set_stat(side, STAT_HEALTH, 75);
                                ctest_sbar_set_stat(side, STAT_ITEMS, FACE_BASE);
                            }
                        },
                        // SAFETY: as above.
                        move |side| unsafe { ctest_sbar_draw_face(side, 112, 0, classic) },
                    );
                }
            }
        }
    }

    // colors & 0xf0 == 0 is the only way Sbar_ColorForMap yields 8, so 0x00
    // and 0x0f take the alternate glyph row while 0x11 and 0xf0 do not. The
    // frag values cover a three-digit, a space-padded and a negative "%3i".
    for (frags, colors) in [
        (123, 0x00),
        (9, 0x00),
        (-4, 0x00),
        (9, 0x0f),
        (9, 0x11),
        (123, 0xf0),
    ] {
        for gametype in [GAME_COOP, GAME_DEATHMATCH] {
            assert_drawing(
                &format!("draw_face/ctf/draw/{frags}/{colors:#x}/{gametype}"),
                move |side| {
                    seed_loaded(side, 2, FACE_BASE, 75);
                    seed_players(side, &[3, 9, 9, -2, 14, 0]);
                    // SAFETY: shared cvar seed plus per-side client state.
                    unsafe {
                        ctest_sbar_set_cvars(0.0, 1.0, 0.75, 1.0, 4.0);
                        ctest_sbar_set_score(side, 0, c"player0".as_ptr(), frags, colors, 0.0, 20);
                        ctest_sbar_set_client(
                            side,
                            12.0,
                            11.95,
                            0,
                            0,
                            12.3,
                            FACE_BASE,
                            1,
                            6,
                            gametype,
                            c"e1m1".as_ptr(),
                            c"level".as_ptr(),
                        );
                        ctest_sbar_set_stat(side, STAT_HEALTH, 75);
                        ctest_sbar_set_stat(side, STAT_ITEMS, FACE_BASE);
                    }
                },
                // SAFETY: as above.
                move |side| unsafe { ctest_sbar_draw_face(side, 112, 0, true) },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// the scoreboards (sbar.c:485, :533, :1390, :1471)

#[test]
fn solo_scoreboard_matches() {
    for (secrets, total_secrets, monsters, total_monsters, skill, level) in [
        (0, 0, 0, 0, 0.0f32, "the slipgate complex"),
        (3, 7, 12, 40, 1.0, "e1m1"),
        (
            99,
            99,
            999,
            999,
            3.0,
            "a very long level name that scrolls right off",
        ),
        (1, 2, 3, 4, 2.4, "\u{80}bronze\u{80} name"),
        // skill 2.5 is the only value in this table where + 0.5 and + 0.4
        // truncate differently (sbar.c:508).
        (1, 2, 3, 4, 2.5, "e1m1"),
        // 34 characters, so "<name> (e1m1)" is exactly 41 and lands on the
        // far side of the len > 40 scroll gate (sbar.c:514); the 33-character
        // name is the 40 that stays on the near side.
        (1, 2, 3, 4, 1.0, "abcdefghijklmnopqrstuvwxyz01234567"),
        (1, 2, 3, 4, 1.0, "abcdefghijklmnopqrstuvwxyz0123456"),
    ] {
        assert_drawing(
            &format!("solo_scoreboard/{level}"),
            move |side| {
                let name = std::ffi::CString::new(level).unwrap();
                // SAFETY: shared cvar seed plus per-side client state.
                unsafe {
                    ctest_sbar_set_cvars(0.0, 1.0, 0.75, skill, 0.0);
                    ctest_sbar_set_client(
                        side,
                        12.0,
                        11.95,
                        0,
                        0,
                        0.0,
                        0,
                        1,
                        1,
                        GAME_COOP,
                        c"e1m1".as_ptr(),
                        name.as_ptr(),
                    );
                    ctest_sbar_set_stat(side, STAT_SECRETS, secrets);
                    ctest_sbar_set_stat(side, STAT_TOTALSECRETS, total_secrets);
                    ctest_sbar_set_stat(side, STAT_MONSTERS, monsters);
                    ctest_sbar_set_stat(side, STAT_TOTALMONSTERS, total_monsters);
                }
            },
            // SAFETY: as above.
            |side| unsafe { ctest_sbar_solo_scoreboard(side) },
        );
    }
}

#[test]
fn draw_scoreboard_matches_in_both_gametypes() {
    for gametype in [GAME_COOP, GAME_DEATHMATCH] {
        assert_drawing(
            &format!("draw_scoreboard/gametype={gametype}"),
            move |side| {
                seed_loaded(side, 0, LOADOUT, 75);
                seed_players(side, &[3, 9, 9, -2, 14, 0]);
                // SAFETY: per-side client seeder.
                unsafe {
                    ctest_sbar_set_client(
                        side,
                        12.0,
                        11.95,
                        0,
                        0,
                        0.0,
                        0,
                        1,
                        6,
                        gametype,
                        c"e1m1".as_ptr(),
                        c"the slipgate complex".as_ptr(),
                    )
                };
            },
            // SAFETY: as above.
            |side| unsafe { ctest_sbar_draw_scoreboard(side) },
        );
    }
}

#[test]
fn deathmatch_overlay_matches() {
    for players in [
        vec![3],
        vec![3, 9, 9, -2, 14, 0],
        (0..MAX_SCOREBOARD).map(|i| i * 3 - 7).collect::<Vec<_>>(),
    ] {
        let label = format!("deathmatch_overlay/{}", players.len());
        assert_drawing(
            &label,
            move |side| {
                seed_loaded(side, 0, LOADOUT, 75);
                seed_players(side, &players);
            },
            // SAFETY: as above.
            |side| unsafe { ctest_sbar_deathmatch_overlay(side) },
        );
    }
}

#[test]
fn mini_deathmatch_overlay_matches_across_sb_lines_and_viewentity() {
    for sb_lines in [0, 8, 24, 48, 200] {
        for viewentity in [1, 3, 16] {
            assert_drawing(
                &format!("mini_deathmatch_overlay/{sb_lines}/{viewentity}"),
                move |side| {
                    seed_loaded(side, 0, LOADOUT, 75);
                    seed_players(side, &[3, 9, 9, -2, 14, 0, 21, 7]);
                    // SAFETY: per-side seeders.
                    unsafe {
                        ctest_sbar_set_sb_lines(side, sb_lines);
                        ctest_sbar_set_client(
                            side,
                            12.0,
                            11.95,
                            0,
                            0,
                            0.0,
                            0,
                            viewentity,
                            8,
                            GAME_DEATHMATCH,
                            c"e1m1".as_ptr(),
                            c"level".as_ptr(),
                        );
                    }
                },
                // SAFETY: as above.
                |side| unsafe { ctest_sbar_mini_deathmatch_overlay(side) },
            );
        }
    }
}

/// `Sbar_MiniDeathmatchOverlay` returns before drawing when the scaled width
/// is under 512 in a classic style, or when `scr_viewsize >= 120`
/// (`sbar.c:1483`); past that gate `numlines` switches at `scr_viewsize >= 110`
/// (`sbar.c:1489`), which changes both the row count and where the clamped
/// start index lands.
#[test]
fn mini_deathmatch_overlay_matches_across_the_width_and_viewsize_gates() {
    for (glwidth, sbarscale, viewsize, style, draws) in [
        (512, 1.0f32, 100.0f32, 0.0f32, true),
        (511, 1.0, 100.0, 0.0, false),
        (513, 1.0, 100.0, 0.0, true),
        (511, 1.0, 100.0, 2.0, true),
        (640, 2.0, 100.0, 0.0, false),
        (640, 1.0, 109.0, 0.0, true),
        (640, 1.0, 110.0, 0.0, true),
        (640, 1.0, 110.0, 2.0, true),
        (640, 1.0, 119.0, 0.0, true),
        (640, 1.0, 120.0, 0.0, false),
    ] {
        for viewentity in [1, 4, 8] {
            let assert_it = if draws { assert_drawing } else { assert_quiet };
            assert_it(
                &format!("mini_deathmatch_overlay/gate/{glwidth}/{sbarscale}/{viewsize}/{style}/{viewentity}"),
                move |side| {
                    seed_loaded(side, 0, LOADOUT, 75);
                    seed_players(side, &[3, 9, 9, -2, 14, 0, 21, 7]);
                    // SAFETY: shared screen/cvar seed plus per-side client
                    // state; seed_players rewrites maxclients and gametype.
                    unsafe {
                        ctest_sbar_set_screen(glwidth, 480, glwidth, 480, 0.0, viewsize);
                        ctest_sbar_set_cvars(style, sbarscale, 0.75, 1.0, 0.0);
                        ctest_sbar_set_client(
                            side,
                            12.0,
                            11.95,
                            0,
                            0,
                            0.0,
                            0,
                            viewentity,
                            8,
                            GAME_DEATHMATCH,
                            c"e1m1".as_ptr(),
                            c"level".as_ptr(),
                        );
                    }
                },
                // SAFETY: as above.
                |side| unsafe { ctest_sbar_mini_deathmatch_overlay(side) },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// the intermission/finale text machinery (sbar.c:1309-1400, :1629)

#[test]
fn intermission_number_matches() {
    assert_drawing(
        "intermission_number",
        |side| seed_loaded(side, 0, LOADOUT, 75),
        // SAFETY: as above.
        |side| unsafe {
            // i32::MIN excluded: see itoa_matches_across_the_sign_and_width_range.
            for &num in &[0, 7, 42, 999, 1000, -1, -42] {
                for digits in [1, 3, 6] {
                    for color in [0, 1] {
                        ctest_sbar_intermission_number(side, 160, 56, num, digits, color);
                    }
                }
            }
        },
    );
}

#[test]
fn intermission_pic_for_char_matches_over_the_char_range() {
    for color in [0, 1] {
        assert_quiet(
            &format!("intermission_pic_for_char/color={color}"),
            |side| seed_loaded(side, 0, LOADOUT, 75),
            move |side| {
                (-128i32..128)
                    .map(|c| {
                        // SAFETY: returns an interned name owned by draw_ref.c.
                        unsafe {
                            CStr::from_ptr(ctest_sbar_intermission_pic_for_char(
                                side,
                                c as c_char,
                                color,
                            ))
                            .to_string_lossy()
                            .into_owned()
                        }
                    })
                    .collect::<Vec<_>>()
            },
        );
    }
}

#[test]
fn intermission_text_and_width_match() {
    for text in ["0", "12:34", "999/999", "-1", "kills: 12/40"] {
        for color in [0, 1] {
            let owned = std::ffi::CString::new(text).unwrap();
            let owned2 = owned.clone();
            assert_drawing(
                &format!("intermission_text({text:?}, {color})"),
                |side| seed_loaded(side, 0, LOADOUT, 75),
                move |side| {
                    // SAFETY: the width query draws nothing; the text draw does.
                    unsafe {
                        let w = ctest_sbar_intermission_text_width(side, owned2.as_ptr(), color);
                        ctest_sbar_intermission_text(side, 64, 56, owned2.as_ptr(), color);
                        w
                    }
                },
            );
        }
    }
}

/// `Sbar_IntermissionPicForChar` (`sbar.c:1342`) answers only for digits, `/`,
/// `:` and `-`; everything else returns NULL, which `Sbar_IntermissionText`
/// skips and `Sbar_IntermissionTextWidth` still charges 24 for. These inputs
/// draw nothing on either side, so the width is the whole observable.
#[test]
fn intermission_text_of_unmapped_characters_draws_nothing() {
    for text in ["", "abc", "\u{80}\u{81}\u{82}", " "] {
        for color in [0, 1] {
            let owned = std::ffi::CString::new(text).unwrap();
            assert_quiet(
                &format!("intermission_text/unmapped({text:?}, {color})"),
                |side| seed_loaded(side, 0, LOADOUT, 75),
                move |side| {
                    // SAFETY: neither call can draw for these inputs.
                    unsafe {
                        let w = ctest_sbar_intermission_text_width(side, owned.as_ptr(), color);
                        ctest_sbar_intermission_text(side, 64, 56, owned.as_ptr(), color);
                        w
                    }
                },
            );
        }
    }
}

#[test]
fn finale_overlay_matches() {
    assert_drawing(
        "finale_overlay",
        |side| seed_loaded(side, 0, LOADOUT, 75),
        // SAFETY: as above.
        |side| unsafe { ctest_sbar_finale_overlay(side) },
    );
}

// ---------------------------------------------------------------------------
// Sbar_Draw (sbar.c:1263) -- both HUD styles and the early-outs

#[test]
fn draw_matches_across_scr_style_and_the_early_outs() {
    for style in [0.0f32, 0.5, 1.0, 1.5, 2.0, 3.0] {
        for viewsize in [30.0f32, 100.0, 110.0, 120.0] {
            for intermission in [0, 1] {
                for showscores in [false, true] {
                    // cl.intermission is an early-out (sbar.c:1276): nothing
                    // is drawn, and "nothing on both sides" is the assertion.
                    let assert_it = if intermission != 0 {
                        assert_quiet
                    } else {
                        assert_drawing
                    };
                    assert_it(
                        &format!("draw/style={style}/viewsize={viewsize}/im={intermission}/ss={showscores}"),
                        move |side| {
                            seed_loaded(side, 0, LOADOUT, 75);
                            seed_players(side, &[3, 9, 9, -2, 14, 0]);
                            // SAFETY: shared + per-side seeders.
                            unsafe {
                                ctest_sbar_set_screen(640, 480, 640, 480, 0.0, viewsize);
                                ctest_sbar_set_cvars(style, 1.0, 0.75, 1.0, 0.0);
                                ctest_sbar_set_client(
                                    side, 12.0, 11.95, intermission, 90, 12.3, LOADOUT, 1, 6,
                                    GAME_DEATHMATCH, c"e1m1".as_ptr(), c"level".as_ptr(),
                                );
                                ctest_sbar_set_stat(side, STAT_ITEMS, LOADOUT);
                                ctest_sbar_set_sb_showscores(side, showscores);
                            }
                        },
                        // SAFETY: guarded dispatch; nothing here can raise.
                        |side| unsafe { ctest_sbar_draw(side) },
                    );
                }
            }
        }
    }
}

#[test]
fn draw_returns_early_when_the_console_is_full_screen() {
    let _g = lock();
    let (c, r) = both(
        |side| {
            seed_loaded(side, 0, LOADOUT, 75);
            // SAFETY: scr_con_current == vid.height is the early-out at
            // sbar.c:1266.
            unsafe { ctest_sbar_set_screen(640, 480, 640, 480, 480.0, 100.0) };
        },
        // SAFETY: as above.
        |side| unsafe { ctest_sbar_draw(side) },
    );
    assert_eq!(c.0, "", "the C oracle drew with a full-screen console");
    assert_eq!(c, r);
}

#[test]
fn draw_matches_across_sbarscale_and_sbaralpha_tileclear() {
    for scale in [0.5f32, 1.0, 2.0, 8.0] {
        for alpha in [0.0f32, 0.75, 1.0] {
            for sb_lines in [0, 24, 48] {
                for gametype in [GAME_COOP, GAME_DEATHMATCH] {
                    assert_drawing(
                        &format!("draw/tileclear/{scale}/{alpha}/{sb_lines}/{gametype}"),
                        move |side| {
                            seed_loaded(side, 0, LOADOUT, 75);
                            // SAFETY: shared + per-side seeders.
                            unsafe {
                                ctest_sbar_set_cvars(0.0, scale, alpha, 1.0, 0.0);
                                ctest_sbar_set_sb_lines(side, sb_lines);
                                ctest_sbar_set_client(
                                    side,
                                    12.0,
                                    11.95,
                                    0,
                                    0,
                                    12.3,
                                    LOADOUT,
                                    1,
                                    1,
                                    gametype,
                                    c"e1m1".as_ptr(),
                                    c"level".as_ptr(),
                                );
                                ctest_sbar_set_stat(side, STAT_ITEMS, LOADOUT);
                            }
                        },
                        // SAFETY: guarded dispatch, then an immediate readback
                        // -- nothing in sbar.c writes sb_lines, and neither
                        // half may start.
                        |side| unsafe { (ctest_sbar_draw(side), ctest_sbar_get_sb_lines(side)) },
                    );
                }
            }
        }
    }
}

/// One row per branch of `Sbar_DrawClassic` (`sbar.c:891`) and
/// `Sbar_DrawModern` (`sbar.c:996`) that the flat `LOADOUT` fixture cannot
/// reach: the armour tiers, the ammo icons, the powerup row, the per-hudtype
/// key and weapon slots, and the two CTF faces.
#[derive(Clone, Copy)]
struct DrawCase {
    what: &'static str,
    hud: c_int,
    style: c_float,
    viewsize: c_float,
    items: c_int,
    health: c_int,
    armor: c_int,
    ammo: c_int,
    weapon: c_int,
    teamplay: c_float,
    maxclients: c_int,
    gametype: c_int,
}

const DRAW_CASE: DrawCase = DrawCase {
    what: "",
    hud: 0,
    style: 0.0,
    viewsize: 100.0,
    items: DRAW_BASE,
    health: 75,
    armor: 87,
    ammo: 25,
    weapon: 6,
    teamplay: 0.0,
    maxclients: 1,
    gametype: GAME_COOP,
};

#[test]
fn draw_matches_across_the_hud_variants() {
    for case in [
        DrawCase {
            what: "classic/invuln",
            items: DRAW_BASE | IT_INVULNERABILITY | IT_INVISIBILITY,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/armor1",
            items: DRAW_BASE | IT_ARMOR1,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/armor2",
            items: DRAW_BASE | IT_ARMOR2,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/armor3",
            items: DRAW_BASE | IT_ARMOR3,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/low",
            health: 26,
            ammo: 11,
            ..DRAW_CASE
        },
        // ammo + 1 == 11 is the one STAT_AMMO that separates `<= 10` from
        // `<= 11` in the low-ammo colour flag (sbar.c:984, :1091).
        DrawCase {
            what: "classic/ammo11",
            ammo: 10,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/nails-no-shells",
            items: DRAW_BASE | IT_NAILS,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/shells",
            items: DRAW_BASE | IT_SHELLS,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/hipnotic/keys",
            hud: 1,
            items: DRAW_BASE | IT_KEY1 | IT_KEY2,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/rogue/armor",
            hud: 2,
            items: DRAW_BASE | RIT_ARMOR1 | RIT_ARMOR2 | RIT_ARMOR3,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/rogue/lavanails",
            hud: 2,
            items: DRAW_BASE | RIT_LAVA_NAILS | RIT_MULTI_ROCKETS,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/rogue/plasma",
            hud: 2,
            items: DRAW_BASE | RIT_PLASMA_AMMO,
            ..DRAW_CASE
        },
        DrawCase {
            what: "classic/rogue/ctf",
            hud: 2,
            teamplay: 4.0,
            maxclients: 6,
            gametype: GAME_DEATHMATCH,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/plain",
            style: 2.0,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/rogue/ctf",
            style: 2.0,
            hud: 2,
            teamplay: 4.0,
            maxclients: 6,
            gametype: GAME_DEATHMATCH,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/armor0",
            style: 2.0,
            armor: 0,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/armor1",
            style: 2.0,
            items: DRAW_BASE | IT_ARMOR1,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/armor3",
            style: 2.0,
            items: DRAW_BASE | IT_ARMOR3,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/powerups",
            style: 2.0,
            items: DRAW_BASE | IT_QUAD | IT_SUIT | IT_INVISIBILITY | IT_INVULNERABILITY,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/key2",
            style: 2.0,
            items: DRAW_BASE | IT_KEY2,
            ..DRAW_CASE
        },
        // both keys and no hipnotic set: the only shape in which the
        // `offset += 16` of the first key is read by the second (sbar.c:1152).
        DrawCase {
            what: "modern/keys-both",
            style: 2.0,
            items: DRAW_BASE | IT_KEY1 | IT_KEY2,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/ammo11",
            style: 2.0,
            ammo: 10,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/hipnotic/keys",
            hud: 1,
            style: 2.0,
            items: DRAW_BASE | IT_KEY1 | IT_KEY2,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/ammo-icon",
            style: 2.0,
            items: DRAW_BASE | IT_SHELLS | IT_NAILS,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/rogue/lavanailgun",
            hud: 2,
            style: 2.0,
            items: DRAW_BASE | RIT_LAVA_NAILGUN,
            weapon: RIT_LAVA_NAILGUN,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/rogue/shield",
            hud: 2,
            style: 2.0,
            items: DRAW_BASE | RIT_SHIELD | RIT_PLASMA_AMMO,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/weapon-nailgun",
            style: 2.0,
            weapon: IT_NAILGUN,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/weapon-lightning",
            style: 2.0,
            weapon: IT_LIGHTNING,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/hipnotic/prox",
            hud: 1,
            style: 2.0,
            items: DRAW_BASE | HIT_PROXIMITY_GUN,
            weapon: HIT_PROXIMITY_GUN,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/hipnotic/mjolnir",
            hud: 1,
            style: 2.0,
            items: DRAW_BASE | HIT_MJOLNIR | HIT_LASER_CANNON | HIT_WETSUIT,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/health1",
            style: 2.0,
            health: 1,
            ..DRAW_CASE
        },
        DrawCase {
            what: "modern/viewsize110",
            style: 2.0,
            viewsize: 110.0,
            ..DRAW_CASE
        },
    ] {
        assert_drawing(
            &format!("draw/variant/{}", case.what),
            move |side| {
                seed_players(side, &[3, 9, 9, -2, 14, 0]);
                // SAFETY: shared wad/screen/cvar seed, then this side's pic
                // table and client state. seed_players runs first because it
                // rewrites maxclients, viewentity and gametype.
                unsafe {
                    ctest_sbar_seed_hud(case.hud);
                    ctest_sbar_load_pics(side);
                    ctest_sbar_set_screen(640, 480, 640, 480, 0.0, case.viewsize);
                    ctest_sbar_set_cvars(case.style, 1.0, 0.75, 1.0, case.teamplay);
                    ctest_sbar_set_client(
                        side,
                        12.0,
                        11.95,
                        0,
                        0,
                        12.3,
                        case.items,
                        1,
                        case.maxclients,
                        case.gametype,
                        c"e1m1".as_ptr(),
                        c"level".as_ptr(),
                    );
                    ctest_sbar_set_stat(side, STAT_HEALTH, case.health);
                    ctest_sbar_set_stat(side, STAT_ARMOR, case.armor);
                    ctest_sbar_set_stat(side, STAT_AMMO, case.ammo + 1);
                    ctest_sbar_set_stat(side, STAT_SHELLS, case.ammo);
                    ctest_sbar_set_stat(side, STAT_NAILS, case.ammo + 3);
                    ctest_sbar_set_stat(side, STAT_ROCKETS, case.ammo + 7);
                    ctest_sbar_set_stat(side, STAT_CELLS, case.ammo + 11);
                    ctest_sbar_set_stat(side, STAT_ACTIVEWEAPON, case.weapon);
                    ctest_sbar_set_stat(side, STAT_ITEMS, case.items);
                    for i in 0..32 {
                        ctest_sbar_set_item_gettime(side, i, 11.9);
                    }
                    ctest_sbar_set_sb_lines(side, 24);
                }
            },
            // SAFETY: guarded dispatch; no CSQC is installed, so nothing here
            // can raise.
            |side| unsafe { ctest_sbar_draw(side) },
        );
    }
}

// ---------------------------------------------------------------------------
// Sbar_IntermissionOverlay (sbar.c:1559)

#[test]
fn intermission_overlay_matches_in_both_gametypes() {
    for gametype in [GAME_COOP, GAME_DEATHMATCH] {
        // 240 is the only completed_time here whose minutes and seconds both
        // change under an off-by-one divisor (sbar.c:1602). The last two rows
        // make the secret and monster widths differ from each other and from
        // the time width, and push the total past the q_min(320) clamp
        // (sbar.c:1613-1615).
        for (secrets, monsters, completed) in [
            (0, 0, 0),
            (3, 12, 90),
            (99, 999, 3599),
            (0, 0, 240),
            (12345, 1234567890, 3600),
            (1234567890, 12345, 3600),
        ] {
            assert_drawing(
                &format!("intermission_overlay/{gametype}/{completed}"),
                move |side| {
                    seed_loaded(side, 0, LOADOUT, 75);
                    seed_players(side, &[3, 9, 9, -2, 14, 0]);
                    // SAFETY: per-side seeders.
                    unsafe {
                        ctest_sbar_set_client(
                            side,
                            12.0,
                            11.95,
                            1,
                            completed,
                            0.0,
                            LOADOUT,
                            1,
                            6,
                            gametype,
                            c"e1m1".as_ptr(),
                            c"level".as_ptr(),
                        );
                        ctest_sbar_set_stat(side, STAT_SECRETS, secrets);
                        ctest_sbar_set_stat(side, STAT_TOTALSECRETS, secrets + 4);
                        ctest_sbar_set_stat(side, STAT_MONSTERS, monsters);
                        ctest_sbar_set_stat(side, STAT_TOTALMONSTERS, monsters + 11);
                    }
                },
                // SAFETY: guarded dispatch.
                |side| unsafe { ctest_sbar_intermission_overlay(side) },
            );
        }
    }
}

/// `Sbar_IntermissionOverlay` clamps the layout width with `q_min (320, total)`
/// (`sbar.c:1615`). `total` only lands on 321 for one secret count under the
/// fixture geometry: `stubs/draw_ref.c` derives every pic width from the pic
/// name by a fixed hash, so the digit widths are uneven rather than a flat 24.
/// Sweeping the secret count walks `total` across 320, exercising the clamp
/// from both sides instead of only from far above it.
#[test]
fn intermission_overlay_total_width_crosses_the_clamp() {
    for secrets in 11150..11200 {
        assert_drawing(
            &format!("intermission_overlay/clamp/{secrets}"),
            move |side| {
                seed_loaded(side, 0, LOADOUT, 75);
                // SAFETY: per-side seeders.
                unsafe {
                    ctest_sbar_set_client(
                        side,
                        12.0,
                        11.95,
                        1,
                        90,
                        0.0,
                        LOADOUT,
                        1,
                        1,
                        GAME_COOP,
                        c"e1m1".as_ptr(),
                        c"level".as_ptr(),
                    );
                    ctest_sbar_set_stat(side, STAT_SECRETS, secrets);
                    ctest_sbar_set_stat(side, STAT_TOTALSECRETS, 123);
                    ctest_sbar_set_stat(side, STAT_MONSTERS, 0);
                    ctest_sbar_set_stat(side, STAT_TOTALMONSTERS, 0);
                }
            },
            // SAFETY: guarded dispatch.
            |side| unsafe { ctest_sbar_intermission_overlay(side) },
        );
    }
}

// ---------------------------------------------------------------------------
// the CSQC paths (sbar.c:75, :841, :1569)

/// The per-side CSQC readback: the extglobals slots sbar.c writes, all three
/// lanes of the vector argument, the scalar argument, the return slot,
/// `STAT_ITEMS` (which `Sbar_DrawCSCQ` zeroes and restores around the call,
/// `sbar.c:846`/`:876`) and whether the ambient qcvm was left switched.
#[derive(Debug, PartialEq)]
struct Qc {
    raised: c_int,
    builtins: c_int,
    qcvm_active: bool,
    stat_items: c_int,
    globals: Vec<c_int>,
}

fn qc_probe(side: c_int, raised: c_int) -> Qc {
    // SAFETY: reads the side's globals immediately after its invocation, then
    // clears the ambient qcvm the raise path deliberately left switched.
    unsafe {
        let q = Qc {
            raised,
            builtins: ctest_sbar_builtin_calls(),
            qcvm_active: ctest_sbar_qcvm_active(),
            stat_items: ctest_sbar_get_stat(side, STAT_ITEMS),
            globals: [
                OFS_RETURN,
                OFS_RETURN + 1,
                OFS_PARM0,
                OFS_PARM0 + 1,
                OFS_PARM0 + 2,
                OFS_PARM1,
                G_CLTIME,
                G_CLFRAME,
                G_INTERM,
                G_INTERMTIME,
                G_LOCALENT,
                G_PR_TIME,
                G_PR_FRAMETIME,
            ]
            .iter()
            .map(|&ofs| ctest_sbar_get_qc_global_int(side, ofs))
            .collect(),
        };
        if q.qcvm_active {
            ctest_sbar_clear_qcvm();
        }
        q
    }
}

/// The raise path's probe. `raised` and the still-switched ambient qcvm are
/// both asserted here rather than only compared, so neither side can pass by
/// quietly not raising at all (`sbar.c:84`, `:874`, `:1591`).
fn qc_probe_raised(side: c_int, raised: c_int) -> Qc {
    assert_eq!(raised, 1, "the raising QC entry did not raise");
    let q = qc_probe(side, raised);
    assert!(
        q.qcvm_active,
        "the raise left the ambient qcvm cleared; sbar.c leaves it switched"
    );
    q
}

#[test]
fn csqc_command_matches_with_and_without_a_handler() {
    for mode in [CSQC_NONE, CSQC_MARKER] {
        for style in [0.0f32, 0.5, 1.0, 2.0] {
            let assert_it = if mode == CSQC_MARKER && style < 1.0 {
                assert_drawing
            } else {
                assert_quiet
            };
            assert_it(
                &format!("csqc_command/mode={mode}/style={style}"),
                move |side| {
                    // SAFETY: shared cvar seed plus this side's VM.
                    unsafe {
                        ctest_sbar_set_cvars(style, 1.0, 0.75, 1.0, 0.0);
                        ctest_sbar_install_csqc(side, mode, CSQC_NONE, CSQC_NONE);
                        ctest_sbar_set_qc_global_int(side, OFS_RETURN, 0);
                        ctest_sbar_tokenize(side, c"+showscores extra".as_ptr());
                    }
                },
                |side| {
                    let mut ret: c_int = -1;
                    // SAFETY: guarded dispatch, then an immediate probe.
                    let raised = unsafe { ctest_sbar_csqc_command(side, &mut ret) };
                    (ret, qc_probe(side, raised))
                },
            );
        }
    }
}

/// `Sbar_CSQCCommand` reaches `PR_ExecuteProgram` (`sbar.c:82`). When the QC
/// raises, C never reaches its `PR_SwitchQCVM (NULL)` at `sbar.c:84` and leaves
/// the ambient qcvm switched -- the port has to leave exactly the same state.
#[test]
fn csqc_command_raise_matches_and_leaves_the_qcvm_switched() {
    assert_drawing(
        "csqc_command/raises",
        |side| {
            // SAFETY: installs the raising QC entry for this side.
            unsafe {
                ctest_sbar_install_csqc(side, CSQC_RAISES, CSQC_NONE, CSQC_NONE);
                ctest_sbar_tokenize(side, c"+showscores".as_ptr());
            }
        },
        |side| {
            let mut ret: c_int = -1;
            // SAFETY: guarded dispatch, then an immediate probe.
            let raised = unsafe { ctest_sbar_csqc_command(side, &mut ret) };
            (ret, qc_probe_raised(side, raised))
        },
    );
}

#[test]
fn show_scores_and_dont_show_scores_match() {
    for mode in [CSQC_NONE, CSQC_MARKER] {
        for initial in [false, true] {
            let assert_it = if mode == CSQC_MARKER {
                assert_drawing
            } else {
                assert_quiet
            };
            assert_it(
                &format!("show_scores/mode={mode}/initial={initial}"),
                move |side| {
                    // SAFETY: per-side seeders.
                    unsafe {
                        ctest_sbar_install_csqc(side, mode, CSQC_NONE, CSQC_NONE);
                        ctest_sbar_set_sb_showscores(side, initial);
                        ctest_sbar_tokenize(side, c"+showscores".as_ptr());
                    }
                },
                |side| {
                    // SAFETY: guarded dispatches over this side's storage, each
                    // read back before the other side runs.
                    unsafe {
                        let raised = ctest_sbar_show_scores(side);
                        let after_show = ctest_sbar_get_sb_showscores(side);
                        let raised2 = ctest_sbar_dont_show_scores(side);
                        let after_hide = ctest_sbar_get_sb_showscores(side);
                        ((after_show, after_hide, raised2), qc_probe(side, raised))
                    }
                },
            );
        }
    }
}

#[test]
fn show_scores_raise_matches() {
    assert_drawing(
        "show_scores/raises",
        |side| {
            // SAFETY: per-side seeders.
            unsafe {
                ctest_sbar_install_csqc(side, CSQC_RAISES, CSQC_NONE, CSQC_NONE);
                ctest_sbar_set_sb_showscores(side, false);
                ctest_sbar_tokenize(side, c"+showscores".as_ptr());
            }
        },
        |side| {
            // SAFETY: guarded dispatch, then an immediate readback.
            unsafe {
                let raised = ctest_sbar_show_scores(side);
                let after = ctest_sbar_get_sb_showscores(side);
                (after, qc_probe_raised(side, raised))
            }
        },
    );
}

#[test]
fn dont_show_scores_raise_matches() {
    assert_drawing(
        "dont_show_scores/raises",
        |side| {
            // SAFETY: per-side seeders.
            unsafe {
                ctest_sbar_install_csqc(side, CSQC_RAISES, CSQC_NONE, CSQC_NONE);
                ctest_sbar_set_sb_showscores(side, true);
                ctest_sbar_tokenize(side, c"-showscores".as_ptr());
            }
        },
        |side| {
            // SAFETY: guarded dispatch, then an immediate readback.
            unsafe {
                let raised = ctest_sbar_dont_show_scores(side);
                let after = ctest_sbar_get_sb_showscores(side);
                (after, qc_probe_raised(side, raised))
            }
        },
    );
}

/// `Sbar_Draw` dispatches to `Sbar_DrawCSCQ` when `scr_style < 1`, a
/// `CSQC_DrawHud` exists and no qcvm is already ambient (`sbar.c:1270`).
#[test]
fn draw_csqc_matches_across_the_hud_and_scores_handlers() {
    for drawscores in [CSQC_NONE, CSQC_MARKER] {
        for keydest in [KEY_GAME, KEY_MENU] {
            // scr_style gates the whole dispatch at sbar.c:1270: 1.0 and 1.5
            // must both fall through to the classic bar instead.
            for (showscores, health, gametype, style) in [
                (false, 75, GAME_COOP, 0.0f32),
                (true, 75, GAME_DEATHMATCH, 0.0),
                (false, -5, GAME_DEATHMATCH, 0.0),
                (false, 75, GAME_DEATHMATCH, 1.0),
                (false, 75, GAME_DEATHMATCH, 1.5),
            ] {
                assert_drawing(
                    &format!(
                        "draw_cscq/{drawscores}/{keydest}/{showscores}/{health}/{gametype}/{style}"
                    ),
                    move |side| {
                        seed_loaded(side, 0, LOADOUT, health);
                        seed_players(side, &[3, 9, 9, -2, 14, 0]);
                        // SAFETY: shared + per-side seeders.
                        unsafe {
                            ctest_sbar_set_cvars(style, 1.0, 0.75, 1.0, 0.0);
                            ctest_sbar_set_key_dest(keydest);
                            ctest_sbar_set_client(
                                side,
                                12.0,
                                11.95,
                                0,
                                0,
                                12.3,
                                LOADOUT,
                                1,
                                6,
                                gametype,
                                c"e1m1".as_ptr(),
                                c"level".as_ptr(),
                            );
                            ctest_sbar_set_stat(side, STAT_ITEMS, LOADOUT);
                            ctest_sbar_set_stat(side, STAT_HEALTH, health);
                            ctest_sbar_set_sb_showscores(side, showscores);
                            ctest_sbar_install_csqc(side, CSQC_NONE, CSQC_MARKER, drawscores);
                        }
                    },
                    |side| {
                        // SAFETY: guarded dispatch, then an immediate probe.
                        let raised = unsafe { ctest_sbar_draw(side) };
                        qc_probe(side, raised)
                    },
                );
            }
        }
    }
}

/// `cl.time < cl.oldtime` makes `Sbar_DrawCSCQ` zero `STAT_ITEMS` for the
/// duration of the QC call and restore it afterwards (`sbar.c:846`, `:876`).
#[test]
fn draw_csqc_restores_stat_items_after_a_time_warp() {
    assert_drawing(
        "draw_cscq/timewarp",
        |side| {
            seed_loaded(side, 0, LOADOUT, 75);
            // SAFETY: per-side seeder; oldtime > time.
            unsafe {
                ctest_sbar_set_client(
                    side,
                    5.0,
                    12.0,
                    0,
                    0,
                    12.3,
                    LOADOUT,
                    1,
                    1,
                    GAME_COOP,
                    c"e1m1".as_ptr(),
                    c"level".as_ptr(),
                );
                ctest_sbar_set_stat(side, STAT_ITEMS, LOADOUT);
                ctest_sbar_install_csqc(side, CSQC_NONE, CSQC_MARKER, CSQC_NONE);
            }
        },
        |side| {
            // SAFETY: guarded dispatch, then an immediate probe.
            let raised = unsafe { ctest_sbar_draw(side) };
            qc_probe(side, raised)
        },
    );
}

#[test]
fn draw_csqc_raise_matches_and_leaves_the_qcvm_switched() {
    for (hud, scores) in [(CSQC_RAISES, CSQC_NONE), (CSQC_MARKER, CSQC_RAISES)] {
        assert_drawing(
            &format!("draw_cscq/raises/{hud}/{scores}"),
            move |side| {
                seed_loaded(side, 0, LOADOUT, 75);
                seed_players(side, &[3, 9]);
                // SAFETY: per-side seeder.
                unsafe { ctest_sbar_install_csqc(side, CSQC_NONE, hud, scores) };
            },
            |side| {
                // SAFETY: guarded dispatch, then an immediate probe.
                let raised = unsafe { ctest_sbar_draw(side) };
                qc_probe_raised(side, raised)
            },
        );
    }
}

/// `Sbar_IntermissionOverlay`'s own CSQC branch (`sbar.c:1569`), including the
/// raise that skips `PR_SwitchQCVM (NULL)` at `sbar.c:1591`.
#[test]
fn intermission_overlay_csqc_matches() {
    // style 1.0 and 1.5 are on the far side of the sbar.c:1569 gate, so they
    // must reach Sbar_DeathmatchOverlay instead of the QC entry.
    for (mode, style) in [
        (CSQC_MARKER, 0.0f32),
        (CSQC_RAISES, 0.0),
        (CSQC_MARKER, 1.0),
        (CSQC_MARKER, 1.5),
    ] {
        assert_drawing(
            &format!("intermission_overlay/csqc/{mode}/{style}"),
            move |side| {
                seed_loaded(side, 0, LOADOUT, 75);
                seed_players(side, &[3, 9, 9, -2]);
                // SAFETY: per-side seeders.
                unsafe {
                    ctest_sbar_set_cvars(style, 1.0, 0.75, 1.0, 0.0);
                    ctest_sbar_set_client(
                        side,
                        12.0,
                        11.95,
                        1,
                        90,
                        0.0,
                        LOADOUT,
                        1,
                        4,
                        GAME_DEATHMATCH,
                        c"e1m1".as_ptr(),
                        c"level".as_ptr(),
                    );
                    ctest_sbar_install_csqc(side, CSQC_NONE, CSQC_NONE, mode);
                }
            },
            |side| {
                // SAFETY: guarded dispatch, then an immediate probe.
                let raised = unsafe { ctest_sbar_intermission_overlay(side) };
                qc_probe(side, raised)
            },
        );
    }
}
