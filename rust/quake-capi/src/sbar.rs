//! `Quake/sbar.c` -- the status bar, scoreboards and intermission overlays.
//!
//! Rust migration Phase 7 M10d, Pattern A (whole-file swap): `Quake/sbar.c`
//! is replaced by this module plus `Quake/sbar_glue.c` under
//! `-Duse_rust_host`. Every plain `Sbar_*` name stays in the glue; this
//! module exports only `quake_rs_sbar_*` cores.
//!
//! ## ADR-009 raise-topology audit
//!
//! `sbar.c` reaches exactly one longjmp-capable callee, `PR_ExecuteProgram`,
//! at three sites: `sbar.c:82` (`Sbar_CSQCCommand`), `:864`/`:870`
//! (`Sbar_DrawCSCQ`) and `:1590` (`Sbar_IntermissionOverlay`). All three run
//! through the existing `Host_Glue_PR_ExecuteProgram` trampoline
//! (`host_glue.c:532`), so no new `Host_Guard` is needed. Five entry points
//! can therefore return a pending jump and are re-raised from the glue:
//! `Sbar_CSQCCommand`, `Sbar_ShowScores`, `Sbar_Glue_DontShowScores`,
//! `Sbar_Draw` and `Sbar_IntermissionOverlay`. Everything else is a plain
//! forward.
//!
//! `Sys_Error` (reachable through `Draw_CachePic`) ends in `exit (1)`, not a
//! longjmp, so it needs no guard.
//!
//! Accepted, pre-existing exposure: `Draw_PicFromWad2` warns through
//! `Con_Warning`/`Con_DPrintf`, and `Con_Printf`'s screen-update tail can
//! reach `Mod_LoadModel` -> `Host_Error` (`gl_model.c:531`). That is the
//! standing project exposure recorded for every client-stratum port; no
//! guard is added for it here.
//!
//! ## Ownership (ADR-007)
//!
//! The four objects `sbar.c` gave external linkage -- `sb_showscores`,
//! `sb_lines`, `fragsort[]` and `scoreboardlines` -- have live C readers
//! (`gl_screen.c:397-410`, `:682`, `pr_ext.c:5344-5347`), so their storage
//! stays in `Quake/sbar_glue.c` and is reached through
//! [`quake_c_sys::sbar`]. Every file-static of `sbar.c` -- the ~150 pic
//! handles, `hipweapons[]` and `hudtype` -- had no reader outside the file
//! and moves here.
//!
//! ## ADR-008
//!
//! The `PR_SwitchQCVM (&cl.qcvm)` / `PR_SwitchQCVM (NULL)` pairs at
//! `sbar.c:79-84`, `:851-874` and `:1573-1591` are ordering-bearing:
//! `pr_global_struct` and `qcvm->extglobals` are re-read after each switch,
//! never cached across one.

