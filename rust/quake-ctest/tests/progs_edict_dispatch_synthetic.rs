//! `quake_rs_ed_parse_globals` / `quake_rs_ed_parse_edict` synthetic-VM unit
//! tests (Rust migration Phase 7 M5 T5.2).
//!
//! `Quake/pr_edict.c` is not in `rust/quake-ctest/build.rs`'s `C_SOURCES`
//! (the contract forbids adding it -- far larger than T5.2), so there is no
//! `c_ref_ED_ParseGlobals`/`c_ref_ED_ParseEdict` oracle to differential
//! against, unlike `progs_parse_differential.rs`'s `ED_ParseEpair`/
//! `ED_NewString` coverage. These are synthetic-only unit tests instead: they
//! drive the compiled `quake_rs_ed_parse_globals`/`quake_rs_ed_parse_edict`
//! (`rust/quake-capi/src/progs_edict_dispatch.rs`) directly over the real
//! ambient-`qcvm` C ABI, the same way `sv_phys_differential.rs`/
//! `sv_move_differential.rs` drive `quake-capi`'s world/physics shims.
//!
//! Two fixture families:
//! - the plain synthetic VM (`ctest_progs_synth_vm`/`select_vm`, fixture 0)
//!   for the ordinary key/value parsing paths and everything gated on `qcvm
//!   != &sv.qcvm` (COMPAT: the zoned-string path, and the `_precache_*`/
//!   `traileffect`/`emiteffect` hacks being *inert* off the server VM);
//! - `ctest_world_reset(2, ...)` (which publishes `&sv.qcvm` itself as the
//!   ambient `qcvm`) plus `ctest_predd_set_defs`/`ctest_predd_set_strings`
//!   for the branches that require `PRLoad_Glue_IsServerVM` to hold: the
//!   `_precache_model`/`_precache_sound`/`traileffect`/`emiteffect` hacks.
//!
//! Requires `quake-capi`'s `progs` feature enabled alongside `host` (off by
//! default in `rust/quake-ctest/Cargo.toml` as of this writing -- see the
//! T5.2 manifest; `quake_rs_ed_parse_globals`/`quake_rs_ed_parse_edict` and
//! the `PREdictDispatch_Glue_*`/`COM_ParseEx` symbols this file's `extern
//! "C"` block declares do not exist in the linked binary without it).

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;

use quake_ctest as _;
use quake_progs::arena::{EdictArena, EdictId, VmRaw};
use quake_types::progs::{etype, DDef, QcVm};

extern "C" {
    fn ctest_progs_synth_vm(
        which: c_int,
        max_edicts: c_int,
        entityfields: c_int,
        numglobals: c_int,
        stmts: *const c_void,
        nstmts: c_int,
        funcs: *const c_void,
        nfuncs: c_int,
        strings: *const c_char,
        stringssize: c_int,
    ) -> *mut c_void;
    fn ctest_progs_select_vm(which: c_int);
    fn ctest_progs_vm(which: c_int) -> *mut c_void;
    fn ctest_progs_synth_free();
    fn ctest_progs_set_defs(
        which: c_int,
        fielddefs: *const DDef,
        numfielddefs: c_int,
        globaldefs: *const DDef,
        numglobaldefs: c_int,
        extfields_alpha: c_int,
    );
    fn ctest_progs_set_sv_state(active: c_int);

    fn ctest_world_reset(vm_kind: c_int, num_edicts: c_int);

    fn ctest_predd_set_defs(
        fielddefs: *const DDef,
        numfielddefs: c_int,
        globaldefs: *const DDef,
        numglobaldefs: c_int,
        extfields_alpha: c_int,
        extfields_traileffectnum: c_int,
        extfields_emiteffectnum: c_int,
    );
    fn ctest_predd_set_strings(data: *const c_void, size: c_int);
    fn ctest_predd_set_model_error(should_error: bool);
    fn ctest_predd_get_last_model() -> *const c_char;
    fn ctest_predd_get_last_sound() -> *const c_char;
    fn ctest_predd_get_last_particle() -> *const c_char;
    fn ctest_predd_get_particle_calls() -> c_int;
    fn ctest_predd_reset_doubles();

    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;

    fn quake_rs_ed_parse_globals(
        data: *const c_char,
        out_data: *mut *const c_char,
        detail: *mut c_int,
    ) -> c_int;
    fn quake_rs_ed_parse_edict(
        data: *const c_char,
        edict_num: c_int,
        out_data: *mut *const c_char,
        detail: *mut c_int,
    ) -> c_int;
}

