//! Differential/characterization gate for `Quake/cl_demo.c` -- demo playback,
//! recording, and the `seek`/`stop`/`record`/`playdemo`/`timedemo` commands.
//! Rust migration Phase 7, M7, task T7.4.
//!
//! The oracle fixture is `stubs/cl_demo_ref.c`; read its module doc first.
//! `cl_demo.c` owns no external-linkage file-scope object, so that file
//! defines no storage twins -- only the thirteen `ClDemo_Glue_*` trampolines,
//! the re-raising command entry points, and the fixtures below.
//!
//! ## Two real files, not one
//!
//! `cl_demo.c` reads AND writes through `cls.demofile`, so the two sides get
//! two independent stdio handles onto identical bytes
//! (`ctest_cldemo_attach_demo`). A shared handle would let one side's file
//! position leak into the other's read and make every position assertion
//! meaningless. `ctest_cldemo_file_pos` is therefore asserted alongside the
//! parsed result: a record that both sides claim to have read must have
//! advanced both handles by the same number of bytes.
//!
//! ## What makes a comparison here non-vacuous
//!
//! With no demo attached, every entry point in this file returns at its first
//! guard and both sides trivially agree. `ctest_cldemo_reset` therefore runs
//! `ctest_clmain_reset` (which owns `cl`/`cls`), attaches a real
//! `net_message` buffer to each side, and puts `cmd_source`, the demo flags,
//! the seek fields and `scr_clock_off` into a defined state; the playback
//! tests then attach a record built by `ctest_cldemo_build_record` and assert
//! on the bytes that landed in `net_message`, on `cl.mviewangles`, and on the
//! file offset -- not merely on "neither side crashed".
//!
//! ## Deliberate divergence that is NOT driven
//!
//! `cl_demo.c:150` documents an accepted `USE_RUST_NET` divergence: the Rust
//! record-header read is atomic where C read the length and each viewangle
//! separately, so a demo truncated 4-15 bytes into a header leaves
//! `cl.mviewangles` updated on the C side and untouched here. The ctest C
//! oracle is compiled WITHOUT `USE_RUST_NET` while `quake-capi` has the `net`
//! feature on, so that divergence is live in this link and a truncated-header
//! fixture would fail by design. Only well-formed records and a clean
//! zero-byte tail are fed. That is a coverage gap, and an intentional one.
//!
//! ## The abort-stub ceiling
//!
//! `Harness_DemoEnded` (`stubs.c`) unconditionally `Sys_Error`s, so every
//! path that reaches the end of a demo ends in `GUARD_SYS_ERROR` on both
//! sides. That is a usable identical-outcome comparison -- it pins down which
//! stub was reached and everything mutated before it -- but it is not proof
//! of what the real engine would do next.
//!
//! ## ADR-005
//!
//! Every format specifier reachable from `cl_demo.c` is `%s`, `%i`, `%d` or
//! `%3.1f`. There is no `%g` and no `%e`, so the Rust float formatter's
//! documented panic is not reachable from this file.

use core::ffi::{c_char, c_double, c_float, c_int, c_long, c_longlong};
use std::cell::RefCell;
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `stubs.c:1465-1467` -- `Host_Guard`'s status codes.
const GUARD_OK: c_int = 0;
const GUARD_SYS_ERROR: c_int = 2;

/// `cmd.h` -- `cmd_source_t`.
const SRC_CLIENT: c_int = 0;
const SRC_COMMAND: c_int = 1;

/// `quakedef.h` -- `SIGNONS`.
const SIGNONS: c_int = 4;

/// The oracle runs first so a shared object it dirties is observed before the
/// port's run resets it.
const SIDES: [c_int; 2] = [1 /* oracle */, 0 /* rust */];

fn side_name(side: c_int) -> &'static str {
    if side == 1 {
        "C"
    } else {
        "Rust"
    }
}