use core::ffi::{c_char, c_float, c_int, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::console as gc;
use quake_c_sys::host as gh;
use quake_c_sys::sbar as g;
use quake_c_sys::sv_main as gs;
use quake_types::host::ScoreBoard;
use quake_types::progs::{GlobalVars, QcVm, OFS_PARM0, OFS_PARM1, OFS_RETURN};

use crate::cl_main::cl;

/// A `Host_Guard` status: 0 means the guarded call returned normally, any
/// other value is a pending longjmp that must reach a C frame untouched.
type Raise = c_int;

macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

// ---------------------------------------------------------------------------
// Constants transcribed from the engine headers.

/// `sbar.c:26` -- `#define STAT_MINUS 10`, the num frame for a '-' digit.
const STAT_MINUS: c_int = 10;

/// `quakedef.h:112-127` -- the `stat_t` members `sbar.c` reads.
const STAT_HEALTH: usize = 0;
const STAT_AMMO: usize = 3;
const STAT_ARMOR: usize = 4;
const STAT_SHELLS: usize = 6;
const STAT_ACTIVEWEAPON: usize = 10;
const STAT_TOTALSECRETS: usize = 11;
const STAT_TOTALMONSTERS: usize = 12;
const STAT_SECRETS: usize = 13;
const STAT_MONSTERS: usize = 14;
const STAT_ITEMS: usize = 15;

/// `quakedef.h:140-167` -- the `items_t` members `sbar.c` reads.
const IT_SHOTGUN: c_int = 1;
const IT_GRENADE_LAUNCHER: c_int = 16;
const IT_SHELLS: c_int = 256;
const IT_NAILS: c_int = 512;
const IT_ROCKETS: c_int = 1024;
const IT_CELLS: c_int = 2048;
const IT_ARMOR1: c_int = 8192;
const IT_ARMOR2: c_int = 16384;
const IT_ARMOR3: c_int = 32768;
const IT_KEY1: c_int = 131072;
const IT_KEY2: c_int = 262144;
const IT_INVISIBILITY: c_int = 524288;
const IT_INVULNERABILITY: c_int = 1048576;
const IT_QUAD: c_int = 4194304;

/// `quakedef.h:171-195` -- the `rogueitems_t` members `sbar.c` reads.
const RIT_SHELLS: c_int = 128;
const RIT_NAILS: c_int = 256;
const RIT_ROCKETS: c_int = 512;
const RIT_CELLS: c_int = 1024;
const RIT_LAVA_NAILGUN: c_int = 4096;
const RIT_ARMOR1: c_int = 8388608;
const RIT_ARMOR2: c_int = 16777216;
const RIT_ARMOR3: c_int = 33554432;
const RIT_LAVA_NAILS: c_int = 67108864;
const RIT_PLASMA_AMMO: c_int = 134217728;
const RIT_MULTI_ROCKETS: c_int = 268435456;

/// `quakedef.h:198-207` -- the `hipnoticitems_t` members `sbar.c` reads.
const HIT_PROXIMITY_GUN_BIT: c_int = 16;
const HIT_MJOLNIR_BIT: c_int = 7;
const HIT_LASER_CANNON_BIT: c_int = 23;
const HIT_PROXIMITY_GUN: c_int = 1 << HIT_PROXIMITY_GUN_BIT;

/// `protocol.h:239-240` -- `GAME_DEATHMATCH`.
const GAME_DEATHMATCH: c_int = 1;

/// `quakedef.h:237-249` -- the `canvastype` members `sbar.c` selects.
const CANVAS_DEFAULT: c_int = 1;
const CANVAS_MENU: c_int = 3;
const CANVAS_SBAR: c_int = 4;
const CANVAS_BOTTOMLEFT: c_int = 7;
const CANVAS_TOPLEFT: c_int = 8;
const CANVAS_BOTTOMRIGHT: c_int = 9;
const CANVAS_TOPRIGHT: c_int = 10;
const CANVAS_CSQC: c_int = 11;

/// `keys.h:141` -- `key_menu`, the third `keydest_t`.
const KEY_MENU: c_int = 3;

// ---------------------------------------------------------------------------
// sbar.c:28-66 -- the file statics. None had external linkage and nothing
// outside sbar.c ever named them, so they move to Rust (ADR-007).
//
// Every pic handle is a `qpic_t *`, kept as `*mut c_void` for the same reason
// the renderer prototypes are: `qpic_t` has no cbindgen spelling, and sbar.c
// only ever dereferences `->width` (through `pic_width` below).

static mut SB_NUMS: [[*mut c_void; 11]; 2] = [[ptr::null_mut(); 11]; 2];
static mut SB_COLON: *mut c_void = ptr::null_mut();
static mut SB_SLASH: *mut c_void = ptr::null_mut();
static mut SB_IBAR: *mut c_void = ptr::null_mut();
static mut SB_SBAR: *mut c_void = ptr::null_mut();
static mut SB_SCOREBAR: *mut c_void = ptr::null_mut();

/// `sbar.c:34` -- 0 is active, 1 is owned, 2-5 are flashes.
static mut SB_WEAPONS: [[*mut c_void; 8]; 7] = [[ptr::null_mut(); 8]; 7];
static mut SB_AMMO: [*mut c_void; 4] = [ptr::null_mut(); 4];
static mut SB_SIGIL: [*mut c_void; 4] = [ptr::null_mut(); 4];
static mut SB_ARMOR: [*mut c_void; 3] = [ptr::null_mut(); 3];
static mut SB_ITEMS: [*mut c_void; 32] = [ptr::null_mut(); 32];

/// `sbar.c:40` -- 0 is gibbed, 1 is dead, 2-6 are alive; the second index is
/// 0 for the static face and 1 for the temporary animation.
static mut SB_FACES: [[*mut c_void; 2]; 7] = [[ptr::null_mut(); 2]; 7];
static mut SB_FACE_INVIS: *mut c_void = ptr::null_mut();
static mut SB_FACE_QUAD: *mut c_void = ptr::null_mut();
static mut SB_FACE_INVULN: *mut c_void = ptr::null_mut();
static mut SB_FACE_INVIS_INVULN: *mut c_void = ptr::null_mut();

static mut RSB_INVBAR: [*mut c_void; 2] = [ptr::null_mut(); 2];
static mut RSB_WEAPONS: [*mut c_void; 5] = [ptr::null_mut(); 5];
static mut RSB_ITEMS: [*mut c_void; 2] = [ptr::null_mut(); 2];
static mut RSB_AMMO: [*mut c_void; 3] = [ptr::null_mut(); 3];
/// `sbar.c:56` -- PGM 01/19/97, the team color border.
static mut RSB_TEAMBORD: *mut c_void = ptr::null_mut();

/// `sbar.c:59` -- MED 01/04/97 added two more weapons + 3 alternates for the
/// grenade launcher.
static mut HSB_WEAPONS: [[*mut c_void; 5]; 7] = [[ptr::null_mut(); 5]; 7];
/// `sbar.c:61` -- MED 01/04/97 added an array to simplify weapon parsing.
const HIPWEAPONS: [c_int; 4] = [
    HIT_LASER_CANNON_BIT,
    HIT_MJOLNIR_BIT,
    4,
    HIT_PROXIMITY_GUN_BIT,
];
static mut HSB_ITEMS: [*mut c_void; 2] = [ptr::null_mut(); 2];

/// `sbar.c:65` -- spike: fix `-game hipnotic` by autodetecting hud types.
static mut HUDTYPE: c_int = 0;

/// `sbar.c:66` -- `#define hipnotic (hudtype == 1)`.
#[inline]
unsafe fn hipnotic() -> bool {
    // SAFETY: single-threaded engine state.
    unsafe { HUDTYPE == 1 }
}

/// `sbar.c:67` -- `#define rogue (hudtype == 2)`.
#[inline]
unsafe fn rogue() -> bool {
    // SAFETY: single-threaded engine state.
    unsafe { HUDTYPE == 2 }
}

// ---------------------------------------------------------------------------
// Small helpers standing in for engine macros and libc.

/// `q_minmax.h:64` -- `CLAMP` on `double`. `_Generic` picks `clamp_double`
/// whenever any operand is a `double` literal, so the whole expression is
/// evaluated at double precision before it lands in a `float`.
#[inline]
fn clamp_f64(minval: f64, val: f64, maxval: f64) -> f64 {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// `q_minmax.h:64` -- `CLAMP` on `float`.
#[inline]
fn clamp_f32(minval: f32, val: f32, maxval: f32) -> f32 {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// The ambient qcvm (ADR-008).
#[inline]
unsafe fn vm() -> *mut QcVm {
    // SAFETY: single-threaded engine state.
    unsafe { ptr::addr_of_mut!(c::qcvm).read().cast::<QcVm>() }
}

/// `pr_global_struct`, as the mirror. Re-read after every `PR_SwitchQCVM`.
#[inline]
unsafe fn pgs() -> *mut GlobalVars {
    // SAFETY: the same storage as `qcvm->globals`; the VM is selected by the
    // caller, exactly as in C.
    unsafe {
        ptr::addr_of_mut!(gs::pr_global_struct)
            .read()
            .cast::<GlobalVars>()
    }
}

/// `progs.h:165` -- the base of the `G_*` macro block.
#[inline]
unsafe fn globals() -> *mut c_float {
    // SAFETY: the ambient qcvm is selected by the caller.
    unsafe { ptr::addr_of!((*vm()).globals).read() }
}

/// `wad.h:52-56` -- `pic->width`. Called only on pointers the renderer just
/// handed back.
#[inline]
unsafe fn pic_width(pic: *mut c_void) -> c_int {
    // SAFETY: `pic` is a live `qpic_t *` from Draw_CachePic/Draw_PicFromWad.
    unsafe { ptr::addr_of!((*pic.cast::<g::qpic_t>()).width).read() }
}

/// `cl.scores[i]`.
#[inline]
unsafe fn score(i: c_int) -> *mut ScoreBoard {
    // SAFETY: `cl.scores` is the client's MAX_SCOREBOARD-entry array and `i`
    // comes from `fragsort`, which `Sbar_SortFrags` fills from
    // `0 .. cl.maxclients`.
    unsafe { ptr::addr_of!(cl.scores).read().add(i as usize) }
}

// ---------------------------------------------------------------------------
// sbar.c:75 -- Sbar_CSQCCommand

/// `sbar.c:75` -- `qboolean Sbar_CSQCCommand (void)`, split into a status and
/// an out-parameter (ADR-009 rule 2). `*out` is written only on the paths
/// where C assigns `ret`.
///
/// COMPAT: ADR-008/ADR-009. When `PR_ExecuteProgram` raises, C never reaches
/// the `PR_SwitchQCVM (NULL)` at `sbar.c:84`, so the client qcvm stays
/// selected. The early return below leaves exactly that state; no cleanup C
/// does not have is added.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_csqc_command(out: *mut bool) -> Raise {
    // SAFETY: mirrors sbar.c:75; `out` is the glue's stack slot and the rest
    // is engine state.
    unsafe {
        let mut ret = false;
        if g::scr_style.value < 1.0f32 && cl.qcvm.extfuncs.csqc_console_command != 0 {
            gs::PR_SwitchQCVM(ptr::addr_of_mut!(cl.qcvm).cast());
            globals()
                .add(OFS_PARM0)
                .cast::<c_int>()
                .write(g::PR_MakeTempString(c::Cmd_Argv(0)));
            raise!(gh::Host_Glue_PR_ExecuteProgram(
                cl.qcvm.extfuncs.csqc_console_command as c_int
            ));
            ret = globals().add(OFS_RETURN).read() != 0.0;
            gs::PR_SwitchQCVM(ptr::null_mut());
        }
        out.write(ret);
        gh::HOST_GUARD_OK
    }
}

/// `sbar.c:96` -- `Sbar_ShowScores`, the tab key going down.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_show_scores() -> Raise {
    // SAFETY: mirrors sbar.c:96; `sb_showscores` is glue-owned storage.
    unsafe {
        let mut ignored = false;
        raise!(quake_rs_sbar_csqc_command(&mut ignored));
        if g::sb_showscores {
            return gh::HOST_GUARD_OK;
        }
        g::sb_showscores = true;
        gh::HOST_GUARD_OK
    }
}

/// `sbar.c:110` -- `Sbar_DontShowScores`, the tab key coming up. `static` in
/// C, so the glue exports it as `Sbar_Glue_DontShowScores`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_dont_show_scores() -> Raise {
    // SAFETY: mirrors sbar.c:110.
    unsafe {
        let mut ignored = false;
        raise!(quake_rs_sbar_csqc_command(&mut ignored));
        g::sb_showscores = false;
        gh::HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// sbar.c:117 -- pic loading

/// `sbar.c:117` -- `Sbar_CheckPicFromWad`, which clears `hudtype` the first
/// time a lump of the candidate hud is missing.
unsafe fn check_pic_from_wad(name: *const c_char) -> *mut c_void {
    // SAFETY: `name` is a NUL-terminated literal or a `va` buffer.
    unsafe {
        let pic_nul = ptr::addr_of!(g::pic_nul).read();
        if HUDTYPE == 0 {
            return pic_nul; // one already failed, don't waste cpu
        }
        let mut info: *mut c_void = ptr::null_mut();
        let r = if g::W_GetLumpName(name, &mut info).is_null() {
            pic_nul
        } else {
            g::Draw_PicFromWad(name)
        };
        if r == pic_nul {
            HUDTYPE = 0;
        }
        r
    }
}

/// `sbar.c:138` -- `Sbar_LoadPics`, johnfitz: load all the sbar pics.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
#[allow(clippy::needless_range_loop)] // two parallel statics per iteration
pub unsafe extern "C" fn quake_rs_sbar_load_pics() {
    // SAFETY: mirrors sbar.c:138; every callee is a renderer entry point and
    // the destinations are this module's statics.
    unsafe {
        for i in 0..10 {
            SB_NUMS[0][i] = g::Draw_PicFromWad(g::va(c"num_%i".as_ptr(), i as c_int));
            SB_NUMS[1][i] = g::Draw_PicFromWad(g::va(c"anum_%i".as_ptr(), i as c_int));
        }

        SB_NUMS[0][10] = g::Draw_PicFromWad(c"num_minus".as_ptr());
        SB_NUMS[1][10] = g::Draw_PicFromWad(c"anum_minus".as_ptr());

        SB_COLON = g::Draw_PicFromWad(c"num_colon".as_ptr());
        SB_SLASH = g::Draw_PicFromWad(c"num_slash".as_ptr());

        SB_WEAPONS[0][0] = g::Draw_PicFromWad(c"inv_shotgun".as_ptr());
        SB_WEAPONS[0][1] = g::Draw_PicFromWad(c"inv_sshotgun".as_ptr());
        SB_WEAPONS[0][2] = g::Draw_PicFromWad(c"inv_nailgun".as_ptr());
        SB_WEAPONS[0][3] = g::Draw_PicFromWad(c"inv_snailgun".as_ptr());
        SB_WEAPONS[0][4] = g::Draw_PicFromWad(c"inv_rlaunch".as_ptr());
        SB_WEAPONS[0][5] = g::Draw_PicFromWad(c"inv_srlaunch".as_ptr());
        SB_WEAPONS[0][6] = g::Draw_PicFromWad(c"inv_lightng".as_ptr());

        SB_WEAPONS[1][0] = g::Draw_PicFromWad(c"inv2_shotgun".as_ptr());
        SB_WEAPONS[1][1] = g::Draw_PicFromWad(c"inv2_sshotgun".as_ptr());
        SB_WEAPONS[1][2] = g::Draw_PicFromWad(c"inv2_nailgun".as_ptr());
        SB_WEAPONS[1][3] = g::Draw_PicFromWad(c"inv2_snailgun".as_ptr());
        SB_WEAPONS[1][4] = g::Draw_PicFromWad(c"inv2_rlaunch".as_ptr());
        SB_WEAPONS[1][5] = g::Draw_PicFromWad(c"inv2_srlaunch".as_ptr());
        SB_WEAPONS[1][6] = g::Draw_PicFromWad(c"inv2_lightng".as_ptr());

        for i in 0..5 {
            let n = i as c_int + 1;
            SB_WEAPONS[2 + i][0] = g::Draw_PicFromWad(g::va(c"inva%i_shotgun".as_ptr(), n));
            SB_WEAPONS[2 + i][1] = g::Draw_PicFromWad(g::va(c"inva%i_sshotgun".as_ptr(), n));
            SB_WEAPONS[2 + i][2] = g::Draw_PicFromWad(g::va(c"inva%i_nailgun".as_ptr(), n));
            SB_WEAPONS[2 + i][3] = g::Draw_PicFromWad(g::va(c"inva%i_snailgun".as_ptr(), n));
            SB_WEAPONS[2 + i][4] = g::Draw_PicFromWad(g::va(c"inva%i_rlaunch".as_ptr(), n));
            SB_WEAPONS[2 + i][5] = g::Draw_PicFromWad(g::va(c"inva%i_srlaunch".as_ptr(), n));
            SB_WEAPONS[2 + i][6] = g::Draw_PicFromWad(g::va(c"inva%i_lightng".as_ptr(), n));
        }

        SB_AMMO[0] = g::Draw_PicFromWad(c"sb_shells".as_ptr());
        SB_AMMO[1] = g::Draw_PicFromWad(c"sb_nails".as_ptr());
        SB_AMMO[2] = g::Draw_PicFromWad(c"sb_rocket".as_ptr());
        SB_AMMO[3] = g::Draw_PicFromWad(c"sb_cells".as_ptr());

        SB_ARMOR[0] = g::Draw_PicFromWad(c"sb_armor1".as_ptr());
        SB_ARMOR[1] = g::Draw_PicFromWad(c"sb_armor2".as_ptr());
        SB_ARMOR[2] = g::Draw_PicFromWad(c"sb_armor3".as_ptr());

        SB_ITEMS[0] = g::Draw_PicFromWad(c"sb_key1".as_ptr());
        SB_ITEMS[1] = g::Draw_PicFromWad(c"sb_key2".as_ptr());
        SB_ITEMS[2] = g::Draw_PicFromWad(c"sb_invis".as_ptr());
        SB_ITEMS[3] = g::Draw_PicFromWad(c"sb_invuln".as_ptr());
        SB_ITEMS[4] = g::Draw_PicFromWad(c"sb_suit".as_ptr());
        SB_ITEMS[5] = g::Draw_PicFromWad(c"sb_quad".as_ptr());

        SB_SIGIL[0] = g::Draw_PicFromWad(c"sb_sigil1".as_ptr());
        SB_SIGIL[1] = g::Draw_PicFromWad(c"sb_sigil2".as_ptr());
        SB_SIGIL[2] = g::Draw_PicFromWad(c"sb_sigil3".as_ptr());
        SB_SIGIL[3] = g::Draw_PicFromWad(c"sb_sigil4".as_ptr());

        SB_FACES[4][0] = g::Draw_PicFromWad(c"face1".as_ptr());
        SB_FACES[4][1] = g::Draw_PicFromWad(c"face_p1".as_ptr());
        SB_FACES[3][0] = g::Draw_PicFromWad(c"face2".as_ptr());
        SB_FACES[3][1] = g::Draw_PicFromWad(c"face_p2".as_ptr());
        SB_FACES[2][0] = g::Draw_PicFromWad(c"face3".as_ptr());
        SB_FACES[2][1] = g::Draw_PicFromWad(c"face_p3".as_ptr());
        SB_FACES[1][0] = g::Draw_PicFromWad(c"face4".as_ptr());
        SB_FACES[1][1] = g::Draw_PicFromWad(c"face_p4".as_ptr());
        SB_FACES[0][0] = g::Draw_PicFromWad(c"face5".as_ptr());
        SB_FACES[0][1] = g::Draw_PicFromWad(c"face_p5".as_ptr());

        SB_FACE_INVIS = g::Draw_PicFromWad(c"face_invis".as_ptr());
        SB_FACE_INVULN = g::Draw_PicFromWad(c"face_invul2".as_ptr());
        SB_FACE_INVIS_INVULN = g::Draw_PicFromWad(c"face_inv2".as_ptr());
        SB_FACE_QUAD = g::Draw_PicFromWad(c"face_quad".as_ptr());

        SB_SBAR = g::Draw_PicFromWad(c"sbar".as_ptr());
        SB_IBAR = g::Draw_PicFromWad(c"ibar".as_ptr());
        SB_SCOREBAR = g::Draw_PicFromWad(c"scorebar".as_ptr());

        HUDTYPE = 0;

        // MED 01/04/97 added new hipnotic weapons
        if HUDTYPE == 0 {
            HUDTYPE = 1;
            HSB_WEAPONS[0][0] = check_pic_from_wad(c"inv_laser".as_ptr());
            HSB_WEAPONS[0][1] = check_pic_from_wad(c"inv_mjolnir".as_ptr());
            HSB_WEAPONS[0][2] = check_pic_from_wad(c"inv_gren_prox".as_ptr());
            HSB_WEAPONS[0][3] = check_pic_from_wad(c"inv_prox_gren".as_ptr());
            HSB_WEAPONS[0][4] = check_pic_from_wad(c"inv_prox".as_ptr());

            HSB_WEAPONS[1][0] = check_pic_from_wad(c"inv2_laser".as_ptr());
            HSB_WEAPONS[1][1] = check_pic_from_wad(c"inv2_mjolnir".as_ptr());
            HSB_WEAPONS[1][2] = check_pic_from_wad(c"inv2_gren_prox".as_ptr());
            HSB_WEAPONS[1][3] = check_pic_from_wad(c"inv2_prox_gren".as_ptr());
            HSB_WEAPONS[1][4] = check_pic_from_wad(c"inv2_prox".as_ptr());

            for i in 0..5 {
                let n = i as c_int + 1;
                HSB_WEAPONS[2 + i][0] = check_pic_from_wad(g::va(c"inva%i_laser".as_ptr(), n));
                HSB_WEAPONS[2 + i][1] = check_pic_from_wad(g::va(c"inva%i_mjolnir".as_ptr(), n));
                HSB_WEAPONS[2 + i][2] = check_pic_from_wad(g::va(c"inva%i_gren_prox".as_ptr(), n));
                HSB_WEAPONS[2 + i][3] = check_pic_from_wad(g::va(c"inva%i_prox_gren".as_ptr(), n));
                HSB_WEAPONS[2 + i][4] = check_pic_from_wad(g::va(c"inva%i_prox".as_ptr(), n));
            }

            HSB_ITEMS[0] = check_pic_from_wad(c"sb_wsuit".as_ptr());
            HSB_ITEMS[1] = check_pic_from_wad(c"sb_eshld".as_ptr());
        }

        if HUDTYPE == 0 {
            HUDTYPE = 2;
            RSB_INVBAR[0] = check_pic_from_wad(c"r_invbar1".as_ptr());
            RSB_INVBAR[1] = check_pic_from_wad(c"r_invbar2".as_ptr());

            RSB_WEAPONS[0] = check_pic_from_wad(c"r_lava".as_ptr());
            RSB_WEAPONS[1] = check_pic_from_wad(c"r_superlava".as_ptr());
            RSB_WEAPONS[2] = check_pic_from_wad(c"r_gren".as_ptr());
            RSB_WEAPONS[3] = check_pic_from_wad(c"r_multirock".as_ptr());
            RSB_WEAPONS[4] = check_pic_from_wad(c"r_plasma".as_ptr());

            RSB_ITEMS[0] = check_pic_from_wad(c"r_shield1".as_ptr());
            RSB_ITEMS[1] = check_pic_from_wad(c"r_agrav1".as_ptr());

            // PGM 01/19/97 - team color border
            RSB_TEAMBORD = check_pic_from_wad(c"r_teambord".as_ptr());

            RSB_AMMO[0] = check_pic_from_wad(c"r_ammolava".as_ptr());
            RSB_AMMO[1] = check_pic_from_wad(c"r_ammomulti".as_ptr());
            RSB_AMMO[2] = check_pic_from_wad(c"r_ammoplasma".as_ptr());
        }
    }
}

/// `sbar.c:283` -- `Sbar_Init`, johnfitz: rewritten.
///
/// The two commands are registered with the glue's re-raising wrappers, never
/// with a Rust status core, so the jump is always issued from a C frame
/// (ADR-009).
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_init() {
    // SAFETY: mirrors sbar.c:283.
    unsafe {
        c::Cmd_AddCommand2(
            c"+showscores".as_ptr(),
            Some(g::Sbar_ShowScores),
            c::cmd_source_t_src_command,
            false,
        );
        c::Cmd_AddCommand2(
            c"-showscores".as_ptr(),
            Some(g::Sbar_Glue_DontShowScores),
            c::cmd_source_t_src_command,
            false,
        );

        quake_rs_sbar_load_pics();
    }
}

// ---------------------------------------------------------------------------
// sbar.c:297-434 -- drawing primitives, relative to the status bar location

/// `sbar.c:297` -- `Sbar_DrawPic`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_pic(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    pic: *mut c_void,
) {
    // SAFETY: mirrors sbar.c:297; `cbx` and `pic` come from the caller.
    unsafe {
        gc::Draw_Pic(cbx, x as f32, (y + 24) as f32, pic, 1.0f32, false);
    }
}

/// `sbar.c:307` -- `Sbar_DrawPicAlpha`, johnfitz.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_pic_alpha(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    pic: *mut c_void,
    alpha: c_float,
) {
    // SAFETY: mirrors sbar.c:307.
    unsafe {
        gc::Draw_Pic(cbx, x as f32, (y + 24) as f32, pic, alpha, true);
    }
}