/// Mirrors `rust/quake-capi/src/progs_edict_dispatch.rs`'s private `PREDD_*`
/// constants (kept in sync with the T5.2 manifest's `pr_edict.c` rewrite).
const PREDD_OK: c_int = 0;
const PREDD_ERR_EOF: c_int = 1;
const PREDD_ERR_CLOSE_NO_DATA: c_int = 2;
const PREDD_ERR_ENTITY_RANGE: c_int = 4;
/// `stubs.c`'s `Host_Guard`: `CTEST_GUARD_HOST_ERROR`, not the real
/// `HOST_GUARD_*`/`ABORTSERVER` -- see stubs.c's own Host_Guard doc comment.
const PREDD_ERR_GUARD: c_int = 8;
const CTEST_GUARD_HOST_ERROR: c_int = 1;

static VM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    VM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

const MAXE: c_int = 8;
const ENTFIELDS: c_int = 64;
const NUMGLOBALS: c_int = 64;

fn def(ty: c_int, ofs: u16, s_name: c_int) -> DDef {
    DDef {
        type_: ty as u16,
        ofs,
        s_name,
    }
}

/// Appends `s` plus a NUL terminator to `blob`, returning the offset it
/// starts at (mirrors how a real progs' string table is laid out: a leading
/// NUL byte reserves offset 0 as "no string").
fn intern(blob: &mut Vec<u8>, s: &str) -> c_int {
    let ofs = blob.len() as c_int;
    blob.extend_from_slice(s.as_bytes());
    blob.push(0);
    ofs
}

fn con_log_contains(needle: &str) -> bool {
    // SAFETY: ctest_con_log_get returns a NUL-terminated buffer for every
    // index below ctest_con_log_len.
    unsafe {
        (0..ctest_con_log_len()).any(|i| {
            CStr::from_ptr(ctest_con_log_get(i))
                .to_string_lossy()
                .contains(needle)
        })
    }
}