extern "C" {
    // stubs/cl_demo_ref.c
    fn ctest_cldemo_reset();
    fn ctest_cldemo_attach_message();
    fn ctest_cldemo_message_size(side: c_int) -> c_int;
    fn ctest_cldemo_message_data(side: c_int) -> *const u8;
    fn ctest_cldemo_attach_demo(data: *const u8, len: c_int);
    fn ctest_cldemo_release_demo();
    fn ctest_cldemo_file_pos(side: c_int) -> c_long;
    fn ctest_cldemo_set_cmd_source(src: c_int);
    fn ctest_cldemo_set_demo_state(
        demoplayback: c_int,
        demorecording: c_int,
        demopaused: c_int,
        demoseeking: c_int,
        signon: c_int,
    );
    fn ctest_cldemo_set_seek_fields(demospeed: c_float, prespawn_end: c_int, seektime: c_float);
    fn ctest_cldemo_get_prespawn_end(side: c_int) -> c_longlong;
    fn ctest_cldemo_get_scr_clock_off() -> c_float;
    fn ctest_cldemo_set_scr_clock_off(v: c_float);
    fn ctest_cldemo_get_mviewangles(side: c_int, out: *mut c_float);
    fn ctest_cldemo_set_viewangles(x: c_float, y: c_float, z: c_float);
    fn ctest_cldemo_build_record(
        out: *mut u8,
        payload: *const u8,
        len: c_int,
        a0: c_float,
        a1: c_float,
        a2: c_float,
    ) -> c_int;

    // stubs/cl_demo_ref.c -- drivers (all enter through Host_Guard)
    fn ctest_cldemo_stop_playback(side: c_int) -> c_int;
    fn ctest_cldemo_get_message(side: c_int, out: *mut c_int) -> c_int;
    fn ctest_cldemo_seek(side: c_int) -> c_int;
    fn ctest_cldemo_stop(side: c_int) -> c_int;
    fn ctest_cldemo_record(side: c_int) -> c_int;
    fn ctest_cldemo_play(side: c_int) -> c_int;
    fn ctest_cldemo_timedemo(side: c_int) -> c_int;
    fn ctest_cldemo_resume_record(side: c_int, recordsignons: c_int) -> c_int;

    // stubs/cl_main_ref.c -- cl/cls images and the time seeder
    fn ctest_clmain_cl_image_size() -> c_int;
    fn ctest_clmain_get_cl_image(side: c_int, out: *mut u8);
    fn ctest_clmain_cls_image_size() -> c_int;
    fn ctest_clmain_get_cls_image(side: c_int, out: *mut u8);
    fn ctest_clmain_set_time(time: c_double, oldtime: c_double, mtime0: c_double, mtime1: c_double);

    // stubs/cl_input_ref.c -- tokenizes BOTH command tables in one call
    fn ctest_clinput_tokenize(text: *const c_char);

    // stubs.c
    fn ctest_sys_error_message() -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;
    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
}