/// `sbar.c:317` -- `Sbar_DrawCharacter`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_character(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    num: c_int,
) {
    // SAFETY: mirrors sbar.c:317.
    unsafe {
        gc::Draw_Character(cbx, x as f32, (y + 24) as f32, num);
    }
}

/// `sbar.c:327` -- `Sbar_DrawString`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_string(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    str_: *const c_char,
) {
    // SAFETY: mirrors sbar.c:327; `str_` is NUL-terminated.
    unsafe {
        gc::Draw_String(cbx, x as f32, (y + 24) as f32, str_);
    }
}

/// `sbar.c:337` -- `Sbar_DrawScrollString`, johnfitz. `width` is unused in
/// the original too.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_scroll_string(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    width: c_int,
    str_: *const c_char,
) {
    // SAFETY: mirrors sbar.c:337.
    unsafe {
        let _ = width;
        let len = (g::strlen(str_) as c_int).wrapping_mul(8).wrapping_add(40);
        let ofs = ((gc::realtime * 30.0) as c_int) % len;
        quake_rs_sbar_draw_string(cbx, x - ofs, y, str_);
        quake_rs_sbar_draw_character(cbx, x - ofs + len - 32, y, '/' as c_int);
        quake_rs_sbar_draw_character(cbx, x - ofs + len - 24, y, '/' as c_int);
        quake_rs_sbar_draw_character(cbx, x - ofs + len - 16, y, '/' as c_int);
        quake_rs_sbar_draw_string(cbx, x - ofs + len, y, str_);
    }
}

/// `sbar.c:356` -- `Sbar_itoa`.
///
/// COMPAT: ADR-010. `num = -num` for `INT_MIN` and `pow10 *= 10` past
/// 1000000000 are both signed overflow, which C leaves undefined and every
/// target the engine builds for implements as two's-complement wraparound.
/// The wrapping operators below reproduce that byte-for-byte instead of
/// panicking in a debug build.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_itoa(mut num: c_int, buf: *mut c_char) -> c_int {
    // SAFETY: mirrors sbar.c:356; `buf` is the caller's `char[12]`, which is
    // exactly the widest string this can write (sign + 10 digits + NUL).
    unsafe {
        let mut str_ = buf;

        if num < 0 {
            str_.write(b'-' as c_char);
            str_ = str_.add(1);
            num = num.wrapping_neg();
        }

        let mut pow10: c_int = 10;
        while num >= pow10 {
            pow10 = pow10.wrapping_mul(10);
        }

        loop {
            pow10 /= 10;
            let dig = num / pow10;
            str_.write((b'0' as c_int).wrapping_add(dig) as c_char);
            str_ = str_.add(1);
            num -= dig.wrapping_mul(pow10);
            if pow10 == 1 {
                break;
            }
        }

        str_.write(0);

        str_.offset_from(buf) as c_int
    }
}

/// `sbar.c:389` -- `Sbar_DrawNum`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_num(
    cbx: *mut c_void,
    mut x: c_int,
    y: c_int,
    mut num: c_int,
    digits: c_int,
    color: c_int,
) {
    // SAFETY: mirrors sbar.c:389.
    unsafe {
        let mut str_ = [0 as c_char; 12];

        num = num.min(999); // johnfitz -- cap high values rather than truncating number

        let l = quake_rs_sbar_itoa(num, str_.as_mut_ptr());
        let mut ptr_ = str_.as_ptr();
        if l > digits {
            ptr_ = ptr_.add((l - digits) as usize);
        }
        if l < digits {
            x += (digits - l) * 24;
        }

        while ptr_.read() != 0 {
            let frame = if ptr_.read() == b'-' as c_char {
                STAT_MINUS
            } else {
                ptr_.read() as c_int - b'0' as c_int
            };

            // johnfitz -- DrawTransPic is obsolete
            quake_rs_sbar_draw_pic(cbx, x, y, SB_NUMS[color as usize][frame as usize]);
            x += 24;
            ptr_ = ptr_.add(1);
        }
    }
}