fn get_string(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the ctest_predd_get_last_* doubles always return a
    // NUL-terminated static buffer.
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

// ---------------------------------------------------------------------------
// Plain synthetic-VM fixture (fixture 0): `qcvm != &sv.qcvm`.

fn setup_plain(fielddefs: &[DDef], globaldefs: &[DDef], strings: &[u8]) {
    // SAFETY: the caller holds VM_LOCK; the slices outlive the copy.
    unsafe {
        ctest_progs_synth_vm(
            0,
            MAXE,
            ENTFIELDS,
            NUMGLOBALS,
            core::ptr::null(),
            0,
            core::ptr::null(),
            0,
            strings.as_ptr().cast::<c_char>(),
            strings.len() as c_int,
        );
        ctest_progs_set_defs(
            0,
            fielddefs.as_ptr(),
            fielddefs.len() as c_int,
            globaldefs.as_ptr(),
            globaldefs.len() as c_int,
            -1,
        );
        ctest_progs_select_vm(0);
    }
    // SAFETY: as above.
    unsafe { ctest_clear_con_log() };
}

fn teardown_plain() {
    // SAFETY: the caller holds VM_LOCK and no view outlives this.
    unsafe { ctest_progs_synth_free() };
}

fn vm_plain() -> VmRaw {
    // SAFETY: fixture 0 is live between setup_plain() and teardown_plain().
    unsafe { VmRaw::new(ctest_progs_vm(0).cast::<QcVm>()) }
}

fn arena_plain() -> EdictArena {
    let vm = vm_plain();
    // SAFETY: the fixture allocated max_edicts * edict_size bytes.
    unsafe { EdictArena::borrowed(vm.edicts_base(), vm.edict_stride() as usize, MAXE as usize) }
}

// ---------------------------------------------------------------------------
// Server-VM fixture: `qcvm == &sv.qcvm`, via ctest_world_reset(2, ...).

fn setup_sv(
    fielddefs: &[DDef],
    extfields_traileffectnum: c_int,
    extfields_emiteffectnum: c_int,
    strings: &[u8],
) {
    // SAFETY: the caller holds VM_LOCK.
    unsafe {
        ctest_world_reset(2, MAXE);
        ctest_predd_set_strings(strings.as_ptr().cast::<c_void>(), strings.len() as c_int);
        ctest_predd_set_defs(
            fielddefs.as_ptr(),
            fielddefs.len() as c_int,
            core::ptr::null(),
            0,
            -1,
            extfields_traileffectnum,
            extfields_emiteffectnum,
        );
        ctest_progs_set_sv_state(0); // ss_loading
        ctest_predd_reset_doubles();
        ctest_clear_con_log();
    }
}

fn vm_sv() -> VmRaw {
    // SAFETY: ctest_world_reset(2, ...) just published &sv.qcvm as ambient.
    unsafe { VmRaw::new(quake_c_sys::qcvm.cast::<QcVm>()) }
}

fn arena_sv() -> EdictArena {
    let vm = vm_sv();
    // SAFETY: ctest_world_reset allocated max_edicts * edict_size bytes.
    unsafe { EdictArena::borrowed(vm.edicts_base(), vm.edict_stride() as usize, MAXE as usize) }
}

fn parse_globals(data: &str) -> (c_int, c_int) {
    let cs = CString::new(data).unwrap();
    let mut out_data: *const c_char = core::ptr::null();
    let mut detail: c_int = -999;
    // SAFETY: `cs` is NUL-terminated; `out_data`/`detail` are valid for one
    // write; the ambient qcvm was set up by setup_plain/setup_sv.
    let status = unsafe { quake_rs_ed_parse_globals(cs.as_ptr(), &mut out_data, &mut detail) };
    (status, detail)
}

fn parse_edict(data: &str, edict_num: c_int) -> (c_int, c_int) {
    let cs = CString::new(data).unwrap();
    let mut out_data: *const c_char = core::ptr::null();
    let mut detail: c_int = -999;
    // SAFETY: as above; `edict_num` is below max_edicts in every fixture
    // used here.
    let status =
        unsafe { quake_rs_ed_parse_edict(cs.as_ptr(), edict_num, &mut out_data, &mut detail) };
    (status, detail)
}

// ---------------------------------------------------------------------------
// ED_ParseGlobals

#[test]
fn parse_globals_known_values_written_to_globals_block() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_float = intern(&mut strings, "g_float");
    let s_str = intern(&mut strings, "g_str");
    setup_plain(
        &[],
        &[
            def(etype::EV_FLOAT, 10, s_float),
            def(etype::EV_STRING, 11, s_str),
        ],
        &strings,
    );

    let (status, _) = parse_globals("\"g_float\" \"1.5\" \"g_str\" \"hello\" }");
    assert_eq!(status, PREDD_OK);

    let vm = vm_plain();
    assert_eq!(vm.g_f32(10), 1.5);
    let handle = vm.g_i32(11);
    assert_eq!(vm.get_string_bytes(handle).unwrap(), b"hello");

    teardown_plain();
}

#[test]
fn parse_globals_unknown_global_warns_and_continues() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_float = intern(&mut strings, "g_float");
    setup_plain(&[], &[def(etype::EV_FLOAT, 10, s_float)], &strings);

    let (status, _) = parse_globals("\"nosuchglobal\" \"1\" \"g_float\" \"2\" }");
    assert_eq!(status, PREDD_OK);
    assert!(con_log_contains("'nosuchglobal' is not a global"));
    assert_eq!(vm_plain().g_f32(10), 2.0);

    teardown_plain();
}

#[test]
fn parse_globals_eof_without_close() {
    let _g = lock();
    setup_plain(&[], &[], &[0u8]);
    let (status, _) = parse_globals("\"a\" \"1\"");
    assert_eq!(status, PREDD_ERR_EOF);
    teardown_plain();
}