fn guard_message(status: c_int) -> String {
    // SAFETY: both getters return a pointer to a static NUL-terminated buffer
    // in stubs.c with process lifetime; callers hold TEST_LOCK.
    unsafe {
        let p = match status {
            1 => ctest_host_error_message(),
            GUARD_SYS_ERROR => ctest_sys_error_message(),
            _ => return String::new(),
        };
        core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

fn con_log() -> Vec<String> {
    // SAFETY: stubs.c getters over static storage; caller holds TEST_LOCK.
    unsafe {
        let n = ctest_con_log_len().clamp(0, 64);
        (0..n)
            .map(|i| {
                core::ffi::CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

fn tokenize(text: &str) {
    let c = CString::new(text).unwrap();
    // SAFETY: NUL-terminated; the fixture tokenizes both command tables and
    // copies the text. Caller holds TEST_LOCK.
    unsafe { ctest_clinput_tokenize(c.as_ptr()) }
}

/// Everything observable after one driver call on one side.
#[derive(Debug, PartialEq)]
struct Snap {
    guard: c_int,
    msg: String,
    ret: c_int,
    cl: Vec<u8>,
    cls: Vec<u8>,
    net_message: Vec<u8>,
    mviewangles: [u32; 6],
    file_pos: c_long,
    clock_off: u32,
    con: Vec<String>,
}

fn snap(side: c_int, guard: c_int, ret: c_int) -> Snap {
    // SAFETY: fixture read-backs over static storage sized by the paired
    // *_size() getters; caller holds TEST_LOCK.
    unsafe {
        let mut cl = vec![0u8; ctest_clmain_cl_image_size() as usize];
        ctest_clmain_get_cl_image(side, cl.as_mut_ptr());
        let mut cls = vec![0u8; ctest_clmain_cls_image_size() as usize];
        ctest_clmain_get_cls_image(side, cls.as_mut_ptr());
        let n = ctest_cldemo_message_size(side).max(0) as usize;
        let net_message = core::slice::from_raw_parts(ctest_cldemo_message_data(side), n).to_vec();
        let mut ang = [0f32; 6];
        ctest_cldemo_get_mviewangles(side, ang.as_mut_ptr());
        Snap {
            guard,
            msg: guard_message(guard),
            ret,
            cl,
            cls,
            net_message,
            mviewangles: ang.map(f32::to_bits),
            file_pos: ctest_cldemo_file_pos(side),
            clock_off: ctest_cldemo_get_scr_clock_off().to_bits(),
            con: con_log(),
        }
    }
}

fn assert_same(c: &Snap, rust: &Snap, what: &str) {
    assert_eq!(
        (c.guard, &c.msg),
        (rust.guard, &rust.msg),
        "{what}: guard status/message differ ({} vs {})",
        side_name(1),
        side_name(0)
    );
    assert_eq!(c.ret, rust.ret, "{what}: return value differs");
    assert_eq!(c.con, rust.con, "{what}: console output differs");
    assert_eq!(
        c.net_message, rust.net_message,
        "{what}: net_message differs"
    );
    assert_eq!(
        c.mviewangles, rust.mviewangles,
        "{what}: cl.mviewangles differ"
    );
    assert_eq!(c.file_pos, rust.file_pos, "{what}: demofile offset differs");
    assert_eq!(c.clock_off, rust.clock_off, "{what}: scr_clock_off differs");
    assert_eq!(c.cls, rust.cls, "{what}: client_static_t image differs");
    assert_eq!(c.cl, rust.cl, "{what}: client_state_t image differs");
}

/// Runs `seed` then `drive` on each side in turn, with a full fixture reset
/// and a cleared console log before each, and returns `(oracle, rust)`.
///
/// `drive` returns `(guard_status, return_value)`; entry points that return
/// `void` report `0`.
fn run_both<S, D>(seed: S, drive: D) -> (Snap, Snap)
where
    S: Fn(),
    D: Fn(c_int) -> (c_int, c_int),
{
    let mut out: Vec<Snap> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture reset over static storage; caller holds TEST_LOCK.
        unsafe {
            ctest_cldemo_reset();
            ctest_clear_con_log();
        }
        seed();
        let (g, r) = drive(side);
        out.push(snap(side, g, r));
    }
    // SAFETY: drops both temp files before the next test attaches new ones.
    unsafe { ctest_cldemo_release_demo() };
    let rust = out.pop().unwrap();
    let c = out.pop().unwrap();
    (c, rust)
}

/// One well-formed demo record: 4-byte LE length, three LE float viewangles,
/// then the payload.
fn record(payload: &[u8], a: [f32; 3]) -> Vec<u8> {
    let mut buf = vec![0u8; 16 + payload.len()];
    // SAFETY: `buf` is 16 + len bytes, exactly what the fixture writes.
    let n = unsafe {
        ctest_cldemo_build_record(
            buf.as_mut_ptr(),
            payload.as_ptr(),
            payload.len() as c_int,
            a[0],
            a[1],
            a[2],
        )
    };
    assert_eq!(n as usize, buf.len());
    buf
}

// ---------------------------------------------------------------------------
// CL_StopPlayback (cl_demo.c:58-76)

#[test]
fn stop_playback_is_a_no_op_when_not_playing() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(0, 0, 0, 0, SIGNONS) }
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_stop_playback(side) }, 0),
    );
    assert_same(&c, &rust, "CL_StopPlayback(not playing)");
    assert_eq!(c.guard, GUARD_OK, "the early return must not raise");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

#[test]
fn stop_playback_closes_the_demo_and_reaches_the_harness_hook() {
    let _g = lock();
    let bytes = record(&[1, 2, 3, 4], [0.0, 0.0, 0.0]);
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 1, 1, SIGNONS);
            }
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_stop_playback(side) }, 0),
    );
    assert_same(&c, &rust, "CL_StopPlayback(playing)");
    assert_eq!(
        c.guard, GUARD_SYS_ERROR,
        "Harness_DemoEnded is an unconditional Sys_Error stub"
    );
    assert!(
        c.msg.contains("Harness_DemoEnded"),
        "unexpected abort: {}",
        c.msg
    );
    // Positive assertion: the flags were cleared BEFORE the hook fired.
    assert_ne!(
        c.cls,
        vec![0u8; c.cls.len()],
        "the cls image must not be all-zero"
    );
}