/// `sbar.c:422` -- `Sbar_DrawSmallAmmoCounter`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_small_ammo_counter(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    mut val: c_int,
) {
    // SAFETY: mirrors sbar.c:422; libc formats into a local `char[6]`.
    unsafe {
        let mut num = [0 as c_char; 6];

        // johnfitz -- cap displayed value to 999
        val = if val < 0 { 0 } else { val.min(999) };
        g::q_snprintf(num.as_mut_ptr(), num.len(), c"%3i".as_ptr(), val);
        #[allow(clippy::needless_range_loop)] // the index is also the character column
        for i in 0..3usize {
            if num[i] != b' ' as c_char {
                quake_rs_sbar_draw_character(
                    cbx,
                    x + i as c_int * 8,
                    y,
                    18 + num[i] as c_int - b'0' as c_int,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// sbar.c:439-527 -- the scoreboard

/// `sbar.c:446` -- `Sbar_SortFrags`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_sort_frags() {
    // SAFETY: mirrors sbar.c:446; `fragsort` and `scoreboardlines` are
    // glue-owned storage (ADR-007) and `cl.scores` is the client array.
    unsafe {
        // sort by frags
        g::scoreboardlines = 0;
        for i in 0..cl.maxclients {
            if ptr::addr_of!((*score(i)).name[0]).read() != 0 {
                g::fragsort[g::scoreboardlines as usize] = i;
                g::scoreboardlines += 1;
            }
        }

        for i in 0..g::scoreboardlines {
            for j in 0..g::scoreboardlines - 1 - i {
                let a = g::fragsort[j as usize];
                let b = g::fragsort[j as usize + 1];
                if ptr::addr_of!((*score(a)).frags).read() < ptr::addr_of!((*score(b)).frags).read()
                {
                    g::fragsort[j as usize] = b;
                    g::fragsort[j as usize + 1] = a;
                }
            }
        }
    }
}

/// `sbar.c:475` -- `Sbar_ColorForMap`.
///
/// COMPAT: the ternary has identical arms in the original
/// (`m < 128 ? m + 8 : m + 8`). Transcribed as the single expression it
/// evaluates to rather than "fixed", because `pr_ext.c` and both scoreboards
/// depend on the palette index it produces.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_color_for_map(m: c_int) -> c_int {
    m + 8
}

/// `sbar.c:485` -- `Sbar_SoloScoreboard`, johnfitz: new layout.
///
/// COMPAT: `sbar.c:514` is an unconditional `return`, so `sbar.c:516-527`
/// (the elapsed-time line and the second level-name draw) is dead code in the
/// original. Only the live prefix is ported; reviving the tail would add draw
/// calls the C build never issues.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_solo_scoreboard(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:485; libc formats into local buffers sized as in
    // the original.
    unsafe {
        let mut str_ = [0 as c_char; 256];

        g::q_snprintf(
            str_.as_mut_ptr(),
            str_.len(),
            c"Kills: %i/%i".as_ptr(),
            cl.stats[STAT_MONSTERS],
            cl.stats[STAT_TOTALMONSTERS],
        );
        let left = 8 + g::strlen(str_.as_ptr()) as c_int * 8;
        quake_rs_sbar_draw_string(cbx, 8, 12, str_.as_ptr());

        g::q_snprintf(
            str_.as_mut_ptr(),
            str_.len(),
            c"Secrets: %i/%i".as_ptr(),
            cl.stats[STAT_SECRETS],
            cl.stats[STAT_TOTALSECRETS],
        );
        let right = 312 - g::strlen(str_.as_ptr()) as c_int * 8;
        quake_rs_sbar_draw_string(cbx, right, 12, str_.as_ptr());

        let mut cleanname = [0 as c_char; 128];
        c::host_cmd::COM_SanitizeDescriptionString(
            cleanname.as_mut_ptr(),
            cleanname.len(),
            ptr::addr_of!(cl.levelname).cast::<c_char>(),
            false,
        );

        /* QuakeSpasm customization: */
        g::q_snprintf(
            str_.as_mut_ptr(),
            str_.len(),
            c"skill %i".as_ptr(),
            (gs::skill.value as f64 + 0.5) as c_int,
        );
        quake_rs_sbar_draw_string(
            cbx,
            (left + right) / 2 - g::strlen(str_.as_ptr()) as c_int * 4,
            12,
            str_.as_ptr(),
        );

        g::q_snprintf(
            str_.as_mut_ptr(),
            str_.len(),
            c"%s (%s)".as_ptr(),
            cleanname.as_ptr(),
            ptr::addr_of!(cl.mapname).cast::<c_char>(),
        );

        let len = g::strlen(str_.as_ptr()) as c_int;
        if len > 40 {
            quake_rs_sbar_draw_scroll_string(cbx, 0, 4, 320, str_.as_ptr());
        } else {
            quake_rs_sbar_draw_string(cbx, 160 - len * 4, 4, str_.as_ptr());
        }
    }
}

/// `sbar.c:533` -- `Sbar_DrawScoreboard`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_scoreboard(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:533.
    unsafe {
        quake_rs_sbar_solo_scoreboard(cbx);
        if cl.gametype == GAME_DEATHMATCH {
            quake_rs_sbar_deathmatch_overlay(cbx);
        }
    }
}

// ---------------------------------------------------------------------------
// sbar.c:546-696 -- the inventory bar

/// `sbar.c:546` -- `Sbar_InventoryBarPic`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_inventory_bar_pic() -> *mut c_void {
    // SAFETY: mirrors sbar.c:546.
    unsafe {
        if rogue() {
            return RSB_INVBAR[(cl.stats[STAT_ACTIVEWEAPON] < RIT_LAVA_NAILGUN) as usize];
        }
        SB_IBAR
    }
}

/// `sbar.c:558` -- `Sbar_CalculateFlashOn`. Note that it also *writes*
/// `cl.item_gettime[val]` when the client clock has gone backwards.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_calculate_flash_on(val: c_int) -> c_int {
    // SAFETY: mirrors sbar.c:558; `val` is 0..6 or a `hipweapons[]` bit index
    // (max 23), well inside `cl.item_gettime[32]`.
    unsafe {
        let mut time = cl.item_gettime[val as usize];
        if time > cl.time as f32 {
            cl.item_gettime[val as usize] = (cl.time - 2.0) as f32;
            time = cl.item_gettime[val as usize];
        }
        let mut flashon = ((cl.time - time as f64) * 10.0) as c_int;

        if flashon >= 10 {
            if cl.stats[STAT_ACTIVEWEAPON] == (1 << val) {
                flashon = 1;
            } else {
                flashon = 0;
            }
        } else {
            flashon = (flashon % 5) + 2;
        }

        flashon
    }
}

/// `sbar.c:581` -- `Sbar_DrawInventory`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_inventory(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:581; every index below is bounded by the loop
    // that produces it.
    unsafe {
        let mut skip: u8 = 255; // avoid wasting GPU time

        // johnfitz -- scr_sbaralpha
        quake_rs_sbar_draw_pic_alpha(
            cbx,
            0,
            -24,
            quake_rs_sbar_inventory_bar_pic(),
            g::scr_sbaralpha.value,
        );

        // MED 01/04/97
        // hipnotic weapons
        if hipnotic() {
            for i in 0..4usize {
                if cl.items & (1 << HIPWEAPONS[i]) != 0 {
                    // check grenade launcher
                    if i == 2 {
                        if cl.items & HIT_PROXIMITY_GUN != 0 {
                            skip = 4;
                            if cl.stats[STAT_ACTIVEWEAPON] == IT_GRENADE_LAUNCHER {
                                let f = quake_rs_sbar_calculate_flash_on(HIPWEAPONS[2]);
                                quake_rs_sbar_draw_pic(cbx, 96, -16, HSB_WEAPONS[f as usize][2]);
                            } else {
                                let f = quake_rs_sbar_calculate_flash_on(HIPWEAPONS[3]);
                                quake_rs_sbar_draw_pic(cbx, 96, -16, HSB_WEAPONS[f as usize][3]);
                            }
                        }
                    } else if i == 3 {
                        if cl.items & IT_GRENADE_LAUNCHER == 0 {
                            let f = quake_rs_sbar_calculate_flash_on(HIPWEAPONS[3]);
                            quake_rs_sbar_draw_pic(cbx, 96, -16, HSB_WEAPONS[f as usize][4]);
                        }
                    } else {
                        let f = quake_rs_sbar_calculate_flash_on(HIPWEAPONS[i]);
                        quake_rs_sbar_draw_pic(
                            cbx,
                            176 + (i as c_int * 24),
                            -16,
                            HSB_WEAPONS[f as usize][i],
                        );
                    }
                }
            }
        }

        if rogue() {
            // check for powered up weapon.
            if cl.stats[STAT_ACTIVEWEAPON] >= RIT_LAVA_NAILGUN {
                for i in 0..5u8 {
                    if cl.stats[STAT_ACTIVEWEAPON] == (RIT_LAVA_NAILGUN << i) {
                        skip = i + 2;
                        quake_rs_sbar_draw_pic(
                            cbx,
                            skip as c_int * 24,
                            -16,
                            RSB_WEAPONS[i as usize],
                        );
                    }
                }
            }
        }

        // weapons
        for i in 0..7u8 {
            if i != skip && cl.items & (IT_SHOTGUN << i) != 0 {
                let f = quake_rs_sbar_calculate_flash_on(i as c_int);
                quake_rs_sbar_draw_pic(
                    cbx,
                    i as c_int * 24,
                    -16,
                    SB_WEAPONS[f as usize][i as usize],
                );
            }
        }

        // ammo counts
        for i in 0..4usize {
            quake_rs_sbar_draw_small_ammo_counter(
                cbx,
                48 * i as c_int + 10,
                -24,
                cl.stats[STAT_SHELLS + i],
            );
        }

        // items
        #[allow(clippy::needless_range_loop)] // the index is also the item bit
        for i in 0..6usize {
            if cl.items & (1 << (17 + i)) != 0 {
                // MED 01/04/97 changed keys
                if !hipnotic() || i > 1 {
                    quake_rs_sbar_draw_pic(cbx, 192 + i as c_int * 16, -16, SB_ITEMS[i]);
                }
            }
        }
        // MED 01/04/97 added hipnotic items
        //  hipnotic items
        if hipnotic() {
            #[allow(clippy::needless_range_loop)] // the index is also the item bit
            for i in 0..2usize {
                if cl.items & (1 << (24 + i)) != 0 {
                    quake_rs_sbar_draw_pic(cbx, 288 + i as c_int * 16, -16, HSB_ITEMS[i]);
                }
            }
        }

        if rogue() {
            // new rogue items
            #[allow(clippy::needless_range_loop)] // the index is also the item bit
            for i in 0..2usize {
                if cl.items & (1 << (29 + i)) != 0 {
                    quake_rs_sbar_draw_pic(cbx, 288 + i as c_int * 16, -16, RSB_ITEMS[i]);
                }
            }
        } else {
            // sigils
            #[allow(clippy::needless_range_loop)] // the index is also the sigil bit
            for i in 0..4usize {
                // COMPAT: `1u << (28 + i)` reaches bit 31, so the mask is
                // unsigned in C and has to be `u32` here to avoid an overflow
                // panic in a debug build.
                if (cl.items as u32) & (1u32 << (28 + i)) != 0 {
                    quake_rs_sbar_draw_pic(cbx, 320 - 32 + i as c_int * 8, -16, SB_SIGIL[i]);
                }
            }
        }
    }
}