#[test]
fn parse_globals_close_no_data() {
    let _g = lock();
    setup_plain(&[], &[], &[0u8]);
    let (status, _) = parse_globals("\"a\" }");
    assert_eq!(status, PREDD_ERR_CLOSE_NO_DATA);
    teardown_plain();
}

#[test]
fn parse_globals_entity_range_passthrough() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_ent = intern(&mut strings, "g_ent");
    setup_plain(&[], &[def(etype::EV_ENTITY, 10, s_ent)], &strings);

    let (status, detail) = parse_globals("\"g_ent\" \"999999\" }");
    assert_eq!(status, PREDD_ERR_ENTITY_RANGE);
    assert_eq!(detail, 999999);

    teardown_plain();
}

// ---------------------------------------------------------------------------
// ED_ParseEdict -- plain fixture (qcvm != &sv.qcvm)

#[test]
fn parse_edict_known_field_written() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_health = intern(&mut strings, "health");
    setup_plain(&[def(etype::EV_FLOAT, 5, s_health)], &[], &strings);

    let (status, _) = parse_edict("\"health\" \"75\" }", 1);
    assert_eq!(status, PREDD_OK);
    assert_eq!(arena_plain().field_f32(EdictId(1), 5 * 4), 75.0);

    teardown_plain();
}

#[test]
fn parse_edict_unknown_field_warns_except_sky_fog_alpha() {
    let _g = lock();
    setup_plain(&[], &[], &[0u8]);

    let (status, _) = parse_edict(
        "\"foobar\" \"1\" \"skyfoo\" \"1\" \"fog\" \"1\" \"alpha\" \"1\" }",
        1,
    );
    assert_eq!(status, PREDD_OK);
    assert!(con_log_contains("\"foobar\" is not a field"));
    assert!(!con_log_contains("skyfoo"));
    assert!(!con_log_contains("\"fog\" is not a field"));
    assert!(!con_log_contains("\"alpha\" is not a field"));

    teardown_plain();
}

#[test]
fn parse_edict_alpha_hack_sets_arena_alpha() {
    let _g = lock();
    setup_plain(&[], &[], &[0u8]);

    let (status, _) = parse_edict("\"alpha\" \"0.5\" }", 1);
    assert_eq!(status, PREDD_OK);
    // ENTALPHA_ENCODE(0.5) = Q_rint(CLAMP(1, 0.5*254+1, 255)) = Q_rint(128) = 128
    assert_eq!(vm_plain().edict_alpha(1), 128);

    teardown_plain();
}

#[test]
fn parse_edict_anglehack_renames_and_bounds_the_value() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_angles = intern(&mut strings, "angles");
    setup_plain(&[def(etype::EV_VECTOR, 8, s_angles)], &[], &strings);

    let (status, _) = parse_edict("\"angle\" \"90\" }", 1);
    assert_eq!(status, PREDD_OK);
    let arena = arena_plain();
    assert_eq!(arena.field_f32(EdictId(1), 8 * 4), 0.0);
    assert_eq!(arena.field_f32(EdictId(1), 9 * 4), 90.0);
    assert_eq!(arena.field_f32(EdictId(1), 10 * 4), 0.0);

    teardown_plain();
}

#[test]
fn parse_edict_light_renamed_to_light_lev() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_light_lev = intern(&mut strings, "light_lev");
    setup_plain(&[def(etype::EV_FLOAT, 12, s_light_lev)], &[], &strings);

    let (status, _) = parse_edict("\"light\" \"128\" }", 1);
    assert_eq!(status, PREDD_OK);
    assert_eq!(arena_plain().field_f32(EdictId(1), 12 * 4), 128.0);
    assert!(!con_log_contains("\"light\" is not a field"));

    teardown_plain();
}