// ---------------------------------------------------------------------------
// CL_GetMessage / CL_GetDemoMessage (cl_demo.c:110-205, 214)

#[test]
fn get_message_reads_one_well_formed_record() {
    let _g = lock();
    let payload: Vec<u8> = (0u8..48).collect();
    let bytes = record(&payload, [10.5, -20.25, 30.125]);
    let expect_pos = bytes.len() as c_long;
    let (c, rust) = run_both(
        || {
            // cl.time must be past cl.mtime[0] or CL_GetDemoMessage decides it
            // does not need another message yet and returns 0 -- the exact
            // degenerate pass this test exists to avoid.
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(2.0, 1.9, 1.0, 0.9);
                ctest_cldemo_set_viewangles(0.0, 0.0, 0.0);
                ctest_cldemo_attach_message();
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
            }
        },
        |side| {
            let mut out: c_int = -9;
            // SAFETY: fixture driver.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            (g, out)
        },
    );
    assert_same(&c, &rust, "CL_GetMessage(one record)");
    assert_eq!(c.guard, GUARD_OK, "{}", c.msg);
    assert_eq!(c.ret, 1, "a complete record must report success");
    assert_eq!(c.net_message, payload, "payload bytes");
    assert_eq!(
        [
            f32::from_bits(c.mviewangles[0]),
            f32::from_bits(c.mviewangles[1]),
            f32::from_bits(c.mviewangles[2])
        ],
        [10.5f32, -20.25, 30.125],
        "cl.mviewangles[0] must come from the record header"
    );
    assert_eq!(c.file_pos, expect_pos, "the whole record must be consumed");
}

#[test]
fn get_message_rotates_mviewangles_across_two_records() {
    let _g = lock();
    // The `VectorCopy (cl.mviewangles[0], cl.mviewangles[1])` in
    // CL_GetDemoMessage is only observable once a SECOND record lands.
    let mut bytes = record(&[7u8; 8], [1.0, 2.0, 3.0]);
    bytes.extend_from_slice(&record(&[9u8; 8], [4.0, 5.0, 6.0]));
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(2.0, 1.9, 1.0, 0.9);
                ctest_cldemo_set_viewangles(0.0, 0.0, 0.0);
                ctest_cldemo_attach_message();
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
            }
        },
        |side| {
            let mut out: c_int = -9;
            // SAFETY: fixture driver, run twice on the same handle.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            assert_eq!(g, GUARD_OK, "{}: first record", side_name(side));
            assert_eq!(out, 1, "{}: first record", side_name(side));
            // SAFETY: fixture driver.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            (g, out)
        },
    );
    assert_same(&c, &rust, "CL_GetMessage(two records)");
    assert_eq!(c.ret, 1);
    let ang = c.mviewangles.map(f32::from_bits);
    assert_eq!(&ang[0..3], &[4.0f32, 5.0, 6.0], "mviewangles[0] = newest");
    assert_eq!(&ang[3..6], &[1.0f32, 2.0, 3.0], "mviewangles[1] = previous");
}

#[test]
fn get_message_returns_zero_while_paused() {
    let _g = lock();
    let bytes = record(&[1, 2, 3, 4], [1.0, 2.0, 3.0]);
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(2.0, 1.9, 1.0, 0.9);
                ctest_cldemo_attach_message();
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 1, 0, SIGNONS);
            }
        },
        |side| {
            let mut out: c_int = -9;
            // SAFETY: fixture driver.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            (g, out)
        },
    );
    assert_same(&c, &rust, "CL_GetMessage(paused)");
    assert_eq!(c.ret, 0, "demopaused short-circuits");
    assert_eq!(c.file_pos, 0, "and reads nothing");
}

#[test]
fn get_message_returns_zero_when_no_message_is_due_yet() {
    let _g = lock();
    let bytes = record(&[1, 2, 3, 4], [1.0, 2.0, 3.0]);
    let (c, rust) = run_both(
        || {
            // cl.time <= cl.mtime[0] -> cl_demo.c:146 returns 0.
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(1.0, 0.9, 2.0, 1.9);
                ctest_cldemo_attach_message();
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
            }
        },
        |side| {
            let mut out: c_int = -9;
            // SAFETY: fixture driver.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            (g, out)
        },
    );
    assert_same(&c, &rust, "CL_GetMessage(not due)");
    assert_eq!(c.ret, 0);
    assert_eq!(c.file_pos, 0, "nothing may be consumed");
}