/// `sbar.c:698` -- `Sbar_DrawFrags`, johnfitz: heavy revision.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_frags(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:698.
    unsafe {
        let mut num = [0 as c_char; 12];

        quake_rs_sbar_sort_frags();

        // draw the text
        let numscores = g::scoreboardlines.min(4);

        let mut x = 184;
        for i in 0..numscores {
            let s = score(g::fragsort[i as usize]);
            if ptr::addr_of!((*s).name[0]).read() == 0 {
                x += 32;
                continue;
            }
            let colors = ptr::addr_of!((*s).colors).read();

            // top color
            let color = quake_rs_sbar_color_for_map(colors & 0xf0);
            gc::Draw_Fill(cbx, (x + 10) as f32, 1.0, 28.0, 4.0, color, 1.0);

            // bottom color
            let color = quake_rs_sbar_color_for_map((colors & 15) << 4);
            gc::Draw_Fill(cbx, (x + 10) as f32, 5.0, 28.0, 3.0, color, 1.0);

            // number
            g::q_snprintf(
                num.as_mut_ptr(),
                num.len(),
                c"%3i".as_ptr(),
                ptr::addr_of!((*s).frags).read(),
            );
            quake_rs_sbar_draw_character(cbx, x + 12, -24, num[0] as c_int);
            quake_rs_sbar_draw_character(cbx, x + 20, -24, num[1] as c_int);
            quake_rs_sbar_draw_character(cbx, x + 28, -24, num[2] as c_int);

            // brackets
            if g::fragsort[i as usize] == cl.viewentity - 1 {
                quake_rs_sbar_draw_character(cbx, x + 6, -24, 16);
                quake_rs_sbar_draw_character(cbx, x + 32, -24, 17);
            }

            x += 32;
        }
    }
}

/// `sbar.c:740` -- `Sbar_DrawFace`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw_face(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    classic_style: bool,
) {
    // SAFETY: mirrors sbar.c:740.
    unsafe {
        // PGM 01/19/97 - team color drawing
        // PGM 03/02/97 - fixed so color swatch only appears in CTF modes
        if classic_style
            && rogue()
            && cl.maxclients != 1
            && c::progs_builtins_sv::teamplay.value > 3.0
            && c::progs_builtins_sv::teamplay.value < 7.0
        {
            let mut num = [0 as c_char; 12];

            let s = score(cl.viewentity - 1);
            // draw background
            let colors = ptr::addr_of!((*s).colors).read();
            let top = quake_rs_sbar_color_for_map(colors & 0xf0);
            let bottom = quake_rs_sbar_color_for_map((colors & 15) << 4);

            let xofs = if cl.gametype == GAME_DEATHMATCH {
                113
            } else {
                ((c::cl_parse::vid.width - 320) >> 1) + 113
            };

            quake_rs_sbar_draw_pic(cbx, 112, 0, RSB_TEAMBORD);
            gc::Draw_Fill(cbx, xofs as f32, (24 + 3) as f32, 22.0, 9.0, top, 1.0);
            gc::Draw_Fill(cbx, xofs as f32, (24 + 12) as f32, 22.0, 9.0, bottom, 1.0);

            // draw number
            let f = ptr::addr_of!((*s).frags).read();
            g::q_snprintf(num.as_mut_ptr(), num.len(), c"%3i".as_ptr(), f);

            if top == 8 {
                if num[0] != b' ' as c_char {
                    quake_rs_sbar_draw_character(cbx, 113, 3, 18 + num[0] as c_int - b'0' as c_int);
                }
                if num[1] != b' ' as c_char {
                    quake_rs_sbar_draw_character(cbx, 120, 3, 18 + num[1] as c_int - b'0' as c_int);
                }
                if num[2] != b' ' as c_char {
                    quake_rs_sbar_draw_character(cbx, 127, 3, 18 + num[2] as c_int - b'0' as c_int);
                }
            } else {
                quake_rs_sbar_draw_character(cbx, 113, 3, num[0] as c_int);
                quake_rs_sbar_draw_character(cbx, 120, 3, num[1] as c_int);
                quake_rs_sbar_draw_character(cbx, 127, 3, num[2] as c_int);
            }

            return;
        }
        // PGM 01/19/97 - team color drawing

        if cl.items & (IT_INVISIBILITY | IT_INVULNERABILITY)
            == (IT_INVISIBILITY | IT_INVULNERABILITY)
        {
            quake_rs_sbar_draw_pic(cbx, x, y, SB_FACE_INVIS_INVULN);
            return;
        }
        if cl.items & IT_QUAD != 0 {
            quake_rs_sbar_draw_pic(cbx, x, y, SB_FACE_QUAD);
            return;
        }
        if cl.items & IT_INVISIBILITY != 0 {
            quake_rs_sbar_draw_pic(cbx, x, y, SB_FACE_INVIS);
            return;
        }
        if cl.items & IT_INVULNERABILITY != 0 {
            quake_rs_sbar_draw_pic(cbx, x, y, SB_FACE_INVULN);
            return;
        }

        let mut f = if cl.stats[STAT_HEALTH] >= 100 {
            4
        } else {
            cl.stats[STAT_HEALTH] / 20
        };
        if f < 0 {
            // in case we ever decide to draw when health <= 0
            f = 0;
        }

        let anim = if cl.time <= cl.faceanimtime as f64 {
            1
        } else {
            0
        };
        quake_rs_sbar_draw_pic(cbx, x, y, SB_FACES[f as usize][anim]);
    }
}

// ---------------------------------------------------------------------------
// sbar.c:842-1259 -- the three HUD styles

/// `sbar.c:842` -- `Sbar_DrawCSCQ` (`static` in C, private here).
///
/// COMPAT: ADR-008/ADR-009. Each `raise!` below returns before the
/// `PR_SwitchQCVM (NULL)` at `sbar.c:874` and before `cl.stats[STAT_ITEMS]`
/// is restored, which is exactly what C does when `PR_ExecuteProgram`
/// longjmps past those statements. No compensating cleanup is added.
unsafe fn draw_cscq(cbx: *mut c_void) -> Raise {
    // SAFETY: mirrors sbar.c:842; the qcvm is selected and cleared in the C
    // order and `pr_global_struct` is re-read after the switch.
    unsafe {
        let mut deathmatchoverlay = false;
        let s = clamp_f64(
            1.0,
            g::scr_sbarscale.value as f64,
            gc::glwidth as f32 as f64 / 320.0,
        ) as f32;
        let items = cl.stats[STAT_ITEMS];
        if cl.time < cl.oldtime {
            cl.stats[STAT_ITEMS] = 0;
        }
        gc::GL_SetCanvas(cbx, CANVAS_CSQC); // johnfitz
        gs::PR_SwitchQCVM(ptr::addr_of_mut!(cl.qcvm).cast());
        ptr::addr_of_mut!((*pgs()).frametime).write(c::host_frametime as f32);
        let cur = vm();
        let cltime = ptr::addr_of!((*cur).extglobals.cltime).read();
        if !cltime.is_null() {
            cltime.write(gc::realtime as f32);
        }
        let clframetime = ptr::addr_of!((*cur).extglobals.clframetime).read();
        if !clframetime.is_null() {
            clframetime.write(c::host_frametime as f32);
        }
        let intermission = ptr::addr_of!((*cur).extglobals.intermission).read();
        if !intermission.is_null() {
            intermission.write(cl.intermission as f32);
        }
        let localentnum = ptr::addr_of!((*cur).extglobals.player_localentnum).read();
        if !localentnum.is_null() {
            localentnum.write(cl.viewentity as f32);
        }
        ptr::addr_of_mut!((*pgs()).time).write(cl.time as f32);
        quake_rs_sbar_sort_frags();
        vectorset(
            OFS_PARM0,
            c::cl_parse::vid.width as f32 / s,
            c::cl_parse::vid.height as f32 / s,
            0.0,
        );
        globals()
            .add(OFS_PARM1)
            .write(g::sb_showscores as c_int as f32);
        raise!(gh::Host_Glue_PR_ExecuteProgram(
            cl.qcvm.extfuncs.csqc_draw_hud as c_int
        ));
        if cl.qcvm.extfuncs.csqc_draw_scores != 0 {
            vectorset(
                OFS_PARM0,
                c::cl_parse::vid.width as f32 / s,
                c::cl_parse::vid.height as f32 / s,
                0.0,
            );
            globals()
                .add(OFS_PARM1)
                .write(g::sb_showscores as c_int as f32);
            if c::keys::key_dest != KEY_MENU {
                raise!(gh::Host_Glue_PR_ExecuteProgram(
                    cl.qcvm.extfuncs.csqc_draw_scores as c_int
                ));
            }
        } else {
            deathmatchoverlay = g::sb_showscores || cl.stats[STAT_HEALTH] <= 0;
        }
        gs::PR_SwitchQCVM(ptr::null_mut());
        cl.stats[STAT_ITEMS] = items;

        if deathmatchoverlay && cl.gametype == GAME_DEATHMATCH {
            gc::GL_SetCanvas(cbx, CANVAS_SBAR);
            quake_rs_sbar_deathmatch_overlay(cbx);
        }

        gh::HOST_GUARD_OK
    }
}

/// `progs.h:177` -- `G_VECTORSET`.
#[inline]
unsafe fn vectorset(r: usize, x: c_float, y: c_float, z: c_float) {
    // SAFETY: the ambient qcvm is selected and `r` is a parameter offset.
    unsafe {
        let base = globals();
        base.add(r).write(x);
        base.add(r + 1).write(y);
        base.add(r + 2).write(z);
    }
}