#[test]
fn parse_edict_wad_key_allows_truncation() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_health = intern(&mut strings, "health");
    setup_plain(&[def(etype::EV_FLOAT, 5, s_health)], &[], &strings);

    // COM_ParseEx(CPE_NOTRUNC) returns NULL (parse fails, as if EOF) for a
    // quoted value over COM_PARSE_MAX_TOKEN_SIZE-1 (4095) bytes;
    // CPE_ALLOWTRUNC truncates silently and keeps parsing (Quake/common.c's
    // COM_ParseEx, verbatim-copied in stubs.c). "wad" is the one key the
    // dispatcher parses in CPE_ALLOWTRUNC mode -- proven here by parsing
    // succeeding (and reaching the following "health" pair) instead of
    // aborting with PREDD_ERR_EOF.
    let overlong = "x".repeat(4200);
    let data = format!("\"wad\" \"{overlong}\" \"health\" \"9\" }}");
    let (status, _) = parse_edict(&data, 1);
    assert_eq!(status, PREDD_OK);
    assert_eq!(arena_plain().field_f32(EdictId(1), 5 * 4), 9.0);

    teardown_plain();
}

#[test]
fn parse_edict_eof_without_close() {
    let _g = lock();
    setup_plain(&[], &[], &[0u8]);
    let (status, _) = parse_edict("\"a\" \"1\"", 1);
    assert_eq!(status, PREDD_ERR_EOF);
    teardown_plain();
}

#[test]
fn parse_edict_close_no_data() {
    let _g = lock();
    setup_plain(&[], &[], &[0u8]);
    let (status, _) = parse_edict("\"a\" }", 1);
    assert_eq!(status, PREDD_ERR_CLOSE_NO_DATA);
    teardown_plain();
}

#[test]
fn parse_edict_entity_range_passthrough() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_enemy = intern(&mut strings, "enemy");
    setup_plain(&[def(etype::EV_ENTITY, 20, s_enemy)], &[], &strings);

    let (status, detail) = parse_edict("\"enemy\" \"999999\" }", 1);
    assert_eq!(status, PREDD_ERR_ENTITY_RANGE);
    assert_eq!(detail, 999999);

    teardown_plain();
}

#[test]
fn parse_edict_edict0_never_clears_fields_first() {
    let _g = lock();
    let mut strings = vec![0u8];
    let s_health = intern(&mut strings, "health");
    let s_armor = intern(&mut strings, "armor");
    setup_plain(
        &[
            def(etype::EV_FLOAT, 5, s_health),
            def(etype::EV_FLOAT, 6, s_armor),
        ],
        &[],
        &strings,
    );

    // pre-set a field on both edict 0 and edict 1 that this parse will not
    // touch (it only sets "armor"); edict 0 must keep it, edict 1 must not.
    {
        let mut arena = arena_plain();
        arena.set_field_f32(EdictId(0), 5 * 4, 42.0);
        arena.set_field_f32(EdictId(1), 5 * 4, 42.0);
    }

    let (s0, _) = parse_edict("\"armor\" \"1\" }", 0);
    assert_eq!(s0, PREDD_OK);
    let (s1, _) = parse_edict("\"armor\" \"1\" }", 1);
    assert_eq!(s1, PREDD_OK);

    let arena = arena_plain();
    assert_eq!(
        arena.field_f32(EdictId(0), 5 * 4),
        42.0,
        "hack, this way never clear edict 0 = world"
    );
    assert_eq!(
        arena.field_f32(EdictId(1), 5 * 4),
        0.0,
        "every other edict's field block is cleared before parsing"
    );

    teardown_plain();
}

#[test]
fn parse_edict_traileffect_ignored_off_server_vm() {
    let _g = lock();
    setup_plain(&[], &[], &[0u8]);

    let (status, _) = parse_edict("\"traileffect\" \"somefx\" }", 1);
    assert_eq!(status, PREDD_OK);
    assert!(con_log_contains("\"traileffect\" is not a field"));

    teardown_plain();
}

// ---------------------------------------------------------------------------
// ED_ParseEdict -- server-VM fixture (qcvm == &sv.qcvm, sv.state == ss_loading)

#[test]
fn parse_edict_precache_model_success_on_server_vm() {
    let _g = lock();
    setup_sv(&[], -1, -1, &[0u8]);
    // SAFETY: setup_sv just reset the model-error latch.
    unsafe { ctest_predd_set_model_error(false) };

    let (status, _) = parse_edict("\"_precache_model\" \"progs/soldier.mdl\" }", 1);
    assert_eq!(status, PREDD_OK);
    // SAFETY: reading the test double's recorded last call.
    let last_model = unsafe { ctest_predd_get_last_model() };
    assert_eq!(get_string(last_model), "progs/soldier.mdl");
}