#[test]
fn get_message_records_the_prespawn_offset_at_signon_two() {
    let _g = lock();
    // cls.signon == SIGNONS - 2 takes the arm that stamps
    // cls.demo_prespawn_end and then always reads.
    let bytes = record(&[5u8; 12], [0.5, 1.5, 2.5]);
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_cldemo_attach_message();
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS - 2);
            }
        },
        |side| {
            let mut out: c_int = -9;
            // SAFETY: fixture driver.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            (g, out)
        },
    );
    assert_same(&c, &rust, "CL_GetMessage(signon 2)");
    assert_eq!(c.ret, 1, "this arm always grabs a message");
    assert_eq!(c.file_pos, bytes.len() as c_long);
}

#[test]
fn get_message_stamps_a_nonzero_prespawn_offset_on_the_second_read() {
    let _g = lock();
    // The single-read case above cannot see the stamp at all: the demo file is
    // still at offset 0 when cl_demo.c:123 runs, so `Sys_ftell` returns 0 over
    // a field that was already 0 and dropping the assignment entirely is
    // invisible. Two records and two reads put the second stamp at the end of
    // the first record instead.
    let mut bytes = record(&[5u8; 12], [0.5, 1.5, 2.5]);
    let first_len = bytes.len() as c_longlong;
    bytes.extend_from_slice(&record(&[7u8; 8], [3.5, 4.5, 5.5]));
    let stamps: RefCell<Vec<c_longlong>> = RefCell::new(Vec::new());
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_cldemo_attach_message();
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS - 2);
            }
        },
        |side| {
            let mut out: c_int = -9;
            // SAFETY: fixture driver; two reads in one drive so the second
            // stamp is taken at a non-zero file offset.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            assert_eq!(g, GUARD_OK, "{}: first read", side_name(side));
            // SAFETY: fixture driver.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            // `run_both` resets the fixture before each side, so the stamp has
            // to be read here rather than after the loop.
            // SAFETY: fixture read-back over static storage.
            stamps
                .borrow_mut()
                .push(unsafe { ctest_cldemo_get_prespawn_end(side) });
            (g, out)
        },
    );
    assert_same(&c, &rust, "CL_GetMessage(signon 2, second read)");
    assert_eq!(c.ret, 1, "this arm always grabs a message");
    assert_eq!(c.file_pos, bytes.len() as c_long, "both records consumed");
    assert_eq!(
        stamps.into_inner(),
        vec![first_len, first_len],
        "both sides must stamp the offset just past the first record"
    );
}

#[test]
fn get_message_at_the_end_of_the_demo_stops_playback() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // An empty file: the very first header read hits EOF, which is the
            // clean tail case, not the documented truncated-header divergence.
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(2.0, 1.9, 1.0, 0.9);
                ctest_cldemo_attach_message();
                ctest_cldemo_attach_demo(core::ptr::null(), 0);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
            }
        },
        |side| {
            let mut out: c_int = -9;
            // SAFETY: fixture driver.
            let g = unsafe { ctest_cldemo_get_message(side, &mut out) };
            (g, out)
        },
    );
    assert_same(&c, &rust, "CL_GetMessage(eof)");
    assert_eq!(
        c.guard, GUARD_SYS_ERROR,
        "EOF -> CL_StopPlayback -> Harness_DemoEnded"
    );
    assert!(
        c.msg.contains("Harness_DemoEnded"),
        "unexpected abort: {}",
        c.msg
    );
}

// ---------------------------------------------------------------------------
// CL_Seek_f (cl_demo.c:214-290)

#[test]
fn seek_ignores_a_non_command_source() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_cldemo_set_cmd_source(SRC_CLIENT);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
            }
            tokenize("seek 10");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_seek(side) }, 0),
    );
    // SAFETY: restore the shared cmd_source for the next test.
    unsafe { ctest_cldemo_set_cmd_source(SRC_COMMAND) };
    assert_same(&c, &rust, "CL_Seek_f(src_client)");
    assert_eq!(c.guard, GUARD_OK);
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