/// `sbar.c:887` -- `Sbar_DrawClassic` (`static` in C, private here).
unsafe fn draw_classic(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:887.
    unsafe {
        gc::GL_SetCanvas(cbx, CANVAS_SBAR);

        // johnfitz -- check viewsize instead of sb_lines
        if gc::scr_viewsize.value < 110.0 {
            quake_rs_sbar_draw_inventory(cbx);
            if cl.maxclients != 1 {
                quake_rs_sbar_draw_frags(cbx);
            }
        }

        if g::sb_showscores || cl.stats[STAT_HEALTH] <= 0 {
            // johnfitz -- scr_sbaralpha
            quake_rs_sbar_draw_pic_alpha(cbx, 0, 0, SB_SCOREBAR, g::scr_sbaralpha.value);
            quake_rs_sbar_draw_scoreboard(cbx);
        } else if gc::scr_viewsize.value < 120.0 {
            // johnfitz -- check viewsize instead of sb_lines
            // johnfitz -- scr_sbaralpha
            quake_rs_sbar_draw_pic_alpha(cbx, 0, 0, SB_SBAR, g::scr_sbaralpha.value);

            // keys (hipnotic only)
            // MED 01/04/97 moved keys here so they would not be overwritten
            if hipnotic() {
                if cl.items & IT_KEY1 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 209, 3, SB_ITEMS[0]);
                }
                if cl.items & IT_KEY2 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 209, 12, SB_ITEMS[1]);
                }
            }
            // armor
            if cl.items & IT_INVULNERABILITY != 0 {
                quake_rs_sbar_draw_num(cbx, 24, 0, 666, 3, 1);
                quake_rs_sbar_draw_pic(cbx, 0, 0, ptr::addr_of!(g::draw_disc).read());
            } else if rogue() {
                quake_rs_sbar_draw_num(
                    cbx,
                    24,
                    0,
                    cl.stats[STAT_ARMOR],
                    3,
                    (cl.stats[STAT_ARMOR] <= 25) as c_int,
                );
                if cl.items & RIT_ARMOR3 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 0, 0, SB_ARMOR[2]);
                } else if cl.items & RIT_ARMOR2 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 0, 0, SB_ARMOR[1]);
                } else if cl.items & RIT_ARMOR1 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 0, 0, SB_ARMOR[0]);
                }
            } else {
                quake_rs_sbar_draw_num(
                    cbx,
                    24,
                    0,
                    cl.stats[STAT_ARMOR],
                    3,
                    (cl.stats[STAT_ARMOR] <= 25) as c_int,
                );
                if cl.items & IT_ARMOR3 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 0, 0, SB_ARMOR[2]);
                } else if cl.items & IT_ARMOR2 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 0, 0, SB_ARMOR[1]);
                } else if cl.items & IT_ARMOR1 != 0 {
                    quake_rs_sbar_draw_pic(cbx, 0, 0, SB_ARMOR[0]);
                }
            }

            // face
            quake_rs_sbar_draw_face(cbx, 112, 0, true);

            // health
            quake_rs_sbar_draw_num(
                cbx,
                136,
                0,
                cl.stats[STAT_HEALTH],
                3,
                (cl.stats[STAT_HEALTH] <= 25) as c_int,
            );

            // ammo icon
            if rogue() {
                if cl.items & RIT_SHELLS != 0 {
                    quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[0]);
                } else if cl.items & RIT_NAILS != 0 {
                    quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[1]);
                } else if cl.items & RIT_ROCKETS != 0 {
                    quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[2]);
                } else if cl.items & RIT_CELLS != 0 {
                    quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[3]);
                } else if cl.items & RIT_LAVA_NAILS != 0 {
                    quake_rs_sbar_draw_pic(cbx, 224, 0, RSB_AMMO[0]);
                } else if cl.items & RIT_PLASMA_AMMO != 0 {
                    quake_rs_sbar_draw_pic(cbx, 224, 0, RSB_AMMO[1]);
                } else if cl.items & RIT_MULTI_ROCKETS != 0 {
                    quake_rs_sbar_draw_pic(cbx, 224, 0, RSB_AMMO[2]);
                }
            } else if cl.items & IT_SHELLS != 0 {
                quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[0]);
            } else if cl.items & IT_NAILS != 0 {
                quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[1]);
            } else if cl.items & IT_ROCKETS != 0 {
                quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[2]);
            } else if cl.items & IT_CELLS != 0 {
                quake_rs_sbar_draw_pic(cbx, 224, 0, SB_AMMO[3]);
            }

            quake_rs_sbar_draw_num(
                cbx,
                248,
                0,
                cl.stats[STAT_AMMO],
                3,
                (cl.stats[STAT_AMMO] <= 10) as c_int,
            );
        }

        // johnfitz -- removed the vid.width > 320 check here
        if cl.gametype == GAME_DEATHMATCH {
            quake_rs_sbar_mini_deathmatch_overlay(cbx);
        }
    }
}

/// `sbar.c:996` -- `Sbar_DrawModern` (`static` in C, private here).
unsafe fn draw_modern(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:996.
    unsafe {
        if gc::scr_viewsize.value >= 120.0f32 {
            return;
        }

        let mut offset: u8 = 0;

        gc::GL_SetCanvas(cbx, CANVAS_BOTTOMLEFT);
        quake_rs_sbar_draw_face(cbx, 20, 135, false);
        quake_rs_sbar_draw_num(
            cbx,
            45,
            135,
            cl.stats[STAT_HEALTH],
            3,
            (cl.stats[STAT_HEALTH] <= 25) as c_int,
        );

        {
            // armor
            const ARMOR_NUM_X: c_int = 45;
            const ARMOR_NUM_Y: c_int = 110;
            const ARMOR_ICON_X: c_int = 20;
            const ARMOR_ICON_Y: c_int = 110;
            if cl.items & IT_INVULNERABILITY != 0 {
                quake_rs_sbar_draw_num(cbx, ARMOR_NUM_X, ARMOR_NUM_Y, 666, 3, 1);
                quake_rs_sbar_draw_pic(
                    cbx,
                    ARMOR_ICON_X,
                    ARMOR_ICON_Y,
                    ptr::addr_of!(g::draw_disc).read(),
                );
            } else if cl.stats[STAT_ARMOR] > 0 {
                if rogue() {
                    quake_rs_sbar_draw_num(
                        cbx,
                        ARMOR_NUM_X,
                        ARMOR_NUM_Y,
                        cl.stats[STAT_ARMOR],
                        3,
                        (cl.stats[STAT_ARMOR] <= 25) as c_int,
                    );
                    if cl.items & RIT_ARMOR3 != 0 {
                        quake_rs_sbar_draw_pic(cbx, ARMOR_ICON_X, ARMOR_ICON_Y, SB_ARMOR[2]);
                    } else if cl.items & RIT_ARMOR2 != 0 {
                        quake_rs_sbar_draw_pic(cbx, ARMOR_ICON_X, ARMOR_ICON_Y, SB_ARMOR[1]);
                    } else if cl.items & RIT_ARMOR1 != 0 {
                        quake_rs_sbar_draw_pic(cbx, ARMOR_ICON_X, ARMOR_ICON_Y, SB_ARMOR[0]);
                    }
                } else {
                    quake_rs_sbar_draw_num(
                        cbx,
                        ARMOR_NUM_X,
                        ARMOR_NUM_Y,
                        cl.stats[STAT_ARMOR],
                        3,
                        (cl.stats[STAT_ARMOR] <= 25) as c_int,
                    );
                    if cl.items & IT_ARMOR3 != 0 {
                        quake_rs_sbar_draw_pic(cbx, ARMOR_ICON_X, ARMOR_ICON_Y, SB_ARMOR[2]);
                    } else if cl.items & IT_ARMOR2 != 0 {
                        quake_rs_sbar_draw_pic(cbx, ARMOR_ICON_X, ARMOR_ICON_Y, SB_ARMOR[1]);
                    } else if cl.items & IT_ARMOR1 != 0 {
                        quake_rs_sbar_draw_pic(cbx, ARMOR_ICON_X, ARMOR_ICON_Y, SB_ARMOR[0]);
                    }
                }
            }
        }

        if gc::scr_viewsize.value < 110.0 {
            let powerup_icon_y: u8 =
                if cl.stats[STAT_ARMOR] > 0 || cl.items & IT_INVULNERABILITY != 0 {
                    93
                } else {
                    118
                };
            // powerups
            #[allow(clippy::needless_range_loop)] // the index is also the item bit
            for i in 2..6usize {
                if cl.items & (1 << (17 + i)) != 0 {
                    quake_rs_sbar_draw_pic(
                        cbx,
                        20 + offset as c_int,
                        powerup_icon_y as c_int,
                        SB_ITEMS[i],
                    );
                    offset = offset.wrapping_add(17);
                }
            }
            //  hipnotic items
            if hipnotic() {
                #[allow(clippy::needless_range_loop)] // the index is also the item bit
                for i in 0..2usize {
                    if cl.items & (1 << (24 + i)) != 0 {
                        quake_rs_sbar_draw_pic(
                            cbx,
                            20 + offset as c_int,
                            powerup_icon_y as c_int,
                            HSB_ITEMS[i],
                        );
                        offset = offset.wrapping_add(17);
                    }
                }
            }
            // new rogue items
            if rogue() {
                #[allow(clippy::needless_range_loop)] // the index is also the item bit
                for i in 0..2usize {
                    if cl.items & (1 << (29 + i)) != 0 {
                        quake_rs_sbar_draw_pic(
                            cbx,
                            20 + offset as c_int,
                            powerup_icon_y as c_int,
                            RSB_ITEMS[i],
                        );
                        offset = offset.wrapping_add(17);
                    }
                }
            }
        }

        if cl.maxclients != 1 {
            gc::GL_SetCanvas(cbx, CANVAS_TOPLEFT);
            quake_rs_sbar_mini_deathmatch_overlay(cbx);
        }

        {
            gc::GL_SetCanvas(cbx, CANVAS_BOTTOMRIGHT);
            quake_rs_sbar_draw_num(
                cbx,
                195,
                135,
                cl.stats[STAT_AMMO],
                3,
                (cl.stats[STAT_AMMO] <= 10) as c_int,
            );

            // ammo icon
            const AMMO_ICON_X: c_int = 280;
            const AMMO_ICON_Y: c_int = 135;
            if rogue() {
                if cl.items & RIT_SHELLS != 0 {
                    quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[0]);
                } else if cl.items & RIT_NAILS != 0 {
                    quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[1]);
                } else if cl.items & RIT_ROCKETS != 0 {
                    quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[2]);
                } else if cl.items & RIT_CELLS != 0 {
                    quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[3]);
                } else if cl.items & RIT_LAVA_NAILS != 0 {
                    quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, RSB_AMMO[0]);
                } else if cl.items & RIT_PLASMA_AMMO != 0 {
                    quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, RSB_AMMO[1]);
                } else if cl.items & RIT_MULTI_ROCKETS != 0 {
                    quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, RSB_AMMO[2]);
                }
            } else if cl.items & IT_SHELLS != 0 {
                quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[0]);
            } else if cl.items & IT_NAILS != 0 {
                quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[1]);
            } else if cl.items & IT_ROCKETS != 0 {
                quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[2]);
            } else if cl.items & IT_CELLS != 0 {
                quake_rs_sbar_draw_pic(cbx, AMMO_ICON_X, AMMO_ICON_Y, SB_AMMO[3]);
            }
        }

        {
            let key_icon_y: u8 = if gc::scr_viewsize.value >= 110.0 {
                118
            } else {
                97
            };
            let key_icon_hipnotic_y: u8 = key_icon_y + 7;
            offset = 0;
            // sigils
            if !rogue() && !hipnotic() {
                let mut i: u8 = 3;
                loop {
                    // COMPAT: `1u << (28 + i)` reaches bit 31 -- see
                    // Sbar_DrawInventory's sigil loop.
                    if (cl.items as u32) & (1u32 << (28 + i as u32)) != 0 {
                        quake_rs_sbar_draw_pic(
                            cbx,
                            296 - offset as c_int,
                            key_icon_y as c_int,
                            SB_SIGIL[i as usize],
                        );
                        offset = offset.wrapping_add(8);
                    }
                    if i == 0 {
                        break;
                    }
                    i -= 1;
                }
            }
            // keys
            #[allow(clippy::needless_range_loop)] // the index is also the key bit
            for i in 0..2usize {
                if cl.items & (1 << (17 + i)) != 0 {
                    if offset != 0 {
                        offset = offset.wrapping_add(1);
                    }
                    if !hipnotic() {
                        quake_rs_sbar_draw_pic(
                            cbx,
                            288 - offset as c_int,
                            key_icon_y as c_int,
                            SB_ITEMS[i],
                        );
                        offset = offset.wrapping_add(16);
                    } else {
                        quake_rs_sbar_draw_pic(
                            cbx,
                            288,
                            key_icon_hipnotic_y as c_int - offset as c_int,
                            SB_ITEMS[i],
                        );
                        offset = offset.wrapping_add(9);
                    }
                }
            }
        }

        if gc::scr_viewsize.value < 110.0 {
            let invpic = quake_rs_sbar_inventory_bar_pic();
            let mut weaponpic: *mut c_void = ptr::null_mut();
            offset = 0;
            let mut skip: u8 = 255;
            // ammo counts
            for i in 0..2usize {
                g::Draw_SubPic(
                    cbx,
                    200.0,
                    148.0 - 10.0 * i as f32,
                    104.0,
                    10.0,
                    invpic,
                    i as f32 * (2.0 * 48.0 / 320.0f32),
                    0.0f32,
                    2.0 * 48.0 / 320.0f32,
                    10.0 / 24.0f32,
                    ptr::null_mut(),
                    g::scr_sbaralpha.value,
                );
            }

            for i in 0..4usize {
                quake_rs_sbar_draw_small_ammo_counter(
                    cbx,
                    211 + 52 * (i as c_int & 1),
                    124 - 10 * (i as c_int >> 1),
                    cl.stats[STAT_SHELLS + i],
                );
            }

            gc::GL_SetCanvas(cbx, CANVAS_TOPRIGHT);

            if rogue() {
                // check for powered up weapon.
                if cl.stats[STAT_ACTIVEWEAPON] >= RIT_LAVA_NAILGUN {
                    for i in 0..5u8 {
                        if cl.stats[STAT_ACTIVEWEAPON] == (RIT_LAVA_NAILGUN << i) {
                            skip = i + 2;
                        }
                    }
                }
            }
            // weapons
            for i in 0..7u8 {
                let mut x_active: u8 = 0;

                // COMPAT: `sbar.c:1195` is `goto altammo`, which jumps *into*
                // the `if (cl.items & currentweapon)` block at `:1200` and
                // then falls through to the Sbar_DrawPic at `:1211`. The flag
                // below reproduces that control flow exactly: on the `skip`
                // path `currentweapon` is never computed and `cl.items` is
                // never tested, but the pic is still drawn and `offset` still
                // advances.
                let mut draw = false;
                if i == skip {
                    weaponpic = RSB_WEAPONS[(i - 2) as usize];
                    x_active = if i != 6 {
                        8
                    } else if hipnotic() {
                        16
                    } else {
                        32
                    };
                    draw = true;
                } else {
                    let currentweapon = IT_SHOTGUN << i;
                    if cl.items & currentweapon != 0 {
                        let f = quake_rs_sbar_calculate_flash_on(i as c_int);
                        weaponpic = SB_WEAPONS[f as usize][i as usize];
                        if cl.stats[STAT_ACTIVEWEAPON] == currentweapon {
                            x_active = if i != 6 {
                                8
                            } else if hipnotic() {
                                16
                            } else {
                                32
                            };
                        }
                        draw = true;
                    }
                }
                if draw {
                    quake_rs_sbar_draw_pic(
                        cbx,
                        304 - x_active as c_int,
                        120 - offset as c_int,
                        weaponpic,
                    );
                    offset = offset.wrapping_add(16);
                }
                if i == 4 && hipnotic() && cl.items & HIT_PROXIMITY_GUN != 0 {
                    let x_active: c_int = if cl.stats[STAT_ACTIVEWEAPON] == HIT_PROXIMITY_GUN {
                        8
                    } else {
                        0
                    };
                    let f = quake_rs_sbar_calculate_flash_on(HIPWEAPONS[3]);
                    quake_rs_sbar_draw_pic(
                        cbx,
                        304 - x_active,
                        120 - offset as c_int,
                        HSB_WEAPONS[f as usize][3],
                    );
                    offset = offset.wrapping_add(16);
                }
            }
            // hipnotic weapons
            if hipnotic() {
                for i in 0..2usize {
                    let mut x_active: c_int = 0;
                    let currentweapon = 1 << HIPWEAPONS[i];

                    if cl.items & currentweapon != 0 {
                        if cl.stats[STAT_ACTIVEWEAPON] == currentweapon {
                            x_active = 8;
                        }
                        let f = quake_rs_sbar_calculate_flash_on(HIPWEAPONS[i]);
                        quake_rs_sbar_draw_pic(
                            cbx,
                            304 - x_active,
                            120 - offset as c_int,
                            HSB_WEAPONS[f as usize][i],
                        );
                        offset = offset.wrapping_add(16);
                    }
                }
            }
        }

        gc::GL_SetCanvas(cbx, CANVAS_SBAR);
        if g::sb_showscores || cl.stats[STAT_HEALTH] <= 0 {
            // johnfitz -- scr_sbaralpha
            quake_rs_sbar_draw_pic_alpha(cbx, 0, 0, SB_SCOREBAR, g::scr_sbaralpha.value);
            quake_rs_sbar_draw_scoreboard(cbx);
        }
    }
}