#[test]
fn parse_edict_precache_model_guard_raises_on_server_vm() {
    let _g = lock();
    setup_sv(&[], -1, -1, &[0u8]);
    // SAFETY: arms SV_Precache_Model's Host_Error double.
    unsafe { ctest_predd_set_model_error(true) };

    let (status, detail) = parse_edict("\"_precache_model\" \"progs/missing.mdl\" }", 1);
    assert_eq!(status, PREDD_ERR_GUARD);
    assert_eq!(detail, CTEST_GUARD_HOST_ERROR);
}

#[test]
fn parse_edict_precache_sound_on_server_vm() {
    let _g = lock();
    setup_sv(&[], -1, -1, &[0u8]);

    let (status, _) = parse_edict("\"_precache_sound\" \"weapons/shotgn2.wav\" }", 1);
    assert_eq!(status, PREDD_OK);
    // SAFETY: as above.
    let last_sound = unsafe { ctest_predd_get_last_sound() };
    assert_eq!(get_string(last_sound), "weapons/shotgn2.wav");
}

#[test]
fn parse_edict_precache_keys_ignored_on_server_vm_when_not_loading() {
    let _g = lock();
    setup_sv(&[], -1, -1, &[0u8]);
    // SAFETY: both doubles were just reset by setup_sv. `sv.state` moves to
    // ss_active so the `sv.state == ss_loading` half of C's guard
    // (`Quake/pr_edict.c:836`) is the only thing that can suppress the
    // precache -- the qcvm == &sv.qcvm half still holds.
    unsafe {
        ctest_predd_set_model_error(true);
        ctest_progs_set_sv_state(1); // ss_active
    }

    let (status, _) = parse_edict(
        "\"_precache_model\" \"progs/soldier.mdl\" \"_precache_sound\" \"weapons/shotgn2.wav\" }",
        1,
    );
    assert_eq!(status, PREDD_OK);
    // SAFETY: reading the test doubles' recorded last calls.
    unsafe {
        assert_eq!(get_string(ctest_predd_get_last_model()), "");
        assert_eq!(get_string(ctest_predd_get_last_sound()), "");
    }
    // Leading-underscore keys are discarded without the "is not a field"
    // warning whether or not they precache.
    assert!(!con_log_contains("is not a field"));
}

#[test]
fn parse_edict_traileffect_ignored_on_server_vm_when_not_loading() {
    let _g = lock();
    setup_sv(&[], 20, -1, &[0u8]);
    // SAFETY: as above -- ss_active disarms the second half of C's guard
    // (`Quake/pr_edict.c:879`), so the key falls through to the
    // sky/fog/alpha warning branch instead.
    unsafe { ctest_progs_set_sv_state(1) };

    let (status, _) = parse_edict("\"traileffect\" \"fx_trail\" }", 1);
    assert_eq!(status, PREDD_OK);
    // SAFETY: reading the test double's call counter.
    assert_eq!(unsafe { ctest_predd_get_particle_calls() }, 0);
    assert!(con_log_contains("\"traileffect\" is not a field"));
    assert_eq!(arena_sv().field_f32(EdictId(1), 20 * 4), 0.0);
}

#[test]
fn parse_edict_traileffect_on_server_vm() {
    let _g = lock();
    // word offset 20: kept well inside entvars_t's real field block, which
    // ctest_world_reset sizes the fixture's edicts from.
    setup_sv(&[], 20, -1, &[0u8]);

    let (status, _) = parse_edict("\"traileffect\" \"fx_trail\" }", 1);
    assert_eq!(status, PREDD_OK);
    // SAFETY: as above.
    let last_particle = unsafe { ctest_predd_get_last_particle() };
    assert_eq!(get_string(last_particle), "fx_trail");
    // SAFETY: as above.
    let particle_calls = unsafe { ctest_predd_get_particle_calls() };
    assert_eq!(particle_calls, 1);
    assert_eq!(arena_sv().field_f32(EdictId(1), 20 * 4), 1.0);
}