#[test]
fn seek_prints_usage_on_the_wrong_argument_count() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS) }
            tokenize("seek");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_seek(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Seek_f(argc 1)");
    assert!(
        c.con.iter().any(|l| l.contains("relative] seek in demo")),
        "expected the usage line: {:?}",
        c.con
    );
}

#[test]
fn seek_requires_an_active_playback() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(0, 0, 0, 0, SIGNONS) }
            tokenize("seek 10");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_seek(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Seek_f(not playing)");
    assert!(
        c.con.iter().any(|l| l.contains("Not playing a demo")),
        "expected the not-playing line: {:?}",
        c.con
    );
}

#[test]
fn seek_rejects_an_unparseable_time() {
    let _g = lock();
    let bytes = record(&[1, 2, 3, 4], [0.0, 0.0, 0.0]);
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
            }
            tokenize("seek zzz");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_seek(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Seek_f(bad format)");
    assert!(
        c.con.iter().any(|l| l.contains("Expected time format")),
        "expected the format-error line: {:?}",
        c.con
    );
    assert_eq!(
        f32::from_bits(c.clock_off),
        0.0,
        "the rejected path must not arm the clock"
    );
}

#[test]
fn seek_absolute_sets_cl_time_and_arms_the_clock() {
    let _g = lock();
    let bytes = record(&[1, 2, 3, 4], [0.0, 0.0, 0.0]);
    let (c, rust) = run_both(
        || {
            // demo_prespawn_end == 0 forces the `else cl.time = cls.seektime`
            // arm, which avoids the Sys_fseek/effects-reset path.
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(1.0, 0.9, 1.0, 0.9);
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 1, 0, SIGNONS);
                ctest_cldemo_set_seek_fields(0.0, 0, 0.0);
                ctest_cldemo_set_scr_clock_off(0.0);
            }
            tokenize("seek 12");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_seek(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Seek_f(absolute)");
    assert_eq!(c.guard, GUARD_OK, "{}", c.msg);
    assert_eq!(
        f32::from_bits(c.clock_off),
        2.5,
        "cl_demo.c:289 sets scr_clock_off = 2.5f"
    );
}

#[test]
fn seek_relative_offset_is_added_to_cl_time() {
    let _g = lock();
    let bytes = record(&[1, 2, 3, 4], [0.0, 0.0, 0.0]);
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(5.0, 4.9, 5.0, 4.9);
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
                ctest_cldemo_set_seek_fields(0.0, 0, 0.0);
                ctest_cldemo_set_scr_clock_off(0.0);
            }
            tokenize("seek +3");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_seek(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Seek_f(relative)");
    assert_eq!(c.guard, GUARD_OK, "{}", c.msg);
    assert_eq!(f32::from_bits(c.clock_off), 2.5);
}

#[test]
fn seek_mm_ss_form_is_parsed_as_minutes_and_seconds() {
    let _g = lock();
    let bytes = record(&[1, 2, 3, 4], [0.0, 0.0, 0.0]);
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_time(1.0, 0.9, 1.0, 0.9);
                ctest_cldemo_attach_demo(bytes.as_ptr(), bytes.len() as c_int);
                ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS);
                ctest_cldemo_set_seek_fields(0.0, 0, 0.0);
                ctest_cldemo_set_scr_clock_off(0.0);
            }
            tokenize("seek 2:30");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_seek(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Seek_f(mm:ss)");
    assert_eq!(c.guard, GUARD_OK, "{}", c.msg);
    assert_eq!(f32::from_bits(c.clock_off), 2.5);
}

// ---------------------------------------------------------------------------
// CL_Stop_f (cl_demo.c:327-352)

#[test]
fn stop_f_ignores_a_non_command_source() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_cldemo_set_cmd_source(SRC_CLIENT);
                ctest_cldemo_set_demo_state(0, 1, 0, 0, SIGNONS);
            }
            tokenize("stop");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_stop(side) }, 0),
    );
    // SAFETY: restore the shared cmd_source for the next test.
    unsafe { ctest_cldemo_set_cmd_source(SRC_COMMAND) };
    assert_same(&c, &rust, "CL_Stop_f(src_client)");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

#[test]
fn stop_f_reports_when_not_recording() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(0, 0, 0, 0, SIGNONS) }
            tokenize("stop");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_stop(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Stop_f(not recording)");
    assert!(
        c.con.iter().any(|l| l.contains("Not recording a demo")),
        "expected the not-recording line: {:?}",
        c.con
    );
}