/// `sbar.c:1262` -- `Sbar_Draw`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_draw(cbx: *mut c_void) -> Raise {
    // SAFETY: mirrors sbar.c:1262.
    unsafe {
        if gc::scr_con_current == c::cl_parse::vid.height as f32 {
            return gh::HOST_GUARD_OK; // console is full screen
        }

        if g::scr_style.value < 1.0f32
            && cl.qcvm.extfuncs.csqc_draw_hud != 0
            && ptr::addr_of!(c::qcvm).read().is_null()
        {
            return draw_cscq(cbx);
        }

        if cl.intermission != 0 {
            return gh::HOST_GUARD_OK; // johnfitz -- never draw sbar during intermission
        }

        gc::GL_SetCanvas(cbx, CANVAS_DEFAULT); // johnfitz

        // johnfitz -- don't waste fillrate by clearing the area behind the sbar
        let w = clamp_f32(
            320.0f32,
            g::scr_sbarscale.value * 320.0f32,
            gc::glwidth as f32,
        );
        if g::sb_lines != 0 && gc::glwidth as f32 > w {
            if g::scr_sbaralpha.value < 1.0 {
                g::Draw_TileClear(cbx, 0, gc::glheight - g::sb_lines, gc::glwidth, g::sb_lines);
            }
            if cl.gametype == GAME_DEATHMATCH {
                g::Draw_TileClear(
                    cbx,
                    w as c_int,
                    gc::glheight - g::sb_lines,
                    (gc::glwidth as f32 - w) as c_int,
                    g::sb_lines,
                );
            } else {
                g::Draw_TileClear(
                    cbx,
                    0,
                    gc::glheight - g::sb_lines,
                    ((gc::glwidth as f32 - w) / 2.0f32) as c_int,
                    g::sb_lines,
                );
                g::Draw_TileClear(
                    cbx,
                    ((gc::glwidth as f32 - w) / 2.0f32 + w) as c_int,
                    gc::glheight - g::sb_lines,
                    ((gc::glwidth as f32 - w) / 2.0f32) as c_int,
                    g::sb_lines,
                );
            }
        }
        // johnfitz

        if g::scr_style.value < 2.0f32 {
            draw_classic(cbx);
        } else {
            draw_modern(cbx);
        }

        gh::HOST_GUARD_OK
    }
}

// ---------------------------------------------------------------------------
// sbar.c:1309-1518 -- intermission and deathmatch overlays

/// `sbar.c:1309` -- `Sbar_IntermissionNumber`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_intermission_number(
    cbx: *mut c_void,
    mut x: c_int,
    y: c_int,
    num: c_int,
    digits: c_int,
    color: c_int,
) {
    // SAFETY: mirrors sbar.c:1309.
    unsafe {
        let mut str_ = [0 as c_char; 12];

        let l = quake_rs_sbar_itoa(num, str_.as_mut_ptr());
        let mut ptr_ = str_.as_ptr();
        if l > digits {
            ptr_ = ptr_.add((l - digits) as usize);
        }
        if l < digits {
            x += (digits - l) * 24;
        }

        while ptr_.read() != 0 {
            let frame = if ptr_.read() == b'-' as c_char {
                STAT_MINUS
            } else {
                ptr_.read() as c_int - b'0' as c_int
            };

            // johnfitz -- stretched menus
            gc::Draw_Pic(
                cbx,
                x as f32,
                y as f32,
                SB_NUMS[color as usize][frame as usize],
                1.0f32,
                false,
            );
            x += 24;
            ptr_ = ptr_.add(1);
        }
    }
}

/// `sbar.c:1340` -- `Sbar_IntermissionPicForChar`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_intermission_pic_for_char(
    ch: c_char,
    color: c_int,
) -> *mut c_void {
    // SAFETY: mirrors sbar.c:1340.
    unsafe {
        // COMPAT: `(unsigned)(c - '0') < 10` -- the unsigned wraparound is
        // what rejects every character below '0' in a single test.
        let digit = (ch as c_int).wrapping_sub(b'0' as c_int) as u32;
        if digit < 10 {
            return SB_NUMS[color as usize][digit as usize];
        }
        if ch == b'/' as c_char {
            return SB_SLASH;
        }
        if ch == b':' as c_char {
            return SB_COLON;
        }
        if ch == b'-' as c_char {
            return SB_NUMS[color as usize][STAT_MINUS as usize];
        }
        ptr::null_mut()
    }
}

/// `sbar.c:1356` -- `Sbar_IntermissionTextWidth`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_intermission_text_width(
    mut str_: *const c_char,
    color: c_int,
) -> c_int {
    // SAFETY: mirrors sbar.c:1356; `str_` is NUL-terminated.
    unsafe {
        let mut len = 0;
        while str_.read() != 0 {
            let pic = quake_rs_sbar_intermission_pic_for_char(str_.read(), color);
            str_ = str_.add(1);
            len += if pic.is_null() { 24 } else { pic_width(pic) };
        }
        len
    }
}

/// `sbar.c:1373` -- `Sbar_IntermissionText`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_intermission_text(
    cbx: *mut c_void,
    mut x: c_int,
    y: c_int,
    mut str_: *const c_char,
    color: c_int,
) {
    // SAFETY: mirrors sbar.c:1373.
    unsafe {
        while str_.read() != 0 {
            let pic = quake_rs_sbar_intermission_pic_for_char(str_.read(), color);
            str_ = str_.add(1);
            if pic.is_null() {
                continue;
            }
            gc::Draw_Pic(cbx, x as f32, y as f32, pic, 1.0f32, false);
            x += pic_width(pic);
        }
    }
}