// ---------------------------------------------------------------------------
// CL_Record_f (cl_demo.c:609-...)

#[test]
fn record_f_refuses_during_playback() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(1, 0, 0, 0, SIGNONS) }
            tokenize("record foo");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_record(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Record_f(during playback)");
    assert!(
        c.con
            .iter()
            .any(|l| l.contains("Can't record during demo playback")),
        "expected the playback refusal: {:?}",
        c.con
    );
}

#[test]
fn record_f_prints_usage_on_a_bad_argument_count() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(0, 0, 0, 0, SIGNONS) }
            tokenize("record a b c d e");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_record(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Record_f(argc 6)");
    assert!(
        c.con.iter().any(|l| l.contains("record <demoname>")),
        "expected the usage line: {:?}",
        c.con
    );
}

#[test]
fn record_f_rejects_relative_pathnames() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(0, 0, 0, 0, SIGNONS) }
            tokenize("record ../escape");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_record(side) }, 0),
    );
    assert_same(&c, &rust, "CL_Record_f(..)");
    assert!(
        c.con
            .iter()
            .any(|l| l.contains("Relative pathnames are not allowed")),
        "expected the path refusal: {:?}",
        c.con
    );
}

#[test]
fn record_f_ignores_a_non_command_source() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_cldemo_set_cmd_source(SRC_CLIENT);
                ctest_cldemo_set_demo_state(0, 0, 0, 0, SIGNONS);
            }
            tokenize("record ../escape");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_record(side) }, 0),
    );
    // SAFETY: restore the shared cmd_source for the next test.
    unsafe { ctest_cldemo_set_cmd_source(SRC_COMMAND) };
    assert_same(&c, &rust, "CL_Record_f(src_client)");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

// ---------------------------------------------------------------------------
// CL_PlayDemo_f / CL_TimeDemo_f (cl_demo.c:753, 850)

#[test]
fn play_demo_f_prints_usage_on_a_bad_argument_count() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            tokenize("playdemo");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_play(side) }, 0),
    );
    assert_same(&c, &rust, "CL_PlayDemo_f(argc 1)");
    assert!(
        c.con.iter().any(|l| l.contains("playdemo <demoname>")),
        "expected the usage line: {:?}",
        c.con
    );
}

#[test]
fn play_demo_f_ignores_a_non_command_source() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_cmd_source(SRC_CLIENT) }
            tokenize("playdemo");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_play(side) }, 0),
    );
    // SAFETY: restore the shared cmd_source for the next test.
    unsafe { ctest_cldemo_set_cmd_source(SRC_COMMAND) };
    assert_same(&c, &rust, "CL_PlayDemo_f(src_client)");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

#[test]
fn timedemo_f_prints_usage_on_a_bad_argument_count() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            tokenize("timedemo");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_timedemo(side) }, 0),
    );
    assert_same(&c, &rust, "CL_TimeDemo_f(argc 1)");
    assert!(
        c.con.iter().any(|l| l.contains("timedemo <demoname>")),
        "expected the usage line: {:?}",
        c.con
    );
}

#[test]
fn timedemo_f_ignores_a_non_command_source() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_cmd_source(SRC_CLIENT) }
            tokenize("timedemo");
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_timedemo(side) }, 0),
    );
    // SAFETY: restore the shared cmd_source for the next test.
    unsafe { ctest_cldemo_set_cmd_source(SRC_COMMAND) };
    assert_same(&c, &rust, "CL_TimeDemo_f(src_client)");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

// ---------------------------------------------------------------------------
// CL_Resume_Record (cl_demo.c:726-744)

#[test]
fn resume_record_reports_a_missing_file() {
    let _g = lock();
    // cl_demo.c's `name` is file-static, so each side has its own empty copy
    // after reset; Sys_fopen("") fails identically on both.
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_cldemo_set_demo_state(0, 0, 0, 0, SIGNONS) }
        },
        // SAFETY: fixture driver.
        |side| (unsafe { ctest_cldemo_resume_record(side, 0) }, 0),
    );
    assert_same(&c, &rust, "CL_Resume_Record(missing file)");
    assert!(
        c.con.iter().any(|l| l.contains("recording stopped")),
        "expected the append-failure line: {:?}",
        c.con
    );
}