/// `sbar.c:1390` -- `Sbar_DeathmatchOverlay`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_deathmatch_overlay(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:1390.
    unsafe {
        let mut num = [0 as c_char; 12];

        gc::GL_SetCanvas(cbx, CANVAS_MENU); // johnfitz

        let pic = g::Draw_CachePic(c"gfx/ranking.lmp".as_ptr());
        g::M_DrawPic(cbx, (320 - pic_width(pic)) / 2, 8, pic);

        // scores
        quake_rs_sbar_sort_frags();

        // draw the text
        let l = g::scoreboardlines;

        // johnfitz -- simplified becuase some positioning is handled elsewhere
        let x = 80;
        let mut y = 40;
        for i in 0..l {
            let k = g::fragsort[i as usize];
            let s = score(k);
            if ptr::addr_of!((*s).name[0]).read() == 0 {
                continue;
            }

            // draw background
            let colors = ptr::addr_of!((*s).colors).read();
            let top = quake_rs_sbar_color_for_map(colors & 0xf0);
            let bottom = quake_rs_sbar_color_for_map((colors & 15) << 4);

            // johnfitz -- stretched overlays
            gc::Draw_Fill(cbx, x as f32, y as f32, 40.0, 4.0, top, 1.0);
            gc::Draw_Fill(cbx, x as f32, (y + 4) as f32, 40.0, 4.0, bottom, 1.0);

            // draw number
            let f = ptr::addr_of!((*s).frags).read();
            g::q_snprintf(num.as_mut_ptr(), num.len(), c"%3i".as_ptr(), f);

            // johnfitz -- stretched overlays
            gc::Draw_Character(cbx, (x + 8) as f32, y as f32, num[0] as c_int);
            gc::Draw_Character(cbx, (x + 16) as f32, y as f32, num[1] as c_int);
            gc::Draw_Character(cbx, (x + 24) as f32, y as f32, num[2] as c_int);

            if k == cl.viewentity - 1 {
                gc::Draw_Character(cbx, (x - 8) as f32, y as f32, 12);
            }

            // draw name
            // johnfitz -- was Draw_String, changed for stretched overlays
            g::M_Print(cbx, x + 64, y, ptr::addr_of!((*s).name).cast::<c_char>());

            y += 10;
        }

        gc::GL_SetCanvas(cbx, CANVAS_SBAR); // johnfitz
    }
}

/// `sbar.c:1471` -- `Sbar_MiniDeathmatchOverlay`.
///
/// COMPAT: `y`, `y_max` and `numlines` are `unsigned char` in the original
/// and `y += 8` therefore wraps at 256. The `u8` arithmetic below keeps that
/// exact behaviour rather than widening to `int`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_mini_deathmatch_overlay(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:1471.
    unsafe {
        let mut num = [0 as c_char; 12];

        // johnfitz
        let scale = clamp_f64(
            1.0,
            g::scr_sbarscale.value as f64,
            gc::glwidth as f32 as f64 / 320.0,
        ) as f32;

        // MAX_SCOREBOARDNAME = 32, so total width for this overlay plus sbar
        // is 632, but we can cut off some i guess
        // johnfitz -- test should consider scr_sbarscale
        if (gc::glwidth as f32 / scale < 512.0 && g::scr_style.value < 2.0f32)
            || gc::scr_viewsize.value >= 120.0
        {
            return;
        }

        // scores
        quake_rs_sbar_sort_frags();

        // draw the text
        // johnfitz
        let numlines: u8 = if gc::scr_viewsize.value >= 110.0 && g::scr_style.value < 2.0f32 {
            3
        } else {
            6
        };

        // find us
        let mut i = 0;
        while i < g::scoreboardlines {
            if g::fragsort[i as usize] == cl.viewentity - 1 {
                break;
            }
            i += 1;
        }
        if i == g::scoreboardlines {
            // we're not there
            i = 0;
        } else {
            // figure out start
            i -= numlines as c_int / 2;
        }
        if i > g::scoreboardlines - numlines as c_int {
            i = g::scoreboardlines - numlines as c_int;
        }
        if i < 0 {
            i = 0;
        }

        let x: c_int;
        let mut y: u8;
        let y_max: u8;
        if g::scr_style.value < 2.0f32 {
            x = 324;
            y = if gc::scr_viewsize.value >= 110.0 {
                24
            } else {
                0
            };
            y_max = 48;
        } else {
            x = 0;
            y = 112;
            y_max = 160;
        }

        // johnfitz -- change y init, test, inc
        while i < g::scoreboardlines && y < y_max {
            let k = g::fragsort[i as usize];
            let s = score(k);
            if ptr::addr_of!((*s).name[0]).read() == 0 {
                i += 1;
                y = y.wrapping_add(8);
                continue;
            }

            // colors
            let colors = ptr::addr_of!((*s).colors).read();
            let top = quake_rs_sbar_color_for_map(colors & 0xf0);
            let bottom = quake_rs_sbar_color_for_map((colors & 15) << 4);

            gc::Draw_Fill(cbx, x as f32, (y as c_int + 1) as f32, 40.0, 4.0, top, 1.0);
            gc::Draw_Fill(
                cbx,
                x as f32,
                (y as c_int + 5) as f32,
                40.0,
                3.0,
                bottom,
                1.0,
            );

            // number
            let f = ptr::addr_of!((*s).frags).read();
            g::q_snprintf(num.as_mut_ptr(), num.len(), c"%3i".as_ptr(), f);
            gc::Draw_Character(cbx, (x + 8) as f32, y as c_int as f32, num[0] as c_int);
            gc::Draw_Character(cbx, (x + 16) as f32, y as c_int as f32, num[1] as c_int);
            gc::Draw_Character(cbx, (x + 24) as f32, y as c_int as f32, num[2] as c_int);

            // brackets
            if k == cl.viewentity - 1 {
                gc::Draw_Character(cbx, x as f32, y as c_int as f32, 16);
                gc::Draw_Character(cbx, (x + 32) as f32, y as c_int as f32, 17);
            }

            // name
            if g::scr_style.value < 2.0f32 {
                gc::Draw_String(
                    cbx,
                    (x + 48) as f32,
                    y as c_int as f32,
                    ptr::addr_of!((*s).name).cast::<c_char>(),
                );
            }

            i += 1;
            y = y.wrapping_add(8);
        }
    }
}

/// `sbar.c:1559` -- `Sbar_IntermissionOverlay`.
///
/// COMPAT: ADR-008/ADR-009. As in `Sbar_DrawCSCQ`, a raise from
/// `PR_ExecuteProgram` skips the `PR_SwitchQCVM (NULL)` at `sbar.c:1591` and
/// leaves the client qcvm selected. The `raise!` below reproduces that.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_intermission_overlay(cbx: *mut c_void) -> Raise {
    // SAFETY: mirrors sbar.c:1559.
    unsafe {
        let mut time = [0 as c_char; 32];
        let mut secrets = [0 as c_char; 32];
        let mut monsters = [0 as c_char; 32];

        if g::scr_style.value < 1.0f32
            && cl.qcvm.extfuncs.csqc_draw_scores != 0
            && ptr::addr_of!(c::qcvm).read().is_null()
        {
            let s = clamp_f64(
                1.0,
                g::scr_sbarscale.value as f64,
                gc::glwidth as f32 as f64 / 320.0,
            ) as f32;
            gc::GL_SetCanvas(cbx, CANVAS_CSQC);
            gs::PR_SwitchQCVM(ptr::addr_of_mut!(cl.qcvm).cast());
            let cur = vm();
            let cltime = ptr::addr_of!((*cur).extglobals.cltime).read();
            if !cltime.is_null() {
                cltime.write(gc::realtime as f32);
            }
            let clframetime = ptr::addr_of!((*cur).extglobals.clframetime).read();
            if !clframetime.is_null() {
                clframetime.write(c::host_frametime as f32);
            }
            let localentnum = ptr::addr_of!((*cur).extglobals.player_localentnum).read();
            if !localentnum.is_null() {
                localentnum.write(cl.viewentity as f32);
            }
            let intermission = ptr::addr_of!((*cur).extglobals.intermission).read();
            if !intermission.is_null() {
                intermission.write(cl.intermission as f32);
            }
            let intermission_time = ptr::addr_of!((*cur).extglobals.intermission_time).read();
            if !intermission_time.is_null() {
                intermission_time.write(cl.completed_time as f32);
            }
            ptr::addr_of_mut!((*pgs()).time).write(cl.time as f32);
            ptr::addr_of_mut!((*pgs()).frametime).write(c::host_frametime as f32);
            quake_rs_sbar_sort_frags();
            vectorset(
                OFS_PARM0,
                c::cl_parse::vid.width as f32 / s,
                c::cl_parse::vid.height as f32 / s,
                0.0,
            );
            globals()
                .add(OFS_PARM1)
                .write(g::sb_showscores as c_int as f32);
            raise!(gh::Host_Glue_PR_ExecuteProgram(
                cl.qcvm.extfuncs.csqc_draw_scores as c_int
            ));
            gs::PR_SwitchQCVM(ptr::null_mut());
            return gh::HOST_GUARD_OK;
        }

        if cl.gametype == GAME_DEATHMATCH {
            quake_rs_sbar_deathmatch_overlay(cbx);
            return gh::HOST_GUARD_OK;
        }

        gc::GL_SetCanvas(cbx, CANVAS_MENU); // johnfitz

        g::q_snprintf(
            time.as_mut_ptr(),
            time.len(),
            c"%d:%02d".as_ptr(),
            cl.completed_time / 60,
            cl.completed_time % 60,
        );
        g::q_snprintf(
            secrets.as_mut_ptr(),
            secrets.len(),
            c"%d/%2d".as_ptr(),
            cl.stats[STAT_SECRETS],
            cl.stats[STAT_TOTALSECRETS],
        );
        g::q_snprintf(
            monsters.as_mut_ptr(),
            monsters.len(),
            c"%d/%2d".as_ptr(),
            cl.stats[STAT_MONSTERS],
            cl.stats[STAT_TOTALMONSTERS],
        );

        let ltime = quake_rs_sbar_intermission_text_width(time.as_ptr(), 0);
        let lsecrets = quake_rs_sbar_intermission_text_width(secrets.as_ptr(), 0);
        let lmonsters = quake_rs_sbar_intermission_text_width(monsters.as_ptr(), 0);

        let mut total = ltime.max(lsecrets);
        total = lmonsters.max(total);

        let pic = g::Draw_CachePic(c"gfx/inter.lmp".as_ptr());
        total += pic_width(pic) + 24;
        total = total.min(320);
        gc::Draw_Pic(cbx, (160 - total / 2) as f32, 56.0, pic, 1.0f32, false);

        let pic = g::Draw_CachePic(c"gfx/complete.lmp".as_ptr());
        gc::Draw_Pic(
            cbx,
            (160 - pic_width(pic) / 2) as f32,
            24.0,
            pic,
            1.0f32,
            false,
        );

        quake_rs_sbar_intermission_text(cbx, 160 + total / 2 - ltime, 64, time.as_ptr(), 0);
        quake_rs_sbar_intermission_text(cbx, 160 + total / 2 - lsecrets, 104, secrets.as_ptr(), 0);
        quake_rs_sbar_intermission_text(
            cbx,
            160 + total / 2 - lmonsters,
            144,
            monsters.as_ptr(),
            0,
        );

        gh::HOST_GUARD_OK
    }
}

/// `sbar.c:1629` -- `Sbar_FinaleOverlay`.
///
/// # Safety
/// C ABI entry point. `Sbar_Init` must have run, the client state must be
/// valid, and any pointer argument must be a valid C string or object of
/// the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sbar_finale_overlay(cbx: *mut c_void) {
    // SAFETY: mirrors sbar.c:1629.
    unsafe {
        gc::GL_SetCanvas(cbx, CANVAS_MENU); // johnfitz

        let pic = g::Draw_CachePic(c"gfx/finale.lmp".as_ptr());
        // johnfitz -- stretched menus
        gc::Draw_Pic(
            cbx,
            ((320 - pic_width(pic)) / 2) as f32,
            16.0,
            pic,
            1.0f32,
            false,
        );
    }
}
